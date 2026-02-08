use crossbeam::channel;
use indicatif::ProgressStyle;
use ndarray::Array3;
#[cfg(feature = "embedded-ffmpeg")]
use rsmpeg::{
    avcodec::{AVCodec, AVCodecContext},
    avformat::AVFormatContextOutput,
    avutil::{AVDictionary, AVFrame, AVRational},
    error::RsmpegError,
    ffi,
    swscale::SwsContext,
};
#[cfg(feature = "embedded-ffmpeg")]
use std::ffi::CString;

#[cfg(feature = "ffmpeg-bin")]
use std::io::Write;
#[cfg(feature = "ffmpeg-bin")]
use std::process::{Child, Command, Stdio};
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    sync::{
        Arc, Once, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};
use tiny_skia::Pixmap;
use tinymist_world::print_diagnostics;
use tracing::{Span, error, info, info_span};
#[cfg(feature = "tracy")]
use tracing::{debug_span, instrument};
use tracing_indicatif::span_ext::IndicatifSpanExt;
use typst::{diag::Warned, layout::PagedDocument, utils::LazyHash};
use typst_render::render;

use super::{
    config::RenderConfig,
    error::{Error, Result},
    merger::merge_mp4_files,
};

type EncoderSenderVec = Vec<channel::Sender<(i32, Vec<u8>)>>;

#[derive(Clone)]
struct FrameDispatcher {
    encoders_senders: Arc<EncoderSenderVec>,
    frames_per_encoder: i32,
}

impl FrameDispatcher {
    fn new(senders: EncoderSenderVec, frames_per_encoder: i32) -> Self {
        Self {
            encoders_senders: Arc::new(senders),
            frames_per_encoder,
        }
    }

    fn dispatch(&self, frame_num: i32, frame: Vec<u8>, begin_t: i32) {
        let target_encoder_index = (frame_num - begin_t) / self.frames_per_encoder;
        if let Some(sender) = self.encoders_senders.get(target_encoder_index as usize)
            && sender.send((frame_num, frame)).is_err()
        {
            // Encoder thread might have panicked and closed the channel.
            // This will be handled when the main thread tries to join the encoder thread.
            error!(
                "Failed to send frame {frame_num} to encoder {target_encoder_index} - channel closed"
            );
        }
    }
}

#[cfg(feature = "embedded-ffmpeg")]
struct RsmpegData {
    encode_ctx: AVCodecContext,
    output_ctx: AVFormatContextOutput,
    frame: AVFrame,
    input_frame: AVFrame,
    sws_ctx: SwsContext,
    stream_index: usize,
}

#[cfg(feature = "ffmpeg-bin")]
struct BinaryData {
    process: Child,
}

enum EncoderBackend {
    #[cfg(feature = "embedded-ffmpeg")]
    Rsmpeg(RsmpegData),
    #[cfg(feature = "ffmpeg-bin")]
    Binary(BinaryData),
}

struct FrameEncoder {
    backend: EncoderBackend,
    received_frames: BTreeMap<i32, Vec<u8>>,
    next_expected: i32,
    width: u32,
    height: u32,
    _fps: u32,
    video_begin_t: i32,
    zstd_level: Option<i32>,
}

