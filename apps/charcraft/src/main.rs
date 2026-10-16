//! `charcraft`: the desktop app.
//!
//! ```text
//! charcraft [FILE]                         open a .puppet, .chproj, .psd, .ai, .svg or .charcraft (the demo puppet without)
//! charcraft --smoke [FILE]                 render one frame offscreen and exit 0 (no window needed)
//! charcraft --screenshot OUT.png [FILE] [--mode rig|record|stream]
//!                                          open the window, wait until it settles, save a PNG of it, exit
//! charcraft --check-live                   check live input (ONNX Runtime, face models) without a camera and exit
//! charcraft --face-file PHOTO|GIF|DIR      track the faces in pictures instead of a camera
//! charcraft --mode stream --stream ndi|virtual-camera|web [--transparent]
//!                                          send the performing puppet out of the app as it starts
//! charcraft --mcp-stdio [FILE]             also serve MCP on stdin and stdout (add --headless for no window)
//! charcraft --control-port 0 --token-file T   serve the loopback control channel for the `anim-mcp` bridge
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use charcraft_app::app::{CharCraftApp, Options};
use charcraft_app::control::{ControlArgs, run_headless};
use charcraft_app::smoke;
use charcraft_app::state::Mode;
use charcraft_app::world::{Opened, World};
use clap::Parser;
use color_eyre::eyre::{Result, WrapErr, bail, eyre};

/// Rig, record and stream puppets that are file-compatible with Adobe Character Animator.
#[derive(Debug, Parser)]
#[command(version, about)]
#[expect(clippy::struct_excessive_bools, reason = "independent command-line switches")]
struct Cli {
    /// A puppet, project, artwork or saved document to open.
    file: Option<PathBuf>,
    /// The mode to start in: rig, record or stream.
    #[arg(long, value_name = "MODE", default_value = "rig")]
    mode: String,
    /// Render one frame offscreen, check it is not blank and exit.
    #[arg(long)]
    smoke: bool,
    /// With --smoke: also write the frame to this PNG.
    #[arg(long, value_name = "OUT.png", requires = "smoke")]
    smoke_png: Option<PathBuf>,
    /// Open the window, wait until it has settled, save a PNG of it to this file and exit.
    #[arg(long, value_name = "OUT.png")]
    screenshot: Option<PathBuf>,
    /// Record a scripted performance into the scene before showing it (the default for demo screenshots).
    #[arg(long)]
    demo_takes: bool,
    /// Start the demo face and the demo voice (the default for record and stream screenshots, never for rig).
    #[arg(long)]
    live_demo: bool,
    /// Track the faces in this photo, animated GIF or folder of images instead of a camera (needs the live build).
    #[arg(long, value_name = "FILE_OR_DIR")]
    face_file: Option<PathBuf>,
    /// Check that live input works (ONNX Runtime loads, the face models download and verify, both run) without
    /// needing a camera, print what was found and exit.
    #[arg(long)]
    check_live: bool,
    /// Light theme.
    #[arg(long)]
    light: bool,
    /// Start a stream output at launch: ndi, virtual-camera or web.
    #[arg(long, value_name = "KIND")]
    stream: Option<String>,
    /// Start the stream view without a backdrop: the window and outputs that carry alpha show only the puppet.
    #[arg(long)]
    transparent: bool,
    /// Show the start workspace (new project, recent files, starter gallery) even when a file is given.
    #[arg(long)]
    start: bool,
    /// Open the Preferences window.
    #[arg(long)]
    prefs: bool,
    /// Open a window at the start: `export` (the export dialog) or `queue` (the export queue with sample items).
    #[arg(long, value_name = "WINDOW")]
    open: Option<String>,
    /// No window: serve MCP on stdio only.
    #[arg(long, requires = "mcp_stdio")]
    headless: bool,
    /// Window size in points, `WIDTHxHEIGHT`.
    #[arg(long, value_name = "WxH", default_value = "1440x900")]
    size: String,
    #[command(flatten)]
    control: ControlArgs,
}

