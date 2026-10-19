//! V4L2 webcam capture: the frame *source*.
//!
//! This module owns everything about talking to `/dev/videoN` -- the raw libc
//! FFI, the kernel struct layouts, buffer setup -- and exposes it as a plain
//! [`V4l2Capture`] that hands out [`Frame`]s. The display side never sees any of
//! this; it only ever receives a `Frame`. Record/replay and dataset sources can
//! be added alongside this file later, producing the same `Frame` type.
//!
//! All ioctl request codes and struct layouts below were verified against the
//! real `/usr/include/linux/videodev2.h` on an x86_64 Ubuntu system using
//! `offsetof()`/`sizeof()`.

use std::ffi::CString;
use std::io;
use std::os::raw::{c_char, c_int, c_void};

use vision_core::{Frame, PixelFormat};

use crate::source::FrameSource;

// ---------------------------------------------------------------------
// Raw libc bindings. We declare only the pieces we need. ioctl's real
// signature is variadic (`int ioctl(int, unsigned long, ...)`), but on
// the Linux x86_64/aarch64 SysV ABI a fixed 3-argument extern "C"
// declaration is call-compatible, since arguments are passed in the same
// registers either way. This is a standard trick for avoiding a libc
// binding crate.
// ---------------------------------------------------------------------
unsafe extern "C" {
    fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn ioctl(fd: c_int, request: u64, argp: *mut c_void) -> c_int;
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
}

const O_RDWR: c_int = 0o2;
const PROT_READ: c_int = 0x1;
const PROT_WRITE: c_int = 0x2;
const MAP_SHARED: c_int = 0x01;

// (void*)-1, the sentinel mmap() returns on failure. Written as a fn
// rather than a `const` because integer-to-pointer casts of this kind
// aren't reliably usable in const context across Rust versions.
fn map_failed() -> *mut c_void {
    usize::MAX as *mut c_void
}

// ---------------------------------------------------------------------
// ioctl request-code derivation: the kernel's _IOC/_IOR/_IOW/_IOWR macros,
// reimplemented as const fns. 'V' == 86 is V4L2's magic type byte for
// all VIDIOC_* ioctls.
// ---------------------------------------------------------------------
const fn ioc(dir: u64, ty: u8, nr: u8, size: usize) -> u64 {
    (dir << 30) | ((ty as u64) << 8) | (nr as u64) | ((size as u64) << 16)
}
const fn iow(ty: u8, nr: u8, size: usize) -> u64 { ioc(1, ty, nr, size) }  // write-only
const fn iowr(ty: u8, nr: u8, size: usize) -> u64 { ioc(3, ty, nr, size) } // read+write

const V: u8 = b'V';

// nr values come straight from videodev2.h (VIDIOC_S_FMT is nr=5, etc.)
// sizes come from size_of::<T>() on OUR structs below, which we verified
// match the kernel's real struct sizes (208, 20, 88, 4 bytes respectively).
fn vidioc_s_fmt() -> u64     { iowr(V, 5,  std::mem::size_of::<V4l2Format>()) }
fn vidioc_reqbufs() -> u64   { iowr(V, 8,  std::mem::size_of::<V4l2RequestBuffers>()) }
fn vidioc_querybuf() -> u64  { iowr(V, 9,  std::mem::size_of::<V4l2Buffer>()) }
fn vidioc_qbuf() -> u64      { iowr(V, 15, std::mem::size_of::<V4l2Buffer>()) }
fn vidioc_dqbuf() -> u64     { iowr(V, 17, std::mem::size_of::<V4l2Buffer>()) }
fn vidioc_streamon() -> u64  { iow(V, 18, std::mem::size_of::<i32>()) }
fn vidioc_streamoff() -> u64 { iow(V, 19, std::mem::size_of::<i32>()) }

