use std::{collections::HashMap, sync::{Arc, Mutex}};
use tinymist_world::TypstSystemUniverse;
use typst::foundations::Dict;

pub struct RenderConfig {
    pub universe: Arc<Mutex<TypstSystemUniverse>>,
    pub ppi: f32,
    pub f_input: Box<dyn Fn(i32) -> Dict + 'static + Send + Sync>,
    pub ffmpeg_options: HashMap<String, String>,
    pub begin_t: i32,
    pub end_t: i32,
    pub fps: i32,
}