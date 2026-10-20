//! Video-file capture: a frame *source* backed by FFmpeg.
//!
//! Where [`crate::capture`] pulls live frames off a webcam, this pulls them out
//! of a video file -- an mp4/mov/mkv/webm from a GoPro, phone, or any other
//! camera. It decodes with the system FFmpeg libraries (via the `ffmpeg-next`
//! bindings) and hands out the exact same [`Frame`] type, so every downstream
//! crate and app treats a file identically to the webcam. That is the whole
//! point of recorded input (ROADMAP §1 "Record & replay"): the same bytes every
//! run, so a flow/VO/SLAM backend can be debugged against deterministic input
//! instead of live-camera nondeterminism.
//!
//! ## Pixel format
//!
//! FFmpeg hands back frames in whatever pixel format the codec uses -- usually
//! planar YUV (`yuv420p` for H.264/HEVC from consumer cameras), but also
//! `yuvj420p`, 10-bit, and so on. We normalise every one of them to packed RGB
//! with libswscale and advertise the result as [`PixelFormat::Rgb24`]. The
//! scaler runs with `BITEXACT | ACCURATE_RND` so a given file decodes to the
//! same bytes on every run, which is the point of recorded input.

use std::io;
use std::path::Path;

use ffmpeg_next as ffmpeg;
use ffmpeg::format::Pixel;
use ffmpeg::media::Type;
use ffmpeg::software::scaling::{Context as Scaler, Flags};

use vision_core::{Frame, PixelFormat};

use crate::source::FrameSource;

/// Turn an FFmpeg error into an `io::Error`, so this module speaks the same
/// `io::Result` the rest of vision-io (and the apps) already use.
fn ff(e: ffmpeg::Error, what: &str) -> io::Error {
    io::Error::other(format!("{what}: {e}"))
}

/// A video file opened for decoding, handing out one [`Frame`] at a time via
/// [`next_frame`]. Decoded YUV is converted to packed RGB in an owned buffer that
/// each returned `Frame` borrows, so a frame is valid until the next call to
/// `next_frame` -- the same "one frame in flight" rule the webcam source has.
///
/// [`next_frame`]: VideoFile::next_frame
pub struct VideoFile {
    ictx: ffmpeg::format::context::Input,
    decoder: ffmpeg::decoder::Video,
    stream_index: usize,
    /// Reusable decode target; refilled by the decoder each frame.
    decoded: ffmpeg::frame::Video,
    /// Lazily built on the first frame, once the decoded pixel format is known.
    scaler: Option<Scaler>,
    /// Reusable swscale output frame (RGB24, possibly with padded rows).
    scaled: ffmpeg::frame::Video,
    /// Reusable RGB24 output, `width * height * 3` bytes, tightly packed.
    rgb: Vec<u8>,
    width: u32,
    height: u32,
    fps: f64,
    /// Set once we have signalled end-of-file to the decoder and are draining
    /// whatever frames it still had buffered. After that, a failed receive means
    /// "truly done" rather than "feed me another packet".
    draining: bool,
}

impl VideoFile {
    /// Open `path` and set up decoding of its best (default) video stream.
    pub fn open(path: &str) -> io::Result<Self> {
        // Idempotent: safe to call once per process or once per file.
        ffmpeg::init().map_err(|e| ff(e, "ffmpeg init"))?;

        let ictx = ffmpeg::format::input(&Path::new(path))
            .map_err(|e| ff(e, format!("open {path}").as_str()))?;

        // Pick the stream FFmpeg considers the best video stream (handles files
        // with multiple video/audio/subtitle streams).
        let stream = ictx
            .streams()
            .best(Type::Video)
            .ok_or_else(|| io::Error::other(format!("{path}: no video stream")))?;
        let stream_index = stream.index();

        // avg_frame_rate is a rational (num/den); 0/0 or 0 denominator means the
        // container didn't record one, in which case we fall back to 30 fps for
        // playback pacing only (it never affects decoded pixels).
        let rate = stream.avg_frame_rate();
        let fps = if rate.denominator() != 0 && rate.numerator() != 0 {
            rate.numerator() as f64 / rate.denominator() as f64
        } else {
            30.0
        };

        // Build a decoder from the stream's codec parameters.
        let codec_ctx = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .map_err(|e| ff(e, "codec context"))?;
        let decoder = codec_ctx
            .decoder()
            .video()
            .map_err(|e| ff(e, "open video decoder"))?;

        let width = decoder.width();
        let height = decoder.height();
        if width == 0 || height == 0 {
            return Err(io::Error::other(format!(
                "{path}: decoder reported zero dimensions ({width}x{height})"
            )));
        }

        Ok(VideoFile {
            ictx,
            decoder,
            stream_index,
            decoded: ffmpeg::frame::Video::empty(),
            scaler: None,
            scaled: ffmpeg::frame::Video::new(Pixel::RGB24, width, height),
            rgb: vec![0; width as usize * height as usize * 3],
            width,
            height,
            fps,
            draining: false,
        })
    }

