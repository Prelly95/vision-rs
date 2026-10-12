// Run:  cargo run --release -p window-cam

use vision_io::V4l2Capture;
use vision_viz::{Display, WindowDisplay};

fn main() {
    if let Err(e) = run() {
        eprintln!("window-cam: {e}");
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

    let mut display = WindowDisplay::new(
        "Window Cam",
        capture.width() as usize,
        capture.height() as usize,
    )?;

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
