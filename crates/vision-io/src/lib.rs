//! vision-io — frame *sources* and sinks that touch the outside world.
//!
//! Today: V4L2 webcam capture and video-file decoding (via FFmpeg). Both produce
//! the [`vision_core::Frame`] type through the common [`FrameSource`] trait, so a
//! consumer never learns where a frame came from. Intended to grow (ROADMAP.md
//! §2) into record/replay of raw frames + timestamps, PPM/PNG image I/O, and
//! dataset loaders (TUM / KITTI / EuRoC) -- each a new `FrameSource`.

pub mod capture;
pub mod source;
pub mod video;

pub use capture::V4l2Capture;
pub use source::FrameSource;
pub use video::VideoFile;
