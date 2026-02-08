use crate::video::error::Result;

#[cfg(feature = "embedded-ffmpeg")]
use rsmpeg::avformat::{AVFormatContextInput, AVFormatContextOutput};
#[cfg(feature = "embedded-ffmpeg")]
use std::ffi::CString;

#[cfg(feature = "ffmpeg-bin")]
use std::process::Command;

#[cfg(not(any(feature = "embedded-ffmpeg", feature = "ffmpeg-bin")))]
compile_error!("At least one of 'embedded-ffmpeg' or 'ffmpeg-bin' features must be enabled");

pub fn merge_mp4_files(
    input_files: &Vec<&str>,
    output_file: &str,
    ffmpeg_path: Option<&str>,
) -> Result<String> {
    #[cfg(feature = "ffmpeg-bin")]
    if let Some(path) = ffmpeg_path {
        return merge_with_binary(input_files, output_file, path);
    }

    #[cfg(feature = "embedded-ffmpeg")]
    {
        return merge_with_rsmpeg(input_files, output_file);
    }

    #[cfg(all(feature = "ffmpeg-bin", not(feature = "embedded-ffmpeg")))]
    {
        return merge_with_binary(input_files, output_file, "ffmpeg");
    }

    #[cfg(all(not(feature = "ffmpeg-bin"), not(feature = "embedded-ffmpeg")))]
    {
        // This is technically unreachable due to the compile_error! above,
        // but needed for type checking if the compiler doesn't stop immediately.
        panic!("No ffmpeg feature enabled");
    }
}

#[cfg(feature = "embedded-ffmpeg")]
fn merge_with_rsmpeg(input_files: &Vec<&str>, output_file: &str) -> Result<String> {
    let output_file_cstr = CString::new(output_file).unwrap();
    let mut output_ctx = AVFormatContextOutput::create(output_file_cstr.as_c_str(), None)?;

    let mut streams_created = false;

    for input_file in input_files {
        let input_file_cstr = CString::new(*input_file).unwrap();
        let mut input_ctx =
            AVFormatContextInput::open(input_file_cstr.as_c_str(), None, &mut None)?;
        input_ctx.dump(0, input_file_cstr.as_c_str())?;

        // Initialize output streams based on the first input file
        if !streams_created {
            for stream in input_ctx.streams() {
                let mut new_stream = output_ctx.new_stream();
                new_stream.set_codecpar(stream.codecpar().clone().into());
            }
            output_ctx.write_header(&mut None)?;
            streams_created = true;
        }

        while let Some(mut packet) = input_ctx.read_packet()? {
            let stream_index = packet.stream_index as usize;
            let input_stream = input_ctx.streams().get(stream_index).unwrap();
            let output_stream = output_ctx.streams().get(stream_index).unwrap();

            // Rescale packet timestamps from Input Timebase -> Output Timebase
            // This modifies pts, dts, and duration.
            packet.rescale_ts(input_stream.time_base, output_stream.time_base);

            packet.set_stream_index(stream_index as i32);
            output_ctx.interleaved_write_frame(&mut packet)?;
        }
    }

    output_ctx.write_trailer()?;

    Ok(output_file.to_string())
}

#[cfg(feature = "ffmpeg-bin")]
fn merge_with_binary(
    input_files: &Vec<&str>,
    output_file: &str,
    ffmpeg_path: &str,
) -> Result<String> {
    use std::io::Write;

    // Create a temporary file to list inputs for ffmpeg concat demuxer
    let mut list_file = tempfile::Builder::new().suffix(".txt").tempfile().map_err(|e| anyhow::anyhow!(e))?;

    for file in input_files {
        // Simple escaping for single quotes. Ideally we should do more robust escaping.
        // The path is put inside single quotes.
        let escaped_path = file.replace("'", "'\\''");
        writeln!(list_file, "file '{}'", escaped_path).map_err(|e| anyhow::anyhow!(e))?;
    }
    list_file.flush().map_err(|e| anyhow::anyhow!(e))?;

    let status = Command::new(ffmpeg_path)
        .arg("-loglevel")
        .arg("error")
        .arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(list_file.path())
        .arg("-c")
        .arg("copy")
        .arg("-y") // Overwrite output
        .arg(output_file)
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to execute ffmpeg: {}", e))?;

    if !status.success() {
        return Err(anyhow::anyhow!("ffmpeg exited with status: {}", status).into());
    }

    Ok(output_file.to_string())
}
