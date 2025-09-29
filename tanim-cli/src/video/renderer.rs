use crossbeam::channel;
use indicatif::ProgressStyle;
use ndarray::Array3;
use core::num;
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    path::PathBuf,
    sync::{
        Arc, Mutex, Once,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};
use tiny_skia::Pixmap;
use tinymist_world::{TaskInputs, print_diagnostics, system::SystemWorldComputeGraph};
use tracing::{Span, error, info, info_span};
#[cfg(feature = "tracy")]
use tracing::{debug_span, instrument};
use tracing_indicatif::span_ext::IndicatifSpanExt;
use typst::{diag::Warned, layout::PagedDocument, utils::LazyHash};
use typst_render::render;
use video_rs::{Encoder, Location, Time};

use super::{
    config::RenderConfig,
    error::{Error, Result},
    merger::merge_mp4_files,
};

#[derive(Clone)]
struct FrameDispatcher {
    encoders_senders: Arc<Vec<channel::Sender<(i32, Vec<u8>)>>>,
    frames_per_encoder: i32,
}

impl FrameDispatcher {
    fn new(senders: Vec<channel::Sender<(i32, Vec<u8>)>>, frames_per_encoder: i32) -> Self {
        Self {
            encoders_senders: Arc::new(senders),
            frames_per_encoder,
        }
    }

    fn dispatch(&self, frame_num: i32, frame: Vec<u8>, begin_t: i32) {
        let target_encoder_index = (frame_num - begin_t) / self.frames_per_encoder;
        if let Some(sender) = self.encoders_senders.get(target_encoder_index as usize) {
            if sender.send((frame_num, frame)).is_err() {
                // Encoder thread might have panicked and closed the channel.
                // This will be handled when the main thread tries to join the encoder thread.
                error!("Failed to send frame {frame_num} to encoder {target_encoder_index} - channel closed");
            }
        }
    }
}

struct FrameEncoder {
    encoder: Encoder,
    received_frames: BTreeMap<i32, Vec<u8>>,
    next_expected: i32,
    width: u32,
    height: u32,
    fps: i32,
    video_begin_t: i32,
}

impl FrameEncoder {
    fn new(
        output_path: &str,
        width: u32,
        height: u32,
        fps: i32,
        begin_t: i32,
        video_begin_t: i32,
        ffmpeg_options: HashMap<String, String>,
    ) -> Result<Self> {
        info!("using ffmpeg options: {ffmpeg_options:?}");
        let settings = video_rs::encode::Settings::preset_h264_custom(
            width as usize,
            height as usize,
            video_rs::ffmpeg::format::Pixel::YUV420P,
            ffmpeg_options.into(),
        );
        let encoder = Encoder::new(Location::File(PathBuf::from(output_path)), settings)?;
        Ok(Self {
            encoder,
            received_frames: BTreeMap::new(),
            next_expected: begin_t,
            width,
            height,
            fps,
            video_begin_t,
        })
    }

    fn encode_frame(
        &mut self,
        frame_num: i32,
        frame: Vec<u8>,
        encode_progress: Option<&Span>,
        stop_signal: &Arc<AtomicBool>,
    ) -> Result<()> {
        #[cfg(feature = "tracy")]
        let _span = tracing::debug_span!("encoder_recv", frame_num = frame_num).entered();
        self.received_frames.insert(frame_num, frame);
        while let Some(raw_frame) = self.received_frames.remove(&self.next_expected) {
            if stop_signal.load(Ordering::SeqCst) {
                break;
            }
            let frame =
                Array3::from_shape_vec((self.height as usize, self.width as usize, 3), raw_frame)?;
            let timestamp =
                Time::from_secs((self.next_expected - self.video_begin_t) as f32 / self.fps as f32);
            #[cfg(feature = "tracy")]
            let _encode_span =
                tracing::debug_span!("encode_frame", frame_num = self.next_expected).entered();
            self.encoder.encode(&frame, timestamp)?;
            if !stop_signal.load(Ordering::SeqCst)
                && let Some(p) = encode_progress
            {
                p.pb_inc(1);
            }
            #[cfg(feature = "tracy")]
            drop(_encode_span);
            self.next_expected += 1;
        }
        Ok(())
    }

