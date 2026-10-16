//! The tool surface: every edit and query of a [`RecordDoc`] as a named command.
//!
//! The commands are `anim-session` [`CommandSpec`]s, so one definition serves the menus, the shortcuts and the
//! MCP server (`anim_session::McpServer` turns each registered command into a tool, names `record_arm`,
//! `take_set` and so on, with the JSON Schema given here). Edits are undoable steps; queries are transient.
//! Times in parameters are seconds; stored times are ticks.
//!
//! | area | commands |
//! |---|---|
//! | scene | `scene.info`, `scene.set`, `scene.export`, `scene.import` |
//! | puppet tracks | `track.add`, `track.remove`, `track.set`, `track.behavior` |
//! | recording | `record.arm`, `record.punch`, `record.state`, `record.script`, `record.inputs` |
//! | takes | `take.list`, `take.set`, `take.remove`, `take.move` |
//! | sound | `audio.add_clip`, `audio.set_clip`, `audio.remove_clip`, `audio.set_track` |
//! | playback | `playback.inputs`, `playback.poses` |

use anim_core::{FrameRate, Rgba8, Size, Tick, TimeRange};
use anim_session::{CommandError, CommandRegistry, CommandSpec, RegistryError, parse_params};
use charcraft_behaviors::api::StackOp;
use charcraft_behaviors::{Inputs, Script};
use charcraft_puppet::PuppetPose;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::doc::RecordDoc;
use crate::group::ParamGroup;
use crate::playback::track_inputs;
use crate::scene::{AudioClip, MAX_OPS, Placement, PuppetTrack, Scene};
use crate::take::Take;

/// Most poses one `playback.poses` call returns.
pub const MAX_POSES: usize = 600;

pub(crate) fn fail(e: impl std::fmt::Display) -> CommandError {
    CommandError::failed(e.to_string())
}

pub(crate) fn seconds(name: &str, value: f64) -> Result<Tick, CommandError> {
    if !value.is_finite() || value.abs() > 1.0e6 {
        return Err(CommandError::invalid_params(format!("{name}: {value} is not a usable time in seconds")));
    }
    Ok(Tick::from_seconds_f64(value))
}

pub(crate) fn secs_opt(name: &str, value: Option<f64>) -> Result<Option<Tick>, CommandError> {
    value.map(|v| seconds(name, v)).transpose()
}

pub(crate) fn params<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, CommandError> {
    parse_params(value)
}

pub(crate) fn to_seconds(t: Tick) -> f64 {
    t.seconds()
}

/// Puppet track `index` for an edit: refuses a locked track.
pub(crate) fn unlocked(doc: &mut RecordDoc, index: usize) -> Result<&mut PuppetTrack, CommandError> {
    let track = doc.scene.track_mut(index).map_err(fail)?;
    if track.locked {
        return Err(CommandError::failed(format!("track {index} ({}) is locked; unlock it with track.set locked=false", track.name)));
    }
    Ok(track)
}

// --- scene ---------------------------------------------------------------------------------------------------

pub(crate) fn take_info(index: usize, take: &Take) -> Value {
    json!({
        "index": index,
        "number": take.number,
        "name": take.name,
        "groups": take.groups,
        "enabled": take.enabled,
        "start_seconds": to_seconds(take.start),
        "duration_seconds": to_seconds(take.duration),
        "trim_in_seconds": to_seconds(take.offset),
        "blend_in_seconds": to_seconds(take.blend_in),
        "blend_out_seconds": to_seconds(take.blend_out),
        "samples": take.samples.len(),
    })
}