    /// Frame width in pixels.
    pub fn width(&self) -> u32 { self.width }
    /// Frame height in pixels.
    pub fn height(&self) -> u32 { self.height }
    /// The stream's average frame rate, for playback pacing. Decoded pixels are
    /// unaffected by this number.
    pub fn fps(&self) -> f64 { self.fps }

    /// Decode and return the next frame, or `Ok(None)` at end of file.
    ///
    /// This is the canonical libav pull loop: ask the decoder for a frame; if it
    /// has none ready, feed it the next video packet and ask again; once the
    /// container is exhausted, flush the decoder and drain its remaining frames.
    pub fn next_frame(&mut self) -> io::Result<Option<Frame<'_>>> {
        loop {
            // 1. Is a decoded frame already waiting inside the decoder? (A single
            //    packet can yield zero, one, or several frames, so we always try
            //    to pull before feeding more input.)
            if self.decoder.receive_frame(&mut self.decoded).is_ok() {
                self.convert()?;
                return Ok(Some(Frame {
                    width: self.width,
                    height: self.height,
                    bytes_per_line: self.width * 3,
                    format: PixelFormat::Rgb24,
                    data: &self.rgb,
                }));
            }

            // 2. Nothing ready. If we've already flushed, the stream is finished.
            if self.draining {
                return Ok(None);
            }

            // 3. Feed the decoder: the next packet belonging to our video stream,
            //    or, at container EOF, the flush signal.
            match self.read_video_packet() {
                Some(packet) => {
                    self.decoder
                        .send_packet(&packet)
                        .map_err(|e| ff(e, "send_packet"))?;
                }
                None => {
                    self.decoder.send_eof().map_err(|e| ff(e, "send_eof"))?;
                    self.draining = true;
                }
            }
        }
    }

    /// Read packets until one belongs to our video stream (skipping audio/other),
    /// or `None` at end of file. A fresh `packets()` iterator each call is fine:
    /// read position lives in the format context, not the iterator.
    fn read_video_packet(&mut self) -> Option<ffmpeg::Packet> {
        for (stream, packet) in self.ictx.packets() {
            if stream.index() == self.stream_index {
                return Some(packet);
            }
        }
        None
    }

    /// Convert the just-decoded frame, whatever its pixel format, into the
    /// packed-RGB [`Self::rgb`] buffer using libswscale.
    fn convert(&mut self) -> io::Result<()> {
        let (w, h) = (self.width, self.height);
        let fmt = self.decoded.format();

        // `cached` rebuilds the scaler if the source format or size changes
        // mid-stream; otherwise it is a cheap no-op.
        let flags = Flags::BILINEAR | Flags::ACCURATE_RND | Flags::BITEXACT;
        match &mut self.scaler {
            Some(s) => s.cached(fmt, w, h, Pixel::RGB24, w, h, flags),
            none => {
                *none = Some(
                    Scaler::get(fmt, w, h, Pixel::RGB24, w, h, flags)
                        .map_err(|e| ff(e, "create scaler"))?,
                );
            }
        }
        let scaler = self.scaler.as_mut().expect("scaler initialised above");
        scaler
            .run(&self.decoded, &mut self.scaled)
            .map_err(|e| ff(e, "scale to rgb24"))?;

        // The scaled frame's rows may be padded; copy them into the tight buffer.
        let stride = self.scaled.stride(0);
        let src = self.scaled.data(0);
        let row = w as usize * 3;
        for y in 0..h as usize {
            self.rgb[y * row..(y + 1) * row].copy_from_slice(&src[y * stride..y * stride + row]);
        }
        Ok(())
    }
}

impl FrameSource for VideoFile {
    fn next_frame(&mut self) -> io::Result<Option<Frame<'_>>> {
        VideoFile::next_frame(self)
    }
    fn width(&self) -> u32 { self.width }
    fn height(&self) -> u32 { self.height }
    fn target_fps(&self) -> Option<f64> { Some(self.fps) }
}
