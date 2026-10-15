//! `animate`: perform a puppet to speech (lip sync, blinks, breathing) and write a video.

use anim_core::{Tick, TimeRange};
use anim_render::{Background, RenderOptions, Renderer};
use color_eyre::eyre::{Result, bail};

use crate::cli::AnimateArgs;
use crate::commands::lipsync::{decode, keys_of};
use crate::frames::{render_pose, scaled_size};
use crate::perform::{Options, Performance};
use crate::source::load_puppet;
use crate::video::{Plan, audio_track};

/// Runs the command.
pub fn run(args: &AnimateArgs) -> Result<()> {
    let puppet = load_puppet(&args.puppet, args.puppet_path.as_deref())?;
    let audio = decode(&args.audio)?;
    let mut length = audio.duration();
    if let Some(seconds) = args.seconds {
        if !seconds.is_finite() || seconds <= 0.0 {
            bail!("--seconds must be positive, got {seconds}");
        }
        length = length.min(Tick::from_seconds_f64(seconds));
    }
    // Only the part that is shown is analysed.
    let heard = audio.slice_ticks(TimeRange::new(Tick::ZERO, length));
    let keys = keys_of(&heard);
    eprintln!("{} mouth keys over {:.1} s", keys.len(), length.seconds());

    let (w, h) = scaled_size(&puppet, args.scale)?;
    let plan = Plan::new(&args.output, w, h, args.fps, length)?;
    let performance = Performance::new(keys, length, Options { seed: args.seed, blink: !args.no_blink, breathe: !args.no_breathe });

    let background = if plan.format.transparent() { Background::Transparent } else { Background::Solid(anim_core::Rgba8::WHITE) };
    let options = RenderOptions::fit_stage(puppet.size, plan.width, plan.height).with_background(background);
    let mut renderer = Renderer::new();
    let soundtrack = audio_track(&audio, Tick::ZERO, length)?;
    let summary = crate::video::write(&args.output, &plan, Some(soundtrack), |_, t| {
        let pose = performance.pose_at(t);
        render_pose(&mut renderer, &puppet, &pose, &options)
    })?;
    println!("{summary}");
    Ok(())
}
