//! vision-core — the shared data substrate every other crate builds on.
//!
//! Today this is the capture/display decoupling boundary: the [`Frame`] type and
//! its pixel-format handling. It is intended to grow into the home for image
//! buffers, pyramids, gradients (Sobel), and warping (see ROADMAP.md §2) — the
//! per-pixel primitives reused by features, flow, and SfM. Nothing here knows
//! about capture devices or renderers; those depend on this, never the reverse.

pub mod frame;

pub use frame::{Frame, PixelFormat, yuv_to_rgb};
