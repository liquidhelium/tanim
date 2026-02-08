use std::collections::HashMap;
#[cfg(feature = "typst-lib")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "typst-lib")]
use tinymist_world::TypstSystemUniverse;
#[cfg(feature = "typst-lib")]
use typst::foundations::Dict;

#[cfg(not(any(feature = "typst-lib", feature = "typst-bin")))]
compile_error!("At least one of 'typst-lib' or 'typst-bin' features must be enabled");

pub struct RenderConfig {
    #[cfg(all(feature = "typst-lib", not(feature = "typst-bin")))]
    pub universe: Arc<Mutex<TypstSystemUniverse>>,
    #[cfg(all(feature = "typst-lib", feature = "typst-bin"))]
    pub universe: Option<Arc<Mutex<TypstSystemUniverse>>>,

    pub ppi: f32,
    #[cfg(feature = "typst-lib")]
    pub f_input: Box<dyn Fn(i32) -> Dict + 'static + Send + Sync>,

    #[cfg(feature = "typst-bin")]
    pub typst_command: Option<String>,
    #[cfg(feature = "typst-bin")]
    pub input_path: String,
    #[cfg(feature = "typst-bin")]
    pub variable: String,

    pub ffmpeg_options: HashMap<String, String>,
    pub begin_t: i32,
    pub end_t: i32,
    pub fps: u32,
    pub rendering_threads: Option<usize>,
    pub encoding_threads: Option<usize>,
    pub zstd_level: Option<i32>,
    #[cfg(feature = "ffmpeg-bin")]
    pub ffmpeg_path: Option<String>,
}
