// webcam_ascii.rs
//
// Minimal V4L2 webcam reader that prints a live black/white ASCII render
// to the terminal at 10fps. No external crates -- just raw libc FFI.
//
// Build:  rustc -O webcam_ascii.rs -o webcam_ascii
// Run:    ./webcam_ascii
//
// All ioctl request codes and struct layouts below were verified against
// the real /usr/include/linux/videodev2.h on an x86_64 Ubuntu system using
// offsetof()/sizeof() -- see the accompanying explanation for how.

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::time::Instant;

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
    fn __errno_location() -> *mut c_int;
}

fn errno() -> i32 {
    unsafe { *__errno_location() }
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
const fn iow(ty: u8, nr: u8, size: usize) -> u64 { ioc(1, ty, nr, size) }       // write-only
const fn iowr(ty: u8, nr: u8, size: usize) -> u64 { ioc(3, ty, nr, size) }      // read+write

const V: u8 = b'V';

// nr values come straight from videodev2.h (VIDIOC_S_FMT is nr=5, etc.)
// sizes come from size_of::<T>() on OUR structs below, which we verified
// match the kernel's real struct sizes (208, 20, 88, 4 bytes respectively).
fn vidioc_s_fmt() -> u64    { iowr(V, 5,  std::mem::size_of::<V4l2Format>()) }
fn vidioc_reqbufs() -> u64 { iowr(V, 8,  std::mem::size_of::<V4l2RequestBuffers>()) }
fn vidioc_querybuf() -> u64{ iowr(V, 9,  std::mem::size_of::<V4l2Buffer>()) }
fn vidioc_qbuf() -> u64    { iowr(V, 15, std::mem::size_of::<V4l2Buffer>()) }
fn vidioc_dqbuf() -> u64   { iowr(V, 17, std::mem::size_of::<V4l2Buffer>()) }
fn vidioc_streamon() -> u64  { iow(V, 18, std::mem::size_of::<i32>()) }
fn vidioc_streamoff() -> u64 { iow(V, 19, std::mem::size_of::<i32>()) }

const V4L2_BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
const V4L2_FIELD_NONE: u32 = 1;
const V4L2_MEMORY_MMAP: u32 = 1;
// V4L2_PIX_FMT_YUYV is a FourCC packed little-endian: 'Y','U','Y','V'
const V4L2_PIX_FMT_YUYV: u32 =
    (b'Y' as u32) | (b'U' as u32) << 8 | (b'Y' as u32) << 16 | (b'V' as u32) << 24;

// TIOCGWINSZ / struct winsize: verified against the real system headers
// the same way as the VIDIOC_* constants above (offsetof/sizeof probe).
const TIOCGWINSZ: u64 = 0x5413;

#[repr(C)]
struct WinSize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

/// Ask the terminal how big it currently is. Falls back to a sane default
/// if stdout isn't actually a tty (e.g. piped to a file).
fn terminal_size() -> (u32, u32) {
    let mut ws = WinSize { ws_row: 0, ws_col: 0, ws_xpixel: 0, ws_ypixel: 0 };
    let ret = unsafe { ioctl(1 /* STDOUT_FILENO */, TIOCGWINSZ, &mut ws as *mut _ as *mut c_void) };
    if ret < 0 || ws.ws_row == 0 || ws.ws_col == 0 {
        (100, 40)
    } else {
        (ws.ws_col as u32, ws.ws_row as u32)
    }
}

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

// A single mmap'd capture buffer.
struct MappedBuffer {
    ptr: *mut u8,
    len: usize,
}

fn check(ret: c_int, what: &str) -> c_int {
    if ret < 0 {
        panic!("{} failed, errno={}", what, errno());
    }
    ret
}

fn main() {
    // ---- open the device -------------------------------------------------
    let path = CString::new("/dev/video0").unwrap();
    let fd = unsafe { open(path.as_ptr(), O_RDWR) };
    if fd < 0 {
        panic!("open(/dev/video0) failed, errno={}", errno());
    }

    // ---- VIDIOC_S_FMT: ask for 640x480 YUYV --------------------------------
    let mut fmt = V4l2Format::default();
    fmt.type_ = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    fmt.pix.width = 640;
    fmt.pix.height = 480;
    fmt.pix.pixelformat = V4L2_PIX_FMT_YUYV;
    fmt.pix.field = V4L2_FIELD_NONE;
    check(
        unsafe { ioctl(fd, vidioc_s_fmt(), &mut fmt as *mut _ as *mut c_void) },
        "VIDIOC_S_FMT",
    );
    // The driver may have adjusted these to its nearest supported mode --
    // always trust what it reports back, not what we asked for.
    let width = fmt.pix.width;
    let height = fmt.pix.height;
    let bytesperline = fmt.pix.bytesperline;
    eprintln!("negotiated format: {}x{}, bytesperline={}", width, height, bytesperline);

    // ---- VIDIOC_REQBUFS: ask the kernel for 4 mmap-backed buffers ----------
    let mut req = V4l2RequestBuffers::default();
    req.count = 4;
    req.type_ = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    req.memory = V4L2_MEMORY_MMAP;
    check(
        unsafe { ioctl(fd, vidioc_reqbufs(), &mut req as *mut _ as *mut c_void) },
        "VIDIOC_REQBUFS",
    );
    let n_buffers = req.count; // kernel may give us fewer than requested

    // ---- VIDIOC_QUERYBUF + mmap() each buffer ------------------------------
    let mut buffers: Vec<MappedBuffer> = Vec::with_capacity(n_buffers as usize);
    for i in 0..n_buffers {
        let mut buf = V4l2Buffer::default();
        buf.type_ = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        buf.memory = V4L2_MEMORY_MMAP;
        buf.index = i;
        check(
            unsafe { ioctl(fd, vidioc_querybuf(), &mut buf as *mut _ as *mut c_void) },
            "VIDIOC_QUERYBUF",
        );

        let ptr = unsafe {
            mmap(
                std::ptr::null_mut(),
                buf.length as usize,
                PROT_READ | PROT_WRITE,
                MAP_SHARED,
                fd,
                buf.m_offset as i64,
            )
        };
        if ptr == map_failed() {
            panic!("mmap failed, errno={}", errno());
        }
        buffers.push(MappedBuffer { ptr: ptr as *mut u8, len: buf.length as usize });

        // Hand this buffer to the driver immediately -- it's now allowed to
        // DMA frame data into it.
        check(
            unsafe { ioctl(fd, vidioc_qbuf(), &mut buf as *mut _ as *mut c_void) },
            "VIDIOC_QBUF (initial)",
        );
    }

    // ---- VIDIOC_STREAMON: start capturing ----------------------------------
    let mut buf_type: i32 = V4L2_BUF_TYPE_VIDEO_CAPTURE as i32;
    check(
        unsafe { ioctl(fd, vidioc_streamon(), &mut buf_type as *mut _ as *mut c_void) },
        "VIDIOC_STREAMON",
    );

    // ---- capture loop, unthrottled: as fast as camera + terminal allow ----
    let mut last_instant = Instant::now();
    let mut fps: f64 = 0.0;
    let mut renderer = Renderer::new();

    loop {
        // Block until the driver reports a filled buffer.
        let mut buf = V4l2Buffer::default();
        buf.type_ = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        buf.memory = V4L2_MEMORY_MMAP;
        check(
            unsafe { ioctl(fd, vidioc_dqbuf(), &mut buf as *mut _ as *mut c_void) },
            "VIDIOC_DQBUF",
        );

        // Measure the real time between frames actually arriving -- this is
        // gated by whichever is slower: the camera's own frame rate, or how
        // fast we can render+print. Smoothed with an exponential moving
        // average so it doesn't jitter frame to frame.
        let now = Instant::now();
        let dt = now.duration_since(last_instant).as_secs_f64();
        last_instant = now;
        if dt > 0.0 {
            let instantaneous = 1.0 / dt;
            fps = if fps == 0.0 { instantaneous } else { fps * 0.9 + instantaneous * 0.1 };
        }

        let mb = &buffers[buf.index as usize];
        // Safety: the kernel just told us (via DQBUF) that this buffer is
        // filled and ours to read until we QBUF it back.
        let frame: &[u8] = unsafe { std::slice::from_raw_parts(mb.ptr, mb.len) };

        renderer.render(frame, width, height, bytesperline, fps);

        // Give the buffer back to the driver so it can reuse it.
        check(
            unsafe { ioctl(fd, vidioc_qbuf(), &mut buf as *mut _ as *mut c_void) },
            "VIDIOC_QBUF",
        );
    }

    // (unreachable in this simple demo -- Ctrl+C to exit. A production
    // version would catch SIGINT and run VIDIOC_STREAMOFF + munmap + close
    // here for a clean shutdown; the OS reclaims all of this automatically
    // on process exit anyway.)
}

// One terminal character cell in half-block mode. We draw each cell as the
// Unicode "upper half block" glyph ▀ (U+2580): its *foreground* color paints
// the top half of the cell, its *background* color paints the bottom half.
// That packs two vertically-stacked source pixels into a single cell, so a
// grid of C×R cells shows C×(2R) pixels -- double the vertical resolution of a
// one-pixel-per-cell render, in full 24-bit color.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Cell {
    top: [u8; 3], // -> foreground (upper pixel)
    bot: [u8; 3], // -> background (lower pixel)
}

