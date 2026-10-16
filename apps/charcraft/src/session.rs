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

use std::collections::BTreeMap;
use std::path::PathBuf;

use anim_audio::PcmBuffer;
use anim_core::Tick;
use anim_session::command::RunFn;
use anim_session::{CommandError, CommandRegistry, CommandSpec, RegistryError, SizeHint, param_opt};
use charcraft_behaviors::Inputs;
use charcraft_maker::Spec;
use charcraft_puppet::{Puppet, PuppetOp};
use charcraft_record::RecordDoc;
use charcraft_record::{AudioClip, AudioTrack, Cast, CastMember, ParamGroup, Recorder};
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use charcraft_stream::{Background, Kind as OutputKind, OutputLink, Spec as OutputSpec};

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
    pub skeletons: BTreeMap<String, Skeleton>,
    /// The edits, oldest first.
    pub edits: Vec<RigEdit>,
    /// The file the artwork came from (what a re-import reads and Edit Original opens).
    pub source: Option<PathBuf>,
    /// How many times the artwork was re-imported. The world keeps the puppets of each version, so undoing a
    /// re-import brings the old artwork back.
    pub revision: u32,
    /// The puppets made with the Puppet Maker, by cast key.
    pub made: BTreeMap<String, Spec>,
}

/// Names given to puppets in the project, and the puppets that came from other files. Part of the undoable document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectMeta {
    /// Display names by cast key (a puppet without an entry shows its key's last segment).
    pub names: std::collections::BTreeMap<String, String>,
    /// Puppets imported from `.puppet` or project files, so saving a project can copy them in.
    pub imports: Vec<ImportedPuppet>,
}

impl ProjectMeta {
    /// The name a puppet shows: the one it was given, or the last segment of its key.
    #[must_use]
    pub fn display_name(&self, key: &str) -> String {
        self.names.get(key).cloned().unwrap_or_else(|| key.rsplit('/').next().unwrap_or(key).to_owned())
    }
}

/// A puppet imported from another file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedPuppet {
    /// The cast key.
    pub key: String,
    /// The `.puppet` or project it was read from.
    pub path: String,
}

/// Which file a project save writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveFormat {
    /// A Character Animator project folder (with the scene and takes beside it).
    Chproj,
    /// A `.puppet` file of one puppet.
    Puppet,
}

/// Work that needs the puppets, which commands cannot reach: the hooks do it on their next tick.
#[derive(Debug)]
pub enum WorldJob {
    /// Put a puppet in the cast.
    AddPuppet {
        /// Its cast key.
        key: String,
        /// The puppet.
        member: Box<charcraft_record::CastMember>,
    },
    /// Copy a cast member under a new key.
    ClonePuppet {
        /// The key to copy.
        from: String,
        /// The key of the copy.
        to: String,
    },
    /// Write the project (`Chproj`) or one puppet (`Puppet`, named by `puppet`).
    Save {
        /// What to write.
        format: SaveFormat,
        /// Where to.
        path: std::path::PathBuf,
        /// The puppet for [`SaveFormat::Puppet`].
        puppet: Option<String>,
    },
    /// Read the document again from the file it was saved to or opened from.
    Revert,
    /// Start an empty project.
    NewProject {
        /// Its name.
        name: String,
    },
}

