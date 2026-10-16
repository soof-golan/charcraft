//! What the MCP layer cannot know about `CharCraft`: the puppets, rendering, exporting, opening and saving.
//!
//! [`Hooks`] plugs into `anim_mcp::SessionHost`. Commands (scene, tracks, takes, rig, live input, view) come from the
//! registry in [`crate::session`]; everything here is read-only queries, the document outline and library, scene
//! properties, `render_frame`, `screenshot`, `export_run`, `document_open`, `document_save`.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Write as _};
use std::path::Path;

use anim_mcp::hooks::{AppHooks, ExportFormat, ExportRequest, HookOutcome, HookResult, QuerySpec, RenderRequest, ScreenshotRequest};
use anim_mcp::{ControlError, ErrorKind, Reply, Ticket, props, run_job};
use anim_render::Renderer;
use anim_session::Session;
use charcraft_behaviors::api::{catalog, stack_info};
use charcraft_puppet::PuppetPose;
use charcraft_record::{Evaluator, Scene, track_stack};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::compose::Stage;
use crate::export::{self, Format, Job};
use crate::session::{AppDoc, Links};
use crate::state::Mode;
use crate::world::World;

/// What the window shows right now, kept up to date by the window and reported by `status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewStatus {
    /// The working mode.
    pub mode: Mode,
    /// The selected puppet track.
    pub track: usize,
    /// The selected layer path.
    pub layer: Option<String>,
    /// The playhead in seconds.
    pub playhead_seconds: f64,
    /// Whether the scene plays.
    pub playing: bool,
    /// Whether a take is being recorded.
    pub recording: bool,
    /// Whether the puppet mirrors the performer.
    pub mirror: bool,
}

impl Default for ViewStatus {
    fn default() -> Self {
        Self { mode: Mode::Rig, track: 0, layer: None, playhead_seconds: 0.0, playing: false, recording: false, mirror: true }
    }
}

/// A screenshot request waiting for the window's next frame.
#[derive(Debug)]
pub struct PendingShot {
    /// Where to answer.
    pub ticket: Ticket,
    /// What was asked.
    pub request: ScreenshotRequest,
}

/// The application hooks.
#[derive(Debug)]
pub struct Hooks {
    /// The puppets, sound and origin of the open document.
    pub world: World,
    /// The window's state.
    pub view: ViewStatus,
    /// Counts up whenever a different document was opened.
    pub epoch: u64,
    /// Whether a window exists (screenshots only work then).
    pub windowed: bool,
    /// Screenshot requests the window has not served yet.
    pub shots: VecDeque<PendingShot>,
    links: Links,
    stage: Option<(u64, u64, Stage)>,
}

impl Hooks {
    /// Hooks around a world.
    #[must_use]
    pub fn new(world: World, links: Links) -> Self {
        Self { world, view: ViewStatus::default(), epoch: 1, windowed: false, shots: VecDeque::new(), links, stage: None }
    }

    /// The stage for the current puppets, rebuilt when they changed.
    fn stage(&mut self) -> &Stage {
        let (epoch, generation) = (self.epoch, self.world.generation);
        if self.stage.as_ref().is_some_and(|(e, g, _)| *e != epoch || *g != generation) {
            self.stage = None;
        }
        let world = &self.world;
        &self.stage.get_or_insert_with(|| (epoch, generation, Stage::new(world.cast.clone()))).2
    }

    fn puppet_key(&self, params: &Value) -> Result<String, ControlError> {
        params.get("puppet").and_then(Value::as_str).map_or_else(
            || self.world.cast.keys().next().map(str::to_owned).ok_or_else(|| ControlError::failed("no puppet is loaded")),
            |key| {
                self.world
                    .cast
                    .get(key)
                    .map(|_| key.to_owned())
                    .ok_or_else(|| ControlError::new(ErrorKind::NotFound, format!("no puppet {key:?}; the cast is {:?}", self.world.cast.keys().collect::<Vec<_>>())))
            },
        )
    }
}

fn failed(e: impl std::fmt::Display) -> ControlError {
    ControlError::failed(e.to_string())
}

fn object(properties: &Value) -> Value {
    json!({ "type": "object", "properties": properties })
}

fn puppet_param() -> Value {
    json!({ "puppet": { "type": "string", "description": "Cast key of the puppet (default: the first)." } })
}

