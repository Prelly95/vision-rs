# Changelog

## pre-release

No versions have been formally released yet. This section is organised by commit

### Replace the demo apps with the `demo-vision` dev harness:

- Added the `demo-vision` app: a clap CLI that picks the frame source, processor,
  overlay, stats and display at run time.
- Added `Box<dyn FrameSource>` and `Box<dyn Processor>` impls so those choices
  can be made at run time.
- Removed `ascii-cam`, `window-cam`, `video-player`, `edge-viz` and
  `image-sequence-demo`. `demo-vision` covers all of them.

### Add image sequence source and stats processor:

- Added the `image-sequence-demo` app and an image-sequence source in
  `vision-io` (`sequence.rs`), with an integration test against a KITTI
  sequence (`tests/kitti_sequence.rs`).
- Added `passthrough` and `stats` modules to `vision-core`.
- Extended `Frame` and `Plane`, and reworked the video decoding in
  `vision-io`.
- Updated `edge-viz`, `video-player` and `window-cam` to match.
- Expanded the README and ROADMAP.

### Video playback and a workspace refactor:

- Added the `video-player` app and video file decoding in `vision-io`.
- Added the `edge-viz` app with an edge detector in `vision-core`.
- Added a `Source` abstraction in `vision-io`, so live capture and video files
  share one interface.
- Added `Canvas`, `Overlay` and `Plane` types, and a `process` module, to
  `vision-core`.
- Renamed `vision-viz` to `vision-app`, which now has a shared app runner
  alongside the terminal and window renderers.
- Reworked the V4L2 capture code.
- Updated the ROADMAP.

### Split the single file into a Cargo workspace:

- Created the workspace layout: library crates under `crates/` and thin
  binaries under `apps/`.
- Added crates `vision-core`, `vision-io`, `vision-viz`, `vision-math`,
  `vision-features`, `vision-flow`, `vision-sfm`, `vision-slam` and
  `vision-codec`. Most are stubs for now.
- Added the `Frame` type in `vision-core`.
- Moved V4L2 capture into `vision-io` and the ASCII renderer into
  `vision-viz`.
- Added a windowed renderer and the `window-cam` app.
- Added the `ascii-cam` app, plus placeholder `flow-viz`, `fpv-rx`, `fpv-tx`
  and `vo-demo` apps.
- Added the README.

### Initial commit:

- Single-file V4L2 webcam demo that renders to the terminal as ASCII.
- Added ROADMAP.md, which sets out the plan for optical flow, SfM, visual SLAM
  and FPV video transmission without OpenCV.