/// What the document commands can reach beyond the document: the live input actor and the window.
#[derive(Debug, Clone)]
pub struct Links {
    /// The camera and microphone actor.
    pub live: LiveLink,
    /// Work for the hooks (files and puppets).
    pub jobs: Sender<WorldJob>,
    /// The other end of [`Self::jobs`], drained by the hooks.
    pub jobs_in: Receiver<WorldJob>,
    /// Requests to change the window's view.
    pub ui: Sender<UiSet>,
    /// Sounds commands have read from files: the hooks move them into the media store (which is not part of the
    /// undoable document) on their next tick.
    pub sounds: Sender<Sound>,
    /// The other end of [`Self::sounds`], drained by the hooks.
    pub sounds_in: Receiver<Sound>,
    /// The stream output thread (NDI, virtual camera, web source).
    pub output: OutputLink,
    /// Puppets commands have built: the hooks move them into the world's cast (which is not part of the undoable
    /// document either).
    pub puppets: Sender<Delivery>,
    /// The other end of [`Self::puppets`], drained by the hooks.
    pub puppets_in: Receiver<Delivery>,
    /// Files to open with the system's default application (Edit Original), opened by the hooks.
    pub opens: Sender<PathBuf>,
    /// The other end of [`Self::opens`], drained by the hooks.
    pub opens_in: Receiver<PathBuf>,
}

/// A sound read from a file, under the media name clips use.
pub type Sound = (String, PcmBuffer);

/// Puppets a command built, on their way to the world.
#[derive(Debug, Clone)]
pub enum Delivery {
    /// The artwork was read again.
    Reimport {
        /// The revision this replaces puppets of.
        from: u32,
        /// The new revision.
        revision: u32,
        /// The puppets read from the file, under their cast keys.
        cast: Cast,
        /// What the reading reported.
        warnings: Vec<String>,
    },
    /// A puppet from the Puppet Maker.
    Made {
        /// Its cast key.
        key: String,
        /// The puppet with its behaviors.
        member: Box<CastMember>,
    },
}

impl Links {
    /// Links to the actors; the channels are made here.
    #[must_use]
    pub fn new(live: LiveLink, ui: Sender<UiSet>) -> Self {
        let (sounds, sounds_in) = crossbeam_channel::unbounded();
        let (jobs, jobs_in) = crossbeam_channel::unbounded();
        let (puppets, puppets_in) = crossbeam_channel::unbounded();
        let (opens, opens_in) = crossbeam_channel::unbounded();
        Self { live, jobs, jobs_in, ui, sounds, sounds_in, output: OutputLink::disconnected(), puppets, puppets_in, opens, opens_in }
    }

    /// These links with the stream output thread attached.
    #[must_use]
    pub fn with_output(mut self, output: OutputLink) -> Self {
        self.output = output;
        self
    }
}

/// The session document.
#[derive(Debug, Clone)]
pub struct AppDoc {
    /// Scene, takes and recording settings.
    pub rec: RecordDoc,
    /// Rig edits.
    pub rig: RigLog,
    /// Puppet names and imports.
    pub meta: ProjectMeta,
    /// Channels to the actors (not part of the document's content; they survive undo).
    pub links: Links,
}

impl AppDoc {
    /// A document around a scene.
    #[must_use]
    pub fn new(rec: RecordDoc, rig: RigLog, links: Links) -> Self {
        Self { rec, rig, meta: ProjectMeta::default(), links }
    }

    /// The same document with puppet names and imports.
    #[must_use]
    pub fn with_meta(mut self, meta: ProjectMeta) -> Self {
        self.meta = meta;
        self
    }
}

