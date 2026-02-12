use thiserror::Error;
#[cfg(feature = "typst-lib")]
use typst::{diag::SourceDiagnostic, ecow::EcoVec};

#[derive(Debug, Error)]
pub enum Error {
    #[cfg(feature = "typst-lib")]
    #[error("Typst compilation failed: {0:?}")]
    TypstCompilation(EcoVec<SourceDiagnostic>),

    #[cfg(feature = "typst-bin")]
    #[error("Typst binary compilation failed: {0}")]
    TypstBinary(String),

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

    #[error("Vello error: {0}")]
    Vello(anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
