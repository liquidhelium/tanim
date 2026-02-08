use crate::video::error::Result;
use rsmpeg::avformat::{AVFormatContextInput, AVFormatContextOutput};
use rsmpeg::ffi;
use std::ffi::CString;

pub fn merge_mp4_files(input_files: &Vec<&str>, output_file: &str) -> Result<String> {
    let output_file_cstr = CString::new(output_file).unwrap();
    let mut output_ctx = AVFormatContextOutput::create(output_file_cstr.as_c_str(), None)?;

    let mut streams_created = false;

    for input_file in input_files {
        let input_file_cstr = CString::new(*input_file).unwrap();
        let mut input_ctx = AVFormatContextInput::open(input_file_cstr.as_c_str(), None, &mut None)?;
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