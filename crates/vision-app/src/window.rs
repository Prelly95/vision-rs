use minifb::{Key, KeyRepeat, Window, WindowOptions};
use std::io;

use vision_core::Frame;

use crate::{Controls, Display};

// Re-exported so apps can name keys (for their own hotkeys) through vision-app
// without taking a direct dependency on minifb -- the windowing backend stays an
// implementation detail of this crate.
pub use minifb::Key as WindowKey;

pub struct WindowDisplay {
    window: Window,
    pixel_buf: Vec<u32>,
    width: usize,
    height: usize,
}

impl WindowDisplay {
    pub fn new(title: &str, width: usize, height: usize) -> io::Result<Self> {
        let mut new_window = Window::new(title, width, height, WindowOptions::default())
            .map_err(io::Error::other)?;

        new_window.set_target_fps(60);
        let buffer: Vec<u32> = vec![0; width * height];

        Ok(WindowDisplay {
            window: new_window,
            pixel_buf: buffer,
            width,
            height,
        })
    }

    /// Cap how fast [`render`](Self::render) / [`pump`](Self::pump) run. Playback
    /// uses this to pace a file to its real frame rate; the window blocks inside
    /// the backing update call to hold the rate.
    pub fn set_target_fps(&mut self, fps: usize) {
        self.window.set_target_fps(fps);
    }

    /// Blit one frame into the window and present it. Unlike [`show`](Self::show)
    /// this makes no decision about quitting -- callers that want custom controls
    /// render, then inspect [`is_open`](Self::is_open) and the key helpers
    /// themselves.
    pub fn render(&mut self, frame: &Frame<'_>) -> io::Result<()> {
        // Sample into our ARGB buffer. The buffer is sized to the window; clamp
        // the frame's extent to it so a frame that doesn't match the window
        // exactly can't write out of bounds.
        let w = (frame.width as usize).min(self.width);
        let h = (frame.height as usize).min(self.height);
        for y in 0..h {
            for x in 0..w {
                let rgb = frame.sample_rgb(x as u32, y as u32);
                let px = (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]);
                self.pixel_buf[x + y * self.width] = px;
            }
        }
        self.window
            .update_with_buffer(&self.pixel_buf, self.width, self.height)
            .map_err(io::Error::other)
    }

    /// Process window events and repaint the last presented buffer *without*
    /// supplying a new frame. Used while paused: keeps the window responsive (so
    /// key presses are seen and the OS doesn't mark it "not responding") while
    /// the same image stays on screen.
    pub fn pump(&mut self) -> io::Result<()> {
        self.window.update();
        Ok(())
    }

    /// Whether the window is still open (false once the user closes it).
    pub fn is_open(&self) -> bool {
        self.window.is_open()
    }

    /// True on the single update where `key` transitioned to pressed -- i.e. one
    /// event per physical press, not held auto-repeat. The right edge-triggered
    /// behavior for toggles (pause) and one-shots (step). Only reflects presses
    /// seen by the most recent [`render`](Self::render) / [`pump`](Self::pump).
    pub fn was_key_pressed(&self, key: WindowKey) -> bool {
        self.window.is_key_pressed(key, KeyRepeat::Yes)
    }

    /// True for as long as `key` is held down.
    pub fn is_key_down(&self, key: WindowKey) -> bool {
        self.window.is_key_down(key)
    }

    /// Decode the standard transport controls from the current key state. Call
    /// after [`render`](Self::render) / [`pump`](Self::pump), which refresh that
    /// state. See [`Controls`].
    pub fn read_controls(&self) -> Controls {
        Controls {
            toggle_pause: self.was_key_pressed(Key::Space),
            step: self.was_key_pressed(Key::Right) || self.was_key_pressed(Key::N),
            step_back: self.was_key_pressed(Key::P) || self.was_key_pressed(Key::Left),
            quit: !self.is_open() || self.was_key_pressed(Key::Q) || self.is_key_down(Key::Escape),
        }
    }
}

impl Display for WindowDisplay {
    fn show(&mut self, frame: &Frame<'_>) -> io::Result<()> {
        self.render(frame)
    }

    fn idle(&mut self) -> io::Result<()> {
        self.pump()
    }

    fn controls(&mut self) -> Controls {
        self.read_controls()
    }

    fn set_target_fps(&mut self, fps: usize) {
        WindowDisplay::set_target_fps(self, fps);
    }
}