const V4L2_BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
const V4L2_FIELD_NONE: u32 = 1;
const V4L2_MEMORY_MMAP: u32 = 1;
// V4L2_PIX_FMT_YUYV is a FourCC packed little-endian: 'Y','U','Y','V'
const V4L2_PIX_FMT_YUYV: u32 =
    (b'Y' as u32) | (b'U' as u32) << 8 | (b'Y' as u32) << 16 | (b'V' as u32) << 24;

// ---------------------------------------------------------------------
// Struct layouts. Field order and explicit padding fields below were
// checked byte-for-byte (via offsetof) against the real kernel structs.
// We only need the "pix" variant of v4l2_format's fmt union, so we
// represent the rest of that union as an opaque trailing byte array.
// ---------------------------------------------------------------------

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct V4l2PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    priv_: u32,
    flags: u32,
    ycbcr_enc: u32,   // union w/ hsv_enc, same 4 bytes either way
    quantization: u32,
    xfer_func: u32,
}
// sizeof == 48, matches the kernel exactly.

#[repr(C)]
struct V4l2Format {
    type_: u32,
    _pad0: u32,               // real struct has a 4-byte gap before the union
    pix: V4l2PixFormat,       // union's first 48 bytes (offset 8..56)
    _reserved: [u8; 152],     // remaining bytes of the 200-byte union
}
// sizeof == 4+4+48+152 == 208, matches sizeof(struct v4l2_format) exactly.

impl Default for V4l2Format {
    fn default() -> Self {
        V4l2Format { type_: 0, _pad0: 0, pix: V4l2PixFormat::default(), _reserved: [0; 152] }
    }
}

#[repr(C)]
#[derive(Default)]
struct V4l2RequestBuffers {
    count: u32,
    type_: u32,
    memory: u32,
    reserved: [u32; 2],
}
// sizeof == 20, matches.

#[repr(C)]
#[derive(Default)]
struct V4l2Buffer {
    index: u32,          // 0
    type_: u32,          // 4
    bytesused: u32,      // 8
    flags: u32,          // 12
    field: u32,          // 16
    _pad0: u32,          // 20  (align the timeval that follows to 8 bytes)
    tv_sec: i64,         // 24
    tv_usec: i64,        // 32   (timeval together == offset 24..40)
    tc_type: u32,        // 40
    tc_flags: u32,       // 44
    tc_frames: u8,       // 48
    tc_seconds: u8,      // 49
    tc_minutes: u8,      // 50
    tc_hours: u8,        // 51
    tc_userbits: [u8; 4],// 52
    sequence: u32,       // 56
    memory: u32,         // 60
    m_offset: u32,       // 64  (the `m` union; we only use the mmap-offset variant)
    _pad_m: u32,         // 68  (pad union up to 8 bytes, matching `unsigned long userptr`)
    length: u32,         // 72
    reserved2: u32,      // 76
    request_fd: i32,     // 80
    _pad_end: u32,       // 84
}
// sizeof == 88, matches sizeof(struct v4l2_buffer) exactly.

/// A single mmap'd capture buffer.
struct MappedBuffer {
    ptr: *mut u8,
    len: usize,
}

/// Turn a failed syscall return (`ret < 0`) into an `io::Error` carrying the
/// current errno plus a label for which call failed.
fn check(ret: c_int, what: &str) -> io::Result<c_int> {
    if ret < 0 {
        Err(io::Error::other(format!("{what}: {}", io::Error::last_os_error())))
    } else {
        Ok(ret)
    }
}

/// A streaming V4L2 capture device. Opens the device, negotiates a format, maps
/// the kernel's capture buffers, and starts streaming; hands out one [`Frame`] at
/// a time via [`next_frame`]. Streaming is stopped and all resources released on
/// drop.
///
/// [`next_frame`]: V4l2Capture::next_frame
pub struct V4l2Capture {
    fd: c_int,
    width: u32,
    height: u32,
    bytes_per_line: u32,
    buffers: Vec<MappedBuffer>,
    /// Index of the buffer currently dequeued and lent out as a `Frame`, if any.
    /// It stays out of the driver's rotation until the next [`next_frame`] call
    /// re-queues it. `None` before the first frame.
    ///
    /// [`next_frame`]: V4l2Capture::next_frame
    current: Option<u32>,
}

