# vision-rs

A Cargo workspace for computer-vision research without OpenCV. The goal is to
own the whole pipeline (capture, display, and the algorithms) and optimize it
directly. See [ROADMAP.md](ROADMAP.md) for the plan and the target research
areas (optical flow, structure from motion, visual SLAM, FPV video transport).

## Requirements

- Rust toolchain with edition 2024 support.
- Linux. The capture code talks to V4L2 through raw syscalls, so it does not
  build on other platforms yet.
- A V4L2 webcam at `/dev/video0` to run `ascii-cam`.

## Layout

Library crates go in `crates/`, binaries in `apps/`. The binaries stay thin: all
reusable logic lives in a crate.

| Crate | Role | Status |
|-------|------|--------|
| `vision-core` | `Frame`, `PixelFormat`, color conversion | implemented |
| `vision-io` | V4L2 capture (more sources planned) | implemented |
| `vision-viz` | `Display` trait, terminal renderer | implemented |
| `vision-math` | SE(3)/SO(3), solvers | stub |
| `vision-features` | corners, descriptors, matching | stub |
| `vision-flow` | optical flow | stub |
| `vision-sfm` | structure from motion | stub |
| `vision-slam` | visual odometry, SLAM | stub |
| `vision-codec` | compression and transport | stub |

| App | Role | Status |
|-----|------|--------|
| `ascii-cam` | webcam viewer in the terminal | implemented |
| `flow-viz` | optical flow viewer | stub |
| `vo-demo` | visual odometry demo | stub |
| `fpv-tx` / `fpv-rx` | FPV video sender and receiver | stub |

A stub crate holds only a doc comment. A stub app prints a message and exits.

The dependency flow is `core` ← `io` / `viz` ← `features` / `flow` ← `sfm` /
`slam`. The root `Cargo.toml` declares each internal crate once in
`[workspace.dependencies]`. A member pulls one in with
`vision-core.workspace = true`. Version and edition also come from the root
through `[workspace.package]`.

## Build

Build every crate and app:

```
cargo build --workspace
```

Build one crate:

```
cargo build -p vision-core
```

## Run

The workspace has more than one binary, so name the app with `-p`:

```
cargo run --release -p ascii-cam
```

`ascii-cam` opens `/dev/video0`, negotiates 640x480 YUYV, and draws the video in
the terminal with truecolor half-block characters. Use `--release`. A debug
build is too slow for a smooth frame rate. Press `Ctrl+C` to quit. The renderer
restores the cursor on exit.

The stub apps run but do no work yet:

```
cargo run -p flow-viz
```

## Develop

- Add a new renderer (a window, for example): add a module to `vision-viz` that
  implements the `Display` trait. The capture side does not change.
- Add a new frame source: produce a `vision_core::Frame` from it in `vision-io`.
  The display side does not change.
- Promote a stub to real code: fill in its crate, then build the matching app on
  top.
- Put experiments in an `examples/` directory inside a crate before you promote
  them to a new app.
- The `datasets/` directory is git-ignored. Put large dataset files there.
