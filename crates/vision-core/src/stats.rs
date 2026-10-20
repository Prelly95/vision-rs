//! Frame statistics printed to the terminal while an app runs.
//!
//! [`Stats`] wraps any [`Processor`], times it, and prints a one-line summary
//! about once a second. Like [`Overlay`](crate::Overlay) it composes -- the
//! wrapped processor doesn't know it's being measured, and `Stats<P>` is itself
//! a [`Processor`]. Where you wrap decides what is measured: outermost covers
//! the whole chain, innermost covers just the algorithm.
//!
//! It reports two numbers:
//!
//! - **fps** -- how often `process` is called, i.e. the pipeline rate the user
//!   sees (capture, processing and display together).
//! - **process** -- mean time spent inside the wrapped processor per frame.
//!
//! Note that `process` is only called when a *new* frame is produced, so pausing
//! or replaying cached history shows up as a dip in fps.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use crate::{Canvas, Frame, Passthrough, Processor};

/// How often the summary line is printed.
const REPORT_INTERVAL: Duration = Duration::from_millis(200);

/// Runs an inner [`Processor`] and prints fps / processing time to stderr.
/// See the [module docs](self).
pub struct Stats<P> {
    inner: P,
    /// Start of the current reporting window.
    window_start: Option<Instant>,
    /// Frames seen in the current window.
    frames: u32,
    /// Time spent inside `inner.process` in the current window.
    process_total: Duration,
}

impl<P> Stats<P> {
    /// Wrap `inner`, reporting its stats to the terminal.
    pub fn new(inner: P) -> Self {
        Stats {
            inner,
            window_start: None,
            frames: 0,
            process_total: Duration::ZERO,
        }
    }
}

impl Stats<Passthrough> {
    /// Stats over a plain viewer: the video unchanged, plus the fps readout.
    pub fn passthrough() -> Self {
        Stats::new(Passthrough)
    }
}

impl<P: Processor> Processor for Stats<P> {
    fn process(&mut self, input: &Frame<'_>, out: &mut Canvas) {
        let start = Instant::now();
        self.inner.process(input, out);
        let done = Instant::now();

        self.process_total += done - start;
        self.frames += 1;

        // The window opens at the first frame's start so that frame counts fully.
        let window_start = *self.window_start.get_or_insert(start);
        let elapsed = done - window_start;
        if elapsed >= REPORT_INTERVAL {
            let fps = self.frames as f64 / elapsed.as_secs_f64();
            let process_ms = self.process_total.as_secs_f64() * 1000.0 / self.frames as f64;
            // `\r` rewrites the line in place; the trailing spaces clear leftovers.
            eprint!("\rfps {fps:6.1} | process {process_ms:7.2} ms   ");
            let _ = io::stderr().flush();

            self.window_start = Some(done);
            self.frames = 0;
            self.process_total = Duration::ZERO;
        }
    }
}
