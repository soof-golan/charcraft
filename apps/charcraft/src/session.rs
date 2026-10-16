//! The document of the app and its command registry.
//!
//! [`AppDoc`] is what the `anim-session` [`Session`](anim_session::Session) snapshots for undo: the scene with its
//! takes and recording settings ([`RecordDoc`]) and the log of rig edits (tags, handles, origins, swap sets) made to
//! the puppets. Pictures and sound do not live here (they would make every undo step huge): the [`World`](crate::world)
//! keeps the puppets and replays the rig log onto them.
//!
//! Every user action is a command, so the menus, the shortcuts and the MCP tools are one list:
//!
//! * all of `charcraft-record`'s scene, track, take, audio, recording and playback commands (wrapped, see below),
//! * `rig.edit` and `rig.layers` for the puppet,
//! * `tracking.*` and `mic.*` for the live input actor (transient: live input is not document state),
//! * `ui.set` to drive the window (mode, selection, playhead).

use anim_session::command::RunFn;
use anim_session::{CommandError, CommandRegistry, CommandSpec, RegistryError, SizeHint, param_opt};
use charcraft_puppet::{Puppet, PuppetOp};
use charcraft_record::RecordDoc;
use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::live::{LiveLink, MicAction};
use crate::state::UiSet;

/// Most rig edits a document keeps.
pub const MAX_RIG_EDITS: usize = 10_000;
/// Longest tag, trigger key or name accepted.
const MAX_TEXT: usize = 256;

/// One layer of a puppet as far as validating an edit needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkelLayer {
    /// Path from the root (`Head/Left Eye`); empty for the root.
    pub path: String,
    /// The name without the `+` marker.
    pub label: String,
    /// An independent group.
    pub independent: bool,
}

/// The layer names of a puppet, so a rig edit can be checked without the pictures.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skeleton {
    /// Layers, root first.
    pub layers: Vec<SkelLayer>,
}

impl Skeleton {
    /// The skeleton of a puppet.
    #[must_use]
    pub fn of(puppet: &Puppet) -> Self {
        Self { layers: puppet.layers().iter().map(|l| SkelLayer { path: l.path.clone(), label: l.label.clone(), independent: l.independent }).collect() }
    }

    /// The layer a path or label names: the path wins, then the first label (as `Puppet::resolve` does).
    #[must_use]
    pub fn resolve(&self, layer: &str) -> Option<&SkelLayer> {
        if layer.is_empty() {
            return self.layers.first();
        }
        self.layers.iter().find(|l| l.path == layer).or_else(|| self.layers.iter().find(|l| l.label == layer))
    }

    fn need(&self, layer: &str) -> Result<&SkelLayer, CommandError> {
        self.resolve(layer).ok_or_else(|| CommandError::invalid_params(format!("no layer {layer:?} (see rig_layers)")))
    }

    fn need_group(&self, group: &str) -> Result<&SkelLayer, CommandError> {
        let layer = self.need(group)?;
        if layer.independent { Ok(layer) } else { Err(CommandError::invalid_params(format!("{group:?} is not an independent group (its name must start with +)"))) }
    }
}

/// One edit of a puppet, as logged in the document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RigEdit {
    /// The cast key of the puppet.
    pub puppet: String,
    /// The edit.
    pub op: PuppetOp,
}

/// The rig edits and the names they were checked against.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RigLog {
    /// Layer names by cast key.
    pub skeletons: std::collections::BTreeMap<String, Skeleton>,
    /// The edits, oldest first.
    pub edits: Vec<RigEdit>,
}

/// What the document commands can reach beyond the document: the live input actor and the window.
#[derive(Debug, Clone)]
pub struct Links {
    /// The camera and microphone actor.
    pub live: LiveLink,
    /// Requests to change the window's view.
    pub ui: Sender<UiSet>,
}

/// The session document.
#[derive(Debug, Clone)]
pub struct AppDoc {
    /// Scene, takes and recording settings.
    pub rec: RecordDoc,
    /// Rig edits.
    pub rig: RigLog,
    /// Channels to the actors (not part of the document's content; they survive undo).
    pub links: Links,
}

impl AppDoc {
    /// A document around a scene.
    #[must_use]
    pub const fn new(rec: RecordDoc, rig: RigLog, links: Links) -> Self {
        Self { rec, rig, links }
    }
}

impl SizeHint for AppDoc {
    fn size_hint(&self) -> usize {
        let names: usize = self.rig.skeletons.values().map(|s| s.layers.len().saturating_mul(96)).sum();
        self.rec.size_hint().saturating_add(names).saturating_add(self.rig.edits.len().saturating_mul(160))
    }
}

