//! Builds a puppet from artwork (AI, PSD, SVG) or from a `.puppet` / project, poses it and renders a PNG.
//!
//! ```text
//! cargo run -p charcraft-puppet --example puppet_render -- INPUT OUT.png
//!     [--viseme Ah] [--blink 1] [--head-rotation 12] [--gaze 0.5,-0.3] [--eyebrows 0.8]
//!     [--drag "left hand/Dangle=10,20"] [--scale 1] [--summary] [--outline] [--contact-sheet]
//! ```
#![allow(
    clippy::print_stdout,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::as_conversions,
    reason = "example code"
)]

use std::error::Error;
use std::path::Path;

use anim_artwork::Limits;
use anim_core::{Rgba8, Vec2};
use anim_render::{Background, RenderOptions, Renderer, straight_rgba};
use charcraft_project::Project;
use charcraft_puppet::{Puppet, PuppetPose, Viseme};

fn pair(s: &str) -> Result<Vec2, Box<dyn Error>> {
    let (a, b) = s.split_once(',').ok_or("expected X,Y")?;
    Ok(Vec2::new(a.trim().parse()?, b.trim().parse()?))
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut positional = Vec::new();
    let mut pose = PuppetPose::neutral();
    let (mut scale, mut summary, mut outline, mut sheet) = (1.0_f64, false, false, false);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--viseme" => pose.viseme = Some(Viseme::from_label(&value("--viseme")?).ok_or("unknown viseme")?),
            "--blink" => {
                let v: f64 = value("--blink")?.parse()?;
                pose.blink_left = v;
                pose.blink_right = v;
            }
            "--head-rotation" => pose.head.rotation = value("--head-rotation")?.parse::<f64>()?.to_radians(),
            "--gaze" => pose.gaze = pair(&value("--gaze")?)?,
            "--eyebrows" => {
                let v: f64 = value("--eyebrows")?.parse()?;
                pose.eyebrow_left = v;
                pose.eyebrow_right = v;
            }
            "--drag" => {
                let spec = value("--drag")?;
                let (key, to) = spec.split_once('=').ok_or("--drag KEY=X,Y")?;
                pose.drag.insert(key.to_owned(), pair(to)?);
            }
            "--scale" => scale = value("--scale")?.parse()?,
            "--summary" => summary = true,
            "--outline" => outline = true,
            "--contact-sheet" => sheet = true,
            other => positional.push(other.to_owned()),
        }
    }
    let input = positional.first().ok_or("usage: puppet_render INPUT OUT.png [options]")?;
    let puppet = load(input)?;
    if summary {
        println!("{}", serde_json::to_string_pretty(&puppet.summary())?);
    }
    if outline {
        print!("{}", puppet.outline());
    }
    let Some(output) = positional.get(1) else { return Ok(()) };

    let mut renderer = Renderer::new();
    let options = RenderOptions::scaled_stage(puppet.size, scale).with_background(Background::Solid(Rgba8::WHITE));
    if sheet {
        // Every mouth shape side by side.
        let mut frames = Vec::new();
        for v in Viseme::ALL {
            let mut p = pose.clone();
            p.viseme = Some(v);
            frames.push(renderer.render(&puppet.evaluate(&p), puppet.images(), &options)?);
        }
        write_sheet(output, &frames)?;
    } else {
        let image = renderer.render(&puppet.evaluate(&pose), puppet.images(), &options)?;
        write_png(output, image.width, image.height, &straight_rgba(&image))?;
    }
    Ok(())
}

fn load(input: &str) -> Result<Puppet, Box<dyn Error>> {
    let lower = input.to_lowercase();
    if lower.ends_with(".puppet") || lower.ends_with(".chproj") || Path::new(input).is_dir() {
        Ok(Puppet::from_project(&Project::open(input)?, None)?)
    } else {
        let tree = anim_artwork::import_path(Path::new(input), &Limits::default())?;
        Ok(Puppet::from_artwork(&tree)?)
    }
}

fn write_png(path: &str, width: u32, height: u32, rgba: &[u8]) -> Result<(), Box<dyn Error>> {
    anim_testkit::write_png(Path::new(path), width, height, rgba)?;
    Ok(())
}

fn write_sheet(path: &str, frames: &[anim_display::Image]) -> Result<(), Box<dyn Error>> {
    let cols = 5_usize;
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
    write_png(path, u32::try_from(cols * cw)?, u32::try_from(rows * ch)?, &sheet)
}
