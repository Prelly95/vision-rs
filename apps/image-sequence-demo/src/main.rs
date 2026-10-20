use std::io;
use std::path::Path;

use vision_core::{Blend, Overlay, SobelEdges, Stats};
use vision_io::{FrameSource, ImageSequence};

const DATASETS: &str = "datasets/kitti/2011_09_26/2011_09_26_drive_0011_sync";

fn main() {
    if let Err(e) = run() {
        eprintln!("image-sequence-demo {e}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let gray_dir = format!("{DATASETS}/image_00/data");
    if !Path::new(&gray_dir).is_dir() {
        eprintln!("skipping: {gray_dir} not found");
        return Err(io::Error::other(format!(
            "Directory not found - {}",
            gray_dir
        )));
    }

    let gray_sequence = ImageSequence::open(&gray_dir).unwrap();


    eprintln!(
        "negotiated format: {}x{}, target fps={}",
        gray_sequence.width(),
        gray_sequence.height(),
        gray_sequence.target_fps().unwrap_or(0.0)
    );

    // Sobel edges composited over the live video: `Lighten` lets the bright
    // edge lines show through while the rest of the frame passes untouched.
    let processor = Overlay::new(Stats::new(SobelEdges::default()), Blend::Lighten);

    // let processor = Stats::passthrough();
    // let processor = Stats::new(SobelEdges::default());
    vision_app::run(gray_sequence, processor, "Window Cam")
}