// --- wrapping the record commands ------------------------------------------------------------------------------

/// How many of `charcraft-record`'s commands the table below can wrap.
const WRAPPED: usize = 64;

fn nth_run<const I: usize>(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let specs = charcraft_record::all_commands();
    let spec = specs.get(I).ok_or_else(|| CommandError::failed("the command table is out of date"))?;
    (spec.run)(&mut doc.rec, params)
}

fn nth_enabled<const I: usize>(doc: &AppDoc) -> bool {
    charcraft_record::all_commands().get(I).is_none_or(|spec| (spec.enabled)(&doc.rec))
}

macro_rules! table {
    ($f:ident, $t:ty; $($i:literal)*) => { [$($f::<$i> as $t),*] };
}

const RUNS: [RunFn<AppDoc>; WRAPPED] = table!(nth_run, RunFn<AppDoc>;
    0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31
    32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63);
const ENABLED: [fn(&AppDoc) -> bool; WRAPPED] = table!(nth_enabled, fn(&AppDoc) -> bool;
    0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31
    32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63);

/// The record crate's commands, retargeted from its `RecordDoc` to the app's document.
fn wrapped() -> Vec<CommandSpec<AppDoc>> {
    charcraft_record::all_commands()
        .into_iter()
        .zip(RUNS.into_iter().zip(ENABLED))
        .map(|(s, (run, enabled))| CommandSpec {
            id: s.id,
            label: s.label,
            menu: s.menu,
            shortcut: s.shortcut,
            separator_before: s.separator_before,
            enabled,
            run,
            description: s.description,
            input_schema: s.input_schema,
            history: s.history,
            action: s.action,
            mcp: s.mcp,
        })
        .collect()
}

// --- rig edits -------------------------------------------------------------------------------------------------

fn text(what: &str, value: &str) -> Result<(), CommandError> {
    if value.trim().is_empty() || value.chars().count() > MAX_TEXT {
        return Err(CommandError::invalid_params(format!("{what} must be 1 to {MAX_TEXT} characters")));
    }
    Ok(())
}

fn finite(what: &str, p: &[f64; 2]) -> Result<(), CommandError> {
    if p.iter().all(|v| v.is_finite()) { Ok(()) } else { Err(CommandError::invalid_params(format!("{what} must be finite numbers"))) }
}

/// Checks an edit against the skeleton and says what it will do (the key a handle gets).
fn check_edit(skeleton: &Skeleton, op: &PuppetOp) -> Result<Option<String>, CommandError> {
    match op {
        PuppetOp::AddTag { layer, tag } | PuppetOp::RemoveTag { layer, tag } => {
            skeleton.need(layer)?;
            text("tag", tag)?;
            Ok(None)
        }
        PuppetOp::AddHandle { group, name, position, .. } => {
            let g = skeleton.need_group(group)?;
            text("handle name", name)?;
            finite("position", position)?;
            Ok(Some(format!("{}/{name}", g.label)))
        }
        PuppetOp::RemoveHandle { group, key } => {
            skeleton.need_group(group)?;
            text("handle key", key)?;
            Ok(None)
        }
        PuppetOp::SetOrigin { group, origin } => {
            skeleton.need_group(group)?;
            finite("origin", origin)?;
            Ok(None)
        }
        PuppetOp::SetKeyTrigger { layer, key } => {
            skeleton.need(layer)?;
            key.as_deref().map_or(Ok(()), |k| text("trigger key", k))?;
            Ok(None)
        }
        PuppetOp::AddSwapSet { name, members } => {
            text("swap set name", name)?;
            if members.len() < 2 {
                return Err(CommandError::invalid_params("a swap set needs at least two layers"));
            }
            for m in members {
                skeleton.need(m)?;
            }
            Ok(None)
        }
        PuppetOp::SetVisible { layer, .. } => {
            skeleton.need(layer)?;
            Ok(None)
        }
    }
}

fn rig_edit(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let puppet: Option<String> = param_opt(params, "puppet")?;
    let op: PuppetOp = serde_json::from_value(params.clone()).map_err(|e| CommandError::invalid_params(format!("not a rig edit: {e}")))?;
    let key = match puppet {
        Some(key) => key,
        None => doc.rig.skeletons.keys().next().cloned().ok_or_else(|| CommandError::failed("no puppet is loaded"))?,
    };
    let skeleton = doc.rig.skeletons.get(&key).ok_or_else(|| CommandError::invalid_params(format!("no puppet {key:?} (cast keys: {:?})", doc.rig.skeletons.keys().collect::<Vec<_>>())))?;
    let handle_key = check_edit(skeleton, &op)?;
    if doc.rig.edits.len() >= MAX_RIG_EDITS {
        return Err(CommandError::failed("too many rig edits"));
    }
    doc.rig.edits.push(RigEdit { puppet: key.clone(), op });
    Ok(json!({ "puppet": key, "key": handle_key, "edits": doc.rig.edits.len() }))
}

