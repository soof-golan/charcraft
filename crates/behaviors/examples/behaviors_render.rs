//! Drives a puppet with its own behaviors through a scripted demo and renders a contact sheet.
//!
//! ```text
//! cargo run -p charcraft-behaviors --example behaviors_render -- INPUT OUT.png [--frames 10] [--seconds 4] [--scale 0.5] [--seed 7]
//! ```
//!
//! `INPUT` is a `.puppet`, a project folder or `.chproj`, or artwork (AI, PSD, SVG). The demo has the face nod and
//! roll, the voice cycle through mouth shapes, the pointer drag the first draggable handle and the arrow keys look
//! around. Poses come from the puppet's own behaviors with the parameters stored in the project.
#![allow(
    clippy::print_stdout,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "example code"
)]

use std::error::Error;
use std::path::Path;

use anim_artwork::Limits;
use anim_core::{Rgba8, Vec2};
use anim_lipsync::Viseme as Voice;
use anim_render::{Background, RenderOptions, Renderer, straight_rgba};
use charcraft_behaviors::{BehaviorStack, FaceInput, Rig, Script, ScriptEvent, simulate};
use charcraft_project::Project;
use charcraft_puppet::{HandleKind, Puppet, PuppetPose};

fn main() -> Result<(), Box<dyn Error>> {
    let mut positional = Vec::new();
    let (mut frames, mut seconds, mut scale, mut seed) = (10_usize, 4.0_f64, 0.5_f64, 7_u64);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--frames" => frames = value("--frames")?.parse()?,
            "--seconds" => seconds = value("--seconds")?.parse()?,
            "--scale" => scale = value("--scale")?.parse()?,
            "--seed" => seed = value("--seed")?.parse()?,
            other => positional.push(other.to_owned()),
        }
    }
    let input = positional.first().ok_or("usage: behaviors_render INPUT OUT.png [options]")?;
    let output = positional.get(1).ok_or("usage: behaviors_render INPUT OUT.png [options]")?;

    let (puppet, mut stack) = load(input, seed)?;
    println!("behaviors: {:?}", stack.slots().iter().map(|s| s.behavior.name()).collect::<Vec<_>>());

    let fps = 30.0;
    let total = (seconds * fps) as u32;
    let script = demo_script(stack.rig(), fps, total);
    let played = simulate(&mut stack, &script, 1)?;

    let mut renderer = Renderer::new();
    let options = RenderOptions::scaled_stage(puppet.size, scale).with_background(Background::Solid(Rgba8::WHITE));
    let mut images = Vec::new();
    for i in 0..frames {
        let at = (played.len() - 1) * i / frames.saturating_sub(1).max(1);
        let pose: &PuppetPose = &played[at].pose;
        println!(
            "frame {at}: viseme {:?}, head {:.1} deg, blink {:.2}, gaze ({:.2},{:.2}), drags {:?}",
            pose.viseme,
            pose.head.rotation.to_degrees(),
            pose.blink_left,
            pose.gaze.x,
            pose.gaze.y,
            pose.drag.keys().collect::<Vec<_>>()
        );
        images.push(renderer.render(&puppet.evaluate(pose), puppet.images(), &options)?);
    }
    write_sheet(output, &images, 5)
}

fn load(input: &str, seed: u64) -> Result<(Puppet, BehaviorStack), Box<dyn Error>> {
    let lower = input.to_lowercase();
    if lower.ends_with(".puppet") || lower.ends_with(".chproj") || Path::new(input).is_dir() {
        let project = Project::open(input)?;
        let puppet = Puppet::from_project(&project, None)?;
        let rig = Rig::from_puppet(&puppet);
        let model = project.model()?;
        let defaults = project.manifest().map(charcraft_project::ArchiveInfo::behavior_defaults).unwrap_or_default();
        if let Some(item) = model.root_puppets().next() {
            let (stack, skipped) = BehaviorStack::from_bound(rig, &item.behaviors, &defaults);
            println!("not implemented: {skipped:?}");
            return Ok((puppet, stack.with_seed(seed)));
        }
        return Ok((puppet, BehaviorStack::standard(Rig::from_puppet(&Puppet::from_project(&project, None)?)).with_seed(seed)));
    }
    let tree = anim_artwork::import_path(Path::new(input), &Limits::default())?;
    let puppet = Puppet::from_artwork(&tree)?;
    let rig = Rig::from_puppet(&puppet);
    Ok((puppet, BehaviorStack::standard(rig).with_seed(seed)))
}

fn demo_script(rig: &Rig, fps: f64, frames: u32) -> Script {
    let mut script = Script::new(fps, frames);
    let voices = [Voice::Ah, Voice::Oh, Voice::Ee, Voice::M, Voice::Uh, Voice::WOo, Voice::S, Voice::Neutral];
    for frame in 0..frames {
        let t = f64::from(frame) / fps;
        let face = FaceInput {
            head_roll: 14.0 * (t * 1.7).sin(),
            head_translation: Vec2::new(0.25 * (t * 1.1).sin(), 0.1 * (t * 2.3).cos()),
            head_yaw: 20.0 * (t * 0.9).sin(),
            eyebrow_left: 0.6 * (t * 2.0).sin().max(0.0),
            eyebrow_right: 0.6 * (t * 2.0).sin().max(0.0),
            mouth_open: 0.4 + 0.3 * (t * 6.0).sin(),
            gaze: Vec2::new(0.5 * (t * 1.3).sin(), 0.3 * (t * 0.8).cos()),
            ..FaceInput::default()
        };
        script.events.push(ScriptEvent::Face { frame, face });
        if frame % 5 == 0 {
            let v = voices[(frame / 5) as usize % voices.len()];
            script.events.push(ScriptEvent::Voice { frame, viseme: Some(v) });
        }
    }
    // The mouse takes the first draggable handle and swings it around, then lets go.
    if let Some(h) = rig.handles_of(HandleKind::Draggable).next() {
        let (x, y) = (h.position.x, h.position.y);
        let (start, end) = (frames / 4, frames * 3 / 4);
        for frame in start..end {
            let phase = f64::from(frame - start) / f64::from((end - start).max(1));
            let r = 90.0 * (phase * std::f64::consts::PI).sin();
            let a = phase * 2.0 * std::f64::consts::PI;
            script.events.push(ScriptEvent::Pointer { frame, id: 0, x: x + r * a.cos(), y: y - r * a.sin().abs(), down: true });
        }
        script.events.push(ScriptEvent::Pointer { frame: end, id: 0, x, y, down: false });
    }
    script
}

fn write_sheet(path: &str, frames: &[anim_display::Image], cols: usize) -> Result<(), Box<dyn Error>> {
    let rows = frames.len().div_ceil(cols);
    let (cw, ch) = (frames[0].width as usize, frames[0].height as usize);
    let mut sheet = vec![255_u8; cols * cw * rows * ch * 4];
    for (i, f) in frames.iter().enumerate() {
        let (cx, cy) = ((i % cols) * cw, (i / cols) * ch);
        let px = straight_rgba(f);
        for y in 0..ch {
            let dst = ((cy + y) * cols * cw + cx) * 4;
            sheet[dst..dst + cw * 4].copy_from_slice(&px[y * cw * 4..(y + 1) * cw * 4]);
        }
    }
    anim_testkit::write_png(Path::new(path), u32::try_from(cols * cw)?, u32::try_from(rows * ch)?, &sheet)?;
    println!("wrote {path}");
    Ok(())
}
