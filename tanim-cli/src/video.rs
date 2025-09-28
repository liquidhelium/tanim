use crossbeam::channel;
use indicatif::ProgressStyle;
use ndarray::Array3;
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Once,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};
use thiserror::Error;
use tiny_skia::Pixmap;
use tinymist_world::{TaskInputs, TypstSystemUniverse};
use tracing::{Span, debug_span, info, info_span, instrument};
use tracing_indicatif::span_ext::IndicatifSpanExt;
use typst::{diag::SourceDiagnostic, foundations::Dict, layout::PagedDocument, utils::LazyHash};
use typst_render::render;
use video_rs::{Encoder, Location, MuxerBuilder, Reader, Time, Writer};

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

    #[error("Rendering or encoding failed")]
    RenderOrEncode,

    #[error("Failed to send task to worker thread")]
    TaskSendError,

    #[error("A thread panicked: {0}")]
    ThreadPanic(String),

    #[error("Failed to create pixmap for resizing")]
    PixmapCreation,

    #[error("Failed to create temp file for video encoding")]
    TempFileCreation(#[source] std::io::Error),

    #[error("Failed to merge video chunks: {0}")]
    MergeVideoChunks(String),
}
pub struct TypstVideoRenderer {
    universe: TypstSystemUniverse,
    ppi: f32,
    f_input: Box<dyn Fn(i32) -> Dict + 'static + Send + Sync>,
    ffmpeg_options: HashMap<String, String>,
}

impl TypstVideoRenderer {
    #[cfg_attr(feature = "tracy", instrument(skip_all))]
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

    #[cfg_attr(feature = "tracy", instrument(level = "debug", skip(self)))]
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

    #[cfg_attr(feature = "tracy", instrument(level = "debug", skip(pixmap)))]
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