impl AppHooks<AppDoc> for Hooks {
    fn status(&self, session: &Session<AppDoc>) -> Value {
        let scene = &session.doc().rec.scene;
        json!({
            "document": { "title": self.world.title, "kind": self.world.kind, "source": self.world.source.as_ref().map(|p| p.display().to_string()), "warnings": self.world.warnings.len() },
            "puppets": self.world.cast.keys().collect::<Vec<_>>(),
            "scene": { "name": scene.name, "frames": scene.frame_count(), "fps": scene.fps_f64(), "tracks": scene.tracks.len(), "audio_tracks": scene.audio.len() },
            "view": self.view,
            "windowed": self.windowed,
        })
    }

    fn queries(&self) -> Vec<QuerySpec> {
        vec![
            QuerySpec::new("puppet_summary", "Summary of a puppet: layers, independent groups, tags, handles, mouth shapes, swap sets.").with_schema(object(&puppet_param())),
            QuerySpec::new("puppet_outline", "The puppet's layer tree as indented text, one line per layer with its tags.").with_schema(object(&puppet_param())),
            QuerySpec::new("puppet_layer", "One layer of a puppet by path or label: kind, tags, bounds, origin, handles, children.").with_schema(json!({
                "type": "object", "required": ["layer"],
                "properties": { "puppet": { "type": "string" }, "layer": { "type": "string", "description": "Path (Head/Left Eye) or label." } }
            })),
            QuerySpec::new("behaviors_catalog", "Every behavior CharCraft implements, with default parameters and what they mean."),
            QuerySpec::new("behaviors_stack", "The behaviors of a puppet track as the scene plays them (including the track's own edits), with parameters.")
                .with_schema(object(&json!({ "track": { "type": "integer", "minimum": 0 } }))),
            QuerySpec::new("world_info", "Where the document came from, the puppets, sounds and loading warnings."),
            QuerySpec::new("view_state", "What the window shows: mode, selected track and layer, playhead, playing, recording."),
        ]
    }

    fn query(&mut self, session: &Session<AppDoc>, name: &str, params: &Value, _ticket: &Ticket) -> HookResult {
        let value = match name {
            "puppet_summary" => {
                let key = self.puppet_key(params)?;
                let member = self.world.cast.get(&key).ok_or_else(|| failed("no such puppet"))?;
                json!({ "puppet": key, "summary": member.puppet.summary() })
            }
            "puppet_outline" => {
                let key = self.puppet_key(params)?;
                let member = self.world.cast.get(&key).ok_or_else(|| failed("no such puppet"))?;
                json!({ "puppet": key, "outline": member.puppet.outline() })
            }
            "puppet_layer" => {
                let key = self.puppet_key(params)?;
                let layer = params.get("layer").and_then(Value::as_str).ok_or_else(|| ControlError::invalid("`layer` is required"))?;
                let member = self.world.cast.get(&key).ok_or_else(|| failed("no such puppet"))?;
                let info = member.puppet.layer_info(layer).ok_or_else(|| ControlError::new(ErrorKind::NotFound, format!("no layer {layer:?}")))?;
                json!({ "puppet": key, "layer": info })
            }
            "behaviors_catalog" => json!({ "behaviors": serde_json::to_value(catalog()).map_err(failed)? }),
            "behaviors_stack" => {
                let doc = session.doc();
                let index = params.get("track").and_then(Value::as_u64).map_or(Ok(self.view.track), |t| usize::try_from(t).map_err(|_| ControlError::invalid("track is too large")))?;
                let track = doc.rec.scene.tracks.get(index).ok_or_else(|| ControlError::new(ErrorKind::NotFound, format!("no track {index}")))?;
                let stack = track_stack(&doc.rec.scene, &doc.rec.templates, index, track).map_err(failed)?;
                json!({ "track": index, "puppet": track.puppet, "stack": serde_json::to_value(stack_info(&stack)).map_err(failed)? })
            }
            "world_info" => json!({
                "title": self.world.title,
                "kind": self.world.kind,
                "source": self.world.source.as_ref().map(|p| p.display().to_string()),
                "puppets": self.world.cast.keys().collect::<Vec<_>>(),
                "sounds": self.world.media.names().collect::<Vec<_>>(),
                "warnings": self.world.warnings,
            }),
            "view_state" => serde_json::to_value(&self.view).map_err(failed)?,
            other => return Err(ControlError::new(ErrorKind::NotFound, format!("no query named {other:?}"))),
        };
        Ok(HookOutcome::Ready(Reply::value(value)))
    }

