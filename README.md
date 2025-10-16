# Tanim

The idea of creating animations using typst comes from [WenSim](https://github.com/WenSimEHRP). His bash script renders each frame as a .png file, then merge them into a video. This project can be seen as a statically linked, concurrent version of his script.

## CLI

Tanim CLI is a command-line tool for creating animations and videos using [Typst](https://typst.app/). It allows you to render Typst documents into a video file, by varying a numerical variable over a range of frames.

## Features

*   Render Typst documents to video.
*   Customize frame range, resolution (PPI), and video encoder options.
*   Pass a variable to your Typst file that changes with each frame.

## Installation

You can install Tanim CLI using `cargo`:

```bash
cargo install tanim-cli
```

## Usage

The basic command to render a video is:

```bash
tanim-cli [OPTIONS] <input>
```

Where `<input>` is your Typst file.

### Example

1.  Create a Typst file named `animation.typ`:

    ```typst
    #let t = sys.inputs.at("t", default: 300)
    
    #rect(width: 100%, height: 100%, fill: rgb("f0f0f0"))
    #text(16pt, "Frame: " + str(t))
    ```

2.  Run `tanim-cli` to render the animation:

    ```bash
    tanim-cli --frames 0..=120 --output animation.mp4 animation.typ
    ```

This will create a video file `animation.mp4` with 121 frames, where the text changes from "Frame: 0" to "Frame: 120".

## Examples

Here are a couple of examples of what you can create with Tanim CLI:

### Manim-style Text Animation


https://github.com/user-attachments/assets/8669607a-e385-4fcf-a33b-53627a5d3512


### Physics Simulation


https://github.com/user-attachments/assets/ee742e51-c172-4eac-90c6-1d2aa02ac85e


## Command-line Arguments

Here are the available command-line arguments:

*   `<input>`: The path to the input Typst file.
*   `--output <output>`: The path to the output video file. Defaults to `out.mp4`.
*   `--frames <frames>`: The range of frames to render (e.g., `0..=240`). Defaults to `0..=240`.
*   `--ppi <ppi>`: The pixels per inch for rendering. Defaults to `150.0`.
*   `--variable <variable>`: The name of the variable to pass to the Typst file. Defaults to `t`.
*   `--codec <codec>`: The video codec to use. Defaults to `libx264`.
*   `--crf <crf>`: The Constant Rate Factor for quality control (lower is better).
*   `--preset <preset>`: The encoding preset. Defaults to `medium`.

For more details on all available options, run `tanim-cli --help`.

## Building from Source

To build Tanim CLI from source:

1.  Clone the repository:
    ```bash
    git clone https://github.com/liquidhelium/tanim-cli.git
    ```
2.  Build the project:
    ```bash
    cargo build --release
    ```
The executable will be located at `target/release/tanim-cli`.