impl SizeHint for AppDoc {
    fn size_hint(&self) -> usize {
        let names: usize = self.rig.skeletons.values().map(|s| s.layers.len().saturating_mul(96)).sum();
        let meta = self.meta.names.len().saturating_add(self.meta.imports.len()).saturating_mul(128);
        self.rec.size_hint().saturating_add(names).saturating_add(meta).saturating_add(self.rig.edits.len().saturating_mul(160))
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
tracking_command!(tracking_cameras, "tracking_list_cameras");
tracking_command!(tracking_rest_pose, "tracking_set_rest_pose");
tracking_command!(tracking_preview, "tracking_preview");

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

fn mic_devices(doc: &mut AppDoc, _: &Value) -> Result<Value, CommandError> {
    mic_command(doc, MicAction::Devices)
}

fn mic_set_device(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let device: Option<String> = param_opt(params, "device")?;
    mic_command(doc, MicAction::SetDevice(device.filter(|d| !d.trim().is_empty())))
}

fn mic_set_gain(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let gain: f32 = param_opt(params, "gain")?.ok_or_else(|| CommandError::invalid_params("gain is required (0.1 to 8; 1 is unchanged)"))?;
    if !gain.is_finite() {
        return Err(CommandError::invalid_params("gain must be a finite number"));
    }
    mic_command(doc, MicAction::SetGain(gain))
}

// --- stream output ---------------------------------------------------------------------------------------------

fn output_failed(e: &charcraft_stream::Error) -> CommandError {
    CommandError::failed(e.to_string())
}

/// A background named in words or as `#rrggbb`.
#[must_use]
pub fn parse_background(text: &str) -> Option<Background> {
    let text = text.trim().to_ascii_lowercase();
    match text.as_str() {
        "transparent" | "none" | "alpha" => Some(Background::Transparent),
        "green" | "chroma" | "greenscreen" => Some(Background::Colour(charcraft_stream::KEY_GREEN)),
        "black" => Some(Background::Colour([0, 0, 0])),
        "white" => Some(Background::Colour([255, 255, 255])),
        hex => {
            let digits = hex.strip_prefix('#')?;
            let value = u32::from_str_radix(digits, 16).ok().filter(|_| digits.len() == 6)?;
            let [_, r, g, b] = value.to_be_bytes();
            Some(Background::Colour([r, g, b]))
        }
    }
}

#[expect(clippy::unnecessary_wraps, reason = "a command runner returns a Result")]
fn stream_outputs(_: &mut AppDoc, _: &Value) -> Result<Value, CommandError> {
    Ok(json!({ "outputs": charcraft_stream::outputs() }))
}

fn stream_status(doc: &mut AppDoc, _: &Value) -> Result<Value, CommandError> {
    let status = doc.links.output.status().map_err(|e| output_failed(&e))?;
    serde_json::to_value(status).map_err(|e| CommandError::failed(e.to_string()))
}

fn stream_stop(doc: &mut AppDoc, _: &Value) -> Result<Value, CommandError> {
    let status = doc.links.output.stop().map_err(|e| output_failed(&e))?;
    serde_json::to_value(status).map_err(|e| CommandError::failed(e.to_string()))
}

fn stream_start(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let mut spec = OutputSpec::default();
    if let Some(kind) = param_opt::<String>(params, "kind")? {
        spec.kind = OutputKind::parse(&kind)
            .ok_or_else(|| CommandError::invalid_params(format!("unknown output {kind:?} (ndi, virtual_camera, web, syphon, spout)")))?;
    }
    if let Some(name) = param_opt::<String>(params, "name")? {
        spec.name = name;
    }
    spec.width = param_opt(params, "width")?.unwrap_or(spec.width);
    spec.height = param_opt(params, "height")?.unwrap_or(spec.height);
    spec.fps = param_opt(params, "fps")?.unwrap_or(spec.fps);
    spec.port = param_opt(params, "port")?.unwrap_or(spec.port);
    spec.device = param_opt(params, "device")?;
    if let Some(background) = param_opt::<String>(params, "background")? {
        spec.background = parse_background(&background)
            .ok_or_else(|| CommandError::invalid_params(format!("unknown background {background:?} (transparent, green, black, white or #rrggbb)")))?;
    }
    spec.check().map_err(|e| CommandError::invalid_params(e.to_string()))?;
    let status = doc.links.output.start(&spec).map_err(|e| output_failed(&e))?;
    serde_json::to_value(status).map_err(|e| CommandError::failed(e.to_string()))
}

fn ui_set(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let set: UiSet = serde_json::from_value(params.clone()).map_err(|e| CommandError::invalid_params(e.to_string()))?;
    if set.seek_seconds.is_some_and(|s| !s.is_finite()) {
        return Err(CommandError::invalid_params("seek_seconds must be a finite number"));
    }
    doc.links.ui.send(set.clone()).map_err(|_| CommandError::failed("there is no window to change"))?;
    Ok(json!({ "requested": set }))
}

// --- import a sound file onto an audio track ---------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AudioImport {
    path: String,
    track: usize,
    start_seconds: f64,
    media: Option<String>,
}

impl Default for AudioImport {
    fn default() -> Self {
        Self { path: String::new(), track: 0, start_seconds: 0.0, media: None }
    }
}

/// Reads a sound file and places it as a clip on an audio track (the track is added when the scene has none).
fn audio_import(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let p: AudioImport = serde_json::from_value(params.clone()).map_err(|e| CommandError::invalid_params(e.to_string()))?;
    if p.path.is_empty() {
        return Err(CommandError::invalid_params("`path` is required (a WAV, MP3, FLAC, Ogg or AAC file)"));
    }
    if !p.start_seconds.is_finite() || p.start_seconds < 0.0 {
        return Err(CommandError::invalid_params("start_seconds must be zero or more"));
    }
    let audio = anim_audio::decode_path(std::path::Path::new(&p.path)).map_err(|e| CommandError::failed(format!("{}: {e}", p.path)))?;
    let name = p.media.unwrap_or_else(|| media_name_for(&p.path));
    if !charcraft_record::valid_media_name(&name) {
        return Err(CommandError::invalid_params(format!("{name:?} is not a usable media name (letters, digits, space, . _ -)")));
    }
    if doc.rec.scene.audio.is_empty() {
        doc.rec.scene.audio.push(AudioTrack::new("Audio 1"));
    }
    let seconds = audio.seconds();
    let clip = AudioClip::new(&name, &name, Tick::from_seconds_f64(p.start_seconds));
    let index = doc.rec.scene.add_clip(p.track, clip).map_err(|e| CommandError::failed(e.to_string()))?;
    doc.links.sounds.send((name.clone(), audio)).map_err(|_| CommandError::failed("the media store is gone"))?;
    Ok(json!({ "media": name, "track": p.track, "clip": index, "seconds": seconds }))
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
            Inputs::at(Tick(offset.0.saturating_add(within.0))).with_scene_viseme(anim_lipsync::viseme_at(&keys, within))
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
    vec![
        CommandSpec::new("rig.edit", "Edit puppet rig", rig_edit)
            .describe("Edit a puppet's rigging. Pick the edit with 'op': add_tag/remove_tag (layer, tag), add_handle (group, name, kind Fixed|Draggable|Dangle|Attach|Stick|Walk, position [x,y] in artwork pixels; the answer names the handle key), remove_handle (group, key), set_origin (group, origin), set_key_trigger (layer, key or null), add_swap_set (name, members), set_visible (layer, visible). Layers are paths (Head/Left Eye) or labels; use rig_layers to list them. 'puppet' picks the puppet (default: the first).")
            .with_schema(r#"{"type":"object","required":["op"],"properties":{"puppet":{"type":"string"},"op":{"enum":["add_tag","remove_tag","add_handle","remove_handle","set_origin","set_key_trigger","add_swap_set","set_visible"]},"layer":{"type":"string"},"tag":{"type":"string"},"group":{"type":"string"},"name":{"type":"string"},"kind":{"enum":["Fixed","Draggable","Dangle","Attach","Stick","Walk"]},"position":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"origin":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"key":{"type":["string","null"]},"members":{"type":"array","items":{"type":"string"},"minItems":2},"visible":{"type":"boolean"}}}"#),
        CommandSpec::new("rig.layers", "List puppet layers", rig_layers)
            .transient()
            .describe("The layers of a puppet (path, label, independent group) and the cast keys of the loaded puppets.")
            .with_schema(r#"{"type":"object","properties":{"puppet":{"type":"string"}}}"#),
        CommandSpec::new("audio.import", "Import a sound file", audio_import)
            .describe("Read a sound file (WAV, MP3, FLAC, Ogg, AAC) and place it as a clip on audio track 'track' (default 0; an audio track is added when the scene has none) starting at start_seconds. 'media' names the sound (default: the file name). Move, trim and split the clip with audio.move_clip, audio.set_clip and audio.split_clip, and make a lip sync take from it with lipsync.from_clip.")
            .with_schema(r#"{"type":"object","required":["path"],"properties":{"path":{"type":"string"},"track":{"type":"integer","minimum":0},"start_seconds":{"type":"number","minimum":0},"media":{"type":"string"}},"additionalProperties":false}"#),
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
        CommandSpec::new("tracking.set_rest_pose", "Set rest pose", tracking_rest_pose)
            .transient()
            .describe("Capture the rest pose while the performer holds still and looks at the camera: target 'face' sets the neutral expressions, 'body' the neutral posture (head position, distance from the camera, turn and tilt: the part of the body a webcam sees), 'both' (default) everything.")
            .with_schema(r#"{"type":"object","properties":{"target":{"enum":["face","body","both"]},"frames":{"type":"integer","minimum":1,"maximum":600}}}"#),
        CommandSpec::new("tracking.list_cameras", "List cameras", tracking_cameras)
            .transient()
            .describe("The cameras the system reports, with the index to pass to tracking.start. Without the 'live' build feature the list holds one scripted demo face.")
            .with_schema(NO_PARAMS),
        CommandSpec::new("tracking.preview", "Camera preview status", tracking_preview)
            .transient()
            .describe("Whether the live camera preview is flowing, the size of its image and the number of tracked landmark points.")
            .with_schema(NO_PARAMS),
        CommandSpec::new("tracking.set_sensitivity", "Tracking sensitivity", tracking_sensitivity)
            .transient()
            .describe("Change how far each measurement must move to reach the end of its range (brow, eye_shut, eye_open, mouth_open, mouth_wide, smile, gaze).")
            .with_schema(r#"{"type":"object","properties":{"brow":{"type":"number"},"eye_shut":{"type":"number"},"eye_open":{"type":"number"},"mouth_open":{"type":"number"},"mouth_wide":{"type":"number"},"smile":{"type":"number"},"gaze":{"type":"number"}}}"#),
        CommandSpec::new("tracking.set_smoothing", "Tracking smoothing", tracking_smoothing)
            .transient()
            .describe("Switch smoothing on or off and tune the One-Euro filters (head, brows, eyes, gaze, mouth: min_cutoff, beta).")
            .with_schema(r#"{"type":"object","properties":{"enabled":{"type":"boolean"},"head":{"type":"object"},"brows":{"type":"object"},"eyes":{"type":"object"},"gaze":{"type":"object"},"mouth":{"type":"object"}}}"#),
        CommandSpec::new("mic.start", "Start microphone", mic_start).transient().describe("Open the selected microphone (the system default unless mic.set_device chose one); live lip sync drives the mouth.").with_schema(NO_PARAMS),
        CommandSpec::new("mic.devices", "List microphones", mic_devices).transient().describe("The input devices the system reports and the one selected.").with_schema(NO_PARAMS),
        CommandSpec::new("mic.set_device", "Choose microphone", mic_set_device)
            .transient()
            .describe("Choose the microphone by name (see mic.devices); omit 'device' for the system default. A running microphone restarts on the new device.")
            .with_schema(r#"{"type":"object","properties":{"device":{"type":"string"}}}"#),
        CommandSpec::new("mic.set_gain", "Microphone sensitivity", mic_set_gain)
            .transient()
            .describe("Set the microphone input sensitivity: a gain from 0.1 to 8 applied before the level meter and the lip sync (1 is unchanged). Raise it for a quiet voice.")
            .with_schema(r#"{"type":"object","required":["gain"],"properties":{"gain":{"type":"number","minimum":0.1,"maximum":8}}}"#),
        CommandSpec::new("stream.outputs", "List stream outputs", stream_outputs)
            .transient()
            .describe("What each stream output (ndi, virtual_camera, web, syphon, spout) needs on this machine: whether it can start now, whether it carries transparency and, if not available, what to install.")
            .with_schema(NO_PARAMS),
        CommandSpec::new("stream.start", "Start stream output", stream_start)
            .transient()
            .describe("Send the performing puppet out of the app. kind: ndi (a network source; needs the NDI runtime), virtual_camera (Linux v4l2loopback), web (a local MJPEG video and PNG-with-alpha source on 127.0.0.1) . background: transparent (alpha where the output carries it, otherwise a chroma green), green, black, white or #rrggbb. Replaces a running output; the answer says where to connect.")
            .with_schema(r#"{"type":"object","properties":{"kind":{"enum":["ndi","virtual_camera","web","syphon","spout"]},"name":{"type":"string"},"width":{"type":"integer","minimum":16,"maximum":8192},"height":{"type":"integer","minimum":16,"maximum":8192},"fps":{"type":"integer","minimum":1,"maximum":120},"background":{"type":"string"},"device":{"type":"string","description":"Virtual camera device, e.g. /dev/video10."},"port":{"type":"integer","minimum":0,"maximum":65535}}}"#),
        CommandSpec::new("stream.stop", "Stop stream output", stream_stop).transient().describe("Stop the running stream output.").with_schema(NO_PARAMS),
        CommandSpec::new("stream.status", "Stream output status", stream_status).transient().describe("Whether a stream output runs, where it goes, frames sent and why the last one stopped.").with_schema(NO_PARAMS),
        CommandSpec::new("mic.stop", "Stop microphone", mic_stop).transient().describe("Close the microphone.").with_schema(NO_PARAMS),
        CommandSpec::new("mic.status", "Microphone status", mic_status).transient().describe("Whether the microphone is on, its level and the mouth shape the voice makes.").with_schema(NO_PARAMS),
        CommandSpec::new("mic.demo_voice", "Demo voice", mic_demo)
            .transient()
            .describe("Play a synthetic talking mouth while no microphone is on (for demos and tests).")
            .with_schema(r#"{"type":"object","properties":{"on":{"type":"boolean"}}}"#),
        CommandSpec::new("ui.set", "Change the view", ui_set)
            .transient()
            .describe("Drive the window: mode (rig|record|stream), track (index), layer (path), seek_seconds, playing, mirror, dark, demo_voice; in the stream view triggers (keys held down, [] lets go), swaps (swap set to layer label), stream_panels, transparent and green_screen; dialog (warnings|maker|none: the import warnings list or the Puppet Maker). Only the given fields change.")
            .with_schema(r#"{"type":"object","properties":{"mode":{"enum":["rig","record","stream"]},"track":{"type":"integer","minimum":0},"layer":{"type":"string"},"seek_seconds":{"type":"number"},"playing":{"type":"boolean"},"mirror":{"type":"boolean"},"dark":{"type":"boolean"},"demo_voice":{"type":"boolean"},"triggers":{"type":"array","items":{"type":"string"}},"swaps":{"type":"object","additionalProperties":{"type":"string"}},"stream_panels":{"type":"boolean"},"transparent":{"type":"boolean"},"green_screen":{"type":"boolean"},"dialog":{"enum":["warnings","maker","none"]}}}"#),
    ]
}

/// Where each command appears in the menus: the app's own placement for the record crate's commands (which only
/// know their ids) and for its own.
fn place(spec: CommandSpec<AppDoc>) -> CommandSpec<AppDoc> {
    match spec.id {
        "scene.set" | "scene.import" | "track.add" | "track.remove" => spec.in_menu(&["Scene"]),
        "rig.edit" => spec.in_menu(&["Rig"]),
        "tracking.start" | "tracking.stop" | "mic.stop" | "tracking.set_rest_pose" => spec.in_menu(&["Input"]),
        "stream.start" | "stream.stop" => spec.in_menu(&["Stream"]),
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
    registry.register_all(
        wrapped().into_iter().chain(app_commands()).chain(crate::project_cmds::commands()).chain(crate::puppet_cmds::commands()).map(place),
    )?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use anim_core::{FrameRate, Size, Tick};
    use anim_session::Session;
    use charcraft_record::Scene;

    use super::*;

    fn doc() -> AppDoc {
        let (ui, _rx) = crossbeam_channel::unbounded();
        doc_with(Links::new(LiveLink::disconnected(), ui))
    }

    fn doc_with(links: Links) -> AppDoc {
        let puppet = crate::demo::puppet().unwrap();
        let mut rig = RigLog::default();
        rig.skeletons.insert("demo".into(), Skeleton::of(&puppet));
        let scene = Scene::new("s", Size::new(640.0, 360.0), FrameRate::FPS_30, Tick::from_seconds_f64(2.0));
        AppDoc::new(RecordDoc::new(scene), rig, links)
    }

    #[test]
    fn backgrounds_parse_from_words_and_hex() {
        assert_eq!(parse_background(" Transparent "), Some(Background::Transparent));
        assert_eq!(parse_background("green"), Some(Background::Colour(charcraft_stream::KEY_GREEN)));
        assert_eq!(parse_background("#102030"), Some(Background::Colour([0x10, 0x20, 0x30])));
        assert_eq!(parse_background("#1020"), None);
        assert_eq!(parse_background("#gg0000"), None);
        assert_eq!(parse_background("plaid"), None);
    }

    #[test]
    fn stream_commands_start_a_web_output_report_it_and_stop_it() {
        let output = charcraft_stream::OutputHandle::spawn();
        let (ui, _rx) = crossbeam_channel::unbounded();
        let links = Links::new(LiveLink::disconnected(), ui).with_output(output.link());
        let mut session = Session::new(doc_with(links), registry().unwrap());
        let listed = session.execute("stream.outputs", &json!({})).unwrap();
        assert_eq!(listed["outputs"].as_array().unwrap().len(), 5);
        let started = session.execute("stream.start", &json!({ "kind": "web", "width": 64, "height": 32, "fps": 30, "background": "#102030" })).unwrap();
        assert_eq!(started["active"], true);
        assert!(started["detail"].as_str().unwrap().starts_with("http://127.0.0.1:"));
        assert_eq!(session.execute("stream.status", &json!({})).unwrap()["kind"], "web");
        assert!(session.execute("stream.start", &json!({ "kind": "hdmi" })).is_err(), "unknown output");
        assert!(session.execute("stream.start", &json!({ "background": "plaid" })).is_err());
        assert!(session.execute("stream.start", &json!({ "width": 7 })).is_err(), "bad size");
        let refused = session.execute("stream.start", &json!({ "kind": "spout" })).unwrap_err().to_string();
        assert!(refused.contains("Spout") && refused.contains("NDI"), "{refused}");
        assert_eq!(session.execute("stream.stop", &json!({})).unwrap()["active"], false);
        assert!(!session.history().can_undo(), "stream commands are not undoable");
    }

    #[test]
    fn the_input_commands_are_registered_and_validate_their_parameters() {
        let registry = registry().unwrap();
        for id in [
            "tracking.set_rest_pose",
            "tracking.list_cameras",
            "tracking.preview",
            "mic.devices",
            "mic.set_device",
            "mic.set_gain",
            "stream.start",
            "stream.stop",
        ] {
            assert!(registry.get(id).is_some(), "{id}");
        }
        let mut session = Session::new(doc(), registry);
        assert!(session.execute("mic.set_gain", &json!({})).is_err(), "gain is required");
        assert!(session.execute("mic.set_gain", &json!({ "gain": "loud" })).is_err());
        assert!(session.execute("tracking.set_rest_pose", &json!({ "target": "face" })).is_err(), "no actor in this document");
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
