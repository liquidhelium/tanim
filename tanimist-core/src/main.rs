use clap::Parser;
use tanimist_core::video::TypstVideoRenderer;
use tinymist_world::args::CompileOnceArgs;
use typst::foundations::{Dict, Str, Value};


#[derive(Debug, Clone, Parser, Default)]
pub struct Args {
    #[clap(flatten)]
    pub compile_once: CompileOnceArgs,
    #[clap(long, default_value = "t")]
    pub variable: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let univ = match args.compile_once.resolve_system() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Error resolving system: {e}");
            return Err(e.into());
        }
    };
    let renderer = TypstVideoRenderer::new(
        300.0,
        move |t| Dict::from_iter([(Str::from(args.variable.clone()), Value::Int(t.into()))]),
        univ,
    );
    let (tx, rx) = crossbeam::channel::unbounded();
    let render_thread = tokio::task::spawn_blocking(move || renderer.render(0, 240, 24, Some(tx)));

    while let Ok(t) = rx.recv() {
        println!("frame {t}");
    }

    let data = render_thread.await??;
    std::fs::write("out.mp4", data)?;
    Ok(())
}