fn scene_summary(doc: &RecordDoc) -> Value {
    let scene = &doc.scene;
    json!({
        "name": scene.name,
        "fps": scene.fps_f64(),
        "fps_exact": [scene.fps.num, scene.fps.den],
        "frames": scene.frame_count(),
        "duration_seconds": to_seconds(scene.duration),
        "size": [scene.size.width, scene.size.height],
        "background": scene.background,
        "seed": scene.seed,
        "scene_index": doc.active_scene(),
        "scenes": doc.scenes().map(|s| s.name.clone()).collect::<Vec<_>>(),
        "camera": camera_json(&scene.camera),
        "tracks": scene.tracks.iter().enumerate().map(|(i, t)| json!({
            "index": i,
            "name": t.name,
            "puppet": t.puppet,
            "enabled": t.enabled,
            "locked": t.locked,
            "placement": t.placement,
            "keyframes": lanes_json(&t.lanes),
            "behavior_edits": t.behaviors.len(),
            "puppet_known": doc.templates.contains_key(&t.puppet),
            "takes": t.takes.iter().enumerate().map(|(k, take)| take_info(k, take)).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "audio": scene.audio.iter().enumerate().map(|(i, t)| json!({
            "index": i,
            "name": t.name,
            "enabled": t.enabled,
            "gain": t.gain,
            "clips": t.clips.iter().enumerate().map(|(k, c)| json!({
                "index": k,
                "name": c.name,
                "media": c.media,
                "start_seconds": to_seconds(c.start),
                "trim_in_seconds": to_seconds(c.trim_in),
                "duration_seconds": c.duration.map(to_seconds),
                "gain": c.gain,
                "pan": c.pan,
                "enabled": c.enabled,
                "mouth_shapes": c.visemes.len(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "recording": arming_json(doc),
    })
}

pub(crate) fn camera_json(camera: &crate::camera::Camera) -> Value {
    json!({
        "position": [camera.position.x, camera.position.y],
        "zoom": camera.zoom,
        "rotation": camera.rotation,
        "zoom_on_face": camera.zoom_on_face.map(|f| json!({ "track": f.track, "amount": f.amount, "zoom": f.zoom })),
        "keyframes": lanes_json(&camera.lanes),
    })
}

pub(crate) fn lanes_json(lanes: &[crate::keyframes::Lane]) -> Value {
    lanes
        .iter()
        .map(|l| {
            json!({
                "target": l.target,
                "keys": l.keys.iter().enumerate().map(|(i, k)| json!({
                    "index": i,
                    "time_seconds": to_seconds(k.t),
                    "value": k.value,
                    "ease": k.ease,
                })).collect::<Vec<_>>(),
            })
        })
        .collect()
}

fn arming_json(doc: &RecordDoc) -> Value {
    json!({
        "armed_groups": doc.arming.groups,
        "audio_armed": doc.arming.audio,
        "punch": doc.arming.punch.map(|p| json!({ "start_seconds": to_seconds(p.start), "end_seconds": to_seconds(p.end()) })),
    })
}

#[expect(clippy::unnecessary_wraps, reason = "must match the RunFn signature")]
fn scene_info(doc: &mut RecordDoc, _: &Value) -> Result<Value, CommandError> {
    Ok(scene_summary(doc))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneSet {
    name: Option<String>,
    fps_num: Option<i64>,
    fps_den: Option<i64>,
    width: Option<f64>,
    height: Option<f64>,
    duration_seconds: Option<f64>,
    /// `[r, g, b, a]` 0-255, or `null` for transparent (absent: unchanged).
    background: Option<[u8; 4]>,
    /// `true` makes the background transparent (and wins over `background`).
    transparent: Option<bool>,
    seed: Option<u64>,
}

fn scene_set(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: SceneSet = params(value)?;
    let mut scene = doc.scene.clone();
    if let Some(name) = p.name {
        scene.name = name;
    }
    if p.fps_num.is_some() || p.fps_den.is_some() {
        let num = p.fps_num.unwrap_or(scene.fps.num);
        let den = p.fps_den.unwrap_or(scene.fps.den);
        if num <= 0 || den <= 0 {
            return Err(CommandError::invalid_params("fps_num and fps_den must be positive"));
        }
        scene.fps = FrameRate::new(num, den);
    }
    if p.width.is_some() || p.height.is_some() {
        scene.size = Size::new(p.width.unwrap_or(scene.size.width), p.height.unwrap_or(scene.size.height));
    }
    if let Some(d) = secs_opt("duration_seconds", p.duration_seconds)? {
        scene.duration = d;
    }
    if let Some([r, g, b, a]) = p.background {
        scene.background = Some(Rgba8 { r, g, b, a });
    }
    if p.transparent == Some(true) {
        scene.background = None;
    }
    if let Some(seed) = p.seed {
        scene.seed = seed;
    }
    scene.sanitize().map_err(fail)?;
    doc.scene = scene;
    Ok(scene_summary(doc))
}

fn scene_export(doc: &mut RecordDoc, _: &Value) -> Result<Value, CommandError> {
    serde_json::to_value(crate::store::SceneFile::new(doc.scene.clone())).map_err(fail)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneImport {
    scene: Scene,
}

fn scene_import(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let mut p: SceneImport = params(value)?;
    p.scene.sanitize().map_err(fail)?;
    doc.scene = p.scene;
    Ok(scene_summary(doc))
}

// --- puppet tracks -------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackAdd {
    puppet: String,
    name: Option<String>,
    placement: Option<Placement>,
}

fn track_add(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: TrackAdd = params(value)?;
    let mut track = PuppetTrack::new(p.name.as_deref().unwrap_or(&p.puppet), &p.puppet);
    if let Some(placement) = p.placement {
        track.placement = placement;
    }
    let index = doc.scene.add_track(track).map_err(fail)?;
    Ok(json!({ "track": index }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackRef {
    track: usize,
}

fn track_remove(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: TrackRef = params(value)?;
    doc.scene.track(p.track).map_err(fail)?;
    let removed = doc.scene.tracks.remove(p.track);
    Ok(json!({ "removed": removed.name, "takes": removed.takes.len() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackSet {
    track: usize,
    name: Option<String>,
    puppet: Option<String>,
    enabled: Option<bool>,
    placement: Option<Placement>,
    locked: Option<bool>,
}

fn track_set(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: TrackSet = params(value)?;
    if p.locked == Some(false) {
        doc.scene.track_mut(p.track).map_err(fail)?.locked = false;
    }
    let track = unlocked(doc, p.track)?;
    if let Some(locked) = p.locked {
        track.locked = locked;
    }
    if let Some(name) = p.name {
        track.name = name;
    }
    if let Some(puppet) = p.puppet {
        track.puppet = puppet;
    }
    if let Some(enabled) = p.enabled {
        track.enabled = enabled;
    }
    if let Some(mut placement) = p.placement {
        placement.sanitize();
        track.placement = placement;
    }
    doc.scene.sanitize().map_err(fail)?;
    Ok(json!({ "track": p.track }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackBehavior {
    track: usize,
    /// One edit of the behavior stack (`behaviors.edit`'s operation), or `null` to clear the track's edits.
    op: Option<StackOp>,
}

fn track_behavior(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: TrackBehavior = params(value)?;
    let track = doc.scene.track_mut(p.track).map_err(fail)?;
    match p.op {
        Some(op) => {
            if track.behaviors.len() >= MAX_OPS {
                return Err(CommandError::failed("this track already has the most behavior edits"));
            }
            track.behaviors.push(op);
        }
        None => track.behaviors.clear(),
    }
    // Prove the edits still apply before keeping them.
    doc.evaluator().map_err(fail)?;
    Ok(json!({ "track": p.track, "edits": doc.scene.track(p.track).map_err(fail)?.behaviors.len() }))
}

// --- recording -----------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordArm {
    /// Groups to arm or disarm.
    #[serde(default)]
    groups: Vec<ParamGroup>,
    /// `true` (default) arms the listed groups, `false` disarms them.
    armed: Option<bool>,
    /// Arm exactly the listed groups and disarm all others.
    #[serde(default)]
    only: bool,
    /// Arms or disarms the microphone.
    audio: Option<bool>,
}

fn record_arm(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: RecordArm = params(value)?;
    if p.only {
        doc.arming.groups = p.groups.iter().copied().collect();
    } else if p.armed.unwrap_or(true) {
        doc.arming.groups.extend(p.groups.iter().copied());
    } else {
        for group in &p.groups {
            doc.arming.groups.remove(group);
        }
    }
    if let Some(audio) = p.audio {
        doc.arming.audio = audio;
    }
    Ok(arming_json(doc))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordPunch {
    start_seconds: f64,
    end_seconds: f64,
}

fn record_punch(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    if value.is_null() || value.as_object().is_some_and(serde_json::Map::is_empty) {
        doc.arming.punch = None;
        return Ok(arming_json(doc));
    }
    let p: RecordPunch = params(value)?;
    let (start, end) = (seconds("start_seconds", p.start_seconds)?, seconds("end_seconds", p.end_seconds)?);
    if end.0 <= start.0 {
        return Err(CommandError::invalid_params("the punch range must end after it starts"));
    }
    doc.arming.punch = Some(TimeRange::from_bounds(start, end));
    Ok(arming_json(doc))
}

#[expect(clippy::unnecessary_wraps, reason = "must match the RunFn signature")]
fn record_state(doc: &mut RecordDoc, _: &Value) -> Result<Value, CommandError> {
    Ok(arming_json(doc))
}

/// Records prepared inputs into a take on `track` using the document's arming.
fn record_into(doc: &mut RecordDoc, track: usize, inputs: &[Inputs]) -> Result<Value, CommandError> {
    unlocked(doc, track)?;
    let mut recorder = doc.recorder();
    let recording = recorder.record_all(inputs, None).map_err(fail)?;
    let Some(take) = recording.take else {
        return Ok(json!({ "take": null, "note": "nothing fell inside the punch range or no armed group had data" }));
    };
    let samples = take.samples.len();
    let index = doc.scene.add_take(track, take).map_err(fail)?;
    let stored = doc.scene.track(track).map_err(fail)?.takes.get(index).map(|t| take_info(index, t));
    Ok(json!({ "take": stored, "samples_recorded": samples }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordScript {
    track: usize,
    /// A `behaviors.simulate`-style script: `fps`, `frames` and `events` (`face`, `no_face`, `voice`, `pointer`, `key`).
    script: Script,
    /// Where on the scene timeline the script starts.
    #[serde(default)]
    start_seconds: f64,
}

fn record_script(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: RecordScript = params(value)?;
    let offset = seconds("start_seconds", p.start_seconds)?;
    let mut inputs = p.script.inputs().map_err(fail)?;
    for input in &mut inputs {
        input.time = Tick(input.time.0.saturating_add(offset.0));
    }
    record_into(doc, p.track, &inputs)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordInputs {
    track: usize,
    /// Inputs with `time` in ticks of scene time (254 016 000 000 per second), in order.
    inputs: Vec<Inputs>,
}

fn record_inputs(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: RecordInputs = params(value)?;
    if p.inputs.len() > crate::take::MAX_SAMPLES {
        return Err(CommandError::failed("too many inputs"));
    }
    record_into(doc, p.track, &p.inputs)
}

// --- takes ---------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TakeRef {
    track: usize,
    take: usize,
}

fn take_list(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: TrackRef = params(value)?;
    let track = doc.scene.track(p.track).map_err(fail)?;
    Ok(json!({ "track": p.track, "takes": track.takes.iter().enumerate().map(|(i, t)| take_info(i, t)).collect::<Vec<_>>() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TakeSet {
    track: usize,
    take: usize,
    name: Option<String>,
    enabled: Option<bool>,
    start_seconds: Option<f64>,
    blend_in_seconds: Option<f64>,
    blend_out_seconds: Option<f64>,
    /// Groups the take overrides (a subset of what it recorded makes it override less).
    groups: Option<Vec<ParamGroup>>,
}

fn take_set(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: TakeSet = params(value)?;
    let (start, blend_in, blend_out) =
        (secs_opt("start_seconds", p.start_seconds)?, secs_opt("blend_in_seconds", p.blend_in_seconds)?, secs_opt("blend_out_seconds", p.blend_out_seconds)?);
    if blend_in.is_some_and(|t| t.0 < 0) || blend_out.is_some_and(|t| t.0 < 0) {
        return Err(CommandError::invalid_params("blend lengths cannot be negative"));
    }
    let track = unlocked(doc, p.track)?;
    let take = track.takes.get_mut(p.take).ok_or_else(|| fail(crate::Error::NoTake { track: p.track, take: p.take }))?;
    if let Some(name) = p.name {
        take.name = name;
    }
    if let Some(enabled) = p.enabled {
        take.enabled = enabled;
    }
    if let Some(start) = start {
        take.start = start;
    }
    if let Some(t) = blend_in {
        take.blend_in = t;
    }
    if let Some(t) = blend_out {
        take.blend_out = t;
    }
    if let Some(groups) = p.groups {
        take.groups = groups.into_iter().collect();
    }
    let info = take_info(p.take, take);
    let end = take.end();
    if end.0 > doc.scene.duration.0 {
        doc.scene.duration = end;
    }
    Ok(info)
}

fn take_remove(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: TakeRef = params(value)?;
    let track = unlocked(doc, p.track)?;
    if p.take >= track.takes.len() {
        return Err(fail(crate::Error::NoTake { track: p.track, take: p.take }));
    }
    let removed = track.takes.remove(p.take);
    Ok(json!({ "removed": removed.name }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TakeMove {
    track: usize,
    from: usize,
    to: usize,
}

fn take_move(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: TakeMove = params(value)?;
    let track = unlocked(doc, p.track)?;
    if p.from >= track.takes.len() || p.to >= track.takes.len() {
        return Err(fail(crate::Error::NoTake { track: p.track, take: p.from.max(p.to) }));
    }
    let take = track.takes.remove(p.from);
    track.takes.insert(p.to, take);
    Ok(json!({ "order": track.takes.iter().map(|t| t.number).collect::<Vec<_>>() }))
}

// --- sound ---------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioAddClip {
    /// The media name (a sound in the media store).
    media: String,
    #[serde(default)]
    track: usize,
    name: Option<String>,
    #[serde(default)]
    start_seconds: f64,
    #[serde(default)]
    trim_in_seconds: f64,
    duration_seconds: Option<f64>,
    gain: Option<f32>,
    pan: Option<f32>,
}

fn audio_add_clip(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: AudioAddClip = params(value)?;
    let mut clip = AudioClip::new(p.name.as_deref().unwrap_or(&p.media), &p.media, seconds("start_seconds", p.start_seconds)?);
    clip.trim_in = seconds("trim_in_seconds", p.trim_in_seconds)?;
    clip.duration = secs_opt("duration_seconds", p.duration_seconds)?;
    clip.gain = p.gain.unwrap_or(1.0);
    clip.pan = p.pan.unwrap_or(0.0);
    let index = doc.scene.add_clip(p.track, clip).map_err(fail)?;
    doc.scene.sanitize().map_err(fail)?;
    Ok(json!({ "track": p.track, "clip": index }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipRef {
    #[serde(default)]
    track: usize,
    clip: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioSetClip {
    #[serde(default)]
    track: usize,
    clip: usize,
    start_seconds: Option<f64>,
    trim_in_seconds: Option<f64>,
    duration_seconds: Option<f64>,
    gain: Option<f32>,
    pan: Option<f32>,
    enabled: Option<bool>,
}

fn clip_mut(doc: &mut RecordDoc, track: usize, clip: usize) -> Result<&mut AudioClip, CommandError> {
    doc.scene
        .audio
        .get_mut(track)
        .ok_or_else(|| fail(crate::Error::NoAudio { what: "track", index: track }))?
        .clips
        .get_mut(clip)
        .ok_or_else(|| fail(crate::Error::NoAudio { what: "clip", index: clip }))
}

fn audio_set_clip(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: AudioSetClip = params(value)?;
    let (start, trim, duration) =
        (secs_opt("start_seconds", p.start_seconds)?, secs_opt("trim_in_seconds", p.trim_in_seconds)?, secs_opt("duration_seconds", p.duration_seconds)?);
    let clip = clip_mut(doc, p.track, p.clip)?;
    if let Some(t) = start {
        clip.start = t;
    }
    if let Some(t) = trim {
        clip.trim_in = t;
    }
    if duration.is_some() {
        clip.duration = duration;
    }
    if let Some(g) = p.gain {
        clip.gain = g;
    }
    if let Some(pan) = p.pan {
        clip.pan = pan;
    }
    if let Some(e) = p.enabled {
        clip.enabled = e;
    }
    doc.scene.sanitize().map_err(fail)?;
    Ok(json!({ "track": p.track, "clip": p.clip }))
}

fn audio_remove_clip(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: ClipRef = params(value)?;
    clip_mut(doc, p.track, p.clip)?;
    let removed = doc.scene.audio.get_mut(p.track).map(|t| t.clips.remove(p.clip));
    Ok(json!({ "removed": removed.map(|c| c.name) }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioSetTrack {
    track: usize,
    name: Option<String>,
    enabled: Option<bool>,
    gain: Option<f32>,
}

fn audio_set_track(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: AudioSetTrack = params(value)?;
    let track = doc.scene.audio.get_mut(p.track).ok_or_else(|| fail(crate::Error::NoAudio { what: "track", index: p.track }))?;
    if let Some(name) = p.name {
        track.name = name;
    }
    if let Some(e) = p.enabled {
        track.enabled = e;
    }
    if let Some(g) = p.gain {
        track.gain = g;
    }
    doc.scene.sanitize().map_err(fail)?;
    Ok(json!({ "track": p.track }))
}

// --- playback ------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaybackInputs {
    track: usize,
    time_seconds: f64,
}

fn playback_inputs(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: PlaybackInputs = params(value)?;
    let t = seconds("time_seconds", p.time_seconds)?;
    let track = doc.scene.track(p.track).map_err(fail)?;
    let inputs = track_inputs(&doc.scene, track, doc.scene.frame_at(t));
    serde_json::to_value(&inputs).map_err(fail)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaybackPoses {
    track: usize,
    #[serde(default)]
    from_frame: i64,
    count: Option<usize>,
    /// Frames between the poses returned (default 1).
    step: Option<usize>,
    /// `"summary"` (default) keeps the main numbers; `"full"` returns the whole pose.
    detail: Option<String>,
}

#[derive(Serialize)]
struct PoseSummary {
    frame: i64,
    time_seconds: f64,
    head_rotation_degrees: f64,
    head_offset: [f64; 2],
    viseme: Option<charcraft_puppet::Viseme>,
    blink: [f64; 2],
    eyebrows: [f64; 2],
    gaze: [f64; 2],
    breathe: f64,
    dragged: Vec<String>,
    triggers: Vec<String>,
}

fn summarize(frame: i64, time: Tick, pose: &PuppetPose) -> PoseSummary {
    PoseSummary {
        frame,
        time_seconds: to_seconds(time),
        head_rotation_degrees: pose.head.rotation.to_degrees(),
        head_offset: [pose.head.translation.x, pose.head.translation.y],
        viseme: pose.viseme,
        blink: [pose.blink_left, pose.blink_right],
        eyebrows: [pose.eyebrow_left, pose.eyebrow_right],
        gaze: [pose.gaze.x, pose.gaze.y],
        breathe: pose.breathe,
        dragged: pose.drag.keys().cloned().collect(),
        triggers: pose.triggers.iter().cloned().collect(),
    }
}

fn playback_poses(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: PlaybackPoses = params(value)?;
    let count = p.count.unwrap_or(30).clamp(1, MAX_POSES);
    let step = i64::try_from(p.step.unwrap_or(1).max(1)).unwrap_or(1);
    let full = match p.detail.as_deref() {
        None | Some("summary") => false,
        Some("full") => true,
        Some(other) => return Err(CommandError::invalid_params(format!("detail must be \"summary\" or \"full\", got {other:?}"))),
    };
    let mut evaluator = doc.evaluator().map_err(fail)?;
    doc.scene.track(p.track).map_err(fail)?;
    let mut out = Vec::new();
    let mut frame = p.from_frame.max(0);
    for _ in 0..count {
        let pose = evaluator.pose(p.track, frame).map_err(fail)?;
        let time = doc.scene.frame_time(frame);
        out.push(if full {
            json!({ "frame": frame, "time_seconds": to_seconds(time), "pose": pose })
        } else {
            serde_json::to_value(summarize(frame, time, &pose)).map_err(fail)?
        });
        frame = frame.saturating_add(step);
    }
    Ok(json!({ "track": p.track, "poses": out }))
}

// --- registry ------------------------------------------------------------------------------------------------

macro_rules! groups {
    () => {
        r#"{"type":"array","items":{"enum":["head","brows","eyes","mouth","voice","pointer","keys"]}}"#
    };
}

macro_rules! schema {
    ($($body:tt)*) => { concat!("{\"type\":\"object\",\"additionalProperties\":false,", $($body)*, "}") };
}

/// All commands of the recording layer.
#[must_use]
pub fn commands() -> Vec<CommandSpec<RecordDoc>> {
    let mut all = base_commands();
    all.extend(crate::edit_commands::commands());
    all
}

fn base_commands() -> Vec<CommandSpec<RecordDoc>> {
    vec![
        CommandSpec::new("scene.info", "Scene info", scene_info)
            .transient()
            .describe("Describe the scene: size, frame rate, length, puppet tracks with their takes, audio tracks and the recording arming.")
            .with_schema(r#"{"type":"object","properties":{}}"#),
        CommandSpec::new("scene.set", "Scene settings", scene_set)
            .describe("Change the scene's name, frame rate (fps_num/fps_den, e.g. 30/1 or 30000/1001), stage size, length in seconds, background colour ([r,g,b,a], or transparent: true) or behavior seed.")
            .with_schema(schema!(r#""properties":{"name":{"type":"string"},"fps_num":{"type":"integer","minimum":1},"fps_den":{"type":"integer","minimum":1},"width":{"type":"number"},"height":{"type":"number"},"duration_seconds":{"type":"number","minimum":0},"background":{"type":"array","items":{"type":"integer","minimum":0,"maximum":255},"minItems":4,"maxItems":4},"transparent":{"type":"boolean"},"seed":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("scene.export", "Export scene JSON", scene_export)
            .transient()
            .describe("The whole scene (tracks, takes and their samples, audio clips) as the JSON the sidecar file holds.")
            .with_schema(r#"{"type":"object","properties":{}}"#),
        CommandSpec::new("scene.import", "Import scene JSON", scene_import)
            .describe("Replace the scene with one from scene.export (the 'scene' member). The scene is checked and repaired first.")
            .with_schema(r#"{"type":"object","required":["scene"],"properties":{"scene":{"type":"object"}},"additionalProperties":false}"#),
        CommandSpec::new("track.add", "Add puppet track", track_add)
            .describe("Put a puppet on the stage. 'puppet' is its cast key (a project path such as ProjectItems/Hero). Placement: position and anchor in pixels from the stage centre, scale factors (1 = 100%), rotation in degrees, opacity 0..1.")
            .with_schema(schema!(r#""required":["puppet"],"properties":{"puppet":{"type":"string"},"name":{"type":"string"},"placement":{"type":"object"}}"#)),
        CommandSpec::new("track.remove", "Remove puppet track", track_remove)
            .describe("Remove a puppet track and its takes.")
            .with_schema(schema!(r#""required":["track"],"properties":{"track":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("track.set", "Edit puppet track", track_set)
            .describe("Rename a track, switch it on or off, lock or unlock it, change its puppet or its placement (position, anchor, scale, rotation, opacity; keyframe these with keyframe.set). A locked track refuses edits and recordings.")
            .with_schema(schema!(r#""required":["track"],"properties":{"track":{"type":"integer","minimum":0},"name":{"type":"string"},"puppet":{"type":"string"},"enabled":{"type":"boolean"},"placement":{"type":"object"},"locked":{"type":"boolean"}}"#)),
        CommandSpec::new("track.behavior", "Edit track behaviors", track_behavior)
            .describe("Lay one edit over the puppet's behavior stack for this track, using behaviors.edit operations: {\"op\":\"set_param\",\"index\":0,\"name\":\"...\",\"value\":...}, set_enabled, set_seed, add, remove, move. Pass op null to clear the track's edits. Edits apply in order before playback.")
            .with_schema(schema!(r#""required":["track"],"properties":{"track":{"type":"integer","minimum":0},"op":{"type":["object","null"]}}"#)),
        CommandSpec::new("record.arm", "Arm inputs", record_arm)
            .describe("Choose which input groups the next recording captures: head, brows, eyes, mouth (the face), voice (lip sync), pointer (drags), keys (triggers, walk). 'groups' with armed true/false arms or disarms them; 'only': true arms exactly the list. 'audio' arms the microphone.")
            .with_schema(schema!(concat!(r#""properties":{"groups":"#, groups!(), r#","armed":{"type":"boolean"},"only":{"type":"boolean"},"audio":{"type":"boolean"}}"#))),
        CommandSpec::new("record.punch", "Set punch-in range", record_punch)
            .describe("Record only between two scene times (seconds). Call with no arguments to clear the range.")
            .with_schema(r#"{"type":"object","properties":{"start_seconds":{"type":"number"},"end_seconds":{"type":"number"}},"additionalProperties":false}"#),
        CommandSpec::new("record.state", "Recording state", record_state)
            .transient()
            .describe("Which groups are armed and the punch-in range.")
            .with_schema(r#"{"type":"object","properties":{}}"#),
        CommandSpec::new("record.script", "Record a scripted performance", record_script)
            .describe("Record a scripted performance (the same script format as behaviors.simulate: fps, frames, events of kind face, no_face, voice, pointer, pointer_gone, key) into a new take on top of a track, capturing only the armed groups and only inside the punch range. start_seconds places the script on the scene timeline.")
            .with_schema(schema!(r#""required":["track","script"],"properties":{"track":{"type":"integer","minimum":0},"script":{"type":"object","required":["frames"],"properties":{"fps":{"type":"number"},"frames":{"type":"integer","minimum":1},"events":{"type":"array","items":{"type":"object"}}}},"start_seconds":{"type":"number"}}"#)),
        CommandSpec::new("record.inputs", "Record raw inputs", record_inputs)
            .describe("Record a list of timestamped inputs (time in ticks, 254016000000 per second; face, viseme, pointers, keys) into a new take. Prefer record.script unless you already have inputs.")
            .with_schema(schema!(r#""required":["track","inputs"],"properties":{"track":{"type":"integer","minimum":0},"inputs":{"type":"array","items":{"type":"object"}}}"#)),
        CommandSpec::new("take.list", "List takes", take_list, )
            .transient()
            .describe("The takes of a track, lowest first. A later take overrides an earlier one wherever they overlap, per group.")
            .with_schema(schema!(r#""required":["track"],"properties":{"track":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("take.set", "Edit take", take_set)
            .describe("Rename, switch on or off, move (start_seconds), fade (blend_in_seconds, blend_out_seconds) or restrict (groups) a take.")
            .with_schema(schema!(concat!(
                r#""required":["track","take"],"properties":{"track":{"type":"integer","minimum":0},"take":{"type":"integer","minimum":0},"name":{"type":"string"},"enabled":{"type":"boolean"},"start_seconds":{"type":"number"},"blend_in_seconds":{"type":"number","minimum":0},"blend_out_seconds":{"type":"number","minimum":0},"groups":"#,
                groups!(),
                "}"
            ))),
        CommandSpec::new("take.remove", "Remove take", take_remove)
            .describe("Delete a take.")
            .with_schema(schema!(r#""required":["track","take"],"properties":{"track":{"type":"integer","minimum":0},"take":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("take.move", "Reorder take", take_move)
            .describe("Move a take in the stack; later positions override earlier ones.")
            .with_schema(schema!(r#""required":["track","from","to"],"properties":{"track":{"type":"integer","minimum":0},"from":{"type":"integer","minimum":0},"to":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("audio.add_clip", "Add audio clip", audio_add_clip)
            .describe("Place a sound from the media store on an audio track (created as needed) at start_seconds, optionally trimmed (trim_in_seconds, duration_seconds), with gain and pan (-1..1).")
            .with_schema(schema!(r#""required":["media"],"properties":{"media":{"type":"string"},"track":{"type":"integer","minimum":0},"name":{"type":"string"},"start_seconds":{"type":"number"},"trim_in_seconds":{"type":"number","minimum":0},"duration_seconds":{"type":"number","minimum":0},"gain":{"type":"number"},"pan":{"type":"number","minimum":-1,"maximum":1}}"#)),
        CommandSpec::new("audio.set_clip", "Edit audio clip", audio_set_clip)
            .describe("Move, trim, scale, pan or mute an audio clip.")
            .with_schema(schema!(r#""required":["clip"],"properties":{"track":{"type":"integer","minimum":0},"clip":{"type":"integer","minimum":0},"start_seconds":{"type":"number"},"trim_in_seconds":{"type":"number","minimum":0},"duration_seconds":{"type":"number","minimum":0},"gain":{"type":"number"},"pan":{"type":"number","minimum":-1,"maximum":1},"enabled":{"type":"boolean"}}"#)),
        CommandSpec::new("audio.remove_clip", "Remove audio clip", audio_remove_clip)
            .describe("Delete an audio clip.")
            .with_schema(schema!(r#""required":["clip"],"properties":{"track":{"type":"integer","minimum":0},"clip":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("audio.set_track", "Edit audio track", audio_set_track)
            .describe("Rename, mute or set the gain of an audio track.")
            .with_schema(schema!(r#""required":["track"],"properties":{"track":{"type":"integer","minimum":0},"name":{"type":"string"},"enabled":{"type":"boolean"},"gain":{"type":"number"}}"#)),
        CommandSpec::new("playback.inputs", "Inputs at a time", playback_inputs)
            .transient()
            .describe("The inputs (face, voice mouth shape, pointers, keys) the takes of a track give the behaviors at a scene time in seconds, with every take layered and blended.")
            .with_schema(schema!(r#""required":["track","time_seconds"],"properties":{"track":{"type":"integer","minimum":0},"time_seconds":{"type":"number"}}"#)),
        CommandSpec::new("playback.poses", "Poses over time", playback_poses)
            .transient()
            .describe("Play the scene's behaviors for a track and return the puppet pose of count frames starting at from_frame, every 'step' frames. detail 'summary' (default) gives head rotation, mouth shape, blinks, eyebrows, gaze and drags; 'full' gives the whole pose. Deterministic: the same scene always gives the same poses.")
            .with_schema(schema!(r#""required":["track"],"properties":{"track":{"type":"integer","minimum":0},"from_frame":{"type":"integer","minimum":0},"count":{"type":"integer","minimum":1,"maximum":600},"step":{"type":"integer","minimum":1},"detail":{"enum":["summary","full"]}}"#)),
    ]
}

/// Registers all commands.
///
/// # Errors
/// [`RegistryError`] when an id is already taken.
pub fn register(registry: &mut CommandRegistry<RecordDoc>) -> Result<(), RegistryError> {
    registry.register_all(commands())
}

/// A registry with undo and redo and all commands.
///
/// # Errors
/// [`RegistryError`], which cannot happen for the built-in ids.
pub fn registry() -> Result<CommandRegistry<RecordDoc>, RegistryError> {
    let mut registry = CommandRegistry::with_history_commands();
    register(&mut registry)?;
    Ok(registry)
}