fn rig_layers(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let puppet: Option<String> = param_opt(params, "puppet")?;
    let key = match puppet {
        Some(key) => key,
        None => doc.rig.skeletons.keys().next().cloned().ok_or_else(|| CommandError::failed("no puppet is loaded"))?,
    };
    let skeleton = doc.rig.skeletons.get(&key).ok_or_else(|| CommandError::invalid_params(format!("no puppet {key:?}")))?;
    Ok(json!({
        "puppet": key,
        "puppets": doc.rig.skeletons.keys().collect::<Vec<_>>(),
        "layers": skeleton.layers.iter().map(|l| json!({ "path": l.path, "label": l.label, "independent": l.independent })).collect::<Vec<_>>(),
        "edits": doc.rig.edits.iter().filter(|e| e.puppet == key).count(),
    }))
}

// --- live input ------------------------------------------------------------------------------------------------

macro_rules! tracking_command {
    ($name:ident, $tool:literal) => {
        fn $name(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
            doc.links.live.tool($tool, params).map_err(CommandError::failed)
        }
    };
}

tracking_command!(tracking_status, "tracking_status");
tracking_command!(tracking_latest, "tracking_latest");
tracking_command!(tracking_start, "tracking_start");
tracking_command!(tracking_stop, "tracking_stop");
tracking_command!(tracking_calibrate, "tracking_calibrate");
tracking_command!(tracking_reset, "tracking_reset_calibration");
tracking_command!(tracking_sensitivity, "tracking_set_sensitivity");
tracking_command!(tracking_smoothing, "tracking_set_smoothing");
#[cfg(feature = "live")]
tracking_command!(tracking_cameras, "tracking_list_cameras");

fn mic_command(doc: &AppDoc, action: MicAction) -> Result<Value, CommandError> {
    doc.links.live.mic(action).map_err(CommandError::failed)
}

fn mic_start(doc: &mut AppDoc, _: &Value) -> Result<Value, CommandError> {
    mic_command(doc, MicAction::Start)
}

fn mic_stop(doc: &mut AppDoc, _: &Value) -> Result<Value, CommandError> {
    mic_command(doc, MicAction::Stop)
}

fn mic_status(doc: &mut AppDoc, _: &Value) -> Result<Value, CommandError> {
    mic_command(doc, MicAction::Status)
}

fn mic_demo(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let on: Option<bool> = param_opt(params, "on")?;
    mic_command(doc, MicAction::DemoVoice(on.unwrap_or(true)))
}

fn ui_set(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let set: UiSet = serde_json::from_value(params.clone()).map_err(|e| CommandError::invalid_params(e.to_string()))?;
    if set.seek_seconds.is_some_and(|s| !s.is_finite()) {
        return Err(CommandError::invalid_params("seek_seconds must be a finite number"));
    }
    doc.links.ui.send(set.clone()).map_err(|_| CommandError::failed("there is no window to change"))?;
    Ok(json!({ "requested": set }))
}

const NO_PARAMS: &str = r#"{"type":"object","properties":{},"additionalProperties":false}"#;

