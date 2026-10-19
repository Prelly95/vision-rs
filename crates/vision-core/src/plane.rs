//! Single-channel image buffers -- the owned substrate compute runs on.
//!
//! A [`Frame`] is a *borrowed*, format-polymorphic view of captured pixels: the
//! right type at the capture/display boundary, but the wrong one to sample in a
//! tight inner loop, where its per-pixel format dispatch and colour decode cost
//! real time. A [`LumaPlane`] is the complement: an owned, tightly packed
//! grayscale buffer you convert *into* once per frame (see
//! [`Frame::decode_luma_into`]) and then index cheaply as many times as an
//! algorithm needs. Gradient operators, pyramids, and corner detectors want
//! luma, not colour -- so this, not `Frame`, is what they read.

use crate::Frame;

/// An owned, tightly packed 8-bit grayscale image: `width * height` bytes, one
/// per pixel, rows back-to-back. Create once and reuse across frames -- the
/// decode methods resize it in place, so the allocation is paid at most once.
#[derive(Clone, Default)]
pub struct LumaPlane {
    width: u32,
    height: u32,
    /// `width * height` bytes, one luma sample per pixel.
    data: Vec<u8>,
}

impl LumaPlane {
    /// A new `width`x`height` plane, cleared to black.
    pub fn new(width: u32, height: u32) -> Self {
        LumaPlane {
            width,
            height,
            data: vec![0; width as usize * height as usize],
        }
    }

    /// Plane width in pixels.
    pub fn width(&self) -> u32 { self.width }
    /// Plane height in pixels.
    pub fn height(&self) -> u32 { self.height }

    /// Resize the plane to `width`x`height`, reusing the existing allocation when
    /// it is large enough. Contents after a resize are unspecified; callers that
    /// resize are expected to overwrite every pixel (as the decoders do).
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.data.resize(width as usize * height as usize, 0);
    }

    /// Read one pixel. Interior-only: panics out of bounds, matching its role as
    /// the cheap accessor in inner loops that already clamp to `1..dim-1`.
    #[inline]
    pub fn at(&self, x: u32, y: u32) -> u8 {
        self.data[(y * self.width + x) as usize]
    }

    /// The backing bytes, row-major. Useful for whole-plane passes.
    pub fn as_slice(&self) -> &[u8] { &self.data }
    /// The backing bytes, mutable -- for decoders filling the plane in place.
    pub fn as_mut_slice(&mut self) -> &mut [u8] { &mut self.data }
}

impl Frame<'_> {
    /// Convert this whole frame to luma in a single pass, writing into `out`
    /// (resized to match). This is the fast replacement for calling
    /// [`Frame::sample_rgb`] per pixel in a gradient loop: it dispatches on the
    /// pixel format *once* here rather than once per sample, and for YUYV it
    /// takes the Y byte directly instead of decoding to RGB and back.
    pub fn decode_luma_into(&self, out: &mut LumaPlane) {
        out.resize(self.width, self.height);
        let (w, h) = (self.width as usize, self.height as usize);
        let stride = self.bytes_per_line as usize;
        let src = self.data;
        let dst = out.as_mut_slice();

        match self.format {
            // YUYV: `[Y0, U, Y1, V]` per pixel pair, so the luma byte for pixel x
            // sits at `2*x` within the row -- already what we want, no colour math.
            crate::PixelFormat::Yuyv => {
                for y in 0..h {
                    let row = y * stride;
                    let out_row = y * w;
                    for x in 0..w {
                        dst[out_row + x] = src.get(row + 2 * x).copied().unwrap_or(0);
                    }
                }
            }
            // RGB24: collapse each `[R, G, B]` to Rec.601 luma, matching the
            // weights `edges::luma` used when it sampled through `sample_rgb`.
            crate::PixelFormat::Rgb24 => {
                for y in 0..h {
                    let row = y * stride;
                    let out_row = y * w;
                    for x in 0..w {
                        let base = row + x * 3;
                        let r = src.get(base).copied().unwrap_or(0) as u32;
                        let g = src.get(base + 1).copied().unwrap_or(0) as u32;
                        let b = src.get(base + 2).copied().unwrap_or(0) as u32;
                        dst[out_row + x] = ((77 * r + 150 * g + 29 * b) >> 8) as u8;
                    }
                }
            }
        }
    }
}
