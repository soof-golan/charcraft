//! `render`: one pose of a puppet as a PNG.

use std::path::Path;

use anim_render::Renderer;
use charcraft_puppet::{PuppetPose, Viseme};
use color_eyre::eyre::{Result, WrapErr, eyre};

use crate::cli::RenderArgs;
use crate::frames::{options, render_pose, save_png};
use crate::source::load_puppet;

/// Reads `neutral` or a pose JSON file.
pub fn read_pose(spec: &str) -> Result<PuppetPose> {
    if spec.eq_ignore_ascii_case("neutral") {
        return Ok(PuppetPose::neutral());
    }
    let text = std::fs::read_to_string(Path::new(spec)).wrap_err_with(|| format!("reading the pose file {spec}"))?;
    serde_json::from_str(&text).wrap_err_with(|| format!("{spec} is not a puppet pose"))
}

/// Runs the command.
pub fn run(args: &RenderArgs) -> Result<()> {
    let puppet = load_puppet(&args.input, args.puppet.as_deref())?;
    let mut pose = read_pose(&args.pose)?;
    if let Some(name) = &args.viseme {
        pose.viseme = Some(Viseme::from_label(name).ok_or_else(|| eyre!("unknown mouth shape {name:?}"))?);
    }
    let options = options(&puppet, args.scale, args.transparent)?;
    let image = render_pose(&mut Renderer::new(), &puppet, &pose, &options)?;
    save_png(&args.output, &image)?;
    println!("{} ({}x{})", args.output.display(), image.width, image.height);
    Ok(())
}
