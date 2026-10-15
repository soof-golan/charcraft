//! `export`: a scene of a project as a video or a PNG sequence.

use anim_core::{Rgba8, Tick};
use color_eyre::eyre::{Result, bail};

use crate::cli::ExportArgs;
use crate::commands::lipsync::keys_of;
use crate::scene::{StageRenderer, load_stage};
use crate::source::open_project;
use crate::video::{Format, Plan, audio_track};

/// Runs the command.
pub fn run(args: &ExportArgs) -> Result<()> {
    let project = open_project(&args.project)?;
    let stage = load_stage(&project, args.scene.as_deref())?;
    for warning in &stage.warnings {
        eprintln!("warning: {warning}");
    }
    if stage.layers.is_empty() {
        bail!("scene {:?} has no puppets that can be drawn", stage.name);
    }
    if !args.start.is_finite() || args.start < 0.0 {
        bail!("--start must be zero or more, got {}", args.start);
    }
    let start = Tick::from_seconds_f64(args.start);
    let mut length = Tick(stage.duration.0.saturating_sub(start.0));
    if let Some(seconds) = args.seconds {
        if !seconds.is_finite() || seconds <= 0.0 {
            bail!("--seconds must be positive, got {seconds}");
        }
        length = length.min(Tick::from_seconds_f64(seconds));
    }

    // The sound heard in the exported part, and the mouth shapes it makes.
    let sound = stage.audio.as_ref().map(|a| a.window(start, length));
    let keys = sound.as_ref().map(keys_of).unwrap_or_default();
    // The sound can be shorter than the scene; the picture still runs to the end of the part.
    let format = Format::of(&args.output);
    let background = (!format.transparent()).then_some(Rgba8::WHITE);
    let mut renderer = StageRenderer::new(&stage, args.scale, format != Format::PngSequence, &keys, length, args.seed, background);
    let (w, h) = renderer.size();
    let plan = Plan::new(&args.output, w, h, stage.fps, length)?;
    eprintln!("scene {:?}: {} puppets, {}x{} at {} fps, {:.1} s, {} mouth keys", stage.name, stage.layers.len(), w, h, stage.fps, length.seconds(), keys.len());
    let soundtrack = sound.as_ref().map(|s| audio_track(s, Tick::ZERO, length)).transpose()?;
    let summary = crate::video::write(&args.output, &plan, soundtrack, |_, t| renderer.frame(t))?;
    println!("{summary}");
    Ok(())
}
