use crate::video::error::Result;
use tracing::instrument;
use std::path::Path;
use video_rs::{MuxerBuilder, Reader, Writer};

#[cfg_attr(feature = "tracy", instrument(level = "debug", skip_all))]
pub fn merge_mp4_files(input_files: Vec<&str>, output_file: &str) -> Result<String> {
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