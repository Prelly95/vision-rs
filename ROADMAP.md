# vision-rs — Computer Vision Research Platform Roadmap

A plan for growing this repo from a single-file V4L2 ASCII webcam demo into a
workspace for researching perception and robotics CV algorithms **without
OpenCV**, so the whole pipeline can be owned and optimized directly.

Target research areas: optical flow, structure from motion (SfM), visual SLAM,
and high-throughput video transmission for FPV / remote robotics.

---

## 1. Quality-of-life & visualization

The current ASCII render is charming but a dead end for CV work — you can't see
sub-pixel flow vectors or feature matches in block characters. The single most
valuable change is to **decouple capture from display** behind a `Frame` type
and a sink trait, then add real renderers.

### Visualization

- **A real window.** [`minifb`](https://crates.io/crates/minifb) is the
  lowest-friction option: hand it a `&[u32]` ARGB buffer, get a window plus
  keyboard events. (`pixels` + `winit`, wgpu-backed, is the heavier alternative
  if GPU compute is wanted later.) Non-negotiable for debugging flow fields,
  keypoints, epipolar lines, and trajectories.
- **Keep the terminal, but upgrade it** for the headless / FPV story:
  - Truecolor half-block rendering — print `▀` with a 24-bit foreground +
    background color, giving *two* vertical pixels per cell plus color (~4× the
    effective resolution of the current block render).
  - Stop doing `\x1B[2J` (full clear) every frame; diff against the previous
    frame and emit only changed cells. Kills flicker and cuts bandwidth, which
    matters for the remote use case.
- **Overlay primitives**: point / line / rect / circle / text drawn onto a frame
  buffer. Every algorithm viz needs this. Write it once in a viz crate.
- **Colormaps**: HSV flow colorizer and a jet/turbo colormap for dense scalar
  fields (depth, flow magnitude, disparity).

### Workflow (what makes research tractable)

- **Record & replay.** Dump raw frames + timestamps to a file and read them
  back. Live-webcam nondeterminism is poison while debugging a SLAM backend —
  the same input every run is what you want. Highest-leverage QoL feature after
  the window.
- **Dataset loaders.** Standard CV datasets ship ground truth: TUM RGB-D and
  EuRoC MAV (SLAM / VO), KITTI (odometry / flow), Middlebury (flow / stereo).
  Running on these lets you *measure* drift / endpoint error instead of
  eyeballing.
- **Image I/O.** PPM/PGM is a ~10-line writer with zero deps (quick dumps); add
  the [`png`](https://crates.io/crates/png) crate for shareable output.
- **Perf/timing HUD and keyboard controls** (pause, single-step, toggle
  overlays). Single-step over recorded frames is how algorithms actually get
  debugged.

### One dependency decision

The "no OpenCV" instinct is right for the *CV algorithms* — that's where the
learning is. But linear algebra (SVD, eigensolvers, SE(3)/SO(3), least squares)
is a different rabbit hole, and SfM/SLAM lean on it heavily. Recommendation: use
[`nalgebra`](https://crates.io/crates/nalgebra) as the math substrate and
reserve from-scratch discipline for the vision itself. Add
[`rayon`](https://crates.io/crates/rayon) too — nearly all of this is
embarrassingly parallel per-pixel / per-feature.

---

## 2. Project structure

Convert the repo to a Cargo **workspace**: shared library crates + thin
binaries. The current `main.rs` becomes one app; its V4L2 code moves into an I/O
crate.

```
vision-rs/
├── Cargo.toml                # [workspace]
├── crates/
│   ├── vision-core/          # Frame, PixelFormat, color conversion (YUYV→gray/RGB),
│   │                         #   image buffers, pyramids, gradients (Sobel), warping
│   ├── vision-io/            # V4L2 capture (current FFI), record/replay,
│   │                         #   PPM/PNG, dataset loaders (TUM/KITTI/EuRoC)
│   ├── vision-viz/           # window (minifb), terminal (ascii + half-block color),
│   │                         #   overlay drawing, colormaps
│   ├── vision-math/          # SE3/SO3, quaternions, RANSAC harness, solvers
│   │                         #   (wraps nalgebra; own geometry on top)
│   ├── vision-features/      # Harris/Shi-Tomasi, FAST, BRIEF/ORB, matching
│   ├── vision-flow/          # optical flow
│   ├── vision-sfm/           # two-view geometry, triangulation, bundle adjustment
│   ├── vision-slam/          # VO → SLAM
│   └── vision-codec/         # compression + transport for FPV
├── apps/
│   ├── ascii-cam/            # today's demo, ported onto vision-io + vision-viz
│   ├── flow-viz/
│   ├── vo-demo/
│   └── fpv-tx / fpv-rx/
└── datasets/                 # gitignored
```

The crate ordering mirrors the dependency flow:
`core` ← `io` / `viz` ← `features` / `flow` ← `sfm` / `slam`. Build `core` +
`io` + `viz` first; everything else is downstream and testable against
recorded / dataset input. Put experiments in `examples/` within each crate
before promoting them to `apps/`.

---

## 3. Starting points per project

Ordered by dependency, not by priority — optical flow first because its building
blocks (gradients, pyramids, corners, warping) are reused by everything else.

### Prerequisite for all of it: camera calibration

Implement Zhang's checkerboard method to get the webcam's intrinsics +
distortion. SfM, VO, and SLAM are all metric-garbage without it. Small,
self-contained, and unblocks the rest.

### Optical flow *(do first)*

- **First milestone:** Lucas–Kanade sparse flow. Prereqs built in `vision-core`:
  Sobel gradients, Shi-Tomasi "good features to track", image pyramids.
  Visualize tracked points as arrows in the minifb window.
- **Then:** pyramidal LK (handles large motion), then a dense method —
  Horn–Schunck or Farnebäck — visualized with the HSV flow colorizer.

### Structure from Motion

- **Prereqs:** feature detection + descriptor + matching (FAST + BRIEF is the
  pragmatic combo; ORB for rotation invariance).
- **First milestone:** two-view reconstruction. Match features across two frames
  → estimate the essential matrix (normalized 8-point + RANSAC) → decompose into
  R,t → triangulate 3D points → render the point cloud. The whole pipeline in
  miniature.
- **Then:** incremental SfM — add views via PnP, refine with bundle adjustment
  (Levenberg–Marquardt over reprojection error).

### Visual SLAM

- Don't start at SLAM — start at **visual odometry** (frame-to-frame pose, no
  map, no loop closure). It's SfM's two-view step applied continuously.
- **First milestone:** track features across consecutive frames (reuse the LK
  tracker), estimate relative pose, accumulate a trajectory, draw it top-down.
  Run on TUM RGB-D and plot drift against ground truth.
- **Then:** layer on keyframes → local map → pose-graph optimization → loop
  closure. Dataset ground truth tells you each addition actually helped.

### High-throughput video transmission (FPV)

- Different muscle (networking + compression + latency), so it can proceed in
  parallel.
- **First milestone:** sender/receiver over UDP (localhost first) passing raw
  frames, with a round-trip latency + bandwidth HUD. Surfaces MTU /
  packetization immediately.
- **Then:** intra-frame compression (own DCT + quantization, JPEG-style), then
  inter-frame — and here's the synergy: **inter-frame compression is motion
  compensation, which is optical flow.** The flow crate feeds directly into the
  codec. Finish with FEC (forward error correction) for lossy RF links and
  adaptive bitrate.

---

## Immediate next steps

1. Scaffold the workspace: crate skeletons + port `ascii-cam` + a minifb window
   renderer + the record/replay harness.
2. Decide on pulling in `nalgebra` / `rayon` vs. from-scratch math.
3. Get `vision-core` + `vision-io` + a windowed viewer working — a real canvas
   before touching the first algorithm.
