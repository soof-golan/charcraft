//! The render actor: owns the stage (every puppet's pictures) and the renderer, draws what the window asks for.
//!
//! The window sends poses, never pictures: `Cast` once (and again after a rig edit or a new document), then small
//! `Puppet` and `Scene` requests. When requests pile up only the newest of each kind is drawn, so a slow frame
//! never lags behind the performer.

use std::thread::JoinHandle;
use std::time::Instant;

use anim_core::Rgba8;
use anim_render::Renderer;
use charcraft_puppet::PuppetPose;
use charcraft_record::{Cast, Evaluator, Scene};
use crossbeam_channel::{Receiver, Sender, TryRecvError, unbounded};

use crate::compose::{Picture, Stage};

/// What the window asks the renderer for.
#[derive(Debug)]
pub enum RenderRequest {
    /// Replace the puppets.
    Cast(Cast),
    /// One puppet alone.
    Puppet {
        /// Echoed in the answer.
        id: u64,
        /// Cast key.
        key: String,
        /// Its pose.
        pose: Box<PuppetPose>,
        /// Pixels per artwork pixel.
        scale: f64,
        /// Backdrop (transparent when `None`).
        background: Option<Rgba8>,
    },
    /// A scene frame: tracks play back from the scene except the ones in `live`, which use the given poses.
    Scene {
        /// Echoed in the answer.
        id: u64,
        /// The scene.
        scene: Scene,
        /// Frame to draw.
        frame: i64,
        /// Poses that replace playback for some tracks.
        live: Vec<(usize, PuppetPose)>,
        /// Pixels per stage pixel.
        scale: f64,
    },
    /// End the actor.
    Shutdown,
}

/// One finished picture.
#[derive(Debug)]
pub struct Rendered {
    /// The request's id.
    pub id: u64,
    /// The picture, or why there is none.
    pub picture: Result<Picture, String>,
    /// How long drawing took.
    pub seconds: f32,
}

/// The window's end of the actor. Dropping it ends the thread.
#[derive(Debug)]
pub struct RenderHandle {
    requests: Sender<RenderRequest>,
    results: Receiver<Rendered>,
    thread: Option<JoinHandle<()>>,
}

impl RenderHandle {
    /// Starts the actor; `wake` is called after every picture (to repaint the window).
    #[must_use]
    pub fn spawn(wake: impl Fn() + Send + 'static) -> Self {
        let (requests, request_rx) = unbounded();
        let (result_tx, results) = unbounded();
        let thread = std::thread::Builder::new().name("charcraft-render".into()).spawn(move || run(&request_rx, &result_tx, &wake)).ok();
        Self { requests, results, thread }
    }

    /// Queues a request.
    pub fn send(&self, request: RenderRequest) {
        let _ = self.requests.send(request);
    }

    /// The next finished picture.
    #[must_use]
    pub fn try_result(&self) -> Option<Rendered> {
        self.results.try_recv().ok()
    }
}

impl Drop for RenderHandle {
    fn drop(&mut self) {
        let _ = self.requests.send(RenderRequest::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Playback {
    scene: Scene,
    evaluator: Evaluator,
}

fn run(requests: &Receiver<RenderRequest>, results: &Sender<Rendered>, wake: &dyn Fn()) {
    let mut stage = Stage::new(Cast::new());
    let mut renderer = Renderer::new();
    let mut playback: Option<Playback> = None;
    loop {
        let Ok(first) = requests.recv() else { return };
        // Take everything that is waiting: casts in order, only the newest drawing request of each kind.
        let mut puppet = None;
        let mut scene = None;
        let mut batch = vec![first];
        loop {
            match requests.try_recv() {
                Ok(next) => batch.push(next),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        for request in batch {
            match request {
                RenderRequest::Shutdown => return,
                RenderRequest::Cast(cast) => {
                    stage = Stage::new(cast);
                    renderer.clear_image_cache();
                    playback = None;
                }
                request @ RenderRequest::Puppet { .. } => puppet = Some(request),
                request @ RenderRequest::Scene { .. } => scene = Some(request),
            }
        }
        for request in [puppet, scene].into_iter().flatten() {
            let started = Instant::now();
            let (id, picture) = match request {
                RenderRequest::Puppet { id, key, pose, scale, background } => {
                    let picture = stage
                        .render_puppet(&mut renderer, &key, &pose, scale, background)
                        .map_err(|e| e.to_string())
                        .and_then(|p| p.ok_or_else(|| format!("no puppet {key:?}")));
                    (id, picture)
                }
                RenderRequest::Scene { id, scene, frame, live, scale } => (id, draw_scene(&stage, &mut renderer, &mut playback, scene, frame, &live, scale)),
                RenderRequest::Cast(_) | RenderRequest::Shutdown => continue,
            };
            let _ = results.send(Rendered { id, picture, seconds: started.elapsed().as_secs_f32() });
            wake();
        }
    }
}

fn draw_scene(
    stage: &Stage,
    renderer: &mut Renderer,
    playback: &mut Option<Playback>,
    scene: Scene,
    frame: i64,
    live: &[(usize, PuppetPose)],
    scale: f64,
) -> Result<Picture, String> {
    if playback.as_ref().is_none_or(|p| p.scene != scene) {
        let evaluator = Evaluator::new(&scene, &stage.cast().templates()).map_err(|e| e.to_string())?;
        *playback = Some(Playback { scene, evaluator });
    }
    let Some(play) = playback.as_mut() else { return Err("no scene".to_owned()) };
    let mut poses = play.evaluator.poses(frame.max(0));
    for (track, pose) in live {
        if let Some(slot) = poses.get_mut(*track) {
            slot.clone_from(pose);
        }
    }
    stage.render_scene(renderer, &play.scene, &poses, scale).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use anim_core::{FrameRate, Size, Tick};
    use charcraft_record::{CastMember, PuppetTrack};

    use super::*;

    #[test]
    fn the_actor_draws_puppets_and_scenes_and_skips_stale_requests() {
        let handle = RenderHandle::spawn(|| {});
        let mut cast = Cast::new();
        cast.insert("demo", CastMember::new(crate::demo::puppet().unwrap()));
        handle.send(RenderRequest::Cast(cast));
        let mut scene = Scene::new("s", Size::new(320.0, 180.0), FrameRate::FPS_30, Tick::from_seconds_f64(1.0));
        scene.add_track(PuppetTrack::new("T", "demo")).unwrap();
        for id in 1..=5 {
            handle.send(RenderRequest::Scene { id, scene: scene.clone(), frame: 3, live: vec![(0, PuppetPose::neutral().with_blink(1.0))], scale: 1.0 });
        }
        handle.send(RenderRequest::Puppet { id: 100, key: "demo".into(), pose: Box::new(PuppetPose::neutral()), scale: 0.25, background: None });
        let mut seen = Vec::new();
        for _ in 0..400 {
            while let Some(done) = handle.try_result() {
                seen.push((done.id, done.picture.map(|p| (p.width, p.height))));
            }
            if seen.iter().any(|(id, _)| *id == 100) && seen.iter().any(|(id, _)| *id == 5) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(seen.contains(&(5, Ok((320, 180)))), "{seen:?}");
        assert!(seen.contains(&(100, Ok((160, 180)))), "{seen:?}");
        assert!(seen.len() <= 6, "at most one answer per request: {seen:?}");
    }
}