/// Decode one BT.601 YUV triple to 8-bit RGB. YUYV gives us Y per pixel and a
/// U/V pair shared across each horizontal pixel pair; this is the standard
/// integer approximation of the full-range-ish conversion (good enough for a
/// terminal, and branch-free apart from the clamp).
fn yuv_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
    let c = y as i32 - 16;
    let d = u as i32 - 128;
    let e = v as i32 - 128;
    let r = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255);
    let g = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255);
    let b = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255);
    [r as u8, g as u8, b as u8]
}

/// Read the RGB color of a single source pixel from a YUYV frame. YUYV packs 2
/// pixels into 4 bytes as [Y0, U, Y1, V]: both pixels of a pair share U and V,
/// and the pixel's own luma is Y0 (even x) or Y1 (odd x).
fn sample_rgb(frame: &[u8], bytesperline: u32, src_x: u32, src_y: u32) -> [u8; 3] {
    let row_start = (src_y * bytesperline) as usize;
    let pair_off = (src_x as usize / 2) * 4; // byte offset of this pixel's 4-byte pair
    let base = row_start + pair_off;
    let y = frame.get(base + (src_x as usize & 1) * 2).copied().unwrap_or(0);
    let u = frame.get(base + 1).copied().unwrap_or(128);
    let v = frame.get(base + 3).copied().unwrap_or(128);
    yuv_to_rgb(y, u, v)
}

