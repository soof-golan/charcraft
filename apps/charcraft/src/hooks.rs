//! What the MCP layer cannot know about `CharCraft`: the puppets, rendering, exporting, opening and saving.
//!
//! [`Hooks`] plugs into `anim_mcp::SessionHost`. Commands (scene, tracks, takes, rig, live input, view) come from the
//! registry in [`crate::session`]; everything here is read-only queries, the document outline and library, scene
//! properties, `render_frame`, `screenshot`, `export_run`, `document_open`, `document_save`.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

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
use crate::projectio;
use crate::session::{AppDoc, Links, SaveFormat, WorldJob};
use crate::state::Mode;
use crate::world::{SourceKind, World};

/// Something that happened to the document's file that the window wants to know about (recent files, the status line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocEvent {
    /// A document was opened from this path.
    Opened(PathBuf),
    /// The document was written to this path.
    Saved(PathBuf),
    /// A line for the status bar (a save finished, or failed).
    Note(String),
}

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
    /// File events the window has not taken yet.
    pub events: VecDeque<DocEvent>,
    /// What the last save, export or revert said.
    pub last_job: Option<String>,
    links: Links,
    stage: Option<(u64, u64, Stage)>,
    noted_path: Option<PathBuf>,
}

impl Hooks {
    /// Hooks around a world.
    #[must_use]
    pub fn new(world: World, links: Links) -> Self {
        Self {
            world,
            view: ViewStatus::default(),
            epoch: 1,
            windowed: false,
            shots: VecDeque::new(),
            events: VecDeque::new(),
            last_job: None,
            links,
            stage: None,
            noted_path: None,
        }
    }

    fn note(&mut self, text: String) {
        self.last_job = Some(text.clone());
        self.events.push_back(DocEvent::Note(text));
    }

    /// Replaces the open document with another and tells the window it is a new one.
    fn swap_in(&mut self, session: &mut Session<AppDoc>, opened: crate::world::Opened, path: Option<PathBuf>) {
        self.world = opened.world;
        self.epoch = self.epoch.saturating_add(1);
        self.stage = None;
        session.replace_document(AppDoc::new(opened.rec, opened.rig, self.links.clone()).with_meta(opened.meta), path);
    }

