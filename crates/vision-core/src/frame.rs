//! The decoupling boundary between capture and display.
//!
//! A [`Frame`] is a borrowed view of one captured image plus the metadata needed
//! to interpret its bytes. Capture sources produce frames; display sinks consume
//! them. Neither side needs to know anything about the other -- they meet only at
//! this type. Displays read pixels through [`Frame::sample_rgb`] and never touch
//! the raw pixel format, so a new capture format or a new renderer slots in
//! without the other side changing.

/// Pixel layout of a frame's raw bytes. Only the formats we actually capture are
/// listed; add variants here (and a matching arm in [`Frame::sample_rgb`]) as new
/// capture sources appear.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PixelFormat {
    /// Packed YUV 4:2:2, two pixels per four bytes as `[Y0, U, Y1, V]`.
    Yuyv,
    /// Packed 8-bit RGB, three bytes per pixel as `[R, G, B]`, rows laid out
    /// back-to-back (padding, if any, lives in `bytes_per_line`). Sources that
    /// have already decoded to RGB -- e.g. a video file whose YUV frames were
    /// converted once on the way in -- hand frames out in this format so the
    /// display side needs no further conversion.
    Rgb24,
}

/// A borrowed view of one captured frame. The byte slice typically points at a
/// driver-owned mmap buffer, so a `Frame` is valid only for as long as the
/// capture source lets it live -- hence the lifetime. Construct one from whatever
/// buffer a capture source hands out; consume it through [`sample_rgb`].
///
/// [`sample_rgb`]: Frame::sample_rgb
pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    /// Stride: bytes from the start of one row to the start of the next. May
    /// exceed `width * bytes_per_pixel` when the driver pads rows.
    pub bytes_per_line: u32,
    pub format: PixelFormat,
    pub data: &'a [u8],
}

impl Frame<'_> {
    /// Decode a single source pixel to 8-bit RGB, regardless of the underlying
    /// pixel format. This is the only pixel accessor a display should need:
    /// everything format-specific lives behind it.
    pub fn sample_rgb(&self, x: u32, y: u32) -> [u8; 3] {
        match self.format {
            PixelFormat::Yuyv => self.sample_yuyv(x, y),
            PixelFormat::Rgb24 => self.sample_rgb24(x, y),
        }
    }

    /// RGB24 is already in the output format: read the three bytes for this
    /// pixel directly. Out-of-range reads fall back to 0 (black) so a
    /// short/garbled buffer can't panic, matching the YUYV path.
    fn sample_rgb24(&self, x: u32, y: u32) -> [u8; 3] {
        let base = (y * self.bytes_per_line) as usize + x as usize * 3;
        [
            self.data.get(base).copied().unwrap_or(0),
            self.data.get(base + 1).copied().unwrap_or(0),
            self.data.get(base + 2).copied().unwrap_or(0),
        ]
    }

    /// YUYV packs two pixels into four bytes as `[Y0, U, Y1, V]`: both pixels of a
    /// pair share U and V, and the pixel's own luma is Y0 (even x) or Y1 (odd x).
    /// Out-of-range reads fall back to neutral values so a short/garbled buffer
    /// renders as grey rather than panicking.
    fn sample_yuyv(&self, x: u32, y: u32) -> [u8; 3] {
        let row_start = (y * self.bytes_per_line) as usize;
        let pair_off = (x as usize / 2) * 4; // byte offset of this pixel's 4-byte pair
        let base = row_start + pair_off;
        let y = self.data.get(base + (x as usize & 1) * 2).copied().unwrap_or(0);
        let u = self.data.get(base + 1).copied().unwrap_or(128);
        let v = self.data.get(base + 3).copied().unwrap_or(128);
        yuv_to_rgb(y, u, v)
    }
}

/// Decode one BT.601 YUV triple to 8-bit RGB. This is the standard integer
/// approximation of the full-range-ish conversion (good enough for a terminal or
/// a window, and branch-free apart from the clamp).
pub fn yuv_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
    let c = y as i32 - 16;
    let d = u as i32 - 128;
    let e = v as i32 - 128;
    let r = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255);
    let g = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255);
    let b = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255);
    [r as u8, g as u8, b as u8]
}