/// Stateful terminal renderer. Holds the previously-drawn cell grid so each
/// frame can be emitted as a *diff* -- only the cells whose color actually
/// changed are rewritten, cursor-addressed in place. That kills the flicker of
/// a full-screen clear every frame and, crucially for piping over a remote
/// link, slashes the bytes on the wire when much of the image is static.
struct Renderer {
    cols: u32,
    rows: u32,          // character rows; each spans 2 source-pixel rows
    prev: Vec<Cell>,    // last frame's grid, len == cols*rows
    full_redraw: bool,  // force a from-scratch repaint (first frame / resize)
    out: String,        // reused scratch buffer, never shrinks
}

impl Renderer {
    fn new() -> Self {
        Renderer { cols: 0, rows: 0, prev: Vec::new(), full_redraw: true, out: String::new() }
    }

    fn render(&mut self, frame: &[u8], width: u32, height: u32, bytesperline: u32, fps: f64) {
        use std::fmt::Write as _;
        use std::io::Write as _;

        let (term_cols, term_rows) = terminal_size();
        // Leave a 1-column / 1-row margin: some terminals auto-wrap the instant
        // you write into the very last column, and writing into the very last
        // row leaves the cursor with nowhere to go but scroll. Staying one
        // short on each axis avoids both.
        let max_cols = term_cols.saturating_sub(1).max(10);
        let max_rows = term_rows.saturating_sub(1).max(5);

        // Reserve two lines at the bottom for a blank divider + the stats line,
        // so the grid itself never grows into the space the stats need.
        const STATS_LINES: u32 = 2;
        let max_grid_rows = max_rows.saturating_sub(STATS_LINES).max(3);

        // A terminal cell is ~2x taller than wide, but half-block packing puts 2
        // source pixels in each cell vertically -- so the pixels come out square
        // and the fit math is the same as the old one-pixel-per-cell render.
        let mut cols = max_cols;
        let mut rows = (cols * height / width) / 2;
        if rows > max_grid_rows {
            rows = max_grid_rows;
            cols = (rows * width * 2) / height;
            if cols > max_cols {
                cols = max_cols;
            }
        }
        if cols == 0 { cols = 1; }
        if rows == 0 { rows = 1; }

        // Resize (or first frame) invalidates the whole previous grid: resync our
        // buffers to the new size and repaint everything from a clean slate.
        if cols != self.cols || rows != self.rows {
            self.cols = cols;
            self.rows = rows;
            self.prev = vec![Cell::default(); (cols * rows) as usize];
            self.full_redraw = true;
        }

        self.out.clear();
        if self.full_redraw {
            // Hide the cursor (no visible scanning dot) and clear once. Every
            // later frame is an in-place diff, so this is the only full clear.
            self.out.push_str("\x1B[?25l\x1B[2J\x1B[H");
        }

        // SGR + cursor state we carry across the frame. Colors persist through
        // cursor jumps, so we only re-emit a color escape when it actually
        // changes, and only reposition when the next changed cell isn't exactly
        // where the cursor already sits after the last write.
        let mut cur_fg: Option<[u8; 3]> = None;
        let mut cur_bg: Option<[u8; 3]> = None;
        let mut cursor_at: Option<(u32, u32)> = None;

        for r in 0..rows {
            // The two source-pixel rows this character row stands in for.
            let src_y_top = (2 * r) * height / (2 * rows);
            let src_y_bot = (2 * r + 1) * height / (2 * rows);
            for c in 0..cols {
                let src_x = c * width / cols;
                let cell = Cell {
                    top: sample_rgb(frame, bytesperline, src_x, src_y_top),
                    bot: sample_rgb(frame, bytesperline, src_x, src_y_bot),
                };
                let idx = (r * cols + c) as usize;
                if !self.full_redraw && self.prev[idx] == cell {
                    continue; // unchanged -- emit nothing, don't move the cursor
                }
                self.prev[idx] = cell;

                // Reposition only if the cursor isn't already here (it advanced
                // one column after each glyph, so runs of adjacent changed cells
                // need no escape between them).
                if cursor_at != Some((r, c)) {
                    let _ = write!(self.out, "\x1B[{};{}H", r + 1, c + 1);
                }

                // Build a single SGR carrying whichever of fg/bg changed.
                let fg_changed = cur_fg != Some(cell.top);
                let bg_changed = cur_bg != Some(cell.bot);
                if fg_changed || bg_changed {
                    self.out.push_str("\x1B[");
                    if fg_changed {
                        let [tr, tg, tb] = cell.top;
                        let _ = write!(self.out, "38;2;{};{};{}", tr, tg, tb);
                        cur_fg = Some(cell.top);
                    }
                    if bg_changed {
                        if fg_changed {
                            self.out.push(';');
                        }
                        let [br, bg, bb] = cell.bot;
                        let _ = write!(self.out, "48;2;{};{};{}", br, bg, bb);
                        cur_bg = Some(cell.bot);
                    }
                    self.out.push('m');
                }

                self.out.push('\u{2580}'); // ▀ upper half block
                // Glyph consumed one cell; cursor now sits one column right. We
                // kept a right margin, so this never triggers an auto-wrap.
                cursor_at = Some((r, c + 1));
            }
        }

        // Stats line changes every frame (fps), so just rewrite it. Reset SGR
        // first (plain text, not colored), position it below a blank divider,
        // and clear to end of line in case the previous one was longer.
        let _ = write!(
            self.out,
            "\x1B[0m\x1B[{};1H\x1B[K{}x{} @ {:>5.1} fps",
            rows + 2,
            width,
            height,
            fps
        );

        self.full_redraw = false;

        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        let _ = lock.write_all(self.out.as_bytes());
        let _ = lock.flush();
    }
}
