//! vision-codec — compression + transport for FPV / remote robotics.
//!
//! Planned (ROADMAP.md §3): intra-frame compression (own DCT + quantization,
//! JPEG-style), then inter-frame — which *is* motion compensation, so it feeds
//! off `vision-flow` — finishing with FEC and adaptive bitrate for lossy RF
//! links.
//!
//! Not yet implemented.