    fn outline(&self, doc: &AppDoc, depth: Option<u32>) -> Result<Value, ControlError> {
        let depth = depth.unwrap_or(3);
        let scene = &doc.rec.scene;
        let mut root = serde_json::Map::new();
        root.insert("scene".into(), json!(scene.name));
        root.insert("size".into(), json!([scene.size.width, scene.size.height]));
        root.insert("fps".into(), json!(scene.fps_f64()));
        root.insert("frames".into(), json!(scene.frame_count()));
        if depth >= 1 {
            let tracks: Vec<Value> = scene
                .tracks
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let mut node = serde_json::Map::new();
                    node.insert("index".into(), json!(i));
                    node.insert("name".into(), json!(t.name));
                    node.insert("puppet".into(), json!(t.puppet));
                    node.insert("enabled".into(), json!(t.enabled));
                    if depth >= 2 {
                        node.insert("takes".into(), json!(t.takes.iter().map(|k| json!({ "number": k.number, "name": k.name, "groups": k.groups, "samples": k.samples.len() })).collect::<Vec<_>>()));
                    }
                    Value::Object(node)
                })
                .collect();
            root.insert("tracks".into(), Value::Array(tracks));
            root.insert("audio".into(), json!(scene.audio.iter().map(|a| json!({ "name": a.name, "clips": a.clips.iter().map(|c| c.name.clone()).collect::<Vec<_>>() })).collect::<Vec<_>>()));
        }
        Ok(Value::Object(root))
    }

    fn library(&self, _doc: &AppDoc) -> Result<Value, ControlError> {
        let puppets: Vec<Value> = self
            .world
            .cast
            .keys()
            .filter_map(|k| self.world.cast.get(k).map(|m| (k, m)))
            .map(|(k, m)| {
                let s = m.puppet.summary();
                json!({ "key": k, "size": [m.puppet.size.width, m.puppet.size.height], "summary": s })
            })
            .collect();
        let sounds: Vec<Value> = self
            .world
            .media
            .names()
            .filter_map(|n| self.world.media.get(n).map(|b| json!({ "name": n, "seconds": b.seconds(), "sample_rate": b.sample_rate, "channels": b.channels })))
            .collect();
        Ok(json!({ "puppets": puppets, "sounds": sounds }))
    }

    fn get_property(&self, doc: &AppDoc, path: &str) -> Result<Value, ControlError> {
        let value = serde_json::to_value(&doc.rec.scene).map_err(failed)?;
        let parts = props::parse_path(path)?;
        props::get(&value, &parts).cloned()
    }

    fn set_property(&self, doc: &mut AppDoc, path: &str, value: Value) -> Result<Value, ControlError> {
        let mut root = serde_json::to_value(&doc.rec.scene).map_err(failed)?;
        let parts = props::parse_path(path)?;
        props::set(&mut root, &parts, value)?;
        let mut scene: Scene = serde_json::from_value(root).map_err(|e| ControlError::invalid(format!("that value does not fit the scene: {e}")))?;
        scene.sanitize().map_err(|e| ControlError::invalid(e.to_string()))?;
        doc.rec.scene = scene;
        Ok(Value::Null)
    }

    fn render(&mut self, doc: &AppDoc, request: &RenderRequest, _ticket: &Ticket) -> HookResult {
        let scale = f64::from(request.scale);
        let mut renderer = Renderer::new();
        let (picture, what) = if let Some(key) = request.options.get("puppet").and_then(Value::as_str) {
            let mut pose = PuppetPose::neutral();
            if let Some(name) = request.options.get("viseme").and_then(Value::as_str) {
                pose.viseme = charcraft_puppet::Viseme::ALL.into_iter().find(|v| v.label().eq_ignore_ascii_case(name));
            }
            if request.options.get("blink").and_then(Value::as_bool) == Some(true) {
                pose = pose.with_blink(1.0);
            }
            let stage = self.stage();
            let picture = stage.render_puppet(&mut renderer, key, &pose, scale, Some(anim_core::Rgba8::WHITE)).map_err(failed)?.ok_or_else(|| ControlError::new(ErrorKind::NotFound, format!("no puppet {key:?}")))?;
            (picture, serde_json::Map::from_iter([("puppet".to_owned(), json!(key))]))
        } else {
            let frame = request.frame.unwrap_or_else(|| doc.rec.scene.frame_at(anim_core::Tick::from_seconds_f64(self.view.playhead_seconds)));
            let frames = i64::from(doc.rec.scene.frame_count());
            if frame < 0 || frame >= frames.max(1) {
                return Err(ControlError::invalid(format!("frame {frame} is outside 0..{frames}")));
            }
            let scene = doc.rec.scene.clone();
            let stage = self.stage();
            let mut evaluator = Evaluator::new(&scene, &stage.cast().templates()).map_err(failed)?;
            let poses = evaluator.poses(frame);
            let picture = stage.render_scene(&mut renderer, &scene, &poses, scale).map_err(failed)?;
            (picture, serde_json::Map::from_iter([("frame".to_owned(), json!(frame))]))
        };
        let mut value = what;
        value.insert("width".into(), json!(picture.width));
        value.insert("height".into(), json!(picture.height));
        let reply = Reply::value(Value::Object(value)).with_png_rgba(picture.width, picture.height, &picture.rgba)?;
        Ok(HookOutcome::Ready(reply))
    }

    fn screenshot(&mut self, request: &ScreenshotRequest, ticket: &Ticket) -> HookResult {
        if !self.windowed {
            return Err(ControlError::unsupported("screenshots (this session has no window; start charcraft without --headless)"));
        }
        self.shots.push_back(PendingShot { ticket: ticket.clone(), request: request.clone() });
        Ok(HookOutcome::Deferred)
    }

    fn export_formats(&self) -> Vec<ExportFormat> {
        let options = json!({
            "type": "object",
            "properties": {
                "scale": { "type": "number", "exclusiveMinimum": 0, "maximum": 8, "description": "Pixels per stage pixel (default 1)." },
                "start_frame": { "type": "integer", "minimum": 0 },
                "frames": { "type": "integer", "minimum": 1, "description": "How many frames (default: to the end)." },
                "transparent": { "type": "boolean", "description": "Leave the stage background out (default for mov and png-sequence)." }
            }
        });
        let mut formats = vec![
            ExportFormat::new("mp4", "H.264 video", "mp4"),
            ExportFormat::new("mov", "ProRes 4444 video with alpha", "mov"),
            ExportFormat::new("png-sequence", "PNG sequence (a folder)", ""),
            ExportFormat::new("png", "One PNG (the frame is start_frame)", "png"),
        ];
        for format in &mut formats {
            format.schema = options.clone();
            "Renders the scene with its takes, behaviors and sound.".clone_into(&mut format.description);
        }
        formats
    }

    fn export(&mut self, doc: &AppDoc, request: &ExportRequest, ticket: &Ticket) -> HookResult {
        let format = Format::parse(&request.format).ok_or_else(|| ControlError::new(ErrorKind::NotFound, format!("no export format {:?}", request.format)))?;
        let options: export::Options = if request.options.is_null() { export::Options::default() } else { serde_json::from_value(request.options.clone()).map_err(|e| ControlError::invalid(format!("export options: {e}")))? };
        let job = Job { cast: self.world.cast.clone(), scene: doc.rec.scene.clone(), media: self.world.media.clone(), format, path: request.path.clone(), options };
        run_job(ticket, "export", move |ticket| {
            let value = export::run(job, &mut |fraction, message| {
                ticket.progress(fraction, message);
                !ticket.is_cancelled()
            })
            .map_err(|message| if message == "cancelled" { ControlError::cancelled() } else { ControlError::failed(message) })?;
            Ok(Reply::value(value))
        })
    }

    fn load(&mut self, path: &Path) -> Result<AppDoc, ControlError> {
        let opened = World::open(path).map_err(failed)?;
        self.world = opened.world;
        self.epoch = self.epoch.saturating_add(1);
        self.stage = None;
        Ok(AppDoc::new(opened.rec, opened.rig, self.links.clone()))
    }

    fn new_document(&mut self) -> Result<AppDoc, ControlError> {
        let opened = World::demo().map_err(failed)?;
        self.world = opened.world;
        self.epoch = self.epoch.saturating_add(1);
        self.stage = None;
        Ok(AppDoc::new(opened.rec, opened.rig, self.links.clone()))
    }

    fn save(&mut self, doc: &AppDoc, file: &mut File) -> io::Result<()> {
        let bytes = self.world.save_bytes(&doc.rec, &doc.rig).map_err(io::Error::other)?;
        file.write_all(&bytes)
    }

    fn tick(&mut self, session: &mut Session<AppDoc>) {
        if self.world.sync(&session.doc().rig) {
            session.doc_mut().rec.templates = self.world.cast.templates();
        }
        // Opening a puppet or project must never make a later plain `save` overwrite that file with our JSON.
        if session.path().is_some_and(|p| !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("charcraft"))) {
            session.set_path(None);
        }
    }
}

