//! Command line definition.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Headless `CharCraft`: inspect, render, lip sync, animate, export and convert puppets and projects.
#[derive(Debug, Parser)]
#[command(name = "charcraft-cli", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Describe a puppet, a project folder, a PSD or an AI file.
    Info(InfoArgs),
    /// Render a puppet, PSD or AI file in a pose to a PNG.
    Render(RenderArgs),
    /// Turn speech audio into mouth shapes (visemes) as JSON.
    Lipsync(LipsyncArgs),
    /// Perform a puppet to speech: lip sync, automatic blinking and breathing, written as a video.
    Animate(AnimateArgs),
    /// Render a scene of a project to a video file or a PNG sequence.
    Export(ExportArgs),
    /// Read a puppet and write it again.
    Convert(ConvertArgs),
}

#[derive(Debug, Args)]
pub struct InfoArgs {
    /// A .puppet, a .chproj (or its folder), a .psd or an .ai file.
    pub input: PathBuf,
    /// Print JSON instead of text.
    #[arg(long)]
    pub json: bool,
    /// Also print the layer tree with its tags.
    #[arg(long)]
    pub outline: bool,
}

#[derive(Debug, Args)]
pub struct RenderArgs {
    /// A .puppet, a .chproj (or its folder), a .psd or an .ai file.
    pub input: PathBuf,
    /// Where to write the PNG.
    #[arg(short, long)]
    pub output: PathBuf,
    /// `neutral`, or the path of a JSON file holding a puppet pose.
    #[arg(long, default_value = "neutral")]
    pub pose: String,
    /// Mouth shape to show (Ah, D, Ee, F, L, M, Oh, R, S, Uh, W-Oo, Smile, Surprised); overrides the pose.
    #[arg(long)]
    pub viseme: Option<String>,
    /// Which puppet of a project to render (store path); the first top-level puppet by default.
    #[arg(long)]
    pub puppet: Option<String>,
    /// Pixel scale relative to the artwork size.
    #[arg(long, default_value_t = 1.0)]
    pub scale: f64,
    /// Keep the background transparent instead of white.
    #[arg(long)]
    pub transparent: bool,
}

#[derive(Debug, Args)]
pub struct LipsyncArgs {
    /// Speech audio (WAV, MP3, FLAC, Ogg, AAC).
    pub audio: PathBuf,
    /// Where to write the viseme JSON.
    #[arg(short, long)]
    pub output: PathBuf,
}

#[derive(Debug, Args)]
pub struct AnimateArgs {
    /// A .puppet, a .chproj (or its folder), a .psd or an .ai file.
    pub puppet: PathBuf,
    /// Speech audio that drives the mouth.
    #[arg(long)]
    pub audio: PathBuf,
    /// The video to write (.mp4, or .mov for `ProRes` 4444 with transparency).
    #[arg(short, long)]
    pub output: PathBuf,
    /// Which puppet of a project to perform (store path).
    #[arg(long)]
    pub puppet_path: Option<String>,
    /// Frames per second.
    #[arg(long, default_value_t = 30.0)]
    pub fps: f64,
    /// Pixel scale relative to the artwork size.
    #[arg(long, default_value_t = 1.0)]
    pub scale: f64,
    /// Stop after this many seconds.
    #[arg(long)]
    pub seconds: Option<f64>,
    /// Seed for the random blink timing.
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// Do not blink.
    #[arg(long)]
    pub no_blink: bool,
    /// Do not breathe.
    #[arg(long)]
    pub no_breathe: bool,
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    /// A .chproj file or its folder.
    pub project: PathBuf,
    /// The scene to export (display name); the first scene by default.
    #[arg(long)]
    pub scene: Option<String>,
    /// A video file (.mp4 or .mov), or a directory (or a name without extension) for a PNG sequence.
    #[arg(short, long)]
    pub output: PathBuf,
    /// Pixel scale relative to the scene size.
    #[arg(long, default_value_t = 1.0)]
    pub scale: f64,
    /// Stop after this many seconds.
    #[arg(long)]
    pub seconds: Option<f64>,
    /// Start this many seconds into the scene.
    #[arg(long, default_value_t = 0.0)]
    pub start: f64,
    /// Seed for the random blink timing.
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
}

#[derive(Debug, Args)]
pub struct ConvertArgs {
    /// The .puppet to read.
    pub input: PathBuf,
    /// The .puppet to write.
    pub output: PathBuf,
}
