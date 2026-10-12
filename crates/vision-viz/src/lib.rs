//! vision-viz — frame *sinks*: anything that presents a [`Frame`] to a human.

pub mod terminal;
pub mod window;

pub use terminal::TerminalDisplay;
pub use window::WindowDisplay;

use std::io;
use std::ops::ControlFlow;

use vision_core::Frame;

/// A sink that presents frames. One `show` call renders one frame; the display
/// owns whatever state it needs across calls (diff buffers, an open window, an
/// fps estimate).
pub trait Display {
    /// Present a single frame. `Err` means the underlying output failed (e.g. a
    /// broken pipe on stdout). `Ok(ControlFlow::Break(()))` means the user asked
    /// to stop (e.g. closed the window) -- a clean exit, not a failure --
    /// otherwise `Ok(ControlFlow::Continue(()))`.
    fn show(&mut self, frame: &Frame<'_>) -> io::Result<ControlFlow<()>>;
}
