//! The identity processor.

use crate::{Canvas, Frame, Processor};

/// The identity [`Processor`]: show each input frame unchanged. Turns a harness
/// `run` into a plain viewer (what video-player is) and serves as the minimal
/// example of the trait.
#[derive(Default)]
pub struct Passthrough;

impl Processor for Passthrough {
    fn process(&mut self, input: &Frame<'_>, out: &mut Canvas) {
        out.blit_frame(input);
    }
}
