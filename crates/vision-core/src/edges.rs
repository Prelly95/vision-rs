//! Sobel edge detection -- the first of the shared image primitives.
//!
//! [`SobelEdges`] packages it as a ready-to-run [`Processor`] that writes the
//! gradient magnitude as a grayscale image. Internally it decodes each frame to
//! a [`LumaPlane`] once and runs the kernel over that (see [`sobel_magnitude_on_plane`]),
//! rather than sampling colour per pixel. The frame-sampling kernel
//! [`sobel_magnitude_at`] is kept for callers that want a one-off magnitude at a
//! single pixel without materializing a plane.

use crate::{Canvas, Frame, LumaPlane, Processor};

/// Sobel gradient magnitude at one interior pixel of a [`LumaPlane`]
/// (`1 <= x < width-1`, `1 <= y < height-1`). Each neighbour is a single cheap
/// indexed read, so this is the kernel a full-image sweep should use. Returns
/// `sqrt(gx^2 + gy^2)`, not yet clamped to 8-bit range.
#[inline]
pub fn sobel_magnitude(luma: &LumaPlane, x: u32, y: u32) -> f64 {
    let l = |x: u32, y: u32| luma.at(x, y) as i32;
    let gx = -l(x - 1, y - 1) - 2 * l(x - 1, y) - l(x - 1, y + 1)
        + l(x + 1, y - 1)
        + 2 * l(x + 1, y)
        + l(x + 1, y + 1);
    let gy = -l(x - 1, y - 1) - 2 * l(x, y - 1) - l(x + 1, y - 1)
        + l(x - 1, y + 1)
        + 2 * l(x, y + 1)
        + l(x + 1, y + 1);
    ((gx * gx + gy * gy) as f64).sqrt()
}

/// A [`Processor`] that renders Sobel edge magnitude as a grayscale image. Holds
/// a reusable [`LumaPlane`] so the per-frame conversion allocates at most once.
#[derive(Default)]
pub struct SobelEdges {
    luma: LumaPlane,
}

impl Processor for SobelEdges {
    fn process(&mut self, input: &Frame<'_>, out: &mut Canvas) {
        // One colour decode of the whole frame, then the sweep reads luma only.
        input.decode_luma_into(&mut self.luma);

        // Interior only: border pixels lack a full 3x3 neighborhood. They were
        // cleared to black at construction and we never overwrite them. Clamp to
        // whichever of plane/canvas is smaller so every read and write is in range.
        let w = self.luma.width().min(out.width());
        let h = self.luma.height().min(out.height());
        for y in 1..h.saturating_sub(1) {
            for x in 1..w.saturating_sub(1) {
                let v = (sobel_magnitude(&self.luma, x, y) as i32).clamp(0, 255) as u8;
                out.set_pixel(x, y, [v, v, v]);
            }
        }
    }
}
