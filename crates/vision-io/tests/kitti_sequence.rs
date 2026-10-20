//! Smoke test against a local KITTI drive. Skipped when the dataset is absent
//! (`datasets/` is git-ignored).

use vision_core::PixelFormat;
use vision_io::{FrameSource, ImageSequence};

const DRIVE: &str = "../../datasets/kitti/2011_09_26/2011_09_26_drive_0011_sync";

#[test]
fn reads_gray_and_color_cameras() {
    let gray_dir = format!("{DRIVE}/image_00/data");
    if !std::path::Path::new(&gray_dir).is_dir() {
        eprintln!("skipping: {gray_dir} not found");
        return;
    }

    let mut gray = ImageSequence::open(&gray_dir).unwrap();
    assert_eq!((gray.width(), gray.height()), (1242, 375));
    let n = gray.len();
    let f = gray.next_frame().unwrap().unwrap();
    assert_eq!(f.format, PixelFormat::Gray8);
    assert_eq!(gray.frame_index(), Some(0));
    assert!((gray.target_fps().unwrap() - 10.0).abs() < 0.5);

    let mut color = ImageSequence::open(format!("{DRIVE}/image_02/data")).unwrap();
    assert_eq!(color.len(), n);
    assert_eq!(color.next_frame().unwrap().unwrap().format, PixelFormat::Rgb24);

    let mut count = 1;
    while gray.next_frame().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, n);
}
