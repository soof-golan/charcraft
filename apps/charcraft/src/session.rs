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

use anim_audio::PcmBuffer;
use anim_core::Tick;
use anim_session::command::RunFn;
use anim_session::{CommandError, CommandRegistry, CommandSpec, RegistryError, SizeHint, param_opt};
use charcraft_behaviors::Inputs;
use charcraft_puppet::{Puppet, PuppetOp};
use charcraft_record::RecordDoc;
use charcraft_record::{AudioClip, ParamGroup, Recorder};
use crossbeam_channel::{Receiver, Sender};
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
    /// Holds layers of its own (a layer cycle needs at least two).
    #[serde(default)]
    pub children: usize,
    /// The keys of its handles.
    #[serde(default)]
    pub handles: Vec<String>,
}

/// The layer names of a puppet, so a rig edit can be checked without the pictures.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skeleton {
    /// Layers, root first.
    pub layers: Vec<SkelLayer>,
    /// The names of the swap sets.
    #[serde(default)]
    pub swap_sets: Vec<String>,
}

impl Skeleton {
    /// The skeleton of a puppet.
    #[must_use]
    pub fn of(puppet: &Puppet) -> Self {
        Self {
            layers: puppet
                .layers()
                .iter()
                .map(|l| SkelLayer {
                    path: l.path.clone(),
                    label: l.label.clone(),
                    independent: l.independent,
                    children: l.children.len(),
                    handles: l.handles.iter().map(|h| h.key.clone()).collect(),
                })
                .collect(),
            swap_sets: puppet.swap_sets.iter().map(|s| s.name.clone()).collect(),
        }
    }