/// The host the app and its tests run: the document, the registry, the hooks and (unrestricted unless told) paths.
///
#[must_use]
pub fn make_host(
    opened: crate::world::Opened,
    links: Links,
    registry: anim_session::CommandRegistry<AppDoc>,
    confinement: anim_mcp::Confinement,
    info: anim_mcp::HostInfo,
) -> anim_mcp::SessionHost<AppDoc, Hooks> {
    let doc = AppDoc::new(opened.rec, opened.rig, links.clone());
    let session = Session::new(doc, registry);
    anim_mcp::SessionHost::new(info, session, Hooks::new(opened.world, links)).with_confinement(confinement)
}

#[cfg(test)]
mod tests {
    use anim_mcp::{Confinement, HostInfo, Request, spawn_headless};

    use super::*;
    use crate::live::LiveLink;

    fn headless() -> anim_mcp::HeadlessSession {
        spawn_headless(|| {
            let opened = World::demo().unwrap();
            let (ui, _rx) = crossbeam_channel::unbounded();
            let links = Links { live: LiveLink::disconnected(), ui };
            let registry = crate::session::registry().unwrap();
            make_host(opened, links, registry, Confinement::unrestricted(), HostInfo { name: "charcraft".into(), version: "test".into() })
        })
        .unwrap()
    }

