//! Pictures for the Start workspace: the neutral pose of each starter puppet and recent file, drawn on a background
//! thread that owns its own stage and renderer, and sent back over a channel.

use std::path::PathBuf;

use anim_render::Renderer;
use charcraft_puppet::PuppetPose;
use crossbeam_channel::{Receiver, unbounded};

use crate::compose::{Picture, Stage};
use crate::workspace::StarterSource;
use crate::world::World;

/// Height of the pictures, in pixels.
const HEIGHT: f64 = 160.0;

/// One thing to draw: the name the picture is stored under and where the puppet comes from.
pub type Job = (String, StarterSource);

fn draw(source: &StarterSource) -> Option<Picture> {
    let opened = match source {
        StarterSource::Demo => World::demo(),
        StarterSource::Template(id) => World::from_spec(charcraft_maker::template(id)?.spec),
        StarterSource::File(path) => World::open(path),
    }
    .ok()?;
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
            jobs.push((format!("starter:{i}"), starter.source.clone()));
        }
        for path in self.view.prefs.common.recent.iter().take(8) {
            jobs.push((format!("recent:{}", path.display()), StarterSource::File(PathBuf::from(path))));
        }
        let names: Vec<String> = jobs.iter().map(|(n, _)| n.clone()).collect();
        if names == self.thumbs.start_jobs {
            return;
        }
        self.thumbs.start_jobs = names;
        self.thumbs.start_rx = Some(spawn(jobs));
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
pub fn spawn(jobs: Vec<Job>) -> Receiver<(String, Option<Picture>)> {
    let (tx, rx) = unbounded();
    let _ = std::thread::Builder::new().name("charcraft-start-thumbs".into()).spawn(move || {
        for (name, source) in jobs {
            let picture = draw(&source);
            if tx.send((name, picture)).is_err() {
                return;
            }
        }
    });
    rx
}
