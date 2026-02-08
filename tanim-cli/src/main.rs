use std::sync::{Arc, Mutex, atomic::AtomicBool};

use clap::{Parser, builder::ValueParser};
use tanim_cli::video::{TypstVideoRenderer, config::RenderConfig};
#[cfg(feature = "typst-lib")]
use tinymist_world::{args::CompileOnceArgs, print_diagnostics};
use tracing::{error, info};
use tracing_indicatif::IndicatifLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
#[cfg(feature = "typst-lib")]
use typst::foundations::{Dict, Str, Value};

#[derive(Debug, Clone, Parser)]
pub struct Args {
    #[cfg(feature = "typst-lib")]
    #[clap(flatten)]
    pub compile_once: CompileOnceArgs,
    #[cfg(not(feature = "typst-lib"))]
    pub input: String,
    #[cfg(feature = "typst-bin")]
    #[clap(long)]
    pub typst_command: Option<String>,
    #[clap(long, default_value = "t")]
    pub variable: String,
    #[clap(long, short, default_value = "out.mp4")]
    pub output: String,
    #[clap(long, short, default_value = "0..=240", value_parser = ValueParser::new(parse_range))]
    pub frames: std::ops::RangeInclusive<i32>,
    #[clap(long, default_value = "24", value_parser = clap::value_parser!(u32).range(1..))]
    pub fps: u32,
    #[clap(long, default_value = "150.0")]
    pub ppi: f32,
    /// Number of rendering threads to use (default: number of CPUs - 4, minimum 1)
    #[clap(long = "rthreads")]
    pub rendering_threads: Option<usize>,
    /// Number of encoding threads will be number of CPUs - rendering threads (minimum 1)
    #[clap(long = "ethreads")]
    pub encoding_threads: Option<usize>,
    /// Encoder options
    #[clap(flatten)]
    pub encoder: EncoderArgs,
    /// Zstd compression level for frames before sending to encoder
    #[clap(long)]
    pub zstd_level: Option<i32>,
    #[cfg(feature = "ffmpeg-bin")]
    /// Path to ffmpeg binary
    #[clap(long)]
    pub ffmpeg_path: Option<String>,
}

fn parse_range(s: &str) -> Result<std::ops::RangeInclusive<i32>, String> {
    let parts: Vec<&str> = s.split("..=").collect();
    if parts.len() != 2 {
        return Err("Range must be in the format start..=end".to_string());
    }
    let start: i32 = parts[0]
        .parse()
        .map_err(|_| "Invalid start of range".to_string())?;
    let end: i32 = parts[1]
        .parse()
        .map_err(|_| "Invalid end of range".to_string())?;
    if start > end {
        return Err("Start of range must be less than or equal to end".to_string());
    }
    Ok(start..=end)
}

#[derive(Debug, Clone, Parser, Default)]
pub struct EncoderArgs {
    #[clap(long, default_value = "libx264")]
    pub codec: String,
    /// Constant Rate Factor (CRF) for quality control (lower is better quality, range 0-51)
    #[clap(long)]
    pub crf: Option<u8>,
    #[clap(long, default_value = "medium")]
    pub preset: String,
}

