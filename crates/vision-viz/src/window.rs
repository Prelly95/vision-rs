use minifb::{Key, Window, WindowOptions};
use std::io::{self};
use std::ops::ControlFlow;

use vision_core::Frame;

use crate::Display;

pub struct WindowDisplay {
    window: Window,
    pixel_buf: Vec<u32>,
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
        })
    }
}

impl Display for WindowDisplay {
    fn show(&mut self, frame: &Frame<'_>) -> io::Result<ControlFlow<()>> {
        let width: usize = frame.width.try_into().unwrap();
        let height: usize = frame.height.try_into().unwrap();
        for y in 0..height {
            for x in 0..width {
                let rgb = frame.sample_rgb(x as u32, y as u32);
                let ii = x + y * width;
                let px: u32 =
                    ((u32::from(rgb[0])) << 16) + ((u32::from(rgb[1])) << 8) + u32::from(rgb[2]);
                self.pixel_buf[ii] = px;
            }
        }

        self.window
            .update_with_buffer(&self.pixel_buf, width, height)
            .unwrap();

        if !self.window.is_open() || self.window.is_key_down(Key::Escape) {
            return Ok(ControlFlow::Break(()));
        }
        Ok(ControlFlow::Continue(()))
    }
}