    /// Does one piece of work a command queued.
    fn run_job(&mut self, session: &mut Session<AppDoc>, job: WorldJob) {
        match job {
            WorldJob::AddPuppet { key, member } => self.world.add_member(&key, *member),
            WorldJob::ClonePuppet { from, to } => {
                if let Some(member) = self.world.cast.get(&from).cloned() {
                    self.world.add_member(&to, member);
                }
            }
            WorldJob::Save { format, path, puppet } => {
                let doc = session.doc();
                let result = match format {
                    SaveFormat::Chproj => projectio::save_project(&self.world, doc, &path).map(|line| (line, projectio::project_folder(&path))),
                    SaveFormat::Puppet => {
                        let key = puppet.or_else(|| self.world.key_for_track(&doc.rec, self.view.track));
                        key.ok_or_else(|| "there is no puppet to write".to_owned())
                            .and_then(|key| projectio::export_puppet(&self.world, doc, &key, &path).map(|line| (line, path.clone())))
                    }
                };
                match result {
                    Ok((line, written)) => {
                        if format == SaveFormat::Chproj {
                            self.world.title = written.file_stem().map_or_else(|| self.world.title.clone(), |s| s.to_string_lossy().into_owned());
                            self.world.source = Some(written.clone());
                            self.world.kind = SourceKind::Project;
                            session.mark_saved();
                        }
                        self.events.push_back(DocEvent::Saved(written));
                        self.note(line);
                    }
                    Err(e) => self.note(format!("Save failed: {e}")),
                }
            }
            WorldJob::Revert => {
                let path = session.path().map(Path::to_path_buf).or_else(|| self.world.source.clone());
                match path.map(|p| World::open(&p).map(|o| (o, p))) {
                    Some(Ok((opened, p))) => {
                        let keep = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("charcraft")).then(|| p.clone());
                        self.swap_in(session, opened, keep);
                        self.note(format!("Reverted to {}", p.display()));
                    }
                    Some(Err(e)) => self.note(format!("Revert failed: {e}")),
                    None => self.note("Nothing to revert to: the document was never saved".to_owned()),
                }
            }
            WorldJob::NewProject { name } => {
                self.swap_in(session, World::empty(&name), None);
                self.note(format!("New project {name}"));
            }
        }
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
                self.world.cast.get(key).map(|_| key.to_owned()).ok_or_else(|| {
                    ControlError::new(ErrorKind::NotFound, format!("no puppet {key:?}; the cast is {:?}", self.world.cast.keys().collect::<Vec<_>>()))
                })
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

/// The `render_frame` options that ask for one puppet in a hand-made pose instead of a scene frame.
const POSE_OPTIONS: [&str; 6] = ["puppet", "pose", "viseme", "blink", "triggers", "swaps"];

/// Merges `patch` into `base`: objects merge key by key, anything else replaces.
fn merge_json(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (key, value) in patch {
                merge_json(base.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (base, patch) => patch.clone_into(base),
    }
}

/// The pose the `render_frame` options describe: `pose` (any part of a `PuppetPose`) with `viseme` (a mouth shape by
/// its Character Animator name), `blink`, `triggers` (keys that are on) and `swaps` (swap set to chosen member) on top.
fn pose_from_options(options: &Value) -> Result<PuppetPose, ControlError> {
    let mut value = serde_json::to_value(PuppetPose::neutral()).map_err(failed)?;
    if let Some(pose) = options.get("pose").filter(|p| !p.is_null()) {
        if !pose.is_object() {
            return Err(ControlError::invalid("`pose` must be an object (any part of a puppet pose, for example {\"head\":{\"rotation\":10}})"));
        }
        merge_json(&mut value, pose);
    }
    let mut pose: PuppetPose = serde_json::from_value(value).map_err(|e| ControlError::invalid(format!("`pose` does not fit a puppet pose: {e}")))?;
    if let Some(name) = options.get("viseme").and_then(Value::as_str) {
        pose.viseme = Some(charcraft_puppet::Viseme::from_label(name).ok_or_else(|| {
            let known: Vec<&str> = charcraft_puppet::Viseme::ALL.iter().map(|v| v.label()).collect();
            ControlError::invalid(format!("no mouth shape {name:?}; the shapes are {known:?}"))
        })?);
    }
    if options.get("blink").and_then(Value::as_bool) == Some(true) {
        pose = pose.with_blink(1.0);
    }
    if let Some(keys) = options.get("triggers").filter(|t| !t.is_null()) {
        let keys: Vec<String> = serde_json::from_value(keys.clone()).map_err(|_| ControlError::invalid("`triggers` must be a list of trigger keys"))?;
        pose.triggers.extend(keys);
    }
    if let Some(swaps) = options.get("swaps").filter(|t| !t.is_null()) {
        let swaps: std::collections::BTreeMap<String, String> =
            serde_json::from_value(swaps.clone()).map_err(|_| ControlError::invalid("`swaps` must map a swap set name to a layer label"))?;
        pose.swaps.extend(swaps);
    }
    Ok(pose)
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
            QuerySpec::new("puppet_summary", "Summary of a puppet: layers, independent groups, tags, handles, mouth shapes, swap sets.")
                .with_schema(object(&puppet_param())),
            QuerySpec::new("puppet_outline", "The puppet's layer tree as indented text, one line per layer with its tags.")
                .with_schema(object(&puppet_param())),
            QuerySpec::new("puppet_layer", "One layer of a puppet by path or label: kind, tags, bounds, origin, handles, children.").with_schema(json!({
                "type": "object", "required": ["layer"],
                "properties": { "puppet": { "type": "string" }, "layer": { "type": "string", "description": "Path (Head/Left Eye) or label." } }
            })),
            QuerySpec::new("behaviors_catalog", "Every behavior CharCraft implements, with default parameters and what they mean."),
            QuerySpec::new("behaviors_stack", "The behaviors of a puppet track as the scene plays them (including the track's own edits), with parameters.")
                .with_schema(object(&json!({ "track": { "type": "integer", "minimum": 0 } }))),
            QuerySpec::new("world_info", "Where the document came from, the puppets, sounds, loading warnings and what the last save, export or revert said."),
            QuerySpec::new(
                "project_info",
                "The project panel: puppets (name, folders, size, tracks that play them, where imported from), scenes, audio, sounds and the document's file.",
            ),
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
                let index = params
                    .get("track")
                    .and_then(Value::as_u64)
                    .map_or(Ok(self.view.track), |t| usize::try_from(t).map_err(|_| ControlError::invalid("track is too large")))?;
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
                "last_job": self.last_job,
            }),
            "project_info" => projectio::project_info(&self.world, session.doc(), session.path(), session.is_dirty()),
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
                        node.insert(
                            "takes".into(),
                            json!(
                                t.takes
                                    .iter()
                                    .map(|k| json!({ "number": k.number, "name": k.name, "groups": k.groups, "samples": k.samples.len() }))
                                    .collect::<Vec<_>>()
                            ),
                        );
                    }
                    Value::Object(node)
                })
                .collect();
            root.insert("tracks".into(), Value::Array(tracks));
            root.insert(
                "audio".into(),
                json!(
                    scene
                        .audio
                        .iter()
                        .map(|a| json!({ "name": a.name, "clips": a.clips.iter().map(|c| c.name.clone()).collect::<Vec<_>>() }))
                        .collect::<Vec<_>>()
                ),
            );
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
        let single = POSE_OPTIONS.iter().any(|k| request.options.get(*k).is_some_and(|v| !v.is_null()));
        let (picture, what) = if single {
            let key = match request.options.get("puppet").and_then(Value::as_str) {
                Some(key) => key.to_owned(),
                None => self.puppet_key(&request.options)?,
            };
            let pose = pose_from_options(&request.options)?;
            let background = if request.options.get("transparent").and_then(Value::as_bool) == Some(true) { None } else { Some(anim_core::Rgba8::WHITE) };
            let stage = self.stage();
            let picture = stage
                .render_puppet(&mut renderer, &key, &pose, scale, background)
                .map_err(failed)?
                .ok_or_else(|| ControlError::new(ErrorKind::NotFound, format!("no puppet {key:?}")))?;
            (picture, serde_json::Map::from_iter([("puppet".to_owned(), json!(key))]))
        } else {
            let at = match request.options.get("time_seconds").and_then(Value::as_f64) {
                Some(t) if t.is_finite() && t >= 0.0 => Some(doc.rec.scene.frame_at(anim_core::Tick::from_seconds_f64(t))),
                Some(_) => return Err(ControlError::invalid("time_seconds must be a finite number of seconds, zero or more")),
                None => None,
            };
            let frame = at.or(request.frame).unwrap_or_else(|| doc.rec.scene.frame_at(anim_core::Tick::from_seconds_f64(self.view.playhead_seconds)));
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
                "transparent": { "type": "boolean", "description": "Leave the stage background out (default for mov, png-sequence and after-effects)." },
                "fps": { "type": "number", "exclusiveMinimum": 0, "maximum": 240, "description": "Output frame rate (default: the scene's); frames are picked by time." }
            }
        });
        let mut formats: Vec<ExportFormat> = Format::ALL
            .into_iter()
            .map(|f| {
                let mut format = ExportFormat::new(f.id(), f.label(), f.extension());
                format.schema = options.clone();
                let note = match f {
                    Format::AfterEffects => {
                        " The path is a folder: frames/ (PNG sequence), audio.wav, <name>.mov (ProRes 4444 with alpha), export.json and REEXPORT.txt with the re-export command. This is the Dynamic Link alternative."
                    }
                    Format::Wav => " The sound alone; fails when the scene has none.",
                    Format::Gif => " 1-bit alpha when transparent.",
                    _ => "",
                };
                format.description = format!("Renders the scene with its takes, behaviors and sound.{note}");
                format
            })
            .collect();
        for preset in export::presets() {
            let mut format = ExportFormat::new(&preset.id, &preset.label, preset.format.extension());
            format.schema = options.clone();
            format.description = format!("Preset: {} Options given to export_run override the preset's. Writes {}.", preset.description, preset.format.id());
            formats.push(format);
        }
        formats
    }

    fn export(&mut self, doc: &AppDoc, request: &ExportRequest, ticket: &Ticket) -> HookResult {
        let (format, options) = export::resolve(&request.format, &request.options).map_err(|message| {
            if message.starts_with("no export") { ControlError::new(ErrorKind::NotFound, message) } else { ControlError::invalid(message) }
        })?;
        let job = Job {
            cast: self.world.cast.clone(),
            scene: doc.rec.scene.clone(),
            media: self.world.media.clone(),
            format,
            path: request.path.clone(),
            options,
            project: self.world.source.clone(),
        };
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
        self.events.push_back(DocEvent::Opened(path.to_path_buf()));
        Ok(AppDoc::new(opened.rec, opened.rig, self.links.clone()).with_meta(opened.meta))
    }

    fn new_document(&mut self) -> Result<AppDoc, ControlError> {
        let opened = World::demo().map_err(failed)?;
        self.world = opened.world;
        self.epoch = self.epoch.saturating_add(1);
        self.stage = None;
        Ok(AppDoc::new(opened.rec, opened.rig, self.links.clone()).with_meta(opened.meta))
    }

    fn save(&mut self, doc: &AppDoc, file: &mut File) -> io::Result<()> {
        let bytes = self.world.save_bytes_with(&doc.rec, &doc.rig, &doc.meta).map_err(io::Error::other)?;
        file.write_all(&bytes)
    }

    fn tick(&mut self, session: &mut Session<AppDoc>) {
        while let Ok(job) = self.links.jobs_in.try_recv() {
            self.run_job(session, job);
        }
        // A plain save (or save as) to a .charcraft file is a recent file too.
        if let Some(path) = session.path().map(Path::to_path_buf)
            && self.noted_path.as_ref() != Some(&path)
        {
            self.noted_path = Some(path.clone());
            self.events.push_back(DocEvent::Saved(path));
        }
        // Sounds the commands read from files go into the media store, which the document does not hold.
        while let Ok((name, buffer)) = self.links.sounds_in.try_recv() {
            if let Err(e) = self.world.media.insert(&name, buffer) {
                self.world.warnings.push(format!("sound {name:?} was not kept: {e}"));
            }
        }
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
    let doc = AppDoc::new(opened.rec, opened.rig, links.clone()).with_meta(opened.meta);
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
            let links = Links::new(LiveLink::disconnected(), ui);
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
        backend
            .call(Request::CommandRun { id: "scene.set".into(), params: json!({ "width": 320, "height": 180, "duration_seconds": 0.2 }), merge_key: None })
            .unwrap();
        backend
            .call(Request::CommandRun {
                id: "rig.edit".into(),
                params: json!({ "op": "set_visible", "layer": "Head/Nose", "visible": false }),
                merge_key: None,
            })
            .unwrap();
        let rendered = backend.call(Request::Render { frame: Some(0), scale: Some(1.0), path: None, options: json!({}) }).unwrap();
        assert_eq!(rendered.images.len(), 1);
        assert_eq!(rendered.value["width"], 320);
        let alone = backend.call(Request::Render { frame: None, scale: Some(0.25), path: None, options: json!({ "puppet": "demo", "viseme": "Oh" }) }).unwrap();
        assert_eq!(alone.value["width"], 160);
        assert!(backend.call(Request::Render { frame: Some(99), scale: None, path: None, options: json!({}) }).is_err());
        let out = dir.path().join("seq");
        let mut progress = Vec::new();
        let exported = backend
            .call_with_progress(
                Request::Export { format: "png-sequence".into(), path: out.display().to_string(), options: json!({ "scale": 0.5 }) },
                &mut progress,
            )
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

    /// Half a second of silence, a vowel-like buzz, and a hiss: enough for the analyzer to move the mouth.
    #[expect(clippy::as_conversions, clippy::cast_possible_truncation, reason = "test signal")]
    fn speechlike() -> anim_audio::PcmBuffer {
        let rate = 16_000_u32;
        let mut samples = vec![0.0_f32; 8_000];
        for i in 0..16_000_u32 {
            let t = f64::from(i) / f64::from(rate);
            let voiced: f64 = (1..=20).map(|h| (std::f64::consts::TAU * 120.0 * f64::from(h) * t).sin() / f64::from(h)).sum();
            samples.push((0.2 * voiced) as f32);
        }
        let mut state = 7_u32;
        for _ in 0..8_000 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            samples.push((f64::from(state >> 16) / 65_536.0 - 0.5) as f32 * 0.6);
        }
        anim_audio::PcmBuffer::new(rate, 1, samples)
    }

    fn run(backend: &anim_mcp::Backend, id: &str, params: Value) -> Result<Reply, ControlError> {
        // The reply wraps the command's own answer in `result`.
        backend.call(Request::CommandRun { id: id.into(), params, merge_key: None }).map(|mut reply| {
            reply.value = reply.value["result"].take();
            reply
        })
    }

    #[test]
    fn render_takes_a_hand_made_pose_a_time_and_refuses_nonsense() {
        let session = headless();
        let backend = session.backend();
        let render = |options: Value| backend.call(Request::Render { frame: None, scale: Some(0.5), path: None, options });
        let neutral = render(json!({ "puppet": "demo" })).unwrap();
        let oh = render(json!({ "viseme": "Oh" })).unwrap();
        assert_eq!(oh.value["puppet"], "demo", "a pose option alone picks the first puppet");
        assert_ne!(neutral.images.first().map(|i| &i.data), oh.images.first().map(|i| &i.data), "the mouth shape changes the picture");
        let tilted = render(json!({ "pose": { "head": { "rotation": 25.0 } }, "transparent": true })).unwrap();
        assert_ne!(neutral.images.first().map(|i| &i.data), tilted.images.first().map(|i| &i.data));
        render(json!({ "triggers": ["a"], "swaps": { "Hair": "Nope" }, "blink": true })).unwrap();
        assert!(render(json!({ "viseme": "Nonsense" })).is_err());
        assert!(render(json!({ "pose": 3 })).is_err());
        assert!(render(json!({ "pose": { "head": { "rotation": "up" } } })).is_err());
        assert!(render(json!({ "triggers": "a" })).is_err());
        assert!(render(json!({ "time_seconds": -1.0 })).is_err());
        assert!(render(json!({ "time_seconds": 100.0 })).is_err(), "past the end of the scene");
        assert!(render(json!({ "time_seconds": 0.5 })).is_ok());
        session.shutdown();
    }

    #[test]
    fn lipsync_record_makes_a_take_and_keeps_the_sound_for_export() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("hello there.wav");
        std::fs::write(&wav, charcraft_record::wav_bytes(&speechlike())).unwrap();
        let session = headless();
        let backend = session.backend();
        let done = run(backend, "lipsync.record", json!({ "track": 0, "path": wav.display().to_string(), "start_seconds": 0.25 })).unwrap();
        assert_eq!(done.value["media"], "hello there");
        assert!(done.value["keys"].as_u64().unwrap() >= 2, "{}", done.value);
        let takes = run(backend, "take.list", json!({ "track": 0 })).unwrap();
        assert!(!takes.value["takes"].as_array().unwrap().is_empty());
        let scene = run(backend, "scene.info", json!({})).unwrap();
        assert_eq!(scene.value["audio"][0]["clips"].as_array().map(Vec::len), Some(1), "{}", scene.value);
        // The sound reaches the media store on the host's next tick.
        let mut sounds = Vec::new();
        for _ in 0..50 {
            let info = backend.call(Request::Query { name: "world_info".into(), params: json!({}) }).unwrap();
            sounds = info.value["sounds"].as_array().cloned().unwrap_or_default();
            if !sounds.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(sounds, [json!("hello there")]);
        // One undo step takes the take and the clip away again.
        backend.call(Request::Undo {}).unwrap();
        let scene = run(backend, "scene.info", json!({})).unwrap();
        assert!(scene.value["audio"].as_array().is_none_or(Vec::is_empty), "{}", scene.value);
        assert!(run(backend, "lipsync.record", json!({ "track": 0, "path": dir.path().join("missing.wav").display().to_string() })).is_err());
        assert!(run(backend, "lipsync.record", json!({ "track": 9, "path": wav.display().to_string() })).is_err());
        assert!(run(backend, "lipsync.record", json!({ "path": wav.display().to_string(), "start_seconds": -1.0 })).is_err());
        session.shutdown();
    }
}
