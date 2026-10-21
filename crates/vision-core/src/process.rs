//! The per-frame algorithm interface.
//!
//! A [`Processor`] is "an algorithm you can watch run": it takes one input
//! [`Frame`] and draws its result into an owned [`Canvas`] that the harness then
//! displays. This is the contract every demo in the repo plugs into -- optical
//! flow, feature tracks, a VO trajectory -- so the capture/playback/transport
//! machinery is written once and each algorithm is just an `impl Processor`.

use crate::{Canvas, Frame};

/// Transforms one input frame into one visualized output frame.
///
/// `&mut self` is deliberate: most vision algorithms are stateful across frames
/// (optical flow and VO need the *previous* frame / pyramid / pose), so a
/// processor keeps that state in itself and updates it each call. A stateless
/// effect (like an edge filter) simply ignores the `&mut`.
///
/// Typical body: optionally [`Canvas::blit_frame`] the input in as a background,
/// then overlay results with the canvas draw primitives -- or overwrite every
/// pixel, as an edge/colormap pass does.
pub trait Processor {
    fn process(&mut self, input: &Frame<'_>, out: &mut Canvas);
}

/// Lets a runtime-chosen processor (`Box<dyn Processor>`) be used anywhere a
/// concrete one is, including as the inner of [`Stats`](crate::Stats) or
/// [`Overlay`](crate::Overlay).
impl<P: Processor + ?Sized> Processor for Box<P> {
    fn process(&mut self, input: &Frame<'_>, out: &mut Canvas) {
        (**self).process(input, out)
    }
}
