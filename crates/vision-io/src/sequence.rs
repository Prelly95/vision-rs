//! Image-sequence capture: a frame *source* backed by a directory of images.
//!
//! Datasets like KITTI, TUM and EuRoC ship one lossless image per frame
//! (`0000000000.png`, `0000000001.png`, ...) rather than a video. This source
//! walks such a directory in filename order and hands out the exact same
//! [`Frame`] type as the webcam and video file, so the harness, processors and
//! sinks work on dataset input unchanged -- and with no lossy re-encode between
//! the dataset's pixels and the algorithm.
//!
//! The frame index is exposed so ground truth (KITTI `oxts/data/<i>.txt`, LiDAR
//! scans) can be looked up by index. Parsing that ground truth is deliberately
//! *not* this type's job; it stays a generic image source.

use std::io;
use std::path::{Path, PathBuf};

use vision_core::{Frame, PixelFormat};

use crate::source::FrameSource;

/// A directory of images opened for sequential reading, one [`Frame`] at a time
/// via [`next_frame`]. Each image is decoded into an owned buffer that the
/// returned `Frame` borrows, so a frame is valid until the next call -- the same
/// "one frame in flight" rule the other sources have.
///
/// [`next_frame`]: ImageSequence::next_frame
pub struct ImageSequence {
    /// Image paths, sorted by filename. Zero-padded names sort correctly.
    paths: Vec<PathBuf>,
    /// Index into `paths` of the next image to decode.
    next: usize,
    /// Per-frame timestamps in seconds from the first frame, if a timestamps
    /// file was found. Empty otherwise.
    timestamps: Vec<f64>,
    /// Reusable decode target, tightly packed.
    buf: Vec<u8>,
    width: u32,
    height: u32,
    format: PixelFormat,
    fps: Option<f64>,
}

impl ImageSequence {
    /// Open a directory of `.png` files (e.g. KITTI's `image_02/data`).
    ///
    /// If a `timestamps.txt` sits next to the directory (KITTI's layout), it is
    /// loaded and used to derive [`target_fps`].
    ///
    /// [`target_fps`]: FrameSource::target_fps
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref();

        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
            .collect();
        if paths.is_empty() {
            return Err(io::Error::other(format!(
                "{}: no .png files found",
                dir.display()
            )));
        }
        paths.sort();

        // Decode the first image up front: it fixes width/height/format so the
        // harness can size the window before the first `next_frame`.
        let mut buf = Vec::new();
        let (width, height, format) = decode_png(&paths[0], &mut buf)?;

        // KITTI keeps `timestamps.txt` next to `data/`. Optional: other datasets
        // simply have no pacing information.
        let ts_path = dir.parent().map(|p| p.join("timestamps.txt"));
        let timestamps = match ts_path {
            Some(p) if p.is_file() => parse_timestamps(&p)?,
            _ => Vec::new(),
        };
        if !timestamps.is_empty() && timestamps.len() != paths.len() {
            return Err(io::Error::other(format!(
                "{}: {} timestamps for {} images",
                ts_path_display(dir),
                timestamps.len(),
                paths.len()
            )));
        }
        let fps = median_fps(&timestamps);

        Ok(ImageSequence {
            paths,
            next: 0,
            timestamps,
            buf,
            width,
            height,
            format,
            fps,
        })
    }

    /// Number of images in the sequence.
    pub fn len(&self) -> usize { self.paths.len() }
    /// True if the sequence has no images (never the case after `open`).
    pub fn is_empty(&self) -> bool { self.paths.is_empty() }

    /// Index of the frame most recently returned by `next_frame`, or `None`
    /// before the first call. Use it to look up per-frame ground truth.
    pub fn frame_index(&self) -> Option<usize> { self.next.checked_sub(1) }

    /// Timestamp (seconds from the first frame) of frame `i`, if known.
    pub fn timestamp(&self, i: usize) -> Option<f64> { self.timestamps.get(i).copied() }

    /// Frame width in pixels.
    pub fn width(&self) -> u32 { self.width }
    /// Frame height in pixels.
    pub fn height(&self) -> u32 { self.height }

    /// Decode and return the next image, or `Ok(None)` after the last one.
    pub fn next_frame(&mut self) -> io::Result<Option<Frame<'_>>> {
        let Some(path) = self.paths.get(self.next) else {
            return Ok(None);
        };

        // A bad file means a corrupt dataset, so fail loudly with its name
        // rather than skipping it and silently shifting every later index.
        let (width, height, format) = decode_png(path, &mut self.buf)?;
        if (width, height, format) != (self.width, self.height, self.format) {
            return Err(io::Error::other(format!(
                "{}: {width}x{height} {format:?}, expected {}x{} {:?}",
                path.display(),
                self.width,
                self.height,
                self.format
            )));
        }
        self.next += 1;

        let bytes_per_pixel = match format {
            PixelFormat::Gray8 => 1,
            _ => 3,
        };
        Ok(Some(Frame {
            width,
            height,
            bytes_per_line: width * bytes_per_pixel,
            format,
            data: &self.buf,
        }))
    }
}

