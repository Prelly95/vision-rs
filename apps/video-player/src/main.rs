// video-player — play a video file in a window, with transport controls.
//
// The sibling of window-cam: same window sink, but the frame *source* is a file
// (vision-io::VideoFile) instead of the webcam. It is now a thin wrapper over
// the vision-app harness with the identity processor (Passthrough) -- i.e. a
// plain viewer. Swapping Passthrough for any other Processor turns this into an
// algorithm demo (see the edge-viz app for one).
//
// It exists to make development deterministic (ROADMAP §1): run on the same
// recorded frames every time, and single-step through them.
//
// Run:  cargo run --release -p video-player -- /path/to/video.mp4

use std::io;

use vision_app::Passthrough;
use vision_io::VideoFile;

fn main() {
    if let Err(e) = run() {
        eprintln!("video-player: {e}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    // argv[1] is the video path; anything FFmpeg can demux works (mp4/mov/mkv/…).
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: video-player <video-file>");
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

    vision_app::run(video, Passthrough, &format!("Video Player — {path}"))
}