    fn finish(mut self, encode_progress: Option<&Span>) -> Result<()> {
        // After the channel is closed, process any remaining frames in the map.
        while let Some(raw_frame) = self.received_frames.remove(&self.next_expected) {
            let frame =
                Array3::from_shape_vec((self.height as usize, self.width as usize, 3), raw_frame)?;
            let timestamp =
                Time::from_secs((self.next_expected - self.video_begin_t) as f32 / self.fps as f32);
            #[cfg(feature = "tracy")]
            let _encode_span =
                tracing::debug_span!("encode_remaining_frame", frame_num = self.next_expected)
                    .entered();
            self.encoder.encode(&frame, timestamp)?;
            if let Some(p) = encode_progress {
                p.pb_inc(1);
            }
            #[cfg(feature = "tracy")]
            drop(_encode_span);
            self.next_expected += 1;
        }

        self.encoder.finish()?;
        Ok(())
    }
}

pub struct TypstVideoRenderer {
    config: RenderConfig,
}

impl TypstVideoRenderer {
    #[cfg_attr(feature = "tracy", instrument(skip_all))]
    pub fn new(config: RenderConfig) -> Self {
        Self { config }
    }

    #[cfg_attr(feature = "tracy", instrument(level = "debug", skip(self)))]
    fn render_frame(&self, t: i32) -> Result<tiny_skia::Pixmap> {
        let mut universe = self.config.universe.lock().unwrap();
        universe.increment_revision(|univ| {
            univ.set_inputs(Arc::new(LazyHash::new((self.config.f_input)(t))));
        });
        let world = universe.snapshot();
        #[cfg(feature = "tracy")]
        let _span = debug_span!("compilation", frame = t).entered();
        let Warned { output, warnings } = typst::compile(&world);
        #[cfg(feature = "tracy")]
        drop(_span);
        let doc: PagedDocument = {
            if let Err(e) = print_diagnostics(
                &world,
                warnings.iter(),
                tinymist_world::DiagnosticFormat::Human,
            ) {
                error!("Error printing diagnostics: {e}");
            };
            output.map_err(Error::TypstCompilation)?
        };

        let frame = doc.pages.first().ok_or(Error::NoPages)?;
        Ok(render(frame, self.config.ppi / 72.0))
    }