fn parse_size(text: &str) -> Result<[f32; 2]> {
    let (w, h) = text.split_once(['x', 'X']).ok_or_else(|| eyre!("--size wants WIDTHxHEIGHT, got {text:?}"))?;
    let (w, h): (u16, u16) = (w.trim().parse().wrap_err("the width")?, h.trim().parse().wrap_err("the height")?);
    Ok([f32::from(w.clamp(400, 8000)), f32::from(h.clamp(300, 8000))])
}

fn load(file: Option<&PathBuf>) -> Result<Opened> {
    file.map_or_else(World::demo, |path| World::open(path)).map_err(|e| eyre!("{e}"))
}

#[cfg(feature = "live")]
fn check_live() -> Result<ExitCode> {
    for line in charcraft_tracking::live::self_check().map_err(|e| eyre!("live input does not work: {e}"))? {
        println!("{line}");
    }
    println!("live input ok");
    Ok(ExitCode::SUCCESS)
}

#[cfg(not(feature = "live"))]
fn check_live() -> Result<ExitCode> {
    bail!("this build has no live input (it was built without the `live` feature); the demo face still works")
}

fn main() -> Result<ExitCode> {
    color_eyre::install()?;
    let cli = Cli::parse();
    if cli.check_live {
        return check_live();
    }
    let mode = Mode::parse(&cli.mode).ok_or_else(|| eyre!("--mode must be rig, record or stream, got {:?}", cli.mode))?;
    let size = parse_size(&cli.size)?;
    let opened = load(cli.file.as_ref())?;
    for warning in opened.world.warnings.iter().take(5) {
        eprintln!("charcraft: {warning}");
    }

    if cli.smoke {
        let report = smoke::run(opened, cli.smoke_png.as_deref()).map_err(|e| eyre!("smoke test failed: {e}"))?;
        println!("smoke ok: {}x{} px, {} drawn, {} puppets, {} tracks", report.width, report.height, report.drawn, report.puppets, report.tracks);
        return Ok(ExitCode::SUCCESS);
    }
    if cli.headless {
        run_headless(opened, &cli.control).map_err(|e| eyre!("{e}"))?;
        return Ok(ExitCode::SUCCESS);
    }

    let stream = cli
        .stream
        .as_deref()
        .map(|text| charcraft_stream::Kind::parse(text).ok_or_else(|| eyre!("--stream must be ndi, virtual-camera or web, got {text:?}")))
        .transpose()?;
    let demo = cli.file.is_none();
    let shooting = cli.screenshot.is_some();
    let options = Options {
        start: if cli.start {
            Some(true)
        } else if cli.file.is_some() || shooting {
            Some(false)
        } else {
            None
        },
        prefs_open: cli.prefs,
        mode,
        screenshot: cli.screenshot.clone(),
        demo_takes: cli.demo_takes || (shooting && demo),
        // The rig view starts neutral: a demo face would pose the head and neck and the artwork could not be judged.
        live_demo: cli.live_demo || (shooting && cli.face_file.is_none() && !matches!(mode, Mode::Rig)),
        face_file: cli.face_file.clone(),
        dark: !cli.light,
        stream,
        transparent: cli.transparent,
        open: cli.open,
        control: cli.control,
    };
    let (done_tx, done_rx) = crossbeam_channel::unbounded();
    let native = eframe::NativeOptions {
        // Created transparent so the stream view can drop its backdrop (transparent window capture); every other view
        // paints its own opaque panels.
        viewport: egui::ViewportBuilder::default().with_inner_size(size).with_min_inner_size([640.0, 420.0]).with_title("CharCraft").with_transparent(true),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "CharCraft",
        native,
        Box::new(move |cc| CharCraftApp::new(cc, opened, &options, done_tx).map(|app| -> Box<dyn eframe::App> { Box::new(app) }).map_err(Into::into)),
    )
    .map_err(|e| eyre!("the window failed: {e}"))?;
    if shooting {
        match done_rx.try_recv() {
            Ok(Ok(())) => {}
            Ok(Err(message)) => bail!("screenshot failed: {message}"),
            Err(_) => bail!("the window closed before the screenshot was taken"),
        }
    }
    Ok(ExitCode::SUCCESS)
}
