# vision-rs

A Cargo workspace for computer-vision research without OpenCV. The goal is to
own the whole pipeline (capture, display, and the algorithms) and optimize it
directly. See [ROADMAP.md](ROADMAP.md) for the plan and the target research
areas (optical flow, structure from motion, visual SLAM, FPV video transport).

## Requirements

- Rust toolchain with edition 2024 support.
- Linux. The capture code talks to V4L2 through raw syscalls, so it does not
  build on other platforms yet.
- A V4L2 webcam at `/dev/video0` to use the `webcam` source of the `demo-vision` app.
- FFmpeg development libraries (`libavcodec`, `libavformat`, `libavutil`, `libswscale`) and
  `clang`, to build `vision-io`. It uses them to decode video files. The
  `ffmpeg-next` version in `crates/vision-io/Cargo.toml` must match the major
  version of the system FFmpeg (`pkg-config --modversion libavcodec`). It is
  currently `9.0`, for FFmpeg 9 on Arch.
- A desktop session for the window apps (`minifb`).

## Layout

Library crates go in `crates/`, binaries in `apps/`. The binaries stay thin: all
reusable logic lives in a crate.

| Crate | Role | Status |
|-------|------|--------|
| `vision-core` | `Frame`, `PixelFormat` (YUYV, RGB24, Gray8), color conversion, `Canvas` draw primitives, `Processor` trait, `Overlay`/`Blend`, Sobel edges | implemented |
| `vision-io` | `FrameSource` trait, V4L2 webcam capture, video-file decode (FFmpeg), image-sequence reader (`ImageSequence`, PNG) | implemented |
| `vision-app` | `Display` sinks (window, terminal) and the per-frame harness `run` | implemented |
| `vision-math` | SE(3)/SO(3), solvers | stub |
| `vision-features` | corners, descriptors, matching | stub |
| `vision-flow` | optical flow | stub |
| `vision-sfm` | structure from motion | stub |
| `vision-slam` | visual odometry, SLAM | stub |
| `vision-codec` | compression and transport | stub |

| App | Role | Status |
|-----|------|--------|
| `demo-vision` | dev harness: any source, processor and display, chosen on the command line | implemented |
| `flow-viz` | optical flow viewer | stub |
| `vo-demo` | visual odometry demo | stub |
| `fpv-tx` / `fpv-rx` | FPV video sender and receiver | stub |

A stub crate holds only a doc comment. A stub app prints a message and exits.

### Design

Capture, processing and display are separate pieces. They meet only at
`vision_core::Frame`.

- **Sources** implement `vision_io::FrameSource`. `V4l2Capture` (webcam),
  `VideoFile` (FFmpeg decode) and `ImageSequence` (a directory of PNGs) all do. A new source, such as a dataset loader,
  is a new `FrameSource`. Nothing downstream changes.
- **Processors** implement `vision_core::Processor`. This is where an
  algorithm goes. `SobelEdges` is the reference one. `Passthrough` (in
  `vision-core`) is the identity. `Overlay::new(processor, Blend)` composites a
  processor's output over the original frame (`Alpha`, `Lighten` or `Add`).
- **Sinks** implement `vision_app::Display`: `WindowDisplay` (minifb) and
  `TerminalDisplay` (truecolor half-block characters).
- **The harness** `vision_app::run(source, processor, title)` ties the three
  together in one playback loop. A demo is "pick a source, pick a processor,
  call `run`". `Canvas` is the drawing surface for overlays (pixels, lines,
  rects).

`vision-app` replaces the `vision-viz` crate named in earlier plans. The sinks
and the harness now live in one layer.

The dependency flow is `core` ← `io` / `app` ← `features` / `flow` ← `sfm` /
`slam`. The root `Cargo.toml` declares each internal crate once in
`[workspace.dependencies]`. A member pulls one in with
`vision-core.workspace = true`. Version and edition also come from the root
through `[workspace.package]`.

## Build

Build every crate and app:

```
cargo build --workspace
```

If the build fails in `ffmpeg-sys-next` with a bindgen error (for example
`gnu/stubs-32.h not found`), `LIBCLANG_PATH` probably points at another
toolchain's clang, such as an ESP/Xtensa one. Point it at the system clang:

```
LIBCLANG_PATH=/usr/lib cargo build --workspace
```

If the error is `avfft.h not found`, the `ffmpeg-next` version is older than the
system FFmpeg. Bump it.

Build one crate:

```
cargo build -p vision-core
```

## Run

The workspace has more than one binary, so name the app with `-p`. Use
`--release`. A debug build is too slow for a smooth frame rate.

