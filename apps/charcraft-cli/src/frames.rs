//! Rendering poses to pixels and writing PNGs.

use std::path::Path;

use anim_core::Rgba8;
use anim_display::Image;
use anim_render::{Background, RenderOptions, Renderer, straight_rgba};
use charcraft_puppet::{Puppet, PuppetPose};
use color_eyre::eyre::{Result, WrapErr, bail};

/// The pixel size of the puppet at `scale`, rounded to whole pixels (at least 1).
pub fn scaled_size(puppet: &Puppet, scale: f64) -> Result<(u32, u32)> {
    if !scale.is_finite() || scale <= 0.0 {
        bail!("the scale must be a positive number, got {scale}");
    }
    let w = anim_core::num::round_f64_to_u32((puppet.size.width * scale).max(1.0));
    let h = anim_core::num::round_f64_to_u32((puppet.size.height * scale).max(1.0));
    Ok((w, h))
}

/// Render options that draw the puppet's stage at `scale`.
pub fn options(puppet: &Puppet, scale: f64, transparent: bool) -> Result<RenderOptions> {
    // Validate the scale and the resulting size up front for a readable error.
    scaled_size(puppet, scale)?;
    let background = if transparent { Background::Transparent } else { Background::Solid(Rgba8::WHITE) };
    Ok(RenderOptions::scaled_stage(puppet.size, scale).with_background(background))
}

/// Renders one pose.
pub fn render_pose(renderer: &mut Renderer, puppet: &Puppet, pose: &PuppetPose, options: &RenderOptions) -> Result<Image> {
    let list = puppet.evaluate(pose);
    renderer.render(&list, puppet.images(), options).wrap_err("rendering the puppet")
}

/// PNG bytes of a premultiplied image.
pub fn png_bytes(image: &Image) -> Result<Vec<u8>> {
    anim_export::encode_png(image.width, image.height, &straight_rgba(image)).wrap_err("encoding the PNG")
}

/// Writes a PNG, creating the parent folder.
pub fn save_png(path: &Path, image: &Image) -> Result<()> {
    write_file(path, &png_bytes(image)?)
}

/// Writes a file, creating its parent folder.
pub fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).wrap_err_with(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, bytes).wrap_err_with(|| format!("writing {}", path.display()))
}
