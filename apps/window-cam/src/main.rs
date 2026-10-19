// window-cam — live webcam viewer in a window, via the vision-app harness.
//
// The webcam counterpart of video-player: same harness and Passthrough
// processor, but the frame source is V4l2Capture instead of a file. Both
// implement vision_io::FrameSource, so the harness treats them identically --
// the only thing that changes is which source you hand to `run`. (A webcam has
// no inherent frame rate, so FrameSource::target_fps is None for it and the
// harness paces at its default; that's why there's no `fps()` to print here.)
//
// Run:  cargo run --release -p window-cam

use std::io;

use vision_core::{Blend, Overlay, SobelEdges};
use vision_io::V4l2Capture;
use vision_app::Passthrough;

fn main() {
    if let Err(e) = run() {
        eprintln!("window-cam: {e}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let capture = V4l2Capture::open("/dev/video0", 640, 480)?;
    eprintln!(
        "negotiated format: {}x{}, bytesperline={}",
        capture.width(),
        capture.height(),
        capture.bytes_per_line()
    );

    // Sobel edges composited over the live video: `Lighten` lets the bright
    // edge lines show through while the rest of the frame passes untouched.
    let processor = Overlay::new(SobelEdges::default(), Blend::Lighten);
    vision_app::run(capture, processor, "Window Cam")
}