    #[cfg_attr(feature = "tracy", instrument(level = "debug", skip(self)))]
    pub fn render(
        self,
        begin_t: i32,
        end_t: i32,
        fps: i32,
        error_signal: Arc<AtomicBool>,
    ) -> Result<Vec<u8>, Error> {
        #[cfg(feature = "tracy")]
        let root_span = debug_span!("render");
        let rendering_span = info_span!("rendering");
        let encoding_span = info_span!("encoding");
        #[cfg(feature = "tracy")]
        let _enter = root_span.enter();

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

        let num_render_workers = (num_cpus::get() - 2).max(1);
        let num_encode_workers = (num_cpus::get() / 4).max(1);
        let (task_tx, task_rx) = channel::bounded::<i32>(num_render_workers * 10);

        let self_arc = std::sync::Arc::new(self);
        let _re = rendering_span.enter();
        let _en = encoding_span.enter();

        let frames_per_worker = (end_t - begin_t) / num_encode_workers as i32;
        let mut chunk_frame_txs = Vec::new();
        let mut encode_threads = Vec::new();
        let mut output_files = Vec::new();

        for i in 0..num_encode_workers {
            let chunk_begin_t = begin_t + i as i32 * frames_per_worker;

            let (frame_tx, frame_rx) = channel::bounded::<(i32, Vec<u8>)>(num_render_workers * 4);
            chunk_frame_txs.push(frame_tx);

            let file = tempfile::Builder::new()
                .suffix(&format!("_{i}.mp4"))
                .tempfile()
                .map_err(Error::TempFileCreation)?;

            let output_path = file.path().to_owned();
            output_files.push(file);

            let ffmpeg_options = self_arc.ffmpeg_options.clone();
            let stop_signal = stop_signal.clone();
            let error_signal = error_signal.clone();
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
                        stop_signal,
                        error_signal,
                    )
                })
                .unwrap();
            encode_threads.push(encode_thread);
        }

        let render_workers: Vec<_> = (0..num_render_workers)
            .map(|i| {
                let task_rx = task_rx.clone();
                let chunk_frame_txs = chunk_frame_txs.clone();
                let self_clone = self_arc.clone();
                let stop_signal = stop_signal.clone();
                let error_signal = error_signal.clone();
                let pb_render = rendering_span.clone();
                thread::Builder::new()
                    .name(format!("render-worker-{i}"))
                    .spawn(move || {
                        while let Ok(t) = task_rx.recv() {
                            if stop_signal.load(Ordering::SeqCst) {
                                break;
                            }
                            #[cfg(feature = "tracy")]
                            let _span = tracing::debug_span!("worker_frame", t = t).entered();
                            let chunk_index = ((t - begin_t) / frames_per_worker)
                                .min(num_encode_workers as i32 - 1)
                                as usize;
                            match self_clone.render_frame(t) {
                                Ok(pixmap) => match Self::process_frame(pixmap) {
                                    Ok(frame) => {
                                        if chunk_frame_txs[chunk_index]
                                            .send((t, frame.into_raw_vec_and_offset().0))
                                            .is_err()
                                        {
                                            break;
                                        }
                                        if !stop_signal.load(Ordering::SeqCst) {
                                            pb_render.pb_inc(1);
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
                    })
                    .unwrap()
            })
            .collect();

        let num_workers = num_encode_workers as i32;
        let frames_per_worker = (end_t - begin_t) / num_workers;
        let chunks: Vec<_> = (0..num_workers)
            .map(|i| {
                let chunk_begin_t = begin_t + i * frames_per_worker;
                let chunk_end_t = if i == num_workers - 1 {
                    end_t
                } else {
                    chunk_begin_t + frames_per_worker
                };
                (chunk_begin_t..chunk_end_t).collect::<Vec<_>>()
            })
            .collect();

        let max_len = chunks.iter().map(|c| c.len()).max().unwrap_or(0);

        'outer: for i in 0..max_len {
            for chunk in &chunks {
                if let Some(&t) = chunk.get(i) {
                    if stop_signal.load(Ordering::SeqCst) {
                        break 'outer;
                    }
                    if task_tx.send(t).is_err() {
                        break 'outer;
                    }
                }
            }
        }

        drop(task_tx);
        for tx in chunk_frame_txs {
            drop(tx);
        }

        for worker in render_workers {
            worker.join().map_err(|e| {
                Error::ThreadPanic(
                    e.downcast_ref::<&'static str>()
                        .map(|s| s.to_string())
                        .or_else(|| e.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "Unknown panic".to_string()),
                )
            })?;
        }

        for thread in encode_threads {
            let encode_result = thread.join().map_err(|e| {
                Error::ThreadPanic(
                    e.downcast_ref::<&'static str>()
                        .map(|s| s.to_string())
                        .or_else(|| e.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "Unknown panic in encoder thread".to_string()),
                )
            })?;
            encode_result?;
        }

        if stop_signal.load(Ordering::SeqCst) {
            return Err(Error::RenderOrEncode);
        }

        let mut output_file = tempfile::Builder::new().suffix(".mp4").tempfile()?;
        let final_output_path = output_file.path().to_string_lossy().to_string();
        let input_paths_str: Vec<&str> = output_files
            .iter()
            .map(|p| p.path().to_str().unwrap())
            .collect();

        merge_mp4_files(input_paths_str, &final_output_path)
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
    ) -> Result<(), Error> {
        info!("using ffmpeg options: {ffmpeg_options:?}");
        let settings = video_rs::encode::Settings::preset_h264_custom(
            width as usize,
            height as usize,
            video_rs::ffmpeg::format::Pixel::YUV420P,
            ffmpeg_options.into(),
        );
        let mut encoder = Encoder::new(Location::File(PathBuf::from(output_path)), settings)?;

        let mut received_frames: BTreeMap<i32, Vec<u8>> = BTreeMap::new();
        let mut next_expected = begin_t;

        for (frame_num, frame) in rx {
            if stop_signal.load(Ordering::SeqCst) {
                break;
            }
            #[cfg(feature = "tracy")]
            let _span = tracing::debug_span!("encoder_recv", frame_num = frame_num).entered();
            received_frames.insert(frame_num, frame);
            while let Some(raw_frame) = received_frames.remove(&next_expected) {
                if stop_signal.load(Ordering::SeqCst) {
                    break;
                }
                let frame =
                    Array3::from_shape_vec((height as usize, width as usize, 3), raw_frame)?;
                let timestamp =
                    Time::from_secs((next_expected - video_begin_t) as f32 / fps as f32);
                #[cfg(feature = "tracy")]
                let _encode_span =
                    tracing::debug_span!("encode_frame", frame_num = next_expected).entered();
                if let Err(e) = encoder.encode(&frame, timestamp) {
                    stop_signal.store(true, Ordering::SeqCst);
                    error_signal.store(true, Ordering::SeqCst);
                    return Err(e.into());
                }
                if !stop_signal.load(Ordering::SeqCst)
                    && let Some(p) = &encode_progress
                {
                    p.pb_inc(1);
                }
                #[cfg(feature = "tracy")]
                drop(_encode_span);
                next_expected += 1;
            }
        }

        if !stop_signal.load(Ordering::SeqCst) {
            // After the channel is closed, process any remaining frames in the map.
            while let Some(raw_frame) = received_frames.remove(&next_expected) {
                let frame =
                    Array3::from_shape_vec((height as usize, width as usize, 3), raw_frame)?;
                let timestamp =
                    Time::from_secs((next_expected - video_begin_t) as f32 / fps as f32);
                #[cfg(feature = "tracy")]
                let _encode_span =
                    tracing::debug_span!("encode_remaining_frame", frame_num = next_expected)
                        .entered();
                if let Err(e) = encoder.encode(&frame, timestamp) {
                    stop_signal.store(true, Ordering::SeqCst);
                    error_signal.store(true, Ordering::SeqCst);
                    return Err(e.into());
                }
                if let Some(p) = &encode_progress {
                    p.pb_inc(1);
                }
                #[cfg(feature = "tracy")]
                drop(_encode_span);
                next_expected += 1;
            }
        }

        if let Err(e) = encoder.finish() {
            stop_signal.store(true, Ordering::SeqCst);
            error_signal.store(true, Ordering::SeqCst);
            return Err(e.into());
        }
        Ok(())
    }
}
#[cfg_attr(feature = "tracy", instrument(level = "debug", skip_all))]
fn merge_mp4_files(
    input_files: Vec<&str>,
    output_file: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    // 创建输出写入器
    let writer = Writer::new(Path::new(output_file))?;
    let mut muxer_builder = MuxerBuilder::new(writer);

    // 创建读取器并添加流
    let mut readers = Vec::new();
    for input_file in &input_files {
        let reader = Reader::new(Path::new(input_file))?;
        muxer_builder = muxer_builder.with_streams(&reader)?;
        readers.push(reader);
    }

    // 构建 muxer
    let mut muxer = muxer_builder.build();

    // 处理每个文件的数据包
    for mut reader in readers {
        // 获取最佳视频流索引
        let stream_index = reader
            .input
            .streams()
            .find(|stream| stream.parameters().medium() == video_rs::ffmpeg::media::Type::Video)
            .map(|stream| stream.index())
            .unwrap_or(0);

        // 读取并合并数据包
        while let Ok(packet) = reader.read(stream_index) {
            muxer.mux(packet)?;
        }
    }

    // 完成合并
    muxer.finish()?;

    Ok(output_file.to_string())
}
