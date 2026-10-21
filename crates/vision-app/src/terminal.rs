//! Terminal display: truecolor half-block rendering with per-cell diffing.
//!
//! This is the headless / FPV renderer. It draws each character cell as the
//! Unicode "upper half block" glyph `▀` (U+2580), whose *foreground* color paints
//! the top half of the cell and whose *background* color paints the bottom half.
//! That packs two vertically-stacked source pixels into one cell, so a grid of
//! C×R cells shows C×(2R) pixels in full 24-bit color. Frames are emitted as a
//! diff against the previously drawn grid -- only cells whose color changed are
//! rewritten -- which kills the flicker of a full-screen clear and slashes bytes
//! on the wire when much of the image is static.

use std::io::{self, Write};
use std::os::raw::{c_int, c_void};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

use vision_core::Frame;

use crate::{Controls, Display};

// We only need ioctl here (for the window size); same fixed-3-arg trick as the
// capture module -- see its header comment.
unsafe extern "C" {
    fn ioctl(fd: c_int, request: u64, argp: *mut c_void) -> c_int;
}

// TIOCGWINSZ / struct winsize: verified against the real system headers via an
// offsetof/sizeof probe.
const TIOCGWINSZ: u64 = 0x5413;

#[repr(C)]
struct WinSize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

/// Ask the terminal how big it currently is. Falls back to a sane default if
/// stdout isn't actually a tty (e.g. piped to a file).
fn terminal_size() -> (u32, u32) {
    let mut ws = WinSize { ws_row: 0, ws_col: 0, ws_xpixel: 0, ws_ypixel: 0 };
    let ret = unsafe { ioctl(1 /* STDOUT_FILENO */, TIOCGWINSZ, &mut ws as *mut _ as *mut c_void) };
    if ret < 0 || ws.ws_row == 0 || ws.ws_col == 0 {
        (100, 40)
    } else {
        (ws.ws_col as u32, ws.ws_row as u32)
    }
}

/// One terminal character cell in half-block mode: the top source pixel becomes
/// the glyph's foreground, the bottom pixel its background.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Cell {
    top: [u8; 3], // -> foreground (upper pixel)
    bot: [u8; 3], // -> background (lower pixel)
}

/// Stateful terminal renderer. Holds the previously-drawn cell grid so each frame
/// can be emitted as a diff, plus a smoothed estimate of its own frame rate for
/// the stats line.
pub struct TerminalDisplay {
    cols: u32,
    rows: u32,          // character rows; each spans 2 source-pixel rows
    prev: Vec<Cell>,    // last frame's grid, len == cols*rows
    full_redraw: bool,  // force a from-scratch repaint (first frame / resize)
    out: String,        // reused scratch buffer, never shrinks
    last_instant: Option<Instant>,
    fps: f64,
    interval: Option<Duration>, // pacing target set by `set_target_fps`
    raw_mode: bool,             // we switched the tty to raw mode and must undo it
}

impl TerminalDisplay {
    /// Creates the display and puts the terminal in raw mode so single keypresses
    /// can be read as transport controls (restored on drop). Raw mode disables
    /// the tty's Ctrl+C signal, so [`controls`](Display::controls) reports it as
    /// `quit` instead -- a caller must poll `controls` to be able to exit. If
    /// stdin isn't a tty, raw mode is skipped and no input is ever reported.
    pub fn new() -> Self {
        let raw_mode = terminal::enable_raw_mode().is_ok();
        TerminalDisplay {
            cols: 0,
            rows: 0,
            prev: Vec::new(),
            full_redraw: true,
            out: String::new(),
            last_instant: None,
            fps: 0.0,
            interval: None,
            raw_mode,
        }
    }

    /// Update the smoothed frame-rate estimate from the wall-clock gap since the
    /// previous `show`. This measures the real pipeline rate -- gated by whichever
    /// is slower, the camera delivering frames or us rendering them -- smoothed
    /// with an exponential moving average so it doesn't jitter frame to frame.
    fn tick_fps(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_instant {
            let dt = now.duration_since(last).as_secs_f64();
            if dt > 0.0 {
                let instantaneous = 1.0 / dt;
                self.fps = if self.fps == 0.0 {
                    instantaneous
                } else {
                    self.fps * 0.9 + instantaneous * 0.1
                };
            }
        }
        self.last_instant = Some(now);
    }
}

