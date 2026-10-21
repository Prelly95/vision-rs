//! vision-app — the presentation layer: frame *sinks* plus the per-frame harness.
//!
//! Two things live here, which used to be split across a separate viz crate:
//!
//! - **Sinks** — anything that presents a [`vision_core::Frame`] to a human:
//!   [`WindowDisplay`] (a real window via minifb) and [`TerminalDisplay`]
//!   (truecolor half-block ASCII), behind the [`Display`] trait.
//! - **The harness** — [`run`], which ties a [`FrameSource`] (webcam or video
//!   file), a [`Processor`] (the algorithm), and any [`Display`] together into one
//!   playback loop with transport controls (play / pause / step forward and back
//!   / quit). The harness is generic over the sink, so the same loop drives a
//!   window or a terminal. Every algorithm demo is just "pick a source, write a
//!   `Processor`, pick a sink, call `run`" ([`run_window`] is the shorthand for
//!   the common window case).
//!
//! It sits above both vision-io (sources) and vision-core (Frame/Canvas/
//! Processor) because it bridges capture and display; nothing in those lower
//! layers depends on it.
//!
//! ## Stepping backward
//!
//! Video is forward-only at the codec level (a frame is a delta from earlier
//! ones), so we don't ask the decoder to run in reverse. Instead the harness
//! keeps a bounded ring buffer of recently *produced* output frames; stepping
//! back replays an earlier one from memory. This is frame-accurate, needs no
//! seeking, and works identically for a live webcam (scrub back through the last
//! captured frames). It replays past *output* -- it does not re-run the
//! processor -- so a stateful algorithm's results are shown exactly as they were
//! computed in forward order, and its internal state is never disturbed.

pub mod terminal;
pub mod window;

pub use terminal::TerminalDisplay;
pub use window::{WindowDisplay, WindowKey};

use std::collections::VecDeque;
use std::io;

use vision_core::{Canvas, Frame, Processor};
use vision_io::FrameSource;

/// Transport-control intent, decoded by a [`Display`] from whatever input it
/// has (window keys, terminal keypresses) as of its most recent
/// [`show`](Display::show) / [`idle`](Display::idle) / [`controls`](Display::controls)
/// call. Each field is edge-triggered: true once per press, not while held.
/// Sinks map their own keys to these, so the playback loop reads *what the user
/// wants* instead of raw keycodes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    /// Space was pressed: flip play/pause.
    pub toggle_pause: bool,
    /// Right arrow or N was pressed: advance exactly one frame.
    pub step: bool,
    /// P or Left was pressed: go back one frame.
    pub step_back: bool,
    /// The user asked to quit: window closed, or Q / Esc (Ctrl+C in a terminal).
    pub quit: bool,
}

