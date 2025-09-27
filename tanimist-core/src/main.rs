use std::sync::{
    atomic::{AtomicBool, AtomicU64},
    Arc,
};

use clap::Parser;
use tanimist_cli::video::TypstVideoRenderer;
use tinymist_world::args::CompileOnceArgs;
use tracing::info;
use tracing_indicatif::IndicatifLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use typst::foundations::{Dict, Str, Value};

#[derive(Debug, Clone, Parser, Default)]
pub struct Args {
    #[clap(flatten)]
    pub compile_once: CompileOnceArgs,
    #[clap(long, default_value = "t")]
    pub variable: String,
    #[clap(long, short, default_value = "out.mp4")]
    pub output: String,
    #[clap(flatten)]
    pub encoder: EncoderArgs,
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
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(indicatif_layer.get_stderr_writer()))
        .with(indicatif_layer)
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tanimist_core=info,video_rs=warn,ffmpeg=error".into()),
        )
        .init();
    let args = Args::parse();
    let univ = match args.compile_once.resolve_system() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Error resolving system: {e}");
            return Err(e.into());
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
    
    let renderer = TypstVideoRenderer::new(
        300.0,
        move |t| Dict::from_iter([(Str::from(args.variable.clone()), Value::Int(t.into()))]),
        univ,
        encoder_option_hashmap,
    );

    let total_frames = 240;

    let render_progress = Arc::new(AtomicU64::new(0));
    let encode_progress = Arc::new(AtomicU64::new(0));
    let error_signal = Arc::new(AtomicBool::new(false));

    let render_thread = std::thread::Builder::new()
        .name("render".to_string())
        .spawn(move || {
            renderer.render(
                0,
                total_frames as i32,
                24,
                Some(render_progress),
                Some(encode_progress),
                error_signal,
            )
        })
        .unwrap();

    let data = match render_thread.join().unwrap() {
        Ok(data) => data,
        Err(e) => {
            return Err(e.into());
        }
    };
    std::fs::write(&args.output, data)?;
    info!("Finished in {:?}", start.elapsed());
    info!("Wrote output to {}", args.output);
    Ok(())
}
