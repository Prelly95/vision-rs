//! The common interface over everything that yields frames.
//!
//! The webcam ([`V4l2Capture`]) and a video file ([`VideoFile`]) produce frames
//! in very different ways, but a consumer -- the playback harness, an algorithm
//! demo -- shouldn't care which it's fed. `FrameSource` is that seam: implement
//! it and you drop into the harness unchanged. Future sources (record/replay,
//! dataset loaders) implement the same trait.
//!
//! [`V4l2Capture`]: crate::V4l2Capture
//! [`VideoFile`]: crate::VideoFile

use std::io;

use vision_core::Frame;

/// A stream of frames, pulled one at a time.
///
/// The returned [`Frame`] borrows the source until the next call, so only one
/// frame is ever in flight -- the lifetime that's safe to read a live capture
/// buffer or a reused decode buffer.
pub trait FrameSource {
    /// Advance to and return the next frame, or `Ok(None)` when the stream ends
    /// (a file reaching EOF). A live source like a webcam never returns `None`.
    fn next_frame(&mut self) -> io::Result<Option<Frame<'_>>>;

    /// Frame width in pixels.
    fn width(&self) -> u32;
    /// Frame height in pixels.
    fn height(&self) -> u32;

    /// The rate the source should be played back at, if it has an inherent one
    /// (a video file does; a live webcam doesn't). The harness uses this to pace
    /// playback; `None` means "run as fast as allowed".
    fn target_fps(&self) -> Option<f64> {
        None
    }
}

/// Lets a runtime-chosen source (`Box<dyn FrameSource>`) drop into generic code
/// such as the harness `run`.
impl<S: FrameSource + ?Sized> FrameSource for Box<S> {
    fn next_frame(&mut self) -> io::Result<Option<Frame<'_>>> {
        (**self).next_frame()
    }
    fn width(&self) -> u32 {
        (**self).width()
    }
    fn height(&self) -> u32 {
        (**self).height()
    }
    fn target_fps(&self) -> Option<f64> {
        (**self).target_fps()
    }
}
