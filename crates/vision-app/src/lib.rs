//! vision-app — the presentation layer: frame *sinks* plus the per-frame harness.
//!
//! Two things live here, which used to be split across a separate viz crate:
//!
//! - **Sinks** — anything that presents a [`vision_core::Frame`] to a human:
//!   [`WindowDisplay`] (a real window via minifb) and [`TerminalDisplay`]
//!   (truecolor half-block ASCII), behind the [`Display`] trait.
//! - **The harness** — [`run`], which ties a [`FrameSource`] (webcam or video
//!   file), a [`Processor`] (the algorithm), and a window together into one
//!   playback loop with transport controls (play / pause / step forward and back
//!   / quit). Every algorithm demo is just "pick a source, write a `Processor`,
//!   call `run`".
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
pub use window::{Controls, WindowDisplay, WindowKey};

use std::collections::VecDeque;
use std::io;
use std::ops::ControlFlow;

use vision_core::{Canvas, Frame, Processor};
use vision_io::FrameSource;

/// A sink that presents frames. One `show` call renders one frame; the display
/// owns whatever state it needs across calls (diff buffers, an open window, an
/// fps estimate).
pub trait Display {
    /// Present a single frame. `Err` means the underlying output failed (e.g. a
    /// broken pipe on stdout). `Ok(ControlFlow::Break(()))` means the user asked
    /// to stop (e.g. closed the window) -- a clean exit, not a failure --
    /// otherwise `Ok(ControlFlow::Continue(()))`.
    fn show(&mut self, frame: &Frame<'_>) -> io::Result<ControlFlow<()>>;
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

/// Run `processor` over every frame from `source`, showing the result in a
/// window titled `title`, with a default history budget. See [`run_with_history`].
pub fn run<S: FrameSource, P: Processor>(source: S, processor: P, title: &str) -> io::Result<()> {
    run_with_history(source, processor, title, DEFAULT_HISTORY_BUDGET_BYTES)
}

/// Like [`run`], but with an explicit history memory budget (bytes). The ring
/// buffer holds `budget / frame_bytes` frames (at least 2).
///
/// Playback is paced to the source's [`FrameSource::target_fps`] (a file plays
/// at its real rate; a webcam runs capped at 60). Transport controls
/// ([`Controls`]):
///
/// - **Space** — play / pause
/// - **Right** or **N** — step one frame forward (pauses)
/// - **P** or **Left** — step one frame back, within the retained history (pauses)
/// - **Q** or **Esc** — quit
///
/// At end of stream the last result stays on screen, paused; you can still step
/// back and forth through the retained history before quitting.
pub fn run_with_history<S: FrameSource, P: Processor>(
    mut source: S,
    mut processor: P,
    title: &str,
    history_budget_bytes: usize,
) -> io::Result<()> {
    let (w, h) = (source.width(), source.height());
    let mut display = WindowDisplay::new(title, w as usize, h as usize)?;
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

    print_controls();

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
                    display.render(&history[pos].as_frame())?;
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
                            display.render(&history[pos].as_frame())?;
                        }
                        None => {
                            if !ended {
                                ended = true;
                                paused = true;
                                println!(
                                    "end of stream — paused (P/Left to review, Q/Esc to quit)"
                                );
                            }
                            display.pump()?;
                        }
                    }
                }
            }
            Dir::Back => {
                // At the oldest retained frame this stays put.
                pos = pos.saturating_sub(1);
                if history.is_empty() {
                    display.pump()?;
                } else {
                    display.render(&history[pos].as_frame())?;
                }
            }
            Dir::Stay => {
                display.pump()?;
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

fn print_controls() {
    println!("controls:");
    println!("  Space        play / pause");
    println!("  Right  or  N  step one frame forward (pauses)");
    println!("  P  or  Left   step one frame back (pauses)");
    println!("  Q  or  Esc    quit");
}