    #[cfg_attr(feature = "tracy", instrument(level = "debug", skip(pixmap)))]
    fn process_frame(pixmap: Pixmap) -> Result<Array3<u8>> {
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

    pub fn render(self, error_signal: Arc<AtomicBool>) -> Result<Vec<u8>> {
        let (width, height, rendering_span, encoding_span, stop_signal) = self.prepare_render()?;
        let self_arc = Arc::new(self);

        let results = self_arc.spawn_render_workers(
            width,
            height,
            rendering_span,
            encoding_span,
            stop_signal.clone(),
            error_signal.clone(),
        );

        let output_files = self_arc.wait_for_workers(results)?;

        if stop_signal.load(Ordering::SeqCst) {
            return Err(Error::RenderOrEncode);
        }

        self_arc.merge_video_chunks(output_files)
    }

    fn prepare_render(&self) -> Result<(u32, u32, Span, Span, Arc<AtomicBool>)> {
        let begin_t = self.config.begin_t;
        let end_t = self.config.end_t;

        let rendering_span = info_span!("rendering");
        let encoding_span = info_span!("encoding");

        let sty = ProgressStyle::with_template(
            "[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}",
        )
        .unwrap()
        .progress_chars("##-");

        let total_frames = (end_t - begin_t) as u64;

        rendering_span.pb_set_length(total_frames);
        rendering_span.pb_set_style(&sty);
        rendering_span.pb_set_message("rendering");

        encoding_span.pb_set_length(total_frames);
        encoding_span.pb_set_style(&sty);
        encoding_span.pb_set_message("encoding");

        let stop_signal = Arc::new(AtomicBool::new(false));
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

        Ok((width, height, rendering_span, encoding_span, stop_signal))
    }

    fn spawn_render_workers(
        self: &Arc<Self>,
        width: u32,
        height: u32,
        rendering_span: Span,
        encoding_span: Span,
        stop_signal: Arc<AtomicBool>,
        error_signal: Arc<AtomicBool>,
    ) -> (
        Vec<thread::JoinHandle<Result<()>>>,
        Vec<tempfile::NamedTempFile>,
        Vec<thread::JoinHandle<()>>,
    ) {
        let begin_t = self.config.begin_t;
        let end_t = self.config.end_t;
        let num_render_workers = self.config.rendering_threads.unwrap_or_else(|| (num_cpus::get() - 4).max(1));
        let num_encode_workers = self.config.encoding_threads.unwrap_or_else(|| (num_cpus::get() - num_render_workers).max(1));
        let frames_per_encoder = (end_t - begin_t) / num_encode_workers as i32;

        let mut encoder_senders = Vec::with_capacity(num_encode_workers);
        let mut encoder_receivers = Vec::with_capacity(num_encode_workers);
        for _ in 0..num_encode_workers {
            let (tx, rx) = channel::bounded(num_encode_workers * 4);
            encoder_senders.push(tx);
            encoder_receivers.push(rx);
        }

        let dispatcher = FrameDispatcher::new(encoder_senders, frames_per_encoder);

        let _re = rendering_span.enter();
        let _en = encoding_span.enter();

        let (encode_threads, temp_files) = self.spawn_encoder_workers(
            width,
            height,
            encoding_span.clone(),
            stop_signal.clone(),
            error_signal.clone(),
            encoder_receivers,
        );

        let render_threads = self.spawn_renderer_workers(
            rendering_span.clone(),
            stop_signal,
            error_signal,
            dispatcher,
            num_render_workers
        );

        (encode_threads, temp_files, render_threads)
    }

    fn spawn_encoder_workers(
        self: &Arc<Self>,
        width: u32,
        height: u32,
        encoding_span: Span,
        stop_signal: Arc<AtomicBool>,
        error_signal: Arc<AtomicBool>,
        encoder_receivers: Vec<channel::Receiver<(i32, Vec<u8>)>>,
    ) -> (Vec<thread::JoinHandle<Result<()>>>, Vec<tempfile::NamedTempFile>) {
        let num_encode_workers = encoder_receivers.len();
        let begin_t = self.config.begin_t;
        let end_t = self.config.end_t;
        let fps = self.config.fps;
        let frames_per_encoder = (end_t - begin_t) / num_encode_workers as i32;

        let mut encode_threads = Vec::with_capacity(num_encode_workers);
        let mut temp_files = Vec::with_capacity(num_encode_workers);

        for (i, frame_rx) in encoder_receivers.into_iter().enumerate() {
            let chunk_begin_t = begin_t + i as i32 * frames_per_encoder;
            let file = tempfile::Builder::new()
                .suffix(&format!("_{i}.mp4"))
                .tempfile()
                .map_err(Error::TempFileCreation)
                .unwrap();
            let output_path = file.path().to_owned();
            temp_files.push(file);

            let ffmpeg_options = self.config.ffmpeg_options.clone();
            let stop_signal1 = stop_signal.clone();
            let error_signal1 = error_signal.clone();
            let encoding_span = encoding_span.clone();
            let video_begin_t = begin_t;

            let encode_thread = thread::Builder::new()
                .name(format!("encoder-{i}"))
                .spawn(move || {
                    Self::encode_video(
                        frame_rx,
                        width,
                        height,
                        fps,
                        &output_path.to_string_lossy(),
                        chunk_begin_t,
                        video_begin_t,
                        Some(encoding_span),
                        ffmpeg_options,
                        stop_signal1,
                        error_signal1,
                    )
                })
                .unwrap();
            encode_threads.push(encode_thread);
        }
        (encode_threads, temp_files)
    }

    fn spawn_renderer_workers(
        self: &Arc<Self>,
        rendering_span: Span,
        stop_signal: Arc<AtomicBool>,
        error_signal: Arc<AtomicBool>,
        dispatcher: FrameDispatcher,
        count: usize
    ) -> Vec<thread::JoinHandle<()>> {
        let begin_t = self.config.begin_t;
        let end_t = self.config.end_t;
        let frames_per_renderer = (end_t - begin_t) / count as i32;

        (0..count)
            .map(|i| {
                let chunk_begin_t = begin_t + i as i32 * frames_per_renderer;
                let chunk_end_t = if i == count - 1 {
                    end_t
                } else {
                    chunk_begin_t + frames_per_renderer
                };

                let self_clone = self.clone();
                let rendering_span_clone = rendering_span.clone();
                let stop_signal_clone = stop_signal.clone();
                let error_signal_clone = error_signal.clone();
                let dispatcher_clone = dispatcher.clone();

                thread::Builder::new()
                    .name(format!("renderer-{i}"))
                    .spawn(move || {
                        self_clone.render_chunk(
                            chunk_begin_t,
                            chunk_end_t,
                            dispatcher_clone,
                            rendering_span_clone,
                            stop_signal_clone,
                            error_signal_clone,
                        );
                    })
                    .unwrap()
            })
            .collect()
    }

    fn render_chunk(
        &self,
        chunk_begin_t: i32,
        chunk_end_t: i32,
        dispatcher: FrameDispatcher,
        rendering_span: Span,
        stop_signal: Arc<AtomicBool>,
        error_signal: Arc<AtomicBool>,
    ) {
        let begin_t = self.config.begin_t;
        for t in chunk_begin_t..chunk_end_t {
            if stop_signal.load(Ordering::SeqCst) {
                break;
            }
            match self.render_frame(t) {
                Ok(pixmap) => match Self::process_frame(pixmap) {
                    Ok(frame) => {
                        dispatcher.dispatch(t, frame.into_raw_vec_and_offset().0, begin_t);
                        if !stop_signal.load(Ordering::SeqCst) {
                            rendering_span.pb_inc(1);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Error processing frame {t}: {e}");
                        stop_signal.store(true, Ordering::SeqCst);
                        error_signal.store(true, Ordering::SeqCst);
                        break;
                    }
                },
                Err(e) => {
                    tracing::error!("Error rendering frame {t}: {e}");
                    stop_signal.store(true, Ordering::SeqCst);
                    error_signal.store(true, Ordering::SeqCst);
                    break;
                }
            }
        }
    }

    fn wait_for_workers(
        &self,
        (encode_threads, temp_files, render_threads): (
            Vec<thread::JoinHandle<Result<()>>>,
            Vec<tempfile::NamedTempFile>,
            Vec<thread::JoinHandle<()>>,
        ),
    ) -> Result<Vec<tempfile::NamedTempFile>> {
        for render_thread in render_threads {
            render_thread.join().map_err(|e| {
                Error::ThreadPanic(
                    e.downcast_ref::<&'static str>()
                        .map(|s| s.to_string())
                        .or_else(|| e.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "Unknown panic in renderer thread".to_string()),
                )
            })?;
        }

        for encode_thread in encode_threads {
            let encode_result = encode_thread.join().map_err(|e| {
                Error::ThreadPanic(
                    e.downcast_ref::<&'static str>()
                        .map(|s| s.to_string())
                        .or_else(|| e.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "Unknown panic in encoder thread".to_string()),
                )
            })?;
            encode_result?;
        }

        Ok(temp_files)
    }

    fn merge_video_chunks(&self, output_files: Vec<tempfile::NamedTempFile>) -> Result<Vec<u8>> {
        let mut output_file = tempfile::Builder::new().suffix(".mp4").tempfile()?;
        let final_output_path = output_file.path().to_string_lossy().to_string();
        let input_paths_str: Vec<&str> = output_files
            .iter()
            .map(|p| p.path().to_str().unwrap())
            .collect();

        merge_mp4_files(&input_paths_str, &final_output_path)
            .map_err(|e| Error::MergeVideoChunks(e.to_string()))?;

        let mut buf = Vec::new();
        output_file.read_to_end(&mut buf)?;
        Ok(buf)
    }

    #[cfg_attr(feature = "tracy", instrument(level = "debug", skip_all, fields(output_path = %output_path)))]
    #[allow(clippy::too_many_arguments)]
    fn encode_video(
        rx: channel::Receiver<(i32, Vec<u8>)>,
        width: u32,
        height: u32,
        fps: i32,
        output_path: &str,
        begin_t: i32,
        video_begin_t: i32,
        encode_progress: Option<Span>,
        ffmpeg_options: HashMap<String, String>,
        stop_signal: Arc<AtomicBool>,
        error_signal: Arc<AtomicBool>,
    ) -> Result<()> {
        let mut encoder = FrameEncoder::new(
            output_path,
            width,
            height,
            fps,
            begin_t,
            video_begin_t,
            ffmpeg_options,
        )?;

        for (frame_num, frame) in rx {
            if stop_signal.load(Ordering::SeqCst) {
                break;
            }
            if let Err(e) =
                encoder.encode_frame(frame_num, frame, encode_progress.as_ref(), &stop_signal)
            {
                stop_signal.store(true, Ordering::SeqCst);
                error_signal.store(true, Ordering::SeqCst);
                return Err(e);
            }
        }

        if !stop_signal.load(Ordering::SeqCst)
            && let Err(e) = encoder.finish(encode_progress.as_ref())
        {
            stop_signal.store(true, Ordering::SeqCst);
            error_signal.store(true, Ordering::SeqCst);
            return Err(e);
        }

        Ok(())
    }
}
