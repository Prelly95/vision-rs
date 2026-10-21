// demo-vision — the dev harness: pick a frame source, a processor, and a display
// from the command line instead of keeping one throwaway app per combination.
//
// Sources, processors and wrappers are chosen at runtime, so they are boxed
// (`Box<dyn FrameSource>` / `Box<dyn Processor>`); the library itself stays
// generic and unaffected. To expose a new source or processor, add a variant
// below and one match arm in `build_source` / `build_processor`.
//
// Run:  cargo run --release -p demo-vision -- webcam
//       cargo run --release -p demo-vision -- --proc sobel --overlay lighten --stats video clip.mp4
//       cargo run --release -p demo-vision -- --display terminal webcam
//       cargo run --release -p demo-vision -- images datasets/kitti/.../image_00/data

use std::io;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use vision_app::{Display, TerminalDisplay};
use vision_core::{Blend, Overlay, Passthrough, Processor, SobelEdges, Stats};
use vision_io::{FrameSource, ImageSequence, V4l2Capture, VideoFile};

#[derive(Parser)]
#[command(about = "Run a frame source through a processor and show the result")]
struct Cli {
    /// Algorithm applied to each frame.
    #[arg(long = "proc", value_enum, default_value_t = Proc::Passthrough, global = true)]
    proc: Proc,

    /// Composite the processor's output over the original video.
    #[arg(long, value_enum, global = true)]
    overlay: Option<OverlayMode>,

    /// Opacity of the result for `--overlay alpha` (0.0 = video, 1.0 = result).
    #[arg(long, default_value_t = 0.5, global = true)]
    alpha: f32,

    /// Print fps and per-frame processing time to stderr.
    #[arg(long, global = true)]
    stats: bool,

    /// Where to present the result.
    #[arg(long, value_enum, default_value_t = Out::Window, global = true)]
    display: Out,

    #[command(subcommand)]
    source: Source,
}

#[derive(Subcommand)]
enum Source {
    /// Live V4L2 webcam.
    Webcam {
        /// Capture device.
        #[arg(long, default_value = "/dev/video0")]
        device: String,
        #[arg(long, default_value_t = 640)]
        width: u32,
        #[arg(long, default_value_t = 480)]
        height: u32,
    },
    /// Video file (anything FFmpeg can demux).
    Video { path: String },
    /// Directory of PNG frames (e.g. a KITTI image_NN/data folder).
    Images { dir: PathBuf },
}

#[derive(Clone, Copy, ValueEnum)]
enum Proc {
    Passthrough,
    Sobel,
}

#[derive(Clone, Copy, ValueEnum)]
enum OverlayMode {
    Alpha,
    Lighten,
    Add,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Out {
    Window,
    Terminal,
}

fn build_source(source: &Source) -> io::Result<(Box<dyn FrameSource>, String)> {
    Ok(match source {
        Source::Webcam {
            device,
            width,
            height,
        } => {
            let cap = V4l2Capture::open(device, *width, *height)?;
            eprintln!(
                "negotiated format: {}x{}, bytesperline={}",
                cap.width(),
                cap.height(),
                cap.bytes_per_line()
            );
            (Box::new(cap), format!("Webcam {device}"))
        }
        Source::Video { path } => {
            let video = VideoFile::open(path)?;
            eprintln!(
                "{path}: {}x{} @ {:.2} fps",
                video.width(),
                video.height(),
                video.fps()
            );
            (Box::new(video), path.clone())
        }
        Source::Images { dir } => {
            let seq = ImageSequence::open(dir)?;
            eprintln!(
                "{}: {}x{}, {} frames, target fps={}",
                dir.display(),
                seq.width(),
                seq.height(),
                seq.len(),
                seq.target_fps().unwrap_or(0.0)
            );
            (Box::new(seq), dir.display().to_string())
        }
    })
}

/// Innermost algorithm, then optional overlay, then optional stats outermost
/// (so stats cover the whole chain).
fn build_processor(cli: &Cli) -> Box<dyn Processor> {
    let mut p: Box<dyn Processor> = match cli.proc {
        Proc::Passthrough => Box::new(Passthrough),
        Proc::Sobel => Box::new(SobelEdges::default()),
    };
    if let Some(mode) = cli.overlay {
        let blend = match mode {
            OverlayMode::Alpha => Blend::Alpha(cli.alpha),
            OverlayMode::Lighten => Blend::Lighten,
            OverlayMode::Add => Blend::Add,
        };
        p = Box::new(Overlay::new(p, blend));
    }
    if cli.stats {
        p = Box::new(Stats::new(p));
    }
    p
}

fn build_sink<F: FrameSource>(cli: &Cli, source: &F, title: &str) -> io::Result<Box<dyn Display>> {
    let disp: Box<dyn Display> = match cli.display {
        Out::Window => Box::new(vision_app::WindowDisplay::new(
            title,
            source.width() as usize,
            source.height() as usize,
        )?),
        Out::Terminal => Box::new(vision_app::TerminalDisplay::new()),
    };
    return Ok(disp);
}

fn print_controls() {
    println!("controls:");
    println!("  Space        play / pause");
    println!("  Right  or  N  step one frame forward (pauses)");
    println!("  P  or  Left   step one frame back (pauses)");
    println!("  Q  or  Esc    quit");
}

fn run(cli: Cli) -> io::Result<()> {
    let (source, title) = build_source(&cli.source)?;
    let processor = build_processor(&cli);

    print_controls();
    let display = build_sink(&cli, &source, &title)?;
    vision_app::run(source, processor, display)
}

fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("demo-vision: {e}");
        std::process::exit(1);
    }
}
