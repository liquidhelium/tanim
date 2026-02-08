use thiserror::Error;
use typst::{diag::SourceDiagnostic, ecow::EcoVec};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Typst compilation failed: {0:?}")]
    TypstCompilation(EcoVec<SourceDiagnostic>),

    #[error("No pages in the document")]
    NoPages,

    #[error("Failed to create ndarray from shape: {0}")]
    NDArrayShape(#[from] ndarray::ShapeError),

    #[cfg(feature = "embedded-ffmpeg")]
    #[error("Video encoding failed: {0}")]
    VideoEncoding(#[from] rsmpeg::error::RsmpegError),

    #[cfg(feature = "ffmpeg-bin")]
    #[error("FFmpeg binary error: {0}")]
    FFmpegBinary(#[from] anyhow::Error),

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

    #[error("Failed to create sws context")]
    SwsContextCreation,
}

pub type Result<T> = std::result::Result<T, Error>;
