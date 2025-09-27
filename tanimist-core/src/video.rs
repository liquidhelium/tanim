use crossbeam::channel;
use ndarray::Array3;
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    path::PathBuf,
    sync::{Arc, Once},
    thread,
};
use thiserror::Error;
use tiny_skia::Pixmap;
use tinymist_world::{TaskInputs, TypstSystemUniverse};
use tracing::{info, instrument};
use typst::{diag::SourceDiagnostic, foundations::Dict, layout::PagedDocument, utils::LazyHash};
use typst_render::render;
use video_rs::{Encoder, Location, Time};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Typst compilation failed: {0:?}")]
    TypstCompilation(Vec<SourceDiagnostic>),

    #[error("No pages in the document")]
    NoPages,

    #[error("Failed to create ndarray from shape: {0}")]
    NDArrayShape(#[from] ndarray::ShapeError),

    #[error("Video encoding failed: {0}")]
    VideoEncoding(#[from] video_rs::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to send frame to encoder thread")]
    FrameSendError,

    #[error("Failed to send task to worker thread")]
    TaskSendError,

    #[error("A thread panicked: {0}")]
    ThreadPanic(String),

    #[error("Failed to create pixmap for resizing")]
    PixmapCreation,

    #[error("Failed to create temp file for video encoding")]
    TempFileCreation(#[source] std::io::Error),
}
pub struct TypstVideoRenderer {
    universe: TypstSystemUniverse,
    ppi: f32,
    f_input: Box<dyn Fn(i32) -> Dict + 'static + Send + Sync>,
    ffmpeg_options: HashMap<String, String>,
}

impl TypstVideoRenderer {
    #[instrument(skip_all)]
    pub fn new(
        ppi: f32,
        f_input: impl Fn(i32) -> Dict + 'static + Send + Sync,
        universe: TypstSystemUniverse,
        ffmpeg_options: HashMap<String, String>,
    ) -> Self {
        Self {
            universe,
            ppi,
            f_input: Box::new(f_input),
            ffmpeg_options,
        }
    }

    #[instrument(skip(self))]
    fn render_frame(&self, t: i32) -> Result<tiny_skia::Pixmap, Error> {
        let world = self.universe.snapshot_with(Some(TaskInputs {
            entry: None,
            inputs: Some(Arc::new(LazyHash::new((self.f_input)(t)))),
        }));
        let doc: PagedDocument = typst::compile(&world)
            .output
            .map_err(|e| Error::TypstCompilation(e.to_vec()))?;
        let frame = doc.pages.first().ok_or(Error::NoPages)?;
        Ok(render(frame, self.ppi / 72.0))
    }

    #[instrument]
    fn process_frame(pixmap: Pixmap) -> Result<Array3<u8>, Error> {
        let width = pixmap.width();
        let height = pixmap.height();

        let new_width = width / 2 * 2;
        let new_height = height / 2 * 2;

        if new_width == 0 || new_height == 0 {
            return Err(Error::NoPages);
        }

        let new_pixmap = if new_width == width && new_height == height {
            pixmap
        } else {
            let mut resized = Pixmap::new(new_width, new_height).ok_or(Error::PixmapCreation)?;
            resized.draw_pixmap(
                0,
                0,
                pixmap.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                tiny_skia::Transform::identity(),
                None,
            );
            resized
        };

        let rgba_data = new_pixmap.data();
        let mut rgb_data = Vec::with_capacity(new_width as usize * new_height as usize * 3);
        for chunk in rgba_data.chunks_exact(4) {
            rgb_data.extend_from_slice(&chunk[0..3]); // a,b,c,d -> a,b,c
        }
        Ok(Array3::from_shape_vec(
            (new_height as usize, new_width as usize, 3),
            rgb_data,
        )?)
    }

    #[instrument(skip(self))]
    pub fn render(
        self,
        begin_t: i32,
        end_t: i32,
        fps: i32,
        render_progress: Option<channel::Sender<()>>,
        encode_progress: Option<channel::Sender<()>>,
    ) -> Result<Vec<u8>, Error> {
        static FFMPEG_INIT: Once = Once::new();
        FFMPEG_INIT.call_once(|| {
            video_rs::init().unwrap();
            video_rs::ffmpeg::log::set_level(video_rs::ffmpeg::log::Level::Error);
        });

        let sample_frame = self.render_frame(begin_t)?;
        let width = sample_frame.width() / 2 * 2;
        let height = sample_frame.height() / 2 * 2;

        if width == 0 || height == 0 {
            return Err(Error::NoPages);
        }

        let (frame_tx, frame_rx) = channel::bounded::<(i32, Array3<u8>)>(64);
        let (task_tx, task_rx) = channel::unbounded::<i32>();

        let mut file = tempfile::Builder::new().suffix(".mp4").tempfile()?;
        let output_path = file.path().to_owned();
        let encode_thread = {
            let ffmpeg_options = self.ffmpeg_options.clone();
            thread::Builder::new()
                .name("encoder".to_string())
                .spawn(move || {
                    Self::encode_video(
                        frame_rx,
                        width,
                    height,
                    fps,
                    &output_path.to_string_lossy(),
                    begin_t,
                    encode_progress,
                    ffmpeg_options,
                )
            })
            .unwrap()
        };

        let self_arc = std::sync::Arc::new(self);
        let num_workers = (num_cpus::get() - 2).max(1);
        let workers: Vec<_> = (0..num_workers)
            .map(|i| {
                let task_rx = task_rx.clone();
                let frame_tx = frame_tx.clone();
                let self_clone = self_arc.clone();

                let render_progress = render_progress.clone();
                thread::Builder::new()
                    .name(format!("worker-{}", i))
                    .spawn(move || {
                        while let Ok(t) = task_rx.recv() {
                            let _span = tracing::info_span!("worker_frame", t = t).entered();
                            match self_clone.render_frame(t) {
                            Ok(pixmap) => {
                                match Self::process_frame(pixmap) {
                                    Ok(frame) => {
                                        if frame_tx.send((t, frame)).is_err() {
                                            // Encoder thread has likely panicked, stop sending.
                                            break;
                                        }
                                        if let Some(p) = &render_progress
                                            && p.send(()).is_err()
                                        {
                                            // Progress receiver has been dropped, stop sending.
                                        }
                                    }
                                    Err(e) => panic!("Error processing frame {t}: {e}"),
                                }
                            }
                            Err(e) => {
                                panic!("Error rendering frame {t}: {e}");
                            }
                        }
                    }
                })
                .unwrap()
            })
            .collect();

        for t in begin_t..end_t {
            if task_tx.send(t).is_err() {
                // All workers have panicked or exited, no point in sending more tasks.
                return Err(Error::TaskSendError);
            }
        }

        drop(task_tx);
        drop(frame_tx);

        for worker in workers {
            worker.join().map_err(|e| {
                Error::ThreadPanic(
                    e.downcast_ref::<&'static str>()
                        .map(|s| s.to_string())
                        .or_else(|| e.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "Unknown panic".to_string()),
                )
            })?;
        }
        let encode_result = encode_thread.join().map_err(|e| {
            Error::ThreadPanic(
                e.downcast_ref::<&'static str>()
                    .map(|s| s.to_string())
                    .or_else(|| e.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "Unknown panic in encoder thread".to_string()),
            )
        })?;
        encode_result?; // Propagate errors from inside the encode thread

        {
            let mut buf = vec![];
            let res = file.read_to_end(&mut buf);
            res.map(|_| buf)
        }
        .map_err(Error::Io)
    }

    #[instrument(skip_all, fields(output_path = %output_path))]
    fn encode_video(
        rx: channel::Receiver<(i32, Array3<u8>)>,
        width: u32,
        height: u32,
        fps: i32,
        output_path: &str,
        begin_t: i32,
        encode_progress: Option<channel::Sender<()>>,
        ffmpeg_options: HashMap<String, String>,
    ) -> Result<(), Error> {
        info!("using ffmpeg options: {ffmpeg_options:?}");
        let settings = video_rs::encode::Settings::preset_h264_custom(
            width as usize,
            height as usize,
            video_rs::ffmpeg::format::Pixel::YUV420P,
            ffmpeg_options.into(),
        );
        let mut encoder = Encoder::new(Location::File(PathBuf::from(output_path)), settings)?;

        let mut received_frames: BTreeMap<i32, Array3<u8>> = BTreeMap::new();
        let mut next_expected = begin_t;

        for (frame_num, frame) in rx {
            let _span = tracing::info_span!("encoder_recv", frame_num = frame_num).entered();
            received_frames.insert(frame_num, frame);
            while let Some(frame) = received_frames.remove(&next_expected) {
                let timestamp = Time::from_secs((next_expected - begin_t) as f32 / fps as f32);
                let _encode_span =
                    tracing::info_span!("encode_frame", frame_num = next_expected).entered();
                encoder.encode(&frame, timestamp)?;
                if let Some(p) = &encode_progress
                    && p.send(()).is_err()
                {
                    // Progress receiver has been dropped, stop sending.
                }
                drop(_encode_span);
                next_expected += 1;
            }
        }

        // After the channel is closed, process any remaining frames in the map.
        while let Some(frame) = received_frames.remove(&next_expected) {
            let timestamp = Time::from_secs((next_expected - begin_t) as f32 / fps as f32);
            let _encode_span =
                tracing::info_span!("encode_remaining_frame", frame_num = next_expected).entered();
            encoder.encode(&frame, timestamp)?;
            if let Some(p) = &encode_progress
                && p.send(()).is_err()
            {
                // Progress receiver has been dropped, stop sending.
            }
            drop(_encode_span);
            next_expected += 1;
        }

        encoder.finish()?;
        Ok(())
    }
}
