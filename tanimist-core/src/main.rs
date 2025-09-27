use clap::Parser;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tanimist_core::video::TypstVideoRenderer;
use tinymist_world::args::CompileOnceArgs;
use tracing::info;
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
    #[clap(long, default_value = "nvenc_hevc")]
    pub codec: String,
    /// Constant Rate Factor (CRF) for quality control (lower is better quality, range 0-51)
    #[clap(long, default_value = "23")]
    pub crf: u8,
    #[clap(long, default_value = "medium")]
    pub preset: String,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_env_filter("tanimist_core=info,video_rs=warn,ffmpeg=error")
        .with_thread_ids(true)
        .with_thread_names(true)
        .compact()
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
        map.insert("crf".to_string(), args.encoder.crf.to_string());
        map.insert("preset".to_string(), args.encoder.preset.clone());
        map
    };
    
    let renderer = TypstVideoRenderer::new(
        300.0,
        move |t| Dict::from_iter([(Str::from(args.variable.clone()), Value::Int(t.into()))]),
        univ,
        encoder_option_hashmap,
    );

    let (render_tx, render_rx) = crossbeam::channel::unbounded();
    let (encode_tx, encode_rx) = crossbeam::channel::unbounded();

    let total_frames = 240;

    let m = MultiProgress::new();
    let sty = ProgressStyle::with_template(
        "[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}",
    )
    .unwrap()
    .progress_chars("##-");

    let pb_render = m.add(ProgressBar::new(total_frames));
    pb_render.set_style(sty.clone());
    pb_render.set_message("rendering");

    let pb_encode = m.add(ProgressBar::new(total_frames));
    pb_encode.set_style(sty);
    pb_encode.set_message("encoding");

    let progress_thread = std::thread::spawn(move || {
        let mut render_done = false;
        let mut encode_done = false;
        while !render_done || !encode_done {
            crossbeam::channel::select! {
                recv(render_rx) -> _ => {
                    pb_render.inc(1);
                    if pb_render.position() == total_frames {
                        render_done = true;
                        pb_render.finish_with_message("rendered");
                    }
                },
                recv(encode_rx) -> _ => {
                    pb_encode.inc(1);
                    if pb_encode.position() == total_frames {
                        encode_done = true;
                        pb_encode.finish_with_message("encoded");
                    }
                }
            }
        }
    });

    let render_thread = std::thread::spawn(move || {
        renderer.render(0, total_frames as i32, 24, Some(render_tx), Some(encode_tx))
    });

    let data = render_thread.join().unwrap()?;
    m.clear()?;
    progress_thread.join().unwrap();
    std::fs::write(&args.output, data)?;
    info!("Wrote output to {}", args.output);
    Ok(())
}