```
cargo run --release -p demo-vision -- webcam
cargo run --release -p demo-vision -- --proc sobel --overlay lighten --stats webcam
cargo run --release -p demo-vision -- video /path/to/video.mp4
cargo run --release -p demo-vision -- --display terminal webcam
cargo run --release -p demo-vision -- images /path/to/png/dir
```

`demo-vision` picks its pieces at run time. Run it with `--help` for the full list.

- Sources: `webcam` (`/dev/video0`, 640x480 by default), `video <path>` (any
  file FFmpeg can demux) and `images <dir>` (a directory of PNG frames, such as
  a KITTI `image_00/data` folder).
- `--proc passthrough|sobel` picks the algorithm.
- `--overlay alpha|lighten|add` blends the result over the original video.
  `--alpha` sets the opacity for `alpha`.
- `--stats` prints fps and processing time.
- `--display window|terminal` picks the output. The terminal output restores the
  cursor on exit.

The `demo-vision` app uses these controls:

| Key | Action |
|-----|--------|
| `Space` | play / pause |
| `Right` / `N` | step forward one frame |
| `Left` / `P` | step back one frame |
| `Q` / `Esc` | quit |

Stepping back replays stored frames. It does not re-run the processor, so a
stateful algorithm is never disturbed.

The stub apps run but do no work yet:

```
cargo run -p flow-viz
```

## KITTI dataset

[KITTI](https://www.cvlibs.net/datasets/kitti/) gives ground truth to measure
against (odometry, flow, stereo). Put it under `datasets/kitti/`. That directory
is git-ignored.

A raw "synced" drive is laid out like this:

```
datasets/kitti/2011_09_26/
├── calib_cam_to_cam.txt            # camera intrinsics, rectification
├── calib_imu_to_velo.txt           # IMU -> LiDAR extrinsics
├── calib_velo_to_cam.txt           # LiDAR -> camera extrinsics
└── 2011_09_26_drive_0011_sync/
    ├── image_00/ image_01/         # grayscale stereo pair
    ├── image_02/ image_03/         # color stereo pair
    │   ├── data/0000000000.png ... # one PNG per frame
    │   └── timestamps.txt
    ├── oxts/                       # GPS/IMU: ground-truth pose, one file per frame
    │   ├── data/  dataformat.txt  timestamps.txt
    └── velodyne_points/            # LiDAR scans
```

Steps:

1. Download a drive from the [raw data page](https://www.cvlibs.net/datasets/kitti/raw_data.php)
   (the `[synced+rectified data]` zip) and unzip it into `datasets/kitti/`.
2. Download the matching calibration zip for the same date (`[calibration]`)
   and unzip it in the same place. The `calib_*.txt` files give the camera
   matrices you need for anything metric (SfM, VO, SLAM).
3. Frames are numbered PNGs with one timestamp per line in `timestamps.txt`.
   `oxts/data/*.txt` holds the ground-truth pose for the same frame index, so
   frame `i` in `image_02/data` lines up with `oxts/data/<i>.txt`.

Read a camera directly with `ImageSequence`. It decodes the PNGs in filename
order, with no lossy re-encode. `image_00` and `image_01` load as `Gray8`,
`image_02` and `image_03` as `Rgb24`. It reads `timestamps.txt` next to `data/`
to pace playback (10 fps for KITTI) and exposes the frame index, so you can look
up `oxts/data/<i>.txt`. A corrupt or missing PNG stops playback with an error
that names the file.

```rust
let seq = vision_io::ImageSequence::open("datasets/kitti/2011_09_26/2011_09_26_drive_0011_sync/image_02/data")?;
vision_app::run(seq, vision_core::Passthrough, "KITTI")?;
```

There is no KITTI app yet. Wire the snippet into a small binary, or swap
`Passthrough` for your own `Processor`. The OXTS and calibration parsers are
planned (see [ROADMAP.md](ROADMAP.md)).

KITTI is recorded at 10 Hz. Positions, LiDAR points and camera images each live
in their own coordinate frame. Read
[KITTI coordinate transformations](https://medium.com/data-science/kitti-coordinate-transformations-125094cd42fb)
before you project LiDAR points into an image or compare an estimated trajectory
with the OXTS ground truth.

## Develop

- Add a new renderer (a window, for example): implement `vision_app::Display`
  in `vision-app`. The capture side does not change.
- Add a new frame source: implement `vision_io::FrameSource` in `vision-io`. The
  display side does not change.
- Add a new algorithm: implement `vision_core::Processor`, then pass it to
  `vision_app::run`. Then add a variant for it in `apps/demo-vision`.
- Promote a stub to real code: fill in its crate, then build the matching app on
  top.
- Put experiments in an `examples/` directory inside a crate before you promote
  them to a new app.
- The `datasets/` directory is git-ignored. Put large dataset files there.