fn main() -> anyhow::Result<()> {
    let start = std::time::Instant::now();
    let indicatif_layer = IndicatifLayer::new();
    let re = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(indicatif_layer.get_stderr_writer()))
        .with(indicatif_layer);
    #[cfg(feature = "tracy")]
    let re = {
        re.with(tracing_tracy::TracyLayer::new(
            tracing_tracy::DefaultConfig::default(),
        ))
    };
    re.with(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "tanim_cli=info,video=warn,ffmpeg=error".into()),
    )
    .init();
    let args = Args::parse();

    #[cfg(feature = "typst-lib")]
    let univ = match args.compile_once.resolve_system() {
        Ok(u) => u,
        Err(e) => {
            error!("Error resolving system: {e}");
            return Err(e.into());
        }
    };
    #[cfg(feature = "typst-lib")]
    let world = univ.snapshot();

    #[cfg(all(feature = "typst-lib", feature = "typst-bin"))]
    let use_binary = args.typst_command.is_some();

    #[cfg(all(not(feature = "typst-lib"), feature = "typst-bin"))]
    let use_binary = true;
    #[cfg(all(not(feature = "typst-lib"), feature = "typst-bin"))]
    let typst_command = args
        .typst_command
        .clone()
        .unwrap_or_else(|| "typst".to_string());

    #[cfg(all(feature = "typst-lib", not(feature = "typst-bin")))]
    let use_binary = false;

    // Resolve input path for binary mode
    #[cfg(feature = "typst-bin")]
    let input_path = {
        #[cfg(feature = "typst-lib")]
        {
            args.compile_once
                .input
                .as_ref()
                .map(|p| p.to_string())
                .unwrap_or_else(|| ".".to_string())
        }
        #[cfg(not(feature = "typst-lib"))]
        {
            args.input.clone()
        }
    };

    let encoder_option_hashmap = {
        let mut map = std::collections::HashMap::new();
        map.insert("codec".to_string(), args.encoder.codec.clone());
        if let Some(crf) = args.encoder.crf {
            map.insert("crf".to_string(), crf.to_string());
        }
        map.insert("preset".to_string(), args.encoder.preset.clone());
        map
    };

    let variable = args.variable.clone();

    let config = RenderConfig {
        #[cfg(all(feature = "typst-lib", not(feature = "typst-bin")))]
        universe: Arc::new(Mutex::new(univ)),
        #[cfg(all(feature = "typst-lib", feature = "typst-bin"))]
        universe: if !use_binary {
            Some(Arc::new(Mutex::new(univ)))
        } else {
            None
        },

        ppi: args.ppi,

        f_input: Box::new(move |t| {
            Dict::from_iter([(Str::from(variable.clone()), Value::Int(t.into()))])
        }),
        #[cfg(feature = "typst-bin")]
        typst_command: if use_binary {
            #[cfg(feature = "typst-lib")]
            {
                args.typst_command
            }
            #[cfg(not(feature = "typst-lib"))]
            {
                Some(typst_command)
            }
        } else {
            None
        },
        #[cfg(feature = "typst-bin")]
        input_path: if use_binary {
            input_path
        } else {
            String::new()
        },
        #[cfg(feature = "typst-bin")]
        variable: args.variable.clone(),

        ffmpeg_options: encoder_option_hashmap,
        begin_t: *args.frames.start(),
        end_t: *args.frames.end(),
        fps: args.fps,
        rendering_threads: args.rendering_threads,
        encoding_threads: args.encoding_threads,
        zstd_level: args.zstd_level,
        ffmpeg_path: args.ffmpeg_path,
    };

    let renderer = TypstVideoRenderer::new(config);

    let error_signal = Arc::new(AtomicBool::new(false));

    let render_thread = std::thread::Builder::new()
        .name("render".to_string())
        .spawn(move || renderer.render(error_signal))
        .unwrap();

    let data = match render_thread.join().unwrap() {
        Ok(data) => data,
        Err(e) => {
            #[cfg(feature = "typst-lib")]
            if let tanim_cli::video::error::Error::TypstCompilation(diags) = &e {
                if let Err(e) = print_diagnostics(
                    &world,
                    diags.iter(),
                    tinymist_world::DiagnosticFormat::Human,
                ) {
                    error!("Error printing diagnostics: {e}");
                }
            } else {
                error!("Error during rendering: {e}");
            }
            #[cfg(not(feature = "typst-lib"))]
            error!("Error during rendering: {e}");
            return Err(e.into());
        }
    };
    std::fs::write(&args.output, data)?;
    info!("Finished in {:?}", start.elapsed());
    info!("Wrote output to {}", args.output);
    Ok(())
}