/// The commands the app adds to the record crate's.
fn app_commands() -> Vec<CommandSpec<AppDoc>> {
    #[cfg_attr(not(feature = "live"), expect(unused_mut, reason = "the live build adds a command"))]
    let mut all = vec![
        CommandSpec::new("rig.edit", "Edit puppet rig", rig_edit)
            .describe("Edit a puppet's rigging. Pick the edit with 'op': add_tag/remove_tag (layer, tag), add_handle (group, name, kind Fixed|Draggable|Dangle|Attach|Stick|Walk, position [x,y] in artwork pixels; the answer names the handle key), remove_handle (group, key), set_origin (group, origin), set_key_trigger (layer, key or null), add_swap_set (name, members), set_visible (layer, visible). Layers are paths (Head/Left Eye) or labels; use rig_layers to list them. 'puppet' picks the puppet (default: the first).")
            .with_schema(r#"{"type":"object","required":["op"],"properties":{"puppet":{"type":"string"},"op":{"enum":["add_tag","remove_tag","add_handle","remove_handle","set_origin","set_key_trigger","add_swap_set","set_visible"]},"layer":{"type":"string"},"tag":{"type":"string"},"group":{"type":"string"},"name":{"type":"string"},"kind":{"enum":["Fixed","Draggable","Dangle","Attach","Stick","Walk"]},"position":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"origin":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"key":{"type":["string","null"]},"members":{"type":"array","items":{"type":"string"},"minItems":2},"visible":{"type":"boolean"}}}"#),
        CommandSpec::new("rig.layers", "List puppet layers", rig_layers)
            .transient()
            .describe("The layers of a puppet (path, label, independent group) and the cast keys of the loaded puppets.")
            .with_schema(r#"{"type":"object","properties":{"puppet":{"type":"string"}}}"#),
        CommandSpec::new("tracking.status", "Tracking status", tracking_status).transient().describe("Whether face tracking runs, frame counts, calibration, sensitivity and smoothing.").with_schema(NO_PARAMS),
        CommandSpec::new("tracking.latest", "Latest tracked face", tracking_latest)
            .transient()
            .describe("The newest tracked face: head pose in radians, brows, eyes, gaze, mouth, smile.")
            .with_schema(r#"{"type":"object","properties":{"mirrored":{"type":"boolean"}}}"#),
        CommandSpec::new("tracking.start", "Start tracking", tracking_start)
            .transient()
            .describe("Open a camera and start tracking (without the 'live' build feature a scripted demo face plays instead).")
            .with_schema(r#"{"type":"object","properties":{"index":{"type":"integer","minimum":0},"width":{"type":"integer","minimum":1},"height":{"type":"integer","minimum":1},"fps":{"type":"integer","minimum":1}}}"#),
        CommandSpec::new("tracking.stop", "Stop tracking", tracking_stop).transient().describe("Stop tracking and release the camera.").with_schema(NO_PARAMS),
        CommandSpec::new("tracking.calibrate", "Calibrate neutral face", tracking_calibrate)
            .transient()
            .describe("Capture the performer's neutral face from the next frames.")
            .with_schema(r#"{"type":"object","properties":{"frames":{"type":"integer","minimum":1,"maximum":600}}}"#),
        CommandSpec::new("tracking.reset_calibration", "Reset calibration", tracking_reset).transient().describe("Forget the captured neutral face.").with_schema(NO_PARAMS),
        CommandSpec::new("tracking.set_sensitivity", "Tracking sensitivity", tracking_sensitivity)
            .transient()
            .describe("Change how far each measurement must move to reach the end of its range (brow, eye_shut, eye_open, mouth_open, mouth_wide, smile, gaze).")
            .with_schema(r#"{"type":"object","properties":{"brow":{"type":"number"},"eye_shut":{"type":"number"},"eye_open":{"type":"number"},"mouth_open":{"type":"number"},"mouth_wide":{"type":"number"},"smile":{"type":"number"},"gaze":{"type":"number"}}}"#),
        CommandSpec::new("tracking.set_smoothing", "Tracking smoothing", tracking_smoothing)
            .transient()
            .describe("Switch smoothing on or off and tune the One-Euro filters (head, brows, eyes, gaze, mouth: min_cutoff, beta).")
            .with_schema(r#"{"type":"object","properties":{"enabled":{"type":"boolean"},"head":{"type":"object"},"brows":{"type":"object"},"eyes":{"type":"object"},"gaze":{"type":"object"},"mouth":{"type":"object"}}}"#),
        CommandSpec::new("mic.start", "Start microphone", mic_start).transient().describe("Open the default microphone; live lip sync drives the mouth.").with_schema(NO_PARAMS),
        CommandSpec::new("mic.stop", "Stop microphone", mic_stop).transient().describe("Close the microphone.").with_schema(NO_PARAMS),
        CommandSpec::new("mic.status", "Microphone status", mic_status).transient().describe("Whether the microphone is on, its level and the mouth shape the voice makes.").with_schema(NO_PARAMS),
        CommandSpec::new("mic.demo_voice", "Demo voice", mic_demo)
            .transient()
            .describe("Play a synthetic talking mouth while no microphone is on (for demos and tests).")
            .with_schema(r#"{"type":"object","properties":{"on":{"type":"boolean"}}}"#),
        CommandSpec::new("ui.set", "Change the view", ui_set)
            .transient()
            .describe("Drive the window: mode (rig|record|stream), track (index), layer (path), seek_seconds, playing, mirror, dark, demo_voice. Only the given fields change.")
            .with_schema(r#"{"type":"object","properties":{"mode":{"enum":["rig","record","stream"]},"track":{"type":"integer","minimum":0},"layer":{"type":"string"},"seek_seconds":{"type":"number"},"playing":{"type":"boolean"},"mirror":{"type":"boolean"},"dark":{"type":"boolean"},"demo_voice":{"type":"boolean"}}}"#),
    ];
    #[cfg(feature = "live")]
    all.push(CommandSpec::new("tracking.list_cameras", "List cameras", tracking_cameras).transient().describe("The cameras the system reports.").with_schema(NO_PARAMS));
    all
}

/// Where each command appears in the menus: the app's own placement for the record crate's commands (which only
/// know their ids) and for its own.
fn place(spec: CommandSpec<AppDoc>) -> CommandSpec<AppDoc> {
    match spec.id {
        "scene.set" | "scene.import" | "track.add" | "track.remove" => spec.in_menu(&["Scene"]),
        "rig.edit" => spec.in_menu(&["Rig"]),
        "tracking.start" | "tracking.stop" | "mic.stop" => spec.in_menu(&["Input"]),
        "tracking.calibrate" => spec.in_menu(&["Input"]).with_shortcut("Cmd+K"),
        "mic.start" => spec.in_menu(&["Input"]).separated(),
        _ => spec,
    }
}

/// The registry: undo and redo plus every command above.
///
/// # Errors
/// [`RegistryError`] for a duplicate id (a bug; the tests build the registry).
pub fn registry() -> Result<CommandRegistry<AppDoc>, RegistryError> {
    let mut registry = CommandRegistry::with_history_commands();
    registry.register_all(wrapped().into_iter().chain(app_commands()).map(place))?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use anim_core::{FrameRate, Size, Tick};
    use anim_session::Session;
    use charcraft_record::Scene;

    use super::*;

    fn doc() -> AppDoc {
        let puppet = crate::demo::puppet().unwrap();
        let mut rig = RigLog::default();
        rig.skeletons.insert("demo".into(), Skeleton::of(&puppet));
        let (ui, _rx) = crossbeam_channel::unbounded();
        let scene = Scene::new("s", Size::new(640.0, 360.0), FrameRate::FPS_30, Tick::from_seconds_f64(2.0));
        AppDoc::new(RecordDoc::new(scene), rig, Links { live: LiveLink::disconnected(), ui })
    }

    #[test]
    fn the_table_covers_every_record_command() {
        assert!(charcraft_record::all_commands().len() <= WRAPPED, "raise WRAPPED");
        let registry = registry().unwrap();
        assert!(registry.get("take.list").is_some() && registry.get("rig.edit").is_some() && registry.get("tracking.status").is_some());
    }

    #[test]
    fn wrapped_commands_edit_the_scene_and_undo() {
        let mut session = Session::new(doc(), registry().unwrap());
        session.execute("track.add", &json!({ "puppet": "demo", "name": "Hero" })).unwrap();
        assert_eq!(session.doc().rec.scene.tracks.len(), 1);
        let info = session.execute("scene.info", &json!({})).unwrap();
        assert_eq!(info["tracks"][0]["name"], "Hero");
        session.undo().unwrap();
        assert!(session.doc().rec.scene.tracks.is_empty());
    }

    #[test]
    fn rig_edits_are_checked_and_logged() {
        let mut session = Session::new(doc(), registry().unwrap());
        let done = session.execute("rig.edit", &json!({ "op": "add_handle", "group": "Head", "name": "Chin", "kind": "Draggable", "position": [320.0, 400.0] })).unwrap();
        assert_eq!(done["key"], "Head/Chin");
        assert_eq!(session.doc().rig.edits.len(), 1);
        let bad = session.execute("rig.edit", &json!({ "op": "add_tag", "layer": "No such layer", "tag": "x" }));
        assert!(bad.is_err());
        let not_group = session.execute("rig.edit", &json!({ "op": "set_origin", "group": "Head/Face", "origin": [1.0, 2.0] }));
        assert!(not_group.is_err(), "Face is not an independent group");
        assert_eq!(session.doc().rig.edits.len(), 1, "failed edits leave nothing behind");
        session.undo().unwrap();
        assert!(session.doc().rig.edits.is_empty());
    }

    #[test]
    fn live_commands_fail_cleanly_without_an_actor() {
        let mut session = Session::new(doc(), registry().unwrap());
        assert!(session.execute("tracking.status", &json!({})).is_err());
        assert!(session.execute("mic.status", &json!({})).is_err());
        assert!(session.execute("ui.set", &json!({ "mode": "stream" })).is_err(), "nobody listens");
    }
}