    #[test]
    fn queries_describe_the_puppet_and_the_stack() {
        let session = headless();
        let backend = session.backend();
        for (name, params) in [
            ("puppet_summary", json!({})),
            ("puppet_outline", json!({})),
            ("puppet_layer", json!({ "layer": "Head/mouth" })),
            ("behaviors_catalog", json!({})),
            ("behaviors_stack", json!({ "track": 0 })),
            ("world_info", json!({})),
            ("view_state", json!({})),
        ] {
            let reply = backend.call(Request::Query { name: name.into(), params }).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!reply.value.is_null(), "{name}");
        }
        assert!(backend.call(Request::Query { name: "puppet_layer".into(), params: json!({ "layer": "Nope" }) }).is_err());
        assert!(backend.call(Request::Screenshot { panel: None, path: None }).is_err(), "headless has no window");
        session.shutdown();
    }

    #[test]
    fn commands_render_and_export_work_headless() {
        let dir = tempfile::tempdir().unwrap();
        let session = headless();
        let backend = session.backend();
        backend.call(Request::CommandRun { id: "scene.set".into(), params: json!({ "width": 320, "height": 180, "duration_seconds": 0.2 }), merge_key: None }).unwrap();
        backend.call(Request::CommandRun { id: "rig.edit".into(), params: json!({ "op": "set_visible", "layer": "Head/Nose", "visible": false }), merge_key: None }).unwrap();
        let rendered = backend.call(Request::Render { frame: Some(0), scale: Some(1.0), path: None, options: json!({}) }).unwrap();
        assert_eq!(rendered.images.len(), 1);
        assert_eq!(rendered.value["width"], 320);
        let alone = backend.call(Request::Render { frame: None, scale: Some(0.25), path: None, options: json!({ "puppet": "demo", "viseme": "Oh" }) }).unwrap();
        assert_eq!(alone.value["width"], 160);
        assert!(backend.call(Request::Render { frame: Some(99), scale: None, path: None, options: json!({}) }).is_err());
        let out = dir.path().join("seq");
        let mut progress = Vec::new();
        let exported = backend
            .call_with_progress(Request::Export { format: "png-sequence".into(), path: out.display().to_string(), options: json!({ "scale": 0.5 }) }, &mut progress)
            .unwrap();
        assert_eq!(exported.value["frames"], 6);
        assert!(out.join("frame_00001.png").is_file());
        let saved = dir.path().join("doc.charcraft");
        backend.call(Request::Save { path: Some(saved.display().to_string()) }).unwrap();
        backend.call(Request::Open { path: saved.display().to_string() }).unwrap();
        let status = backend.call(Request::Status {}).unwrap();
        assert_eq!(status.value["app"]["scene"]["tracks"], 1);
        backend.call(Request::Undo {}).unwrap_or_else(|_| backend.call(Request::Status {}).unwrap());
        session.shutdown();
    }
}
