//! An owned RGB image buffer: the surface algorithms draw their results onto.
//!
//! [`Frame`] is a *borrowed* view of someone else's pixels (a driver buffer, a
//! decoded video frame) -- read-only and tied to a short lifetime. A [`Canvas`]
//! is the opposite: it *owns* its pixels, so a processor can write into it and a
//! display can present it. The two meet through [`Canvas::as_frame`], which lends
//! the canvas out as a `Frame` for the display side to sample -- so a canvas is
//! both a drawing target and, for free, a displayable frame.
//!
//! Pixels are packed [`PixelFormat::Rgb24`], three bytes per pixel, rows
//! back-to-back. The draw primitives here are the starter set the ROADMAP calls
//! for (overlay point/line/rect); richer overlays (circles, text, colormaps)
//! land here later.

use crate::{Frame, PixelFormat};

/// An owned, writable RGB24 image. Create once and reuse across frames (a
/// processor overwrites the region it cares about each call).
#[derive(Clone)]
pub struct Canvas {
    width: u32,
    height: u32,
    /// `width * height * 3` bytes, tightly packed RGB.
    data: Vec<u8>,
}

impl Canvas {
    /// A new `width`x`height` canvas, cleared to black.
    pub fn new(width: u32, height: u32) -> Self {
        Canvas {
            width,
            height,
            data: vec![0; width as usize * height as usize * 3],
        }
    }

    /// Canvas width in pixels.
    pub fn width(&self) -> u32 { self.width }
    /// Canvas height in pixels.
    pub fn height(&self) -> u32 { self.height }

    /// Lend the canvas out as a borrowed [`Frame`] so a display can sample it.
    /// Cheap: no copy, the frame just points at the canvas's buffer.
    pub fn as_frame(&self) -> Frame<'_> {
        Frame {
            width: self.width,
            height: self.height,
            bytes_per_line: self.width * 3,
            format: PixelFormat::Rgb24,
            data: &self.data,
        }
    }

    /// Paint the whole canvas one color.
    pub fn fill(&mut self, rgb: [u8; 3]) {
        for px in self.data.chunks_exact_mut(3) {
            px.copy_from_slice(&rgb);
        }
    }

    /// Set one pixel. Out-of-bounds coordinates are ignored (no panic), so
    /// drawing code can be sloppy at the edges.
    pub fn set_pixel(&mut self, x: u32, y: u32, rgb: [u8; 3]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let o = (y * self.width + x) as usize * 3;
        self.data[o..o + 3].copy_from_slice(&rgb);
    }

    /// Read one pixel back. Out-of-bounds coordinates return black, mirroring
    /// `set_pixel`'s forgiving bounds handling. The read counterpart used when a
    /// pass needs to composite over what it just drew.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        if x >= self.width || y >= self.height {
            return [0, 0, 0];
        }
        let o = (y * self.width + x) as usize * 3;
        [self.data[o], self.data[o + 1], self.data[o + 2]]
    }

    /// Copy a source frame into the canvas as a background to draw over. Samples
    /// through [`Frame::sample_rgb`], so any source pixel format works. The
    /// overlapping top-left region is copied; if sizes differ the rest is left
    /// untouched.
    pub fn blit_frame(&mut self, frame: &Frame<'_>) {
        let w = self.width.min(frame.width);
        let h = self.height.min(frame.height);
        for y in 0..h {
            for x in 0..w {
                self.set_pixel(x, y, frame.sample_rgb(x, y));
            }
        }
    }

    /// Draw a line between two points (integer Bresenham). The workhorse overlay
    /// primitive -- flow vectors, epipolar lines, trajectory segments.
    pub fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, rgb: [u8; 3]) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        loop {
            if x >= 0 && y >= 0 {
                self.set_pixel(x as u32, y as u32, rgb);
            }
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Draw an axis-aligned rectangle outline (e.g. a detection / ROI box).
    pub fn draw_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: [u8; 3]) {
        if w <= 0 || h <= 0 {
            return;
        }
        let (x1, y1) = (x + w - 1, y + h - 1);
        self.draw_line(x, y, x1, y, rgb);
        self.draw_line(x, y1, x1, y1, rgb);
        self.draw_line(x, y, x, y1, rgb);
        self.draw_line(x1, y, x1, y1, rgb);
    }
}