impl V4l2Capture {
    /// Open `path` (e.g. `/dev/video0`) and start streaming YUYV at as close to
    /// `width`x`height` as the driver supports. The negotiated geometry (which
    /// may differ from what was requested) is readable via [`width`](Self::width)
    /// / [`height`](Self::height) afterwards.
    pub fn open(path: &str, width: u32, height: u32) -> io::Result<Self> {
        let cpath = CString::new(path)
            .map_err(|_| io::Error::other("device path contains an interior NUL byte"))?;
        let fd = unsafe { open(cpath.as_ptr(), O_RDWR) };
        if fd < 0 {
            return Err(io::Error::other(format!(
                "open({path}): {}",
                io::Error::last_os_error()
            )));
        }

        // From here on, any early return must not leak the fd. Build the device
        // first, then run setup through it so Drop handles cleanup on failure.
        let mut dev =
            V4l2Capture { fd, width: 0, height: 0, bytes_per_line: 0, buffers: Vec::new(), current: None };
        dev.configure(width, height)?;
        Ok(dev)
    }

    /// Negotiated frame width in pixels.
    pub fn width(&self) -> u32 { self.width }
    /// Negotiated frame height in pixels.
    pub fn height(&self) -> u32 { self.height }
    /// Negotiated row stride in bytes.
    pub fn bytes_per_line(&self) -> u32 { self.bytes_per_line }

    /// Run S_FMT -> REQBUFS -> QUERYBUF/mmap/QBUF -> STREAMON.
    fn configure(&mut self, width: u32, height: u32) -> io::Result<()> {
        // ---- VIDIOC_S_FMT: ask for the requested geometry in YUYV ----------
        let mut fmt = V4l2Format::default();
        fmt.type_ = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        fmt.pix.width = width;
        fmt.pix.height = height;
        fmt.pix.pixelformat = V4L2_PIX_FMT_YUYV;
        fmt.pix.field = V4L2_FIELD_NONE;
        check(
            unsafe { ioctl(self.fd, vidioc_s_fmt(), &mut fmt as *mut _ as *mut c_void) },
            "VIDIOC_S_FMT",
        )?;
        // The driver may have adjusted these to its nearest supported mode --
        // always trust what it reports back, not what we asked for.
        self.width = fmt.pix.width;
        self.height = fmt.pix.height;
        self.bytes_per_line = fmt.pix.bytesperline;

        // ---- VIDIOC_REQBUFS: ask the kernel for 4 mmap-backed buffers -------
        let mut req = V4l2RequestBuffers::default();
        req.count = 4;
        req.type_ = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        req.memory = V4L2_MEMORY_MMAP;
        check(
            unsafe { ioctl(self.fd, vidioc_reqbufs(), &mut req as *mut _ as *mut c_void) },
            "VIDIOC_REQBUFS",
        )?;
        let n_buffers = req.count; // kernel may give us fewer than requested

        // ---- VIDIOC_QUERYBUF + mmap() each buffer ---------------------------
        self.buffers.reserve(n_buffers as usize);
        for i in 0..n_buffers {
            let mut buf = V4l2Buffer::default();
            buf.type_ = V4L2_BUF_TYPE_VIDEO_CAPTURE;
            buf.memory = V4L2_MEMORY_MMAP;
            buf.index = i;
            check(
                unsafe { ioctl(self.fd, vidioc_querybuf(), &mut buf as *mut _ as *mut c_void) },
                "VIDIOC_QUERYBUF",
            )?;

            let ptr = unsafe {
                mmap(
                    std::ptr::null_mut(),
                    buf.length as usize,
                    PROT_READ | PROT_WRITE,
                    MAP_SHARED,
                    self.fd,
                    buf.m_offset as i64,
                )
            };
            if ptr == map_failed() {
                return Err(io::Error::other(format!("mmap: {}", io::Error::last_os_error())));
            }
            self.buffers.push(MappedBuffer { ptr: ptr as *mut u8, len: buf.length as usize });

            // Hand this buffer to the driver immediately -- it's now allowed to
            // DMA frame data into it.
            check(
                unsafe { ioctl(self.fd, vidioc_qbuf(), &mut buf as *mut _ as *mut c_void) },
                "VIDIOC_QBUF (initial)",
            )?;
        }

        // ---- VIDIOC_STREAMON: start capturing -------------------------------
        let mut buf_type: i32 = V4L2_BUF_TYPE_VIDEO_CAPTURE as i32;
        check(
            unsafe { ioctl(self.fd, vidioc_streamon(), &mut buf_type as *mut _ as *mut c_void) },
            "VIDIOC_STREAMON",
        )?;
        Ok(())
    }