impl Default for TerminalDisplay {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for TerminalDisplay {
    fn show(&mut self, frame: &Frame<'_>) -> io::Result<()> {
        use std::fmt::Write as _;

        self.tick_fps();
        let width = frame.width;
        let height = frame.height;

        let (term_cols, term_rows) = terminal_size();
        // Leave a 1-column / 1-row margin: some terminals auto-wrap the instant
        // you write into the very last column, and writing into the very last row
        // leaves the cursor with nowhere to go but scroll. Staying one short on
        // each axis avoids both.
        let max_cols = term_cols.saturating_sub(1).max(10);
        let max_rows = term_rows.saturating_sub(1).max(5);

        // Reserve two lines at the bottom for a blank divider + the stats line, so
        // the grid itself never grows into the space the stats need.
        const STATS_LINES: u32 = 2;
        let max_grid_rows = max_rows.saturating_sub(STATS_LINES).max(3);

        // A terminal cell is ~2x taller than wide, but half-block packing puts 2
        // source pixels in each cell vertically -- so the pixels come out square
        // and the fit math is the same as a one-pixel-per-cell render.
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
                    top: frame.sample_rgb(src_x, src_y_top),
                    bot: frame.sample_rgb(src_x, src_y_bot),
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
        // first (plain text, not colored), position it below a blank divider, and
        // clear to end of line in case the previous one was longer.
        let _ = write!(
            self.out,
            "\x1B[0m\x1B[{};1H\x1B[K{}x{} @ {:>5.1} fps",
            rows + 2,
            width,
            height,
            self.fps
        );

        self.full_redraw = false;

        let stdout = io::stdout();
        let mut lock = stdout.lock();
        lock.write_all(self.out.as_bytes())?;
        lock.flush()?;
        drop(lock);

        // Pace to the target rate: tick_fps stamped the start of this frame, so
        // sleep off whatever of the interval the render didn't use.
        if let (Some(interval), Some(start)) = (self.interval, self.last_instant) {
            if let Some(rest) = interval.checked_sub(start.elapsed()) {
                std::thread::sleep(rest);
            }
        }
        Ok(())
    }

    fn idle(&mut self) -> io::Result<()> {
        // Nothing to repaint; just don't spin while paused.
        std::thread::sleep(self.interval.unwrap_or(Duration::from_millis(16)));
        Ok(())
    }

    fn controls(&mut self) -> Controls {
        let mut c = Controls::default();
        if !self.raw_mode {
            return c;
        }
        // Drain everything queued since the last call, without blocking.
        while event::poll(Duration::ZERO).unwrap_or(false) {
            let Ok(Event::Key(key)) = event::read() else { continue };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char(' ') => c.toggle_pause = true,
                KeyCode::Right | KeyCode::Char('n') => c.step = true,
                KeyCode::Left | KeyCode::Char('p') => c.step_back = true,
                KeyCode::Esc | KeyCode::Char('q') => c.quit = true,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    c.quit = true
                }
                _ => {}
            }
        }
        c
    }

    fn set_target_fps(&mut self, fps: usize) {
        self.interval = (fps > 0).then(|| Duration::from_secs_f64(1.0 / fps as f64));
    }

    // The terminal owns the screen; a println would corrupt the frame. The stats
    // line is the only text we draw.
    fn notice(&mut self, _msg: &str) {}
}

impl Drop for TerminalDisplay {
    fn drop(&mut self) {
        // Un-hide the cursor and reset colors so the terminal is left usable.
        let stdout = io::stdout();
        let mut lock = stdout.lock();
        let _ = lock.write_all(b"\x1B[0m\x1B[?25h");
        let _ = lock.flush();
        if self.raw_mode {
            let _ = terminal::disable_raw_mode();
        }
    }
}
