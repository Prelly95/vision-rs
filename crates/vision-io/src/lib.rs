//! vision-io — frame *sources* and sinks that touch the outside world.
//!
//! Today: V4L2 webcam capture. Intended to grow (ROADMAP.md §2) into record/
//! replay of raw frames + timestamps, PPM/PNG image I/O, and dataset loaders
//! (TUM / KITTI / EuRoC). Everything here produces or consumes the
//! [`vision_core::Frame`] type, so downstream crates never learn where a frame
//! came from.

pub mod capture;

pub use capture::{CapturedFrame, V4l2Capture};
