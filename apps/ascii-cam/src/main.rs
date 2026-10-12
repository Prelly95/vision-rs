// ascii-cam — live webcam viewer in the terminal.
//
// The original single-file ascii webcam demo, now a thin binary over the
// workspace crates. It
// wires one frame source to one sink and pumps frames between them; the two
// halves meet only at `vision_core::Frame`:
//   * capture  -- vision-io::V4l2Capture produces frames
//   * display  -- vision-viz::TerminalDisplay presents them
//
// Swap in a different `Display` (a real window, say) without touching capture, or
// a different frame source without touching the renderer.
//
// Run:  cargo run --release -p ascii-cam

use vision_io::V4l2Capture;
use vision_viz::{Display, TerminalDisplay};

fn main() {
    if let Err(e) = run() {
        // Leave the terminal usable (TerminalDisplay's Drop already un-hides the
        // cursor) and report what went wrong.
        eprintln!("ascii-cam: {e}");
        std::process::exit(1);
    }
}

fn run() -> std::io::Result<()> {
    let mut capture = V4l2Capture::open("/dev/video0", 640, 480)?;
    eprintln!(
        "negotiated format: {}x{}, bytesperline={}",
        capture.width(),
        capture.height(),
        capture.bytes_per_line()
    );

    let mut display = TerminalDisplay::new();

    // Capture loop, unthrottled: as fast as camera + display allow. Ctrl+C to
    // exit; Drop on `capture` stops streaming and releases buffers.
    loop {
        let captured = capture.next_frame()?;
        if display.show(&captured.frame())?.is_break() {
            break;
        }
        // `captured` drops here -> buffer is re-queued to the driver.
    }
    Ok(())
}