impl FrameEncoder {
    #[allow(clippy::too_many_arguments)]
    fn new(
        output_path: &str,
        width: u32,
        height: u32,
        fps: u32,
        begin_t: i32,
        video_begin_t: i32,
        ffmpeg_options: HashMap<String, String>,
        zstd_level: Option<i32>,
        ffmpeg_path: Option<String>,
    ) -> Result<Self> {
        info!("using ffmpeg options: {ffmpeg_options:?}");

        #[cfg(all(feature = "embedded-ffmpeg", feature = "ffmpeg-bin"))]
        let use_binary = ffmpeg_path.is_some();

        #[cfg(all(feature = "ffmpeg-bin", not(feature = "embedded-ffmpeg")))]
        let use_binary = true;

        #[cfg(all(not(feature = "ffmpeg-bin"), feature = "embedded-ffmpeg"))]
        let use_binary = false;

        let backend = if use_binary {
            #[cfg(feature = "ffmpeg-bin")]
            {
                let mut cmd = Command::new(ffmpeg_path.as_deref().unwrap_or("ffmpeg"));
                cmd.arg("-loglevel").arg("error");
                cmd.arg("-y")
                    .arg("-f")
                    .arg("rawvideo")
                    .arg("-pixel_format")
                    .arg("rgb24")
                    .arg("-video_size")
                    .arg(format!("{}x{}", width, height))
                    .arg("-framerate")
                    .arg(fps.to_string())
                    .arg("-i")
                    .arg("-");

                for (k, v) in ffmpeg_options {
                    cmd.arg(format!("-{}", k)).arg(v);
                }

                cmd.arg("-pix_fmt").arg("yuv420p");
                cmd.arg(output_path).stdin(Stdio::piped());

                let process = cmd
                    .spawn()
                    .map_err(|e| anyhow::anyhow!("Failed to spawn ffmpeg: {}", e))?;
                EncoderBackend::Binary(BinaryData { process })
            }
            #[cfg(not(feature = "ffmpeg-bin"))]
            {
                unreachable!("use_binary should be false if ffmpeg-bin is not enabled")
            }
        } else {
            #[cfg(feature = "embedded-ffmpeg")]
            {
                let output_file_cstr = CString::new(output_path).unwrap();
                let mut output_ctx =
                    AVFormatContextOutput::create(output_file_cstr.as_c_str(), None)?;

                let codec = ffmpeg_options
                    .get("codec")
                    .map(|c| {
                        match AVCodec::find_encoder_by_name(&std::ffi::CString::new(
                            c.clone(),
                        ).ok()?) {
                            Some(c) => Some(c),
                            None => {
                                error!("Cannot find a codec with name {c:?}. Falling back to libx264 instead");
                                None
                            }
                        }
                    })
                    .flatten()
                    .or_else(|| {
                        AVCodec::find_encoder_by_name(
                            std::ffi::CStr::from_bytes_with_nul(b"libx264\0").unwrap(),
                        )
                        .or_else(|| AVCodec::find_encoder(ffi::AV_CODEC_ID_H264))
                    })
                    .expect("H.264 encoder not found");

                let mut encode_ctx = AVCodecContext::new(&codec);
                encode_ctx.set_height(height as i32);
                encode_ctx.set_width(width as i32);
                encode_ctx.set_time_base(AVRational {
                    num: 1,
                    den: fps as i32,
                });
                encode_ctx.set_framerate(AVRational {
                    num: fps as i32,
                    den: 1,
                });
                encode_ctx.set_pix_fmt(ffi::AV_PIX_FMT_YUV420P);

                let mut options_dict: Option<AVDictionary> = None;
                for (k, v) in ffmpeg_options {
                    let k = CString::new(k).unwrap();
                    let v = CString::new(v).unwrap();
                    options_dict = match options_dict {
                        Some(dict) => Some(dict.set(&k, &v, 0)),
                        None => Some(AVDictionary::new(&k, &v, 0)),
                    };
                }

                if output_ctx.oformat().flags & ffi::AVFMT_GLOBALHEADER as i32 != 0 {
                    encode_ctx
                        .set_flags(encode_ctx.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
                }

                encode_ctx.open(options_dict)?;

                let stream_index = {
                    let mut stream = output_ctx.new_stream();
                    stream.set_codecpar(encode_ctx.extract_codecpar());
                    stream.set_time_base(encode_ctx.time_base);
                    stream.index as usize
                };

                output_ctx.dump(0, output_file_cstr.as_c_str())?;
                output_ctx.write_header(&mut None)?;

                let mut frame = AVFrame::new();
                frame.set_format(encode_ctx.pix_fmt);
                frame.set_width(encode_ctx.width);
                frame.set_height(encode_ctx.height);
                frame.alloc_buffer()?;

                let mut input_frame = AVFrame::new();
                input_frame.set_format(ffi::AV_PIX_FMT_RGB24);
                input_frame.set_width(width as i32);
                input_frame.set_height(height as i32);
                input_frame.alloc_buffer()?;

                let sws_ctx = SwsContext::get_context(
                    width as i32,
                    height as i32,
                    ffi::AV_PIX_FMT_RGB24,
                    width as i32,
                    height as i32,
                    ffi::AV_PIX_FMT_YUV420P,
                    ffi::SWS_BILINEAR,
                    None,
                    None,
                    None,
                )
                .ok_or(Error::SwsContextCreation)?;
                EncoderBackend::Rsmpeg(RsmpegData {
                    encode_ctx,
                    output_ctx,
                    frame,
                    input_frame,
                    sws_ctx,
                    stream_index,
                })
            }
            #[cfg(not(feature = "embedded-ffmpeg"))]
            {
                unreachable!("use_binary should be true if embedded-ffmpeg is not enabled")
            }
        };

        Ok(Self {
            backend,
            received_frames: BTreeMap::new(),
            next_expected: begin_t,
            width,
            height,
            _fps: fps,
            video_begin_t,
            zstd_level,
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
        while let Some(received_frame) = self.received_frames.remove(&self.next_expected) {
            if stop_signal.load(Ordering::SeqCst) {
                break;
            }
            let raw_frame = if self.zstd_level.is_some() {
                zstd::decode_all(&received_frame[..]).unwrap()
            } else {
                received_frame
            };
            if raw_frame.len() != (self.width * self.height * 3) as usize {
                return Err(Error::NDArrayShape(ndarray::ShapeError::from_kind(
                    ndarray::ErrorKind::IncompatibleShape,
                )));
            }

            match &mut self.backend {
                #[cfg(feature = "embedded-ffmpeg")]
                EncoderBackend::Rsmpeg(data) => {
                    let linesize = data.input_frame.linesize_mut()[0] as usize;
                    let width_bytes = (self.width * 3) as usize;
                    let input_data = data.input_frame.data_mut()[0];

                    unsafe {
                        for i in 0..self.height as usize {
                            std::ptr::copy_nonoverlapping(
                                raw_frame.as_ptr().add(i * width_bytes),
                                input_data.add(i * linesize),
                                width_bytes,
                            );
                        }
                    }

                    data.frame.make_writable()?;

                    unsafe {
                        data.sws_ctx.scale(
                            data.input_frame.data_mut().as_ptr() as *const *const u8,
                            data.input_frame.linesize_mut().as_ptr() as *const i32,
                            0,
                            self.height as i32,
                            data.frame.data_mut().as_ptr() as *const *mut u8,
                            data.frame.linesize_mut().as_ptr() as *const i32,
                        )?;
                    }

                    data.frame
                        .set_pts((self.next_expected - self.video_begin_t) as i64);

                    #[cfg(feature = "tracy")]
                    let _encode_span =
                        tracing::debug_span!("encode_frame", frame_num = self.next_expected)
                            .entered();

                    data.encode_ctx.send_frame(Some(&data.frame))?;

                    loop {
                        let mut packet = match data.encode_ctx.receive_packet() {
                            Ok(packet) => packet,
                            Err(RsmpegError::EncoderDrainError)
                            | Err(RsmpegError::EncoderFlushedError) => {
                                break;
                            }
                            Err(e) => return Err(e.into()),
                        };

                        packet.rescale_ts(
                            data.encode_ctx.time_base,
                            data.output_ctx
                                .streams()
                                .get(data.stream_index)
                                .unwrap()
                                .time_base,
                        );
                        packet.set_stream_index(data.stream_index as i32);
                        data.output_ctx.interleaved_write_frame(&mut packet)?;
                    }
                    #[cfg(feature = "tracy")]
                    drop(_encode_span);
                }
                #[cfg(feature = "ffmpeg-bin")]
                EncoderBackend::Binary(data) => {
                    let stdin =
                        data.process.stdin.as_mut().ok_or_else(|| {
                            anyhow::anyhow!("Failed to get stdin of ffmpeg process")
                        })?;
                    stdin
                        .write_all(&raw_frame)
                        .map_err(|e| anyhow::anyhow!("Failed to write to ffmpeg stdin: {}", e))?;
                }
            }

            if !stop_signal.load(Ordering::SeqCst)
                && let Some(p) = encode_progress
            {
                p.pb_inc(1);
            }
            self.next_expected += 1;
        }
        Ok(())
    }

    fn finish(mut self, encode_progress: Option<&Span>) -> Result<()> {
        // After the channel is closed, process any remaining frames in the map.
        while let Some(received_frame) = self.received_frames.remove(&self.next_expected) {
            let raw_frame = if self.zstd_level.is_some() {
                zstd::decode_all(&received_frame[..]).unwrap()
            } else {
                received_frame
            };
            if raw_frame.len() != (self.width * self.height * 3) as usize {
                return Err(Error::NDArrayShape(ndarray::ShapeError::from_kind(
                    ndarray::ErrorKind::IncompatibleShape,
                )));
            }

            match &mut self.backend {
                #[cfg(feature = "embedded-ffmpeg")]
                EncoderBackend::Rsmpeg(data) => {
                    let linesize = data.input_frame.linesize_mut()[0] as usize;
                    let width_bytes = (self.width * 3) as usize;
                    let input_data = data.input_frame.data_mut()[0];

                    unsafe {
                        for i in 0..self.height as usize {
                            std::ptr::copy_nonoverlapping(
                                raw_frame.as_ptr().add(i * width_bytes),
                                input_data.add(i * linesize),
                                width_bytes,
                            );
                        }
                    }

                    data.frame.make_writable()?;

                    unsafe {
                        data.sws_ctx.scale(
                            data.input_frame.data_mut().as_ptr() as *const *const u8,
                            data.input_frame.linesize_mut().as_ptr() as *const i32,
                            0,
                            self.height as i32,
                            data.frame.data_mut().as_ptr() as *const *mut u8,
                            data.frame.linesize_mut().as_ptr() as *const i32,
                        )?;
                    }

                    data.frame
                        .set_pts((self.next_expected - self.video_begin_t) as i64);

                    #[cfg(feature = "tracy")]
                    let _encode_span = tracing::debug_span!(
                        "encode_remaining_frame",
                        frame_num = self.next_expected
                    )
                    .entered();

                    data.encode_ctx.send_frame(Some(&data.frame))?;

                    loop {
                        let mut packet = match data.encode_ctx.receive_packet() {
                            Ok(packet) => packet,
                            Err(RsmpegError::EncoderDrainError)
                            | Err(RsmpegError::EncoderFlushedError) => {
                                break;
                            }
                            Err(e) => return Err(e.into()),
                        };

                        packet.rescale_ts(
                            data.encode_ctx.time_base,
                            data.output_ctx
                                .streams()
                                .get(data.stream_index)
                                .unwrap()
                                .time_base,
                        );
                        packet.set_stream_index(data.stream_index as i32);
                        data.output_ctx.interleaved_write_frame(&mut packet)?;
                    }
                    #[cfg(feature = "tracy")]
                    drop(_encode_span);
                }
                #[cfg(feature = "ffmpeg-bin")]
                EncoderBackend::Binary(data) => {
                    let stdin =
                        data.process.stdin.as_mut().ok_or_else(|| {
                            anyhow::anyhow!("Failed to get stdin of ffmpeg process")
                        })?;
                    stdin
                        .write_all(&raw_frame)
                        .map_err(|e| anyhow::anyhow!("Failed to write to ffmpeg stdin: {}", e))?;
                }
            }

            if let Some(p) = encode_progress {
                p.pb_inc(1);
            }
            self.next_expected += 1;
        }

        match self.backend {
            #[cfg(feature = "embedded-ffmpeg")]
            EncoderBackend::Rsmpeg(mut data) => {
                data.encode_ctx.send_frame(None)?;
                loop {
                    let mut packet = match data.encode_ctx.receive_packet() {
                        Ok(packet) => packet,
                        Err(RsmpegError::EncoderDrainError)
                        | Err(RsmpegError::EncoderFlushedError) => {
                            break;
                        }
                        Err(e) => return Err(e.into()),
                    };

                    packet.rescale_ts(
                        data.encode_ctx.time_base,
                        data.output_ctx
                            .streams()
                            .get(data.stream_index)
                            .unwrap()
                            .time_base,
                    );
                    packet.set_stream_index(data.stream_index as i32);
                    data.output_ctx.interleaved_write_frame(&mut packet)?;
                }

                data.output_ctx.write_trailer()?;
            }
            #[cfg(feature = "ffmpeg-bin")]
            EncoderBackend::Binary(mut data) => {
                // Drop stdin to signal EOF
                drop(data.process.stdin.take());
                let status = data
                    .process
                    .wait()
                    .map_err(|e| anyhow::anyhow!("Failed to wait for ffmpeg process: {}", e))?;
                if !status.success() {
                    return Err(
                        anyhow::anyhow!("ffmpeg process exited with error: {}", status).into(),
                    );
                }
            }
        }
        Ok(())
    }
}

pub struct TypstVideoRenderer {
    config: RenderConfig,
    size: OnceLock<(u32, u32)>,
}

struct SpawnRenderWorkersResult(
    Vec<thread::JoinHandle<Result<()>>>,
    Vec<tempfile::NamedTempFile>,
    Vec<thread::JoinHandle<()>>,
);

impl TypstVideoRenderer {
    #[cfg_attr(feature = "tracy", instrument(skip_all))]
    pub fn new(config: RenderConfig) -> Self {
        Self {
            config,
            size: OnceLock::new(),
        }
    }

    #[cfg_attr(feature = "tracy", instrument(level = "trace", skip(self)))]
    fn render_frame(&self, t: i32) -> Result<tiny_skia::Pixmap> {
        let mut universe = self.config.universe.lock().unwrap();
        universe.increment_revision(|univ| {
            univ.set_inputs(Arc::new(LazyHash::new((self.config.f_input)(t))));
        });
        let world = universe.snapshot();
        universe.evict(10);
        typst::comemo::evict(10);
        drop(universe); // release the lock as soon as possible
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
        let pixmap = render(frame, self.config.ppi / 72.0);
        // It's okay to ignore the result here, as we don't care if the size was already set.
        // divide by 2 and multiply by 2 to ensure even dimensions, which is required by many video codecs.
        let _ = self.size.set((
            pixmap.width() as u32 / 2 * 2,
            pixmap.height() as u32 / 2 * 2,
        ));
        Ok(pixmap)
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
        let (rendering_span, encoding_span, stop_signal) = self.prepare_render()?;
        let self_arc = Arc::new(self);

        let results = self_arc.spawn_render_workers(
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

    fn prepare_render(&self) -> Result<(Span, Span, Arc<AtomicBool>)> {
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
        #[cfg(feature = "embedded-ffmpeg")]
        static FFMPEG_INIT: Once = Once::new();
        #[cfg(feature = "embedded-ffmpeg")]
        FFMPEG_INIT.call_once(|| unsafe {
            ffi::av_log_set_level(ffi::AV_LOG_ERROR as i32);
        });

        Ok((rendering_span, encoding_span, stop_signal))
    }

    fn spawn_render_workers(
        self: &Arc<Self>,
        rendering_span: Span,
        encoding_span: Span,
        stop_signal: Arc<AtomicBool>,
        error_signal: Arc<AtomicBool>,
    ) -> SpawnRenderWorkersResult {
        let begin_t = self.config.begin_t;
        let end_t = self.config.end_t;
        // each worker should handle at least one frame
        let num_render_workers = self
            .config
            .rendering_threads
            .unwrap_or_else(|| (num_cpus::get() - 2).clamp(1, (end_t - begin_t + 1) as usize));
        let num_encode_workers = self.config.encoding_threads.unwrap_or_else(|| {
            (num_cpus::get() - num_render_workers).clamp(1, (end_t - begin_t + 1) as usize)
        });
        let frames_per_encoder = (end_t - begin_t) / num_encode_workers as i32;

        let mut encoder_senders = Vec::with_capacity(num_encode_workers);
        let mut encoder_receivers = Vec::with_capacity(num_encode_workers);
        for _ in 0..num_encode_workers {
            let (tx, rx) = channel::bounded(32);
            encoder_senders.push(tx);
            encoder_receivers.push(rx);
        }

        let dispatcher = FrameDispatcher::new(encoder_senders, frames_per_encoder);

        let _re = rendering_span.enter();
        let _en = encoding_span.enter();
        // spawn render workers first to ensure encoder can get size info
        let render_threads = self.spawn_renderer_workers(
            rendering_span.clone(),
            stop_signal.clone(),
            error_signal.clone(),
            dispatcher,
            num_render_workers,
        );

        let (encode_threads, temp_files) = self.spawn_encoder_workers(
            encoding_span.clone(),
            stop_signal,
            error_signal,
            encoder_receivers,
        );

        SpawnRenderWorkersResult(encode_threads, temp_files, render_threads)
    }

    fn spawn_encoder_workers(
        self: &Arc<Self>,
        encoding_span: Span,
        stop_signal: Arc<AtomicBool>,
        error_signal: Arc<AtomicBool>,
        encoder_receivers: Vec<channel::Receiver<(i32, Vec<u8>)>>,
    ) -> (
        Vec<thread::JoinHandle<Result<()>>>,
        Vec<tempfile::NamedTempFile>,
    ) {
        let &(width, height) = self.size.wait();
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
            let zstd_level = self.config.zstd_level;
            let ffmpeg_path = self.config.ffmpeg_path.clone();
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
                        zstd_level,
                        ffmpeg_path,
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
        count: usize,
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
                        let raw_frame = frame.into_raw_vec_and_offset().0;
                        let frame_to_dispatch = if let Some(level) = self.config.zstd_level {
                            zstd::encode_all(&raw_frame[..], level).unwrap()
                        } else {
                            raw_frame
                        };
                        dispatcher.dispatch(t, frame_to_dispatch, begin_t);
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
        SpawnRenderWorkersResult (
            encode_threads,
            temp_files,
            render_threads,
        ): SpawnRenderWorkersResult,
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

        merge_mp4_files(
            &input_paths_str,
            &final_output_path,
            self.config.ffmpeg_path.as_deref(),
        )
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
        fps: u32,
        output_path: &str,
        begin_t: i32,
        video_begin_t: i32,
        encode_progress: Option<Span>,
        ffmpeg_options: HashMap<String, String>,
        zstd_level: Option<i32>,
        ffmpeg_path: Option<String>,
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
            zstd_level,
            ffmpeg_path,
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
