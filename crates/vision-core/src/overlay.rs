//! Compositing one processor's result back over the original video.
//!
//! [`Overlay`] wraps any [`Processor`] and blends its output with the input
//! frame, so you can watch an edge map, a gradient, or a mask *on top of* the
//! live image instead of in place of it. It composes -- the wrapped processor
//! doesn't know or care it's being overlaid, and `Overlay<P>` is itself a
//! [`Processor`], so it drops straight into [`run`](crate::Processor).
//!
//! ## What it's for
//!
//! Use this for processors that render a **standalone result image** -- one that
//! fills the canvas with its own output, like [`SobelEdges`](crate::SobelEdges).
//! A processor that instead blits the video in as a background and draws sparse
//! marks on top (points, boxes, flow vectors) is *already* composited over the
//! video; wrapping it in `Overlay` would blend the video with itself and gains
//! nothing.

use crate::{Canvas, Frame, Processor};

/// How [`Overlay`] combines the original pixel with the wrapped processor's
/// result pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Blend {
    /// Linear cross-fade: `(1 - a) * original + a * result`, per channel. `a` is
    /// clamped to `0.0..=1.0`; `0.0` shows the video, `1.0` shows the result.
    Alpha(f32),
    /// Per-channel max. Bright result pixels show through while the rest of the
    /// video passes untouched -- the natural choice for an edge map (bright edges
    /// on a dark field) laid over the image.
    Lighten,
    /// Per-channel saturating add: the result brightens the original.
    Add,
}

impl Default for Blend {
    fn default() -> Self {
        Blend::Alpha(0.5)
    }
}

impl Blend {
    /// Combine one original pixel with one result pixel.
    #[inline]
    fn mix(self, orig: [u8; 3], res: [u8; 3]) -> [u8; 3] {
        match self {
            Blend::Alpha(a) => {
                let a = a.clamp(0.0, 1.0);
                let f = |o: u8, r: u8| {
                    ((1.0 - a) * o as f32 + a * r as f32).round().clamp(0.0, 255.0) as u8
                };
                [f(orig[0], res[0]), f(orig[1], res[1]), f(orig[2], res[2])]
            }
            Blend::Lighten => [
                orig[0].max(res[0]),
                orig[1].max(res[1]),
                orig[2].max(res[2]),
            ],
            Blend::Add => [
                orig[0].saturating_add(res[0]),
                orig[1].saturating_add(res[1]),
                orig[2].saturating_add(res[2]),
            ],
        }
    }
}

/// Runs an inner [`Processor`] and blends its result with the original frame.
/// See the [module docs](self) for when this is the right tool.
pub struct Overlay<P> {
    inner: P,
    blend: Blend,
}

impl<P> Overlay<P> {
    /// Wrap `inner`, compositing its output over the video with `blend`.
    pub fn new(inner: P, blend: Blend) -> Self {
        Overlay { inner, blend }
    }
}

impl<P: Default> Default for Overlay<P> {
    fn default() -> Self {
        Overlay {
            inner: P::default(),
            blend: Blend::default(),
        }
    }
}

impl<P: Processor> Processor for Overlay<P> {
    fn process(&mut self, input: &Frame<'_>, out: &mut Canvas) {
        // The inner pass fills `out` with its result. The original is still in
        // `input` (untouched -- processors only write `out`), so we can composite
        // the two in place: read result from `out`, original from `input`, write
        // the blend back to `out`. Single-pixel op, no neighbor reads, so writing
        // `out` while reading it is safe.
        self.inner.process(input, out);

        let w = out.width().min(input.width);
        let h = out.height().min(input.height);
        for y in 0..h {
            for x in 0..w {
                let orig = input.sample_rgb(x, y);
                let res = out.pixel(x, y);
                out.set_pixel(x, y, self.blend.mix(orig, res));
            }
        }
    }
}