    /// Block until the driver reports a filled buffer, then return it as a
    /// [`Frame`]. The returned frame borrows the capture, so it stays valid until
    /// the next `next_frame` call -- which first re-queues the previous buffer to
    /// the driver, then dequeues the next. That borrow is what limits you to one
    /// frame in flight, exactly the window the mmap buffer is safe to read for.
    ///
    /// Always `Some` for a live device; the `Option` exists to satisfy
    /// [`FrameSource`], whose file-backed implementors return `None` at EOF.
    pub fn next_frame(&mut self) -> io::Result<Option<Frame<'_>>> {
        // Hand the previously-lent buffer back to the driver so it can refill it.
        if let Some(index) = self.current.take() {
            self.queue_buffer(index)?;
        }

        let mut buf = V4l2Buffer::default();
        buf.type_ = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        buf.memory = V4L2_MEMORY_MMAP;
        check(
            unsafe { ioctl(self.fd, vidioc_dqbuf(), &mut buf as *mut _ as *mut c_void) },
            "VIDIOC_DQBUF",
        )?;
        self.current = Some(buf.index);

        let mb = &self.buffers[buf.index as usize];
        // Safety: the kernel told us (via DQBUF) that this buffer is filled and
        // ours to read until we QBUF it back, which only happens on the next
        // call -- after the borrow handed out here has ended.
        let data = unsafe { std::slice::from_raw_parts(mb.ptr, mb.len) };
        Ok(Some(Frame {
            width: self.width,
            height: self.height,
            bytes_per_line: self.bytes_per_line,
            format: PixelFormat::Yuyv,
            data,
        }))
    }

    /// Hand buffer `index` back to the driver (VIDIOC_QBUF) so it may refill it.
    fn queue_buffer(&self, index: u32) -> io::Result<()> {
        let mut buf = V4l2Buffer::default();
        buf.type_ = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        buf.memory = V4L2_MEMORY_MMAP;
        buf.index = index;
        check(
            unsafe { ioctl(self.fd, vidioc_qbuf(), &mut buf as *mut _ as *mut c_void) },
            "VIDIOC_QBUF",
        )?;
        Ok(())
    }
}

impl FrameSource for V4l2Capture {
    fn next_frame(&mut self) -> io::Result<Option<Frame<'_>>> {
        V4l2Capture::next_frame(self)
    }
    fn width(&self) -> u32 { self.width }
    fn height(&self) -> u32 { self.height }
    // target_fps: a live webcam has no inherent playback rate -> None (default).
}

impl Drop for V4l2Capture {
    fn drop(&mut self) {
        // Best-effort clean shutdown; ignore errors since we're tearing down.
        let mut buf_type: i32 = V4L2_BUF_TYPE_VIDEO_CAPTURE as i32;
        unsafe {
            ioctl(self.fd, vidioc_streamoff(), &mut buf_type as *mut _ as *mut c_void);
            for b in &self.buffers {
                munmap(b.ptr as *mut c_void, b.len);
            }
            close(self.fd);
        }
    }
}

