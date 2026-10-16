//! `--smoke`: load a puppet (or the demo), pose it, render one frame without a window, check the picture is not
//! blank and the command registry works. Runs on machines without a display.

use std::path::Path;

use anim_render::Renderer;
use charcraft_puppet::Viseme;
use charcraft_record::Evaluator;
use serde_json::json;

use crate::compose::Stage;
use crate::world::Opened;

/// What a smoke run found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Pixels that are not fully transparent or the stage background.
    pub drawn: usize,
    /// Puppets in the cast.
    pub puppets: usize,
    /// Tracks in the scene.
    pub tracks: usize,
}

/// Renders frame 0 of the opened document at a size of about 640 pixels wide, optionally writing the PNG.
///
/// # Errors
/// A message when nothing can be rendered or the picture is blank.
pub fn run(opened: Opened, png: Option<&Path>) -> Result<Report, String> {
    let Opened { world, mut rec, rig } = opened;
    if rec.scene.tracks.is_empty() {
        return Err("the document has no puppet on the stage".to_owned());
    }
    // The commands work on the document: change the length, ask for the scene back.
    let registry = crate::session::registry().map_err(|e| e.to_string())?;
    let (ui, _unused) = crossbeam_channel::unbounded();
    let links = crate::session::Links { live: crate::live::LiveLink::disconnected(), ui };
    let mut session = anim_session::Session::new(crate::session::AppDoc::new(rec.clone(), rig, links), registry);
    session.execute("scene.set", &json!({ "duration_seconds": 1.0 })).map_err(|e| e.to_string())?;
    let info = session.execute("scene.info", &json!({})).map_err(|e| e.to_string())?;
    if info.get("tracks").and_then(serde_json::Value::as_array).is_none_or(Vec::is_empty) {
        return Err("scene.info reported no tracks".to_owned());
    }
    rec = session.doc().rec.clone();

    let stage = Stage::new(world.cast.clone());
    let mut evaluator = Evaluator::new(&rec.scene, &stage.cast().templates()).map_err(|e| e.to_string())?;
    let mut poses = evaluator.poses(0);
    for pose in &mut poses {
        pose.viseme = Some(Viseme::Ah);
    }
    let scale = (640.0 / rec.scene.size.width.max(1.0)).min(1.0);
    let picture = stage.render_scene(&mut Renderer::new(), &rec.scene, &poses, scale).map_err(|e| e.to_string())?;
    let background = rec.scene.background.map_or([0, 0, 0, 0], |c| [c.r, c.g, c.b, c.a]);
    let drawn = picture.rgba.as_chunks::<4>().0.iter().filter(|px| **px != background).count();
    if drawn == 0 {
        return Err("the rendered frame is blank".to_owned());
    }
    if let Some(path) = png {
        let bytes = anim_export::encode_png(picture.width, picture.height, &picture.rgba).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(Report { width: picture.width, height: picture.height, drawn, puppets: world.cast.len(), tracks: rec.scene.tracks.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    #[test]
    fn the_demo_renders_a_frame() {
        let report = run(World::demo().unwrap(), None).unwrap();
        assert_eq!((report.width, report.tracks, report.puppets), (640, 1, 1));
        assert!(report.drawn > 10_000, "{report:?}");
    }
}