/// A sink that presents frames and reports user input. One `show` call renders
/// one frame; the display owns whatever state it needs across calls (diff
/// buffers, an open window, an fps estimate). Only `show` is required: a
/// display with no input (a recorder, a null sink) can leave the rest at their
/// defaults.
pub trait Display {
    /// Present a single frame. `Err` means the underlying output failed (e.g. a
    /// broken pipe on stdout).
    fn show(&mut self, frame: &Frame<'_>) -> io::Result<()>;

    /// Keep the sink alive without a new frame (e.g. while paused): process
    /// events and leave the last frame on screen. Should not busy-spin, so
    /// sinks that don't block internally should sleep briefly.
    fn idle(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// Transport controls requested since the last call. Call after
    /// [`show`](Self::show) / [`idle`](Self::idle). The default reports none.
    fn controls(&mut self) -> Controls {
        Controls::default()
    }

    /// Hint the rate the source wants to play at; a sink that paces itself
    /// should hold to it. The default ignores it.
    fn set_target_fps(&mut self, _fps: usize) {}

    /// A short human-readable status message from the harness (e.g. end of
    /// stream). The default prints to stdout; sinks that own the screen should
    /// override this so the message doesn't corrupt their output.
    fn notice(&mut self, msg: &str) {
        println!("{msg}");
    }
}

impl<D: Display + ?Sized> Display for Box<D> {
    fn show(&mut self, frame: &Frame<'_>) -> io::Result<()> {
        (**self).show(frame)
    }
    fn idle(&mut self) -> io::Result<()> {
        (**self).idle()
    }
    fn controls(&mut self) -> Controls {
        (**self).controls()
    }
    fn set_target_fps(&mut self, fps: usize) {
        (**self).set_target_fps(fps)
    }
    fn notice(&mut self, msg: &str) {
        (**self).notice(msg)
    }
}

/// Default cap on the backward-history ring buffer, as a memory budget rather
/// than a frame count so it auto-adapts to resolution (a few seconds of SD, a
/// fraction of a second of 4K). See [`run_with_history`].
pub const DEFAULT_HISTORY_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// Which way the loop moves this iteration.
enum Dir {
    /// Advance: replay the next cached frame, or decode a new one at the head.
    Forward,
    /// Go back one cached frame.
    Back,
    /// Hold on the current frame (paused, no step pending).
    Stay,
}

/// Run `processor` over every frame from `source`, presenting the result on
/// `display`, with a default history budget. See [`run_with_history`].
pub fn run<S: FrameSource, P: Processor, D: Display>(
    source: S,
    processor: P,
    display: D,
) -> io::Result<()> {
    run_with_history(source, processor, display, DEFAULT_HISTORY_BUDGET_BYTES)
}

/// Like [`run`], but with an explicit history memory budget (bytes). The ring
/// buffer holds `budget / frame_bytes` frames (at least 2).
///
/// Playback is paced to the source's [`FrameSource::target_fps`] (a file plays
/// at its real rate; a webcam runs capped at 60),
pub fn run_with_history<S: FrameSource, P: Processor, D: Display>(
    mut source: S,
    mut processor: P,
    mut display: D,
    history_budget_bytes: usize,
) -> io::Result<()> {
    let (w, h) = (source.width(), source.height());
    let fps = source.target_fps().unwrap_or(60.0);
    display.set_target_fps((fps.round() as usize).clamp(1, 240));

    // History of produced output frames. `scratch` is the reusable surface the
    // processor draws into each new frame; a snapshot of it is pushed into
    // `history`. `cap` bounds the ring by the memory budget.
    let frame_bytes = (w as usize * h as usize * 3).max(1);
    let cap = (history_budget_bytes / frame_bytes).max(2);
    let mut history: VecDeque<Canvas> = VecDeque::new();
    let mut scratch = Canvas::new(w, h);
    let mut pos: usize = 0; // index into `history` of the frame currently shown

    let mut paused = false;
    let mut ended = false; // source has no more new frames
    let mut pending: Option<Dir> = None; // a one-shot step requested last iteration

    loop {
        // At the head when the shown frame is the newest we hold.
        let at_head = pos + 1 >= history.len();
        // Playing advances automatically; but once the source is exhausted we
        // only keep moving while there's still cached history ahead of us.
        let stream_exhausted = ended && at_head;
        let playing = !(paused || stream_exhausted);

        let mv = match pending.take() {
            Some(dir) => dir, // explicit step (always honored)
            None if playing => Dir::Forward,
            None => Dir::Stay,
        };

        match mv {
            Dir::Forward => {
                if pos + 1 < history.len() {
                    // Replay the next already-produced frame from the ring.
                    pos += 1;
                    display.show(&history[pos].as_frame())?;
                } else {
                    // At the head: produce a new frame from the source.
                    match source.next_frame()? {
                        Some(frame) => {
                            processor.process(&frame, &mut scratch);
                            if history.len() == cap {
                                history.pop_front();
                            }
                            history.push_back(scratch.clone());
                            pos = history.len() - 1;
                            display.show(&history[pos].as_frame())?;
                        }
                        None => {
                            if !ended {
                                ended = true;
                                paused = true;
                                display.notice(
                                    "end of stream — paused (P/Left to review, Q/Esc to quit)",
                                );
                            }
                            display.idle()?;
                        }
                    }
                }
            }
            Dir::Back => {
                // At the oldest retained frame this stays put.
                pos = pos.saturating_sub(1);
                if history.is_empty() {
                    display.idle()?;
                } else {
                    display.show(&history[pos].as_frame())?;
                }
            }
            Dir::Stay => {
                display.idle()?;
            }
        }

        // Handle input (fresh: the present call above refreshed key state).
        let controls = display.controls();
        if controls.quit {
            break;
        }
        if controls.toggle_pause {
            paused = !paused;
        }
        if controls.step {
            paused = true;
            pending = Some(Dir::Forward);
        }
        if controls.step_back {
            paused = true;
            pending = Some(Dir::Back);
        }
    }

    Ok(())
}
