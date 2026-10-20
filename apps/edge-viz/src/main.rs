// edge-viz — Sobel edge detector over a video file.
//
// A reference demo of the per-frame harness: the "algorithm" is vision-core's
// SobelEdges processor, and vision-app::run supplies the source pump, window,
// playback pacing, and transport controls. Swap SobelEdges for any other
// Processor to turn this into a different demo.
//
// Run:  cargo run --release -p edge-viz -- /path/to/video.mp4

use std::io;

use vision_core::{SobelEdges, Stats};
use vision_io::VideoFile;

fn main() {
    if let Err(e) = run() {
        eprintln!("edge-viz: {e}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: edge-viz <video-file>");
            std::process::exit(2);
        }
    };

    let video = VideoFile::open(&path)?;
    eprintln!(
        "{}: {}x{} @ {:.2} fps",
        path,
        video.width(),
        video.height(),
        video.fps()
    );

    vision_app::run(
        video,
        Stats::new(SobelEdges::default()),
        &format!("Edge Viz — {path}"),
    )
}