    /// Follows a checked edit, so the next one is checked against what the first did.
    pub fn follow(&mut self, op: &PuppetOp) {
        let index = |s: &Self, name: &str| s.resolve(name).and_then(|l| s.layers.iter().position(|o| o.path == l.path));
        match op {
            PuppetOp::SetIndependent { layer, independent } => {
                if let Some(l) = index(self, layer).and_then(|i| self.layers.get_mut(i)).filter(|l| !l.path.is_empty()) {
                    l.independent = *independent;
                    if !*independent {
                        l.handles.clear();
                    }
                }
            }
            PuppetOp::AddHandle { group, name, .. } => {
                if let Some(l) = index(self, group).and_then(|i| self.layers.get_mut(i)) {
                    let key = format!("{}/{name}", l.label);
                    if !l.handles.contains(&key) {
                        l.handles.push(key);
                    }
                }
            }
            PuppetOp::RemoveHandle { group, key } => {
                if let Some(l) = index(self, group).and_then(|i| self.layers.get_mut(i)) {
                    l.handles.retain(|k| k != key);
                }
            }
            PuppetOp::AddSwapSet { name, .. } => {
                if !self.swap_sets.contains(name) {
                    self.swap_sets.push(name.clone());
                }
            }
            PuppetOp::RemoveSwapSet { name } => self.swap_sets.retain(|s| s != name),
            PuppetOp::RenameSwapSet { name, new_name } => {
                if let Some(s) = self.swap_sets.iter_mut().find(|s| *s == name) {
                    s.clone_from(new_name);
                }
            }
            _ => {}
        }
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
        if layer.independent {
            Ok(layer)
        } else {
            Err(CommandError::invalid_params(format!("{group:?} is not an independent group (its name must start with +)")))
        }
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
    /// Sounds commands have read from files: the hooks move them into the media store (which is not part of the
    /// undoable document) on their next tick.
    pub sounds: Sender<Sound>,
    /// The other end of [`Self::sounds`], drained by the hooks.
    pub sounds_in: Receiver<Sound>,
}

/// A sound read from a file, under the media name clips use.
pub type Sound = (String, PcmBuffer);

impl Links {
    /// Links to the actors; the sound channel is made here.
    #[must_use]
    pub fn new(live: LiveLink, ui: Sender<UiSet>) -> Self {
        let (sounds, sounds_in) = crossbeam_channel::unbounded();
        Self { live, ui, sounds, sounds_in }
    }
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
        PuppetOp::AddHandle { group, name, position, end, .. } => {
            let g = skeleton.need_group(group)?;
            text("handle name", name)?;
            finite("position", position)?;
            end.as_ref().map_or(Ok(()), |e| finite("end", e))?;
            Ok(Some(format!("{}/{name}", g.label)))
        }
        PuppetOp::MoveHandle { group, key, position, end } => {
            let g = skeleton.need_group(group)?;
            finite("position", position)?;
            end.as_ref().map_or(Ok(()), |e| finite("end", e))?;
            if !g.handles.contains(key) {
                return Err(CommandError::invalid_params(format!("{group:?} has no handle {key:?} (it has {:?})", g.handles)));
            }
            Ok(None)
        }
        PuppetOp::SetIndependent { layer, .. } => {
            if skeleton.need(layer)?.path.is_empty() {
                return Err(CommandError::invalid_params("the puppet itself is always an independent group"));
            }
            Ok(None)
        }
        PuppetOp::SetAttachStyle { group, .. } | PuppetOp::SetWarpIndependent { group, .. } => {
            skeleton.need_group(group)?;
            Ok(None)
        }
        PuppetOp::SetMeshDensity { group, density } => {
            skeleton.need_group(group)?;
            if !density.is_finite() || *density <= 0.0 {
                return Err(CommandError::invalid_params("density must be a positive number"));
            }
            Ok(None)
        }
        PuppetOp::SetCycle { layer, cycle } => {
            let l = skeleton.need(layer)?;
            if cycle.is_some() && l.children < 2 {
                return Err(CommandError::invalid_params(format!("{layer:?} needs at least two layers inside to cycle")));
            }
            Ok(None)
        }
        PuppetOp::RemoveSwapSet { name } => {
            if !skeleton.swap_sets.contains(name) {
                return Err(CommandError::invalid_params(format!("no swap set {name:?} (there are {:?})", skeleton.swap_sets)));
            }
            Ok(None)
        }
        PuppetOp::RenameSwapSet { name, new_name } => {
            text("swap set name", new_name)?;
            if !skeleton.swap_sets.contains(name) {
                return Err(CommandError::invalid_params(format!("no swap set {name:?}")));
            }
            if skeleton.swap_sets.contains(new_name) {
                return Err(CommandError::invalid_params(format!("a swap set {new_name:?} exists already")));
            }
            Ok(None)
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
        PuppetOp::SetVisible { layer, .. } | PuppetOp::SetHideOthers { layer, .. } => {
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
    let skeleton = doc
        .rig
        .skeletons
        .get(&key)
        .ok_or_else(|| CommandError::invalid_params(format!("no puppet {key:?} (cast keys: {:?})", doc.rig.skeletons.keys().collect::<Vec<_>>())))?;
    let handle_key = check_edit(skeleton, &op)?;
    if doc.rig.edits.len() >= MAX_RIG_EDITS {
        return Err(CommandError::failed("too many rig edits"));
    }
    if let Some(skeleton) = doc.rig.skeletons.get_mut(&key) {
        skeleton.follow(&op);
    }
    // A drag is many moves of the same thing: the last one is the one worth keeping.
    let repeats = |last: &RigEdit| {
        last.puppet == key
            && match (&last.op, &op) {
                (PuppetOp::MoveHandle { group: g, key: k, .. }, PuppetOp::MoveHandle { group, key: handle, .. }) => g == group && k == handle,
                (PuppetOp::SetOrigin { group: g, .. }, PuppetOp::SetOrigin { group, .. }) => g == group,
                _ => false,
            }
    };
    if doc.rig.edits.last().is_some_and(repeats) {
        doc.rig.edits.pop();
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
        "layers": skeleton.layers.iter().map(|l| json!({ "path": l.path, "label": l.label, "independent": l.independent, "children": l.children, "handles": l.handles })).collect::<Vec<_>>(),
        "swap_sets": skeleton.swap_sets,
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

// --- lip sync from a sound file ----------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LipsyncRecord {
    track: usize,
    path: String,
    start_seconds: f64,
    add_audio: bool,
    audio_track: usize,
    media: Option<String>,
}

impl Default for LipsyncRecord {
    fn default() -> Self {
        Self { track: 0, path: String::new(), start_seconds: 0.0, add_audio: true, audio_track: 0, media: None }
    }
}

/// A media name for a file: its stem reduced to what `valid_media_name` allows.
fn media_name_for(path: &str) -> String {
    let stem = std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("sound");
    let cleaned: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-') { c } else { '_' }).take(64).collect();
    if charcraft_record::valid_media_name(&cleaned) { cleaned } else { "sound".to_owned() }
}

/// Reads a sound file, finds the mouth shapes in it, records them as a voice take on a puppet track and (by default)
/// puts the sound on an audio track, all as one undo step.
fn lipsync_record(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let p: LipsyncRecord = serde_json::from_value(params.clone()).map_err(|e| CommandError::invalid_params(e.to_string()))?;
    if p.path.is_empty() {
        return Err(CommandError::invalid_params("`path` is required (a WAV, MP3, FLAC, Ogg or AAC file)"));
    }
    if !p.start_seconds.is_finite() || p.start_seconds < 0.0 {
        return Err(CommandError::invalid_params("start_seconds must be zero or more"));
    }
    doc.rec.check_track(p.track).map_err(|e| CommandError::failed(e.to_string()))?;
    let audio = anim_audio::decode_path(std::path::Path::new(&p.path)).map_err(|e| CommandError::failed(format!("{}: {e}", p.path)))?;
    let keys = anim_lipsync::analyze(&audio.to_mono().samples, audio.sample_rate);
    let fps = doc.rec.scene.fps_f64().max(1.0);
    let length = audio.seconds();
    let frames = anim_core::num::round_f64_to_u32((length * fps).ceil()).saturating_add(1);
    if usize::try_from(frames).map_or(true, |n| n > charcraft_record::take::MAX_SAMPLES) {
        return Err(CommandError::failed(format!("the sound is {length:.0} s long, too long to record as one take")));
    }
    let offset = Tick::from_seconds_f64(p.start_seconds);
    let inputs: Vec<Inputs> = (0..frames)
        .map(|f| {
            let within = Tick::from_seconds_f64(f64::from(f) / fps);
            Inputs::at(Tick(offset.0.saturating_add(within.0))).with_viseme(anim_lipsync::viseme_at(&keys, within))
        })
        .collect();
    let mut recorder = Recorder::new();
    recorder.arm_only([ParamGroup::Voice]);
    let recording = recorder.record_all(&inputs, None).map_err(|e| CommandError::failed(e.to_string()))?;
    let take = recording.take.ok_or_else(|| CommandError::failed("the sound produced no mouth data"))?;
    let index = doc.rec.scene.add_take(p.track, take).map_err(|e| CommandError::failed(e.to_string()))?;
    let mut result = serde_json::Map::new();
    result.insert("track".into(), json!(p.track));
    result.insert("take".into(), json!(index));
    result.insert("seconds".into(), json!(length));
    result.insert("keys".into(), json!(keys.len()));
    result.insert(
        "first_shapes".into(),
        keys.iter().take(12).map(|k| json!({ "time_seconds": k.start.seconds() + p.start_seconds, "viseme": k.viseme.ca_layer_name() })).collect(),
    );
    if p.add_audio {
        let name = p.media.unwrap_or_else(|| media_name_for(&p.path));
        if !charcraft_record::valid_media_name(&name) {
            return Err(CommandError::invalid_params(format!("{name:?} is not a usable media name (letters, digits, space, . _ -)")));
        }
        let clip = AudioClip::new(&name, &name, offset);
        let clip_index = doc.rec.scene.add_clip(p.audio_track, clip).map_err(|e| CommandError::failed(e.to_string()))?;
        doc.links.sounds.send((name.clone(), audio)).map_err(|_| CommandError::failed("the media store is gone"))?;
        result.insert("media".into(), json!(name));
        result.insert("audio_track".into(), json!(p.audio_track));
        result.insert("clip".into(), json!(clip_index));
    }
    Ok(Value::Object(result))
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
        CommandSpec::new("lipsync.record", "Lip sync a sound file", lipsync_record)
            .describe("Read a sound file (WAV, MP3, FLAC, Ogg, AAC), find the mouth shapes in it the way Lip Sync does, record them as a new voice take on puppet track 'track' starting at start_seconds, and place the sound on audio track 'audio_track' (add_audio false skips the sound; 'media' names it). One undo step. Render or export the scene to see and hear the result.")
            .with_schema(r#"{"type":"object","required":["path"],"properties":{"path":{"type":"string","description":"The sound file."},"track":{"type":"integer","minimum":0,"description":"Puppet track (default 0)."},"start_seconds":{"type":"number","minimum":0},"add_audio":{"type":"boolean","description":"Also place the sound on an audio track (default true)."},"audio_track":{"type":"integer","minimum":0},"media":{"type":"string","description":"Media name for the sound (default: the file name)."}},"additionalProperties":false}"#),
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
    all.push(
        CommandSpec::new("tracking.list_cameras", "List cameras", tracking_cameras)
            .transient()
            .describe("The cameras the system reports.")
            .with_schema(NO_PARAMS),
    );
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
        AppDoc::new(RecordDoc::new(scene), rig, Links::new(LiveLink::disconnected(), ui))
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
        let done = session
            .execute("rig.edit", &json!({ "op": "add_handle", "group": "Head", "name": "Chin", "kind": "Draggable", "position": [320.0, 400.0] }))
            .unwrap();
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
    fn rig_tool_edits_move_handles_and_define_swap_sets() {
        let mut session = Session::new(doc(), registry().unwrap());
        let run = |s: &mut Session<AppDoc>, op: Value| s.execute("rig.edit", &op);
        run(&mut session, json!({ "op": "add_handle", "group": "Head", "name": "Chin", "kind": "Draggable", "position": [320.0, 400.0] })).unwrap();
        for x in [330.0, 340.0, 350.0] {
            run(&mut session, json!({ "op": "move_handle", "group": "Head", "key": "Head/Chin", "position": [x, 400.0] })).unwrap();
        }
        assert_eq!(session.doc().rig.edits.len(), 2, "a drag keeps only its last move");
        assert!(run(&mut session, json!({ "op": "move_handle", "group": "Head", "key": "Head/Nope", "position": [1.0, 1.0] })).is_err());
        run(&mut session, json!({ "op": "set_attach_style", "group": "Head", "style": "hinge" })).unwrap();
        run(&mut session, json!({ "op": "set_warp_independent", "group": "Head", "on": true })).unwrap();
        run(&mut session, json!({ "op": "set_mesh_density", "group": "Head", "density": 2.0 })).unwrap();
        assert!(run(&mut session, json!({ "op": "set_mesh_density", "group": "Head", "density": 0.0 })).is_err());
        run(&mut session, json!({ "op": "set_independent", "layer": "Head/Nose", "independent": true })).unwrap();
        run(&mut session, json!({ "op": "set_key_trigger", "layer": "Head/Fringe", "key": "f" })).unwrap();
        run(&mut session, json!({ "op": "set_hide_others", "layer": "Head/Fringe", "on": true })).unwrap();
        run(&mut session, json!({ "op": "add_swap_set", "name": "Hair", "members": ["Head/Fringe", "Head/Hair back"] })).unwrap();
        run(&mut session, json!({ "op": "rename_swap_set", "name": "Hair", "new_name": "Do" })).unwrap();
        assert!(run(&mut session, json!({ "op": "remove_swap_set", "name": "Hair" })).is_err(), "renamed away");
        run(&mut session, json!({ "op": "remove_swap_set", "name": "Do" })).unwrap();
        let layers = session.execute("rig.layers", &json!({})).unwrap();
        assert_eq!(layers["swap_sets"], json!([]));
    }

    #[test]
    fn live_commands_fail_cleanly_without_an_actor() {
        let mut session = Session::new(doc(), registry().unwrap());
        assert!(session.execute("tracking.status", &json!({})).is_err());
        assert!(session.execute("mic.status", &json!({})).is_err());
        assert!(session.execute("ui.set", &json!({ "mode": "stream" })).is_err(), "nobody listens");
    }
}
