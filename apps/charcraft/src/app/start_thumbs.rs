//! Pictures for the Start workspace: the neutral pose of each starter puppet and recent file, drawn on a background
//! thread that owns its own stage and renderer, and sent back over a channel.

use std::path::PathBuf;

use anim_render::Renderer;
use charcraft_puppet::PuppetPose;
use crossbeam_channel::{Receiver, unbounded};

use crate::compose::{Picture, Stage};
use crate::world::World;

/// Height of the pictures, in pixels.
const HEIGHT: f64 = 160.0;

/// One thing to draw: the name the picture is stored under and the file (`None` for the demo puppet).
pub type Job = (String, Option<PathBuf>);

fn draw(file: Option<&PathBuf>) -> Option<Picture> {
    let opened = file.map_or_else(World::demo, |p| World::open(p)).ok()?;
    let cast = opened.world.cast;
    let key = cast.keys().next()?.to_owned();
    let height = cast.get(&key)?.puppet.size.height.max(1.0);
    let stage = Stage::new(cast);
    let mut renderer = Renderer::new();
    stage.render_puppet(&mut renderer, &key, &PuppetPose::neutral(), (HEIGHT / height).min(1.0), None).ok()?
}

impl super::CharCraftApp {
    /// Draws the starters and the recent files for the Start workspace (again only when the list changed).
    pub(super) fn spawn_start_thumbs(&mut self) {
        let mut jobs: Vec<Job> = Vec::new();
        for (i, starter) in self.view.start_info.starters.iter().enumerate() {
            let file = match &starter.source {
                crate::workspace::StarterSource::Demo => None,
                crate::workspace::StarterSource::File(path) => Some(path.clone()),
            };
            jobs.push((format!("starter:{i}"), file));
        }
        for path in self.view.prefs.common.recent.iter().take(8) {
            jobs.push((format!("recent:{}", path.display()), Some(path.to_path_buf())));
        }
        let names: Vec<String> = jobs.iter().map(|(n, _)| n.clone()).collect();
        if names == self.thumbs.start_jobs {
            return;
        }
        self.thumbs.start_jobs = names;
        self.thumbs.start_rx = Some(spawn(jobs, || {}));
    }

    /// Takes in the pictures that are done.
    pub(super) fn pump_start_thumbs(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.thumbs.start_rx.as_ref() else { return };
        while let Ok((name, picture)) = rx.try_recv() {
            let Some(picture) = picture else { continue };
            let size = [usize::try_from(picture.width).unwrap_or(1), usize::try_from(picture.height).unwrap_or(1)];
            let image = egui::ColorImage::from_rgba_unmultiplied(size, &picture.rgba);
            let texture = ctx.load_texture(format!("start-{name}"), image, egui::TextureOptions::LINEAR);
            self.thumbs.textures.insert(name, (texture, size));
        }
    }
}

/// Starts drawing the jobs one after the other; the pictures arrive as they are done (`None`: it could not be drawn).
#[must_use]
pub fn spawn(jobs: Vec<Job>, wake: impl Fn() + Send + 'static) -> Receiver<(String, Option<Picture>)> {
    let (tx, rx) = unbounded();
    let _ = std::thread::Builder::new().name("charcraft-start-thumbs".into()).spawn(move || {
        for (name, file) in jobs {
            let picture = draw(file.as_ref());
            if tx.send((name, picture)).is_err() {
                return;
            }
            wake();
        }
    });
    rx
}