/// Decode `path` into `buf` (tightly packed) and return its size and format.
/// 8-bit grayscale maps to [`PixelFormat::Gray8`] and RGB to
/// [`PixelFormat::Rgb24`]; 16-bit samples are stripped to 8 and palettes
/// expanded. Alpha channels are rejected. Errors carry the file name.
fn decode_png(path: &Path, buf: &mut Vec<u8>) -> io::Result<(u32, u32, PixelFormat)> {
    let err = |e: &dyn std::fmt::Display| io::Error::other(format!("{}: {e}", path.display()));

    let file = std::fs::File::open(path).map_err(|e| err(&e))?;
    let mut decoder = png::Decoder::new(io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| err(&e))?;

    buf.resize(reader.output_buffer_size(), 0);
    let info = reader.next_frame(buf).map_err(|e| err(&e))?;
    buf.truncate(info.buffer_size());

    let format = match info.color_type {
        png::ColorType::Grayscale => PixelFormat::Gray8,
        png::ColorType::Rgb => PixelFormat::Rgb24,
        other => return Err(err(&format!("unsupported PNG color type {other:?}"))),
    };
    Ok((info.width, info.height, format))
}

/// Parse a KITTI-style timestamps file (one `YYYY-MM-DD HH:MM:SS.fffffffff` per
/// line) into seconds from the first entry.
fn parse_timestamps(path: &Path) -> io::Result<Vec<f64>> {
    let bad = |n: usize, line: &str| {
        io::Error::other(format!("{}:{}: bad timestamp {line:?}", path.display(), n + 1))
    };
    let mut out = Vec::new();
    for (n, line) in std::fs::read_to_string(path)?.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Time of day only; a drive crossing midnight is not handled.
        let time = line.split_whitespace().nth(1).ok_or_else(|| bad(n, line))?;
        let mut parts = time.split(':');
        let mut next = || parts.next().and_then(|p| p.parse::<f64>().ok());
        let (h, m, s) = (next(), next(), next());
        let (Some(h), Some(m), Some(s)) = (h, m, s) else {
            return Err(bad(n, line));
        };
        out.push(h * 3600.0 + m * 60.0 + s);
    }
    let first = out.first().copied().unwrap_or(0.0);
    out.iter_mut().for_each(|t| *t -= first);
    Ok(out)
}

/// Playback rate from the median frame interval (robust to a dropped frame).
fn median_fps(timestamps: &[f64]) -> Option<f64> {
    let mut dts: Vec<f64> = timestamps.windows(2).map(|w| w[1] - w[0]).collect();
    if dts.is_empty() {
        return None;
    }
    dts.sort_by(|a, b| a.total_cmp(b));
    let dt = dts[dts.len() / 2];
    (dt > 0.0).then(|| 1.0 / dt)
}

fn ts_path_display(dir: &Path) -> String {
    dir.parent()
        .map(|p| p.join("timestamps.txt"))
        .unwrap_or_default()
        .display()
        .to_string()
}

impl FrameSource for ImageSequence {
    fn next_frame(&mut self) -> io::Result<Option<Frame<'_>>> {
        ImageSequence::next_frame(self)
    }
    fn width(&self) -> u32 { self.width }
    fn height(&self) -> u32 { self.height }
    fn target_fps(&self) -> Option<f64> { self.fps }
}
