pub mod config;
pub mod error;
pub mod merger;
pub mod renderer;
#[cfg(feature = "debug")]
pub mod debug;

pub use renderer::TypstVideoRenderer;