//! More tools of the recording layer: scenes, track order, trimming and splitting takes, audio tracks and clips,
//! parameter keyframes, the camera, take timing and lip sync from a clip of the scene.
//!
//! | area | commands |
//! |---|---|
//! | scenes | `scene.list`, `scene.add`, `scene.duplicate`, `scene.remove`, `scene.select`, `scene.move` |
//! | track order | `track.move` |
//! | takes | `take.trim`, `take.split` |
//! | sound | `audio.add_track`, `audio.remove_track`, `audio.move_clip`, `audio.split_clip` |
//! | keyframes | `keyframe.list`, `keyframe.set`, `keyframe.remove`, `keyframe.clear` |
//! | camera | `camera.set` |
//! | timing | `timing.list`, `timing.set_viseme`, `timing.insert_viseme`, `timing.remove_viseme`, `timing.set_key`, `timing.add_key`, `timing.remove_key` |
//! | lip sync | `lipsync.from_clip` |

use anim_core::{Tick, Vec2};
use anim_lipsync::{Viseme, VisemeKey};
use anim_session::{CommandError, CommandSpec};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::camera::{TARGETS as CAMERA_TARGETS, ZoomOnFace};
use crate::commands::{camera_json, fail, lanes_json, params, seconds, secs_opt, take_info, to_seconds, unlocked};
use crate::doc::RecordDoc;
use crate::keyframes::{Ease, Lane, lane_mut};
use crate::playback::{param_json, track_stack};
use crate::scene::{AudioTrack, MAX_AUDIO_TRACKS, PLACEMENT_TARGETS, PuppetTrack, Scene, behavior_target};
use crate::timing::{self, KeyRun, VoicePoint};

macro_rules! schema {
    ($($body:tt)*) => { concat!("{\"type\":\"object\",\"additionalProperties\":false,", $($body)*, "}") };
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::invalid_params(message.into())
}

// --- scenes --------------------------------------------------------------------------------------------------

fn scene_row(index: usize, scene: &Scene, active: bool) -> Value {
    json!({
        "index": index,
        "name": scene.name,
        "active": active,
        "size": [scene.size.width, scene.size.height],
        "fps": scene.fps_f64(),
        "duration_seconds": to_seconds(scene.duration),
        "tracks": scene.tracks.len(),
        "audio_tracks": scene.audio.len(),
    })
}

#[expect(clippy::unnecessary_wraps, reason = "must match the RunFn signature")]
fn scene_list(doc: &mut RecordDoc, _: &Value) -> Result<Value, CommandError> {
    let active = doc.active_scene();
    Ok(json!({ "active": active, "scenes": doc.scenes().enumerate().map(|(i, s)| scene_row(i, s, i == active)).collect::<Vec<_>>() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneAdd {
    name: Option<String>,
    /// A puppet (cast key) to put on the stage of the new scene.
    puppet: Option<String>,
    /// Open the new scene (default true).
    open: Option<bool>,
    /// Where to insert it (default: the end).
    index: Option<usize>,
}

fn scene_add(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: SceneAdd = params(value)?;
    let name = p.name.unwrap_or_else(|| format!("Scene {}", doc.scene_count().saturating_add(1)));
    let mut scene = Scene::new(&name, doc.scene.size, doc.scene.fps, Tick::from_seconds_f64(5.0));
    scene.background = doc.scene.background;
    scene.seed = doc.scene.seed;
    if let Some(puppet) = p.puppet.as_deref() {
        // Same placement as the same puppet has in the open scene, when it stands there.
        let mut track =
            doc.scene.tracks.iter().find(|t| t.puppet == puppet).map_or_else(|| PuppetTrack::new(puppet, puppet), |t| PuppetTrack::new(&t.name, puppet));
        if let Some(existing) = doc.scene.tracks.iter().find(|t| t.puppet == puppet) {
            track.placement = existing.placement;
        }
        scene.add_track(track).map_err(fail)?;
    }
    scene.sanitize().map_err(fail)?;
    let index = doc.add_scene(scene, p.index).map_err(fail)?;
    if p.open.unwrap_or(true) {
        doc.select_scene(index).map_err(fail)?;
    }
    Ok(json!({ "scene": index, "active": doc.active_scene(), "count": doc.scene_count() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneDuplicate {
    index: Option<usize>,
    name: Option<String>,
    open: Option<bool>,
}

fn scene_duplicate(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: SceneDuplicate = params(value)?;
    let from = p.index.unwrap_or_else(|| doc.active_scene());
    let mut copy = doc.scene_at(from).cloned().ok_or_else(|| invalid(format!("no scene {from}")))?;
    copy.name = p.name.unwrap_or_else(|| format!("{} copy", copy.name));
    let index = doc.add_scene(copy, Some(from.saturating_add(1))).map_err(fail)?;
    if p.open.unwrap_or(false) {
        doc.select_scene(index).map_err(fail)?;
    }
    Ok(json!({ "scene": index, "active": doc.active_scene(), "count": doc.scene_count() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneRef {
    index: usize,
}

fn scene_remove(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: SceneRef = params(value)?;
    let removed = doc.remove_scene(p.index).map_err(fail)?;
    Ok(json!({ "removed": removed.name, "active": doc.active_scene(), "count": doc.scene_count() }))
}

fn scene_select(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: SceneRef = params(value)?;
    doc.select_scene(p.index).map_err(fail)?;
    Ok(json!({ "active": doc.active_scene(), "name": doc.scene.name }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FromTo {
    from: usize,
    to: usize,
}

fn scene_move(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: FromTo = params(value)?;
    doc.move_scene(p.from, p.to).map_err(fail)?;
    Ok(json!({ "active": doc.active_scene(), "scenes": doc.scenes().map(|s| s.name.clone()).collect::<Vec<_>>() }))
}

// --- track order ---------------------------------------------------------------------------------------------

fn track_move(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: FromTo = params(value)?;
    doc.scene.move_track(p.from, p.to).map_err(fail)?;
    Ok(json!({ "order": doc.scene.tracks.iter().map(|t| t.name.clone()).collect::<Vec<_>>() }))
}

// --- takes ---------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TakeTrim {
    track: usize,
    take: usize,
    /// New first moment of the take, in scene seconds.
    start_seconds: Option<f64>,
    /// New last moment of the take, in scene seconds.
    end_seconds: Option<f64>,
}

fn take_trim(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: TakeTrim = params(value)?;
    let (start, end) = (secs_opt("start_seconds", p.start_seconds)?, secs_opt("end_seconds", p.end_seconds)?);
    let track = unlocked(doc, p.track)?;
    let take = track.takes.get_mut(p.take).ok_or_else(|| fail(crate::Error::NoTake { track: p.track, take: p.take }))?;
    if let (Some(s), Some(e)) = (start, end)
        && e.0 <= s.0
    {
        return Err(invalid("the trimmed take must end after it starts"));
    }
    if let Some(s) = start {
        take.trim_start(s);
    }
    if let Some(e) = end {
        take.trim_end(e);
    }
    Ok(take_info(p.take, take))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TakeSplit {
    track: usize,
    take: usize,
    time_seconds: f64,
}

fn take_split(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: TakeSplit = params(value)?;
    let at = seconds("time_seconds", p.time_seconds)?;
    unlocked(doc, p.track)?;
    let second = doc.scene.split_take(p.track, p.take, at).map_err(fail)?;
    let track = doc.scene.track(p.track).map_err(fail)?;
    Ok(json!({ "takes": [track.takes.get(p.take).map(|t| take_info(p.take, t)), track.takes.get(second).map(|t| take_info(second, t))] }))
}

// --- sound ---------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioAddTrack {
    name: Option<String>,
}

fn audio_add_track(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: AudioAddTrack = params(value)?;
    if doc.scene.audio.len() >= MAX_AUDIO_TRACKS {
        return Err(CommandError::failed("the scene already has the most audio tracks"));
    }
    let name = p.name.unwrap_or_else(|| format!("Audio {}", doc.scene.audio.len().saturating_add(1)));
    doc.scene.audio.push(AudioTrack::new(&name));
    doc.scene.sanitize().map_err(fail)?;
    Ok(json!({ "track": doc.scene.audio.len().saturating_sub(1) }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioTrackRef {
    track: usize,
}

fn audio_remove_track(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: AudioTrackRef = params(value)?;
    if p.track >= doc.scene.audio.len() {
        return Err(fail(crate::Error::NoAudio { what: "track", index: p.track }));
    }
    let removed = doc.scene.audio.remove(p.track);
    Ok(json!({ "removed": removed.name, "clips": removed.clips.len() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioMoveClip {
    #[serde(default)]
    track: usize,
    clip: usize,
    /// The audio track to move it to (default: stay).
    to_track: Option<usize>,
    start_seconds: Option<f64>,
}

fn audio_move_clip(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: AudioMoveClip = params(value)?;
    let start = secs_opt("start_seconds", p.start_seconds)?;
    let source = doc.scene.audio.get_mut(p.track).ok_or_else(|| fail(crate::Error::NoAudio { what: "track", index: p.track }))?;
    if p.clip >= source.clips.len() {
        return Err(fail(crate::Error::NoAudio { what: "clip", index: p.clip }));
    }
    let mut clip = source.clips.remove(p.clip);
    if let Some(t) = start {
        clip.start = Tick(t.0.max(0));
    }
    let target = p.to_track.unwrap_or(p.track);
    let index = match doc.scene.add_clip(target, clip.clone()) {
        Ok(i) => i,
        Err(e) => {
            // Put it back where it was: a refused move changes nothing.
            if let Some(t) = doc.scene.audio.get_mut(p.track) {
                t.clips.insert(p.clip, clip);
            }
            return Err(fail(e));
        }
    };
    let end = doc.scene.audio.get(target).and_then(|t| t.clips.get(index)).and_then(|c| c.duration.map(|d| Tick(c.start.0.saturating_add(d.0))));
    if let Some(end) = end
        && end.0 > doc.scene.duration.0
    {
        doc.scene.duration = end;
    }
    Ok(json!({ "track": target, "clip": index }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioSplitClip {
    #[serde(default)]
    track: usize,
    clip: usize,
    time_seconds: f64,
    /// The length of the whole sound, when the clip plays to its end.
    media_seconds: Option<f64>,
}

fn audio_split_clip(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: AudioSplitClip = params(value)?;
    let at = seconds("time_seconds", p.time_seconds)?;
    let media = secs_opt("media_seconds", p.media_seconds)?;
    let track = doc.scene.audio.get_mut(p.track).ok_or_else(|| fail(crate::Error::NoAudio { what: "track", index: p.track }))?;
    let clip = track.clips.get_mut(p.clip).ok_or_else(|| fail(crate::Error::NoAudio { what: "clip", index: p.clip }))?;
    let length = clip
        .duration
        .or_else(|| media.map(|m| Tick(m.0.saturating_sub(clip.trim_in.0))))
        .ok_or_else(|| CommandError::failed("this clip plays to the end of its sound; give media_seconds (the sound's length) or set its duration first"))?;
    let end = Tick(clip.start.0.saturating_add(length.0));
    if at.0 <= clip.start.0 || at.0 >= end.0 {
        return Err(invalid("the split time must lie inside the clip"));
    }
    let cut = at.0.saturating_sub(clip.start.0);
    let mut second = clip.clone();
    second.start = at;
    second.trim_in = Tick(clip.trim_in.0.saturating_add(cut));
    second.duration = Some(Tick(length.0.saturating_sub(cut)));
    second.name = format!("{} (2)", clip.name);
    clip.duration = Some(Tick(cut));
    let index = p.clip.saturating_add(1);
    if track.clips.len() >= crate::scene::MAX_CLIPS {
        return Err(CommandError::failed("this track already has the most clips"));
    }
    track.clips.insert(index, second);
    Ok(json!({ "track": p.track, "clips": [p.clip, index] }))
}

// --- keyframes -----------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyList {
    /// The puppet track (omit for the camera).
    track: Option<usize>,
}

fn lanes_of(doc: &RecordDoc, track: Option<usize>) -> Result<&Vec<Lane>, CommandError> {
    match track {
        Some(t) => Ok(&doc.scene.track(t).map_err(fail)?.lanes),
        None => Ok(&doc.scene.camera.lanes),
    }
}

fn keyframe_list(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: KeyList = params(value)?;
    let lanes = lanes_of(doc, p.track)?;
    let targets: Vec<&str> = if p.track.is_some() { PLACEMENT_TARGETS.to_vec() } else { CAMERA_TARGETS.to_vec() };
    Ok(json!({ "track": p.track, "lanes": lanes_json(lanes), "targets": targets }))
}

/// Reads an ease given as a preset name or as `{kind, ...}`.
fn ease_from(value: &Value) -> Result<Ease, CommandError> {
    match value {
        Value::String(name) => {
            Ease::parse(name).ok_or_else(|| invalid(format!("no ease {name:?} (linear, hold, ease_in, ease_out, ease_in_out, or a bezier object)")))
        }
        Value::Object(_) => {
            let mut ease: Ease = serde_json::from_value(value.clone()).map_err(|e| invalid(format!("not an ease: {e}")))?;
            ease.sanitize();
            Ok(ease)
        }
        _ => Err(invalid("ease must be a preset name or an object")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeySet {
    track: Option<usize>,
    target: String,
    /// Scene time of the key (with `index`: the key's new time).
    time_seconds: Option<f64>,
    value: Option<f64>,
    ease: Option<Value>,
    /// Edit this key instead of the one at `time_seconds`.
    index: Option<usize>,
}

/// Checks that `target` names something animatable and, for a behavior parameter, that it takes a number.
fn check_target(doc: &RecordDoc, track: Option<usize>, target: &str, probe: f64) -> Result<(), CommandError> {
    let Some(track_index) = track else {
        return if CAMERA_TARGETS.contains(&target) { Ok(()) } else { Err(invalid(format!("the camera has no {target:?}; use one of {CAMERA_TARGETS:?}"))) };
    };
    if PLACEMENT_TARGETS.contains(&target) {
        return Ok(());
    }
    let Some((slot, name)) = behavior_target(target) else {
        return Err(invalid(format!("{target:?} is not animatable; use one of {PLACEMENT_TARGETS:?} or behavior.<slot>.<parameter>")));
    };
    let def = doc.scene.track(track_index).map_err(fail)?;
    let mut stack = track_stack(&doc.scene, &doc.templates, track_index, def).map_err(fail)?;
    let behavior = stack.slot_mut(slot).ok_or_else(|| invalid(format!("the track has no behavior in slot {slot}")))?;
    charcraft_behaviors::behavior::set_param(behavior.behavior.as_mut(), name, param_json(probe)).map_err(|e| invalid(format!("{target}: {e}")))
}

fn keyframe_set(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: KeySet = params(value)?;
    let time = secs_opt("time_seconds", p.time_seconds)?;
    let ease = p.ease.as_ref().map(ease_from).transpose()?;
    if p.value.is_some_and(|v| !v.is_finite()) {
        return Err(invalid("value must be a finite number"));
    }
    let current = lanes_of(doc, p.track)?.iter().find(|l| l.target == p.target).and_then(|l| p.index.and_then(|i| l.keys.get(i))).map(|k| (k.t, k.value));
    if p.index.is_some() && current.is_none() {
        return Err(invalid("there is no such key"));
    }
    let value_now = p.value.or_else(|| current.map(|c| c.1));
    let time_now = time.or_else(|| current.map(|c| c.0));
    let (Some(value_now), Some(time_now)) = (value_now, time_now) else {
        return Err(invalid("give time_seconds and value (or the index of an existing key)"));
    };
    check_target(doc, p.track, &p.target, value_now)?;
    let lanes = match p.track {
        Some(t) => &mut unlocked(doc, t)?.lanes,
        None => &mut doc.scene.camera.lanes,
    };
    let lane = lane_mut(lanes, &p.target).map_err(fail)?;
    let time_now = Tick(time_now.0.max(0));
    let index = match p.index {
        Some(i) => {
            if let Some(key) = lane.keys.get_mut(i) {
                key.value = value_now;
                if let Some(e) = ease {
                    key.ease = e;
                }
            }
            lane.move_key(i, time_now).ok_or_else(|| invalid("there is no such key"))?
        }
        None => lane.set_key(time_now, value_now, ease).map_err(fail)?,
    };
    let end = lane.keys.last().map_or(Tick::ZERO, |k| k.t);
    if end.0 > doc.scene.duration.0 {
        doc.scene.duration = end;
    }
    doc.scene.sanitize().map_err(fail)?;
    let lanes = lanes_of(doc, p.track)?;
    Ok(json!({ "target": p.target, "index": index, "lanes": lanes_json(lanes) }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyRemove {
    track: Option<usize>,
    target: String,
    /// Remove this key (omit to remove the whole lane).
    index: Option<usize>,
}

fn keyframe_remove(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: KeyRemove = params(value)?;
    let lanes = match p.track {
        Some(t) => &mut unlocked(doc, t)?.lanes,
        None => &mut doc.scene.camera.lanes,
    };
    let Some(at) = lanes.iter().position(|l| l.target == p.target) else {
        return Err(invalid(format!("nothing is keyframed for {:?}", p.target)));
    };
    match p.index {
        Some(i) => {
            let lane = lanes.get_mut(at).ok_or_else(|| invalid("no such lane"))?;
            if i >= lane.keys.len() {
                return Err(invalid("there is no such key"));
            }
            lane.keys.remove(i);
            if lane.keys.is_empty() {
                lanes.remove(at);
            }
        }
        None => {
            lanes.remove(at);
        }
    }
    let lanes = lanes_of(doc, p.track)?;
    Ok(json!({ "lanes": lanes_json(lanes) }))
}

fn keyframe_clear(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: KeyList = params(value)?;
    match p.track {
        Some(t) => unlocked(doc, t)?.lanes.clear(),
        None => doc.scene.camera.lanes.clear(),
    }
    Ok(json!({ "lanes": [] }))
}

// --- camera --------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CameraSet {
    position_x: Option<f64>,
    position_y: Option<f64>,
    zoom: Option<f64>,
    rotation: Option<f64>,
    /// `{track, amount, zoom}`, or `null` to stop following a face.
    #[serde(default, with = "double_option")]
    #[expect(clippy::option_option, reason = "absent (keep) and null (clear) are different requests")]
    zoom_on_face: Option<Option<ZoomOnFaceParams>>,
    /// Back to the neutral camera first (keyframes stay).
    #[serde(default)]
    reset: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ZoomOnFaceParams {
    track: Option<usize>,
    amount: Option<f64>,
    zoom: Option<f64>,
}

/// `Option<Option<T>>` that tells an absent field from `null`.
mod double_option {
    use serde::{Deserialize, Deserializer};

    #[expect(clippy::option_option, reason = "absent (keep) and null (clear) are different requests")]
    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(d).map(Some)
    }
}

fn camera_set(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: CameraSet = params(value)?;
    for v in [p.position_x, p.position_y, p.zoom, p.rotation].into_iter().flatten() {
        if !v.is_finite() {
            return Err(invalid("camera values must be finite numbers"));
        }
    }
    let camera = &mut doc.scene.camera;
    if p.reset {
        camera.position = Vec2::ZERO;
        camera.zoom = 1.0;
        camera.rotation = 0.0;
        camera.zoom_on_face = None;
    }
    if let Some(x) = p.position_x {
        camera.position.x = x;
    }
    if let Some(y) = p.position_y {
        camera.position.y = y;
    }
    if let Some(z) = p.zoom {
        camera.zoom = z;
    }
    if let Some(r) = p.rotation {
        camera.rotation = r;
    }
    match p.zoom_on_face {
        Some(Some(f)) => {
            let old = camera.zoom_on_face.unwrap_or_default();
            camera.zoom_on_face =
                Some(ZoomOnFace { track: f.track.unwrap_or(old.track), amount: f.amount.unwrap_or(old.amount), zoom: f.zoom.unwrap_or(old.zoom) });
        }
        Some(None) => camera.zoom_on_face = None,
        None => {}
    }
    if camera.zoom_on_face.is_some_and(|f| f.track >= doc.scene.tracks.len()) {
        return Err(invalid("zoom_on_face.track is not a puppet track of this scene"));
    }
    doc.scene.sanitize().map_err(fail)?;
    Ok(camera_json(&doc.scene.camera))
}

// --- timing --------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TakeAt {
    track: usize,
    take: usize,
}

fn viseme_name(v: Option<Viseme>) -> Value {
    v.map_or(Value::Null, |v| json!(v.ca_layer_name()))
}

fn parse_viseme(name: &str) -> Result<Option<Viseme>, CommandError> {
    if name.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    Viseme::from_ca_layer_name(name)
        .map(Some)
        .ok_or_else(|| invalid(format!("no mouth shape {name:?} (Neutral, Ah, D, Ee, F, L, M, Oh, R, S, Uh, W-Oo, Smile, Surprised or none)")))
}

fn timing_json(take: &crate::take::Take) -> Value {
    json!({
        "take": take.name,
        "voice": timing::voice_runs(take).iter().enumerate().map(|(i, r)| json!({
            "index": i,
            "start_seconds": to_seconds(r.start),
            "end_seconds": to_seconds(r.end),
            "viseme": viseme_name(r.viseme),
        })).collect::<Vec<_>>(),
        "keys": timing::key_runs(take).iter().enumerate().map(|(i, r)| json!({
            "index": i,
            "key": r.key,
            "start_seconds": to_seconds(r.start),
            "end_seconds": to_seconds(r.end),
        })).collect::<Vec<_>>(),
    })
}

fn timing_list(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: TakeAt = params(value)?;
    let track = doc.scene.track(p.track).map_err(fail)?;
    let take = track.takes.get(p.take).ok_or_else(|| fail(crate::Error::NoTake { track: p.track, take: p.take }))?;
    Ok(timing_json(take))
}

fn take_for_edit<'a>(doc: &'a mut RecordDoc, at: &TakeAt) -> Result<&'a mut crate::take::Take, CommandError> {
    let track = unlocked(doc, at.track)?;
    track.takes.get_mut(at.take).ok_or_else(|| fail(crate::Error::NoTake { track: at.track, take: at.take }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetViseme {
    track: usize,
    take: usize,
    /// The run to edit (see timing.list).
    index: usize,
    /// The new shape (`none` clears it).
    viseme: Option<String>,
    /// The new start of the run: the previous run then ends here.
    start_seconds: Option<f64>,
}

fn timing_set_viseme(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: SetViseme = params(value)?;
    let start = secs_opt("start_seconds", p.start_seconds)?;
    let shape = p.viseme.as_deref().map(parse_viseme).transpose()?;
    let take = take_for_edit(doc, &TakeAt { track: p.track, take: p.take })?;
    let mut points = timing::voice_points(take);
    let neighbours = (p.index.checked_sub(1).and_then(|i| points.get(i)).map(|q| q.t), points.get(p.index.saturating_add(1)).map(|q| q.t));
    let point = points.get_mut(p.index).ok_or_else(|| invalid(format!("no run {}", p.index)))?;
    if let Some(shape) = shape {
        point.viseme = shape;
    }
    if let Some(t) = start
        && p.index > 0
    {
        let end = take.end().0;
        let low = neighbours.0.map_or(take.start.0, |n| n.0.saturating_add(1));
        let high = neighbours.1.map_or(end, |n| n.0.saturating_sub(1));
        point.t = Tick(t.0.clamp(low, high.max(low)));
    }
    timing::set_voice_points(take, &points);
    Ok(timing_json(take))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InsertViseme {
    track: usize,
    take: usize,
    start_seconds: f64,
    end_seconds: f64,
    viseme: String,
}

fn timing_insert_viseme(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: InsertViseme = params(value)?;
    let (start, end) = (seconds("start_seconds", p.start_seconds)?, seconds("end_seconds", p.end_seconds)?);
    if end.0 <= start.0 {
        return Err(invalid("the run must end after it starts"));
    }
    let shape = parse_viseme(&p.viseme)?;
    let take = take_for_edit(doc, &TakeAt { track: p.track, take: p.take })?;
    let before_end = take.frame_at(end).and_then(|f| f.viseme);
    let mut points: Vec<VoicePoint> = timing::voice_points(take).into_iter().filter(|q| q.t.0 < start.0 || q.t.0 >= end.0).collect();
    points.push(VoicePoint { t: start, viseme: shape });
    if end.0 < take.end().0 {
        points.push(VoicePoint { t: end, viseme: before_end });
    }
    points.sort_by_key(|q| q.t);
    timing::set_voice_points(take, &points);
    Ok(timing_json(take))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunRef {
    track: usize,
    take: usize,
    index: usize,
}

fn timing_remove_viseme(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: RunRef = params(value)?;
    let take = take_for_edit(doc, &TakeAt { track: p.track, take: p.take })?;
    let mut points = timing::voice_points(take);
    if p.index >= points.len() {
        return Err(invalid(format!("no run {}", p.index)));
    }
    if p.index == 0 {
        if let Some(first) = points.first_mut() {
            first.viseme = None;
        }
    } else {
        points.remove(p.index);
    }
    timing::set_voice_points(take, &points);
    Ok(timing_json(take))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetKey {
    track: usize,
    take: usize,
    index: usize,
    start_seconds: Option<f64>,
    end_seconds: Option<f64>,
    key: Option<String>,
}

fn timing_set_key(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: SetKey = params(value)?;
    let (start, end) = (secs_opt("start_seconds", p.start_seconds)?, secs_opt("end_seconds", p.end_seconds)?);
    let take = take_for_edit(doc, &TakeAt { track: p.track, take: p.take })?;
    let mut runs = timing::key_runs(take);
    let run = runs.get_mut(p.index).ok_or_else(|| invalid(format!("no key run {}", p.index)))?;
    if let Some(s) = start {
        run.start = s;
    }
    if let Some(e) = end {
        run.end = e;
    }
    if let Some(k) = p.key {
        run.key = k;
    }
    if run.end.0 <= run.start.0 {
        return Err(invalid("the key must go up after it goes down"));
    }
    timing::set_key_runs(take, &runs);
    Ok(timing_json(take))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddKey {
    track: usize,
    take: usize,
    key: String,
    start_seconds: f64,
    end_seconds: f64,
}

fn timing_add_key(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: AddKey = params(value)?;
    let (start, end) = (seconds("start_seconds", p.start_seconds)?, seconds("end_seconds", p.end_seconds)?);
    if end.0 <= start.0 || p.key.is_empty() || p.key.chars().count() > crate::group::MAX_KEY_LEN {
        return Err(invalid("a key run needs a key name and an end after its start"));
    }
    let take = take_for_edit(doc, &TakeAt { track: p.track, take: p.take })?;
    let mut runs = timing::key_runs(take);
    runs.push(KeyRun { key: p.key, start, end });
    timing::set_key_runs(take, &runs);
    Ok(timing_json(take))
}

fn timing_remove_key(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: RunRef = params(value)?;
    let take = take_for_edit(doc, &TakeAt { track: p.track, take: p.take })?;
    let mut runs = timing::key_runs(take);
    if p.index >= runs.len() {
        return Err(invalid(format!("no key run {}", p.index)));
    }
    runs.remove(p.index);
    timing::set_key_runs(take, &runs);
    Ok(timing_json(take))
}

// --- lip sync from a clip of the scene ----------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyIn {
    time_seconds: f64,
    viseme: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LipsyncFromClip {
    #[serde(default)]
    track: usize,
    #[serde(default)]
    audio_track: usize,
    clip: usize,
    /// Mouth shapes found in the sound (times from the start of the sound), when the clip has not been analysed yet.
    keys: Option<Vec<KeyIn>>,
    /// The length of the sound, when the clip plays to its end.
    media_seconds: Option<f64>,
}

fn lipsync_from_clip(doc: &mut RecordDoc, value: &Value) -> Result<Value, CommandError> {
    let p: LipsyncFromClip = params(value)?;
    unlocked(doc, p.track)?;
    let media = secs_opt("media_seconds", p.media_seconds)?;
    let keys: Option<Vec<VisemeKey>> = p
        .keys
        .map(|list| {
            list.iter()
                .map(|k| {
                    let viseme = parse_viseme(&k.viseme)?.unwrap_or(Viseme::Neutral);
                    Ok(VisemeKey { start: Tick(seconds("time_seconds", k.time_seconds)?.0.max(0)), viseme })
                })
                .collect::<Result<Vec<_>, CommandError>>()
        })
        .transpose()?;
    let clip = doc
        .scene
        .audio
        .get_mut(p.audio_track)
        .ok_or_else(|| fail(crate::Error::NoAudio { what: "track", index: p.audio_track }))?
        .clips
        .get_mut(p.clip)
        .ok_or_else(|| fail(crate::Error::NoAudio { what: "clip", index: p.clip }))?;
    if let Some(mut keys) = keys {
        if keys.len() > crate::scene::MAX_VISEMES {
            return Err(CommandError::failed("too many mouth shapes"));
        }
        keys.sort_by_key(|k| k.start);
        clip.visemes = keys;
    }
    let take = timing::clip_voice_take(clip, media).ok_or_else(|| {
        CommandError::failed("this clip has no analysed mouth shapes yet; pass keys, or use lipsync.record with the sound file to analyse it")
    })?;
    let samples = take.samples.len();
    let index = doc.scene.add_take(p.track, take).map_err(fail)?;
    let stored = doc.scene.track(p.track).map_err(fail)?.takes.get(index).map(|t| take_info(index, t));
    Ok(json!({ "take": stored, "samples": samples }))
}

// --- registry ------------------------------------------------------------------------------------------------

/// The commands of this module.
#[must_use]
pub fn commands() -> Vec<CommandSpec<RecordDoc>> {
    vec![
        CommandSpec::new("scene.list", "List scenes", scene_list)
            .transient()
            .describe("The scenes of the project in order with their size, frame rate, length and which one is open.")
            .with_schema(r#"{"type":"object","properties":{}}"#),
        CommandSpec::new("scene.add", "New scene", scene_add)
            .describe("Add an empty scene with the open scene's size, frame rate and background, optionally with a puppet (cast key) on its stage, and open it (open false keeps the current one open).")
            .with_schema(schema!(r#""properties":{"name":{"type":"string"},"puppet":{"type":"string"},"open":{"type":"boolean"},"index":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("scene.duplicate", "Duplicate scene", scene_duplicate)
            .describe("Copy a scene (tracks, takes, sound clips, camera) right after itself. index defaults to the open scene; open true opens the copy.")
            .with_schema(schema!(r#""properties":{"index":{"type":"integer","minimum":0},"name":{"type":"string"},"open":{"type":"boolean"}}"#)),
        CommandSpec::new("scene.remove", "Delete scene", scene_remove)
            .describe("Delete a scene. The project keeps at least one.")
            .with_schema(schema!(r#""required":["index"],"properties":{"index":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("scene.select", "Open scene", scene_select)
            .describe("Open another scene of the project: the track, take, audio and camera commands then work on it.")
            .with_schema(schema!(r#""required":["index"],"properties":{"index":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("scene.move", "Reorder scenes", scene_move)
            .describe("Move a scene to another position in the project's scene list.")
            .with_schema(schema!(r#""required":["from","to"],"properties":{"from":{"type":"integer","minimum":0},"to":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("track.move", "Reorder puppet track", track_move)
            .describe("Move a puppet track to another position. Later tracks draw in front of earlier ones.")
            .with_schema(schema!(r#""required":["from","to"],"properties":{"from":{"type":"integer","minimum":0},"to":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("take.trim", "Trim take", take_trim)
            .describe("Cut a take's front (start_seconds) and/or back (end_seconds), in scene seconds. The recording itself stays where it was recorded (nothing is thrown away), the take just plays a shorter stretch of it. A trim never lengthens a take.")
            .with_schema(schema!(r#""required":["track","take"],"properties":{"track":{"type":"integer","minimum":0},"take":{"type":"integer","minimum":0},"start_seconds":{"type":"number"},"end_seconds":{"type":"number"}}"#)),
        CommandSpec::new("take.split", "Split take", take_split)
            .describe("Cut a take in two at a scene time. Both halves play exactly as the whole did; the second becomes a new take right after the first.")
            .with_schema(schema!(r#""required":["track","take","time_seconds"],"properties":{"track":{"type":"integer","minimum":0},"take":{"type":"integer","minimum":0},"time_seconds":{"type":"number"}}"#)),
        CommandSpec::new("audio.add_track", "Add audio track", audio_add_track)
            .describe("Add an empty audio track.")
            .with_schema(schema!(r#""properties":{"name":{"type":"string"}}"#)),
        CommandSpec::new("audio.remove_track", "Remove audio track", audio_remove_track)
            .describe("Delete an audio track and its clips.")
            .with_schema(schema!(r#""required":["track"],"properties":{"track":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("audio.move_clip", "Move audio clip", audio_move_clip)
            .describe("Move a clip to another time (start_seconds) and/or another audio track (to_track).")
            .with_schema(schema!(r#""required":["clip"],"properties":{"track":{"type":"integer","minimum":0},"clip":{"type":"integer","minimum":0},"to_track":{"type":"integer","minimum":0},"start_seconds":{"type":"number","minimum":0}}"#)),
        CommandSpec::new("audio.split_clip", "Split audio clip", audio_split_clip)
            .describe("Cut an audio clip in two at a scene time. media_seconds (the sound's length) is needed when the clip plays to its end.")
            .with_schema(schema!(r#""required":["clip","time_seconds"],"properties":{"track":{"type":"integer","minimum":0},"clip":{"type":"integer","minimum":0},"time_seconds":{"type":"number"},"media_seconds":{"type":"number","minimum":0}}"#)),
        CommandSpec::new("keyframe.list", "List keyframes", keyframe_list)
            .transient()
            .describe("The keyframed parameters of a puppet track (placement fields and behavior parameters) or, without 'track', of the camera, with every key's time, value and ease, and the targets that can be animated.")
            .with_schema(schema!(r#""properties":{"track":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("keyframe.set", "Set keyframe", keyframe_set)
            .describe("Put a key on a parameter at a scene time (replacing the key there), or edit key 'index' (new value, ease, or a new time). track selects a puppet track; omit it for the camera. Targets: position_x, position_y, anchor_x, anchor_y, scale_x, scale_y, rotation (degrees), opacity (0..1), behavior.<slot>.<parameter> (a number parameter of a behavior); camera: position_x, position_y, zoom, rotation. ease: linear, hold, ease_in, ease_out, ease_in_out or {kind:'bezier',x1,y1,x2,y2}; it shapes the motion from this key to the next.")
            .with_schema(schema!(r#""required":["target"],"properties":{"track":{"type":"integer","minimum":0},"target":{"type":"string"},"time_seconds":{"type":"number","minimum":0},"value":{"type":"number"},"ease":{},"index":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("keyframe.remove", "Remove keyframe", keyframe_remove)
            .describe("Delete key 'index' of a parameter, or the whole parameter's keys when no index is given.")
            .with_schema(schema!(r#""required":["target"],"properties":{"track":{"type":"integer","minimum":0},"target":{"type":"string"},"index":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("keyframe.clear", "Clear keyframes", keyframe_clear)
            .describe("Delete every keyframe of a puppet track (or of the camera without 'track').")
            .with_schema(schema!(r#""properties":{"track":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("camera.set", "Scene camera", camera_set)
            .describe("Pan (position_x, position_y: the point looked at, pixels from the middle of the frame), zoom (1 = 100%), rotate (degrees) the scene camera, or make it follow a face: zoom_on_face {track, amount 0..1, zoom} (null stops following). reset true goes back to the neutral camera first. Use keyframe.set without 'track' to animate the camera.")
            .with_schema(schema!(r#""properties":{"position_x":{"type":"number"},"position_y":{"type":"number"},"zoom":{"type":"number","exclusiveMinimum":0},"rotation":{"type":"number"},"zoom_on_face":{"type":["object","null"]},"reset":{"type":"boolean"}}"#)),
        CommandSpec::new("timing.list", "Take timing", timing_list)
            .transient()
            .describe("What happens when inside a take: the mouth shapes of a voice take as runs (shape, start, end) and the key presses of a keys take (key, down, up).")
            .with_schema(schema!(r#""required":["track","take"],"properties":{"track":{"type":"integer","minimum":0},"take":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("timing.set_viseme", "Edit mouth shape run", timing_set_viseme)
            .describe("Change the mouth shape of run 'index' of a voice take (viseme, or none) and/or when it starts (start_seconds; the run before it then ends there).")
            .with_schema(schema!(r#""required":["track","take","index"],"properties":{"track":{"type":"integer","minimum":0},"take":{"type":"integer","minimum":0},"index":{"type":"integer","minimum":0},"viseme":{"type":"string"},"start_seconds":{"type":"number"}}"#)),
        CommandSpec::new("timing.insert_viseme", "Insert mouth shape", timing_insert_viseme)
            .describe("Put a mouth shape on a stretch of a voice take, replacing what was there.")
            .with_schema(schema!(r#""required":["track","take","start_seconds","end_seconds","viseme"],"properties":{"track":{"type":"integer","minimum":0},"take":{"type":"integer","minimum":0},"start_seconds":{"type":"number"},"end_seconds":{"type":"number"},"viseme":{"type":"string"}}"#)),
        CommandSpec::new("timing.remove_viseme", "Remove mouth shape run", timing_remove_viseme)
            .describe("Delete run 'index' of a voice take: the run before it holds longer (the first run becomes empty).")
            .with_schema(schema!(r#""required":["track","take","index"],"properties":{"track":{"type":"integer","minimum":0},"take":{"type":"integer","minimum":0},"index":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("timing.set_key", "Edit key press", timing_set_key)
            .describe("Retime key run 'index' of a keys take: when it goes down (start_seconds), when it goes up (end_seconds), or which key it is.")
            .with_schema(schema!(r#""required":["track","take","index"],"properties":{"track":{"type":"integer","minimum":0},"take":{"type":"integer","minimum":0},"index":{"type":"integer","minimum":0},"start_seconds":{"type":"number"},"end_seconds":{"type":"number"},"key":{"type":"string"}}"#)),
        CommandSpec::new("timing.add_key", "Add key press", timing_add_key)
            .describe("Add a key press (a trigger) to a keys take.")
            .with_schema(schema!(r#""required":["track","take","key","start_seconds","end_seconds"],"properties":{"track":{"type":"integer","minimum":0},"take":{"type":"integer","minimum":0},"key":{"type":"string"},"start_seconds":{"type":"number"},"end_seconds":{"type":"number"}}"#)),
        CommandSpec::new("timing.remove_key", "Remove key press", timing_remove_key)
            .describe("Delete key run 'index' of a keys take.")
            .with_schema(schema!(r#""required":["track","take","index"],"properties":{"track":{"type":"integer","minimum":0},"take":{"type":"integer","minimum":0},"index":{"type":"integer","minimum":0}}"#)),
        CommandSpec::new("lipsync.from_clip", "Compute lip sync from scene audio", lipsync_from_clip)
            .describe("Make a voice take on puppet track 'track' from the mouth shapes of a clip already on an audio track of the scene (Compute Lip Sync Take from Scene Audio). The take starts where the clip does and respects its trim. A clip that has not been analysed needs 'keys' (time_seconds from the sound's start, viseme) or lipsync.record with the sound file.")
            .with_schema(schema!(r#""required":["clip"],"properties":{"track":{"type":"integer","minimum":0},"audio_track":{"type":"integer","minimum":0},"clip":{"type":"integer","minimum":0},"keys":{"type":"array","items":{"type":"object"}},"media_seconds":{"type":"number","minimum":0}}"#)),
    ]
}

#[cfg(test)]
mod tests {
    use anim_core::FrameRate;
    use anim_session::Session;

    use super::*;
    use crate::group::ParamGroup;
    use crate::scene::AudioClip;
    use crate::take::Take;

    fn session() -> Session<RecordDoc> {
        let mut scene = Scene::new("one", anim_core::Size::new(640.0, 360.0), FrameRate::FPS_30, Tick::from_seconds_f64(6.0));
        scene.add_track(PuppetTrack::new("Hero", "p")).unwrap();
        scene.add_track(PuppetTrack::new("Sidekick", "q")).unwrap();
        Session::new(RecordDoc::new(scene), crate::commands::registry().unwrap())
    }

    #[expect(clippy::needless_pass_by_value, reason = "call sites pass json! literals")]
    fn run(s: &mut Session<RecordDoc>, id: &str, params: Value) -> Value {
        s.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    #[test]
    fn scenes_are_commands_and_undo() {
        let mut s = session();
        run(&mut s, "scene.add", json!({ "name": "two", "puppet": "p" }));
        assert_eq!((s.doc().scene.name.as_str(), s.doc().active_scene(), s.doc().scene_count()), ("two", 1, 2));
        assert_eq!(s.doc().scene.tracks.len(), 1);
        let listed = run(&mut s, "scene.list", json!({}));
        assert_eq!(listed["scenes"].as_array().unwrap().len(), 2);
        run(&mut s, "scene.select", json!({ "index": 0 }));
        assert_eq!(s.doc().scene.tracks.len(), 2, "the first scene kept its tracks");
        run(&mut s, "scene.duplicate", json!({ "open": true }));
        assert_eq!(s.doc().scene.name, "one copy");
        s.undo().unwrap();
        s.undo().unwrap();
        assert_eq!((s.doc().scene.name.as_str(), s.doc().scene_count()), ("two", 2), "undo restores the open scene too");
        assert!(s.execute("scene.remove", &json!({ "index": 9 })).is_err());
    }

    #[test]
    fn tracks_reorder_and_lock() {
        let mut s = session();
        run(&mut s, "track.move", json!({ "from": 0, "to": 1 }));
        assert_eq!(s.doc().scene.tracks[1].name, "Hero");
        run(&mut s, "track.set", json!({ "track": 1, "locked": true }));
        assert!(s.execute("track.set", &json!({ "track": 1, "name": "x" })).is_err(), "locked tracks refuse edits");
        assert!(s.execute("keyframe.set", &json!({ "track": 1, "target": "opacity", "time_seconds": 0.0, "value": 0.5 })).is_err());
        run(&mut s, "track.set", json!({ "track": 1, "locked": false }));
        run(&mut s, "track.set", json!({ "track": 1, "name": "x" }));
    }

    #[test]
    fn takes_trim_and_split_through_commands() {
        let mut s = session();
        let take = Take::new(0, [ParamGroup::Head].into_iter().collect(), Tick::ZERO, Tick::from_seconds_f64(4.0));
        s.edit("add take", None, |d| d.scene.add_take(0, take)).unwrap();
        run(&mut s, "take.trim", json!({ "track": 0, "take": 0, "start_seconds": 1.0, "end_seconds": 3.0 }));
        let t = &s.doc().scene.tracks[0].takes[0];
        assert_eq!((t.start, t.end()), (Tick::from_seconds_f64(1.0), Tick::from_seconds_f64(3.0)));
        run(&mut s, "take.split", json!({ "track": 0, "take": 0, "time_seconds": 2.0 }));
        assert_eq!(s.doc().scene.tracks[0].takes.len(), 2);
        assert!(s.execute("take.split", &json!({ "track": 0, "take": 0, "time_seconds": 5.0 })).is_err());
        s.undo().unwrap();
        assert_eq!(s.doc().scene.tracks[0].takes.len(), 1);
    }

    #[test]
    fn keyframes_set_edit_and_remove() {
        let mut s = session();
        run(&mut s, "keyframe.set", json!({ "track": 0, "target": "position_x", "time_seconds": 0.0, "value": 0.0 }));
        run(&mut s, "keyframe.set", json!({ "track": 0, "target": "position_x", "time_seconds": 2.0, "value": 100.0, "ease": "ease_in_out" }));
        let t = &s.doc().scene.tracks[0];
        assert!((t.placement_at(Tick::from_seconds_f64(1.0)).position.x - 50.0).abs() < 1e-6);
        run(
            &mut s,
            "keyframe.set",
            json!({ "track": 0, "target": "position_x", "index": 0, "ease": { "kind": "bezier", "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 0.0 } }),
        );
        run(&mut s, "keyframe.set", json!({ "track": 0, "target": "position_x", "index": 1, "time_seconds": 4.0 }));
        assert_eq!(s.doc().scene.tracks[0].lanes[0].keys[1].t, Tick::from_seconds_f64(4.0));
        assert!(s.execute("keyframe.set", &json!({ "track": 0, "target": "nonsense", "time_seconds": 0.0, "value": 1.0 })).is_err());
        assert!(s.execute("keyframe.set", &json!({ "target": "rotation", "time_seconds": 0.0, "value": 1.0 })).is_ok(), "the camera can rotate");
        assert!(s.execute("keyframe.set", &json!({ "target": "opacity", "time_seconds": 0.0, "value": 1.0 })).is_err(), "the camera has no opacity");
        run(&mut s, "keyframe.remove", json!({ "track": 0, "target": "position_x", "index": 0 }));
        run(&mut s, "keyframe.remove", json!({ "track": 0, "target": "position_x", "index": 0 }));
        assert!(s.doc().scene.tracks[0].lanes.is_empty(), "a lane without keys goes away");
    }

    #[test]
    fn behavior_parameters_can_be_keyframed() {
        let mut s = session();
        let info = run(&mut s, "keyframe.list", json!({ "track": 0 }));
        assert!(info["targets"].as_array().unwrap().len() >= 8);
        let ok = s.execute("keyframe.set", &json!({ "track": 0, "target": "behavior.0.nonexistent_param", "time_seconds": 0.0, "value": 1.0 }));
        assert!(ok.is_err(), "unknown parameters are refused");
        let stack = crate::playback::track_stack(&s.doc().scene, &s.doc().templates, 0, &s.doc().scene.tracks[0]).unwrap();
        let params = stack
            .slots()
            .iter()
            .enumerate()
            .find_map(|(i, slot)| slot.behavior.params().as_object().and_then(|m| m.iter().find(|(_, v)| v.is_number()).map(|(k, _)| (i, k.clone()))));
        let (slot, name) = params.expect("some behavior has a numeric parameter");
        let target = format!("behavior.{slot}.{name}");
        run(&mut s, "keyframe.set", json!({ "track": 0, "target": target, "time_seconds": 0.0, "value": 1.0 }));
        run(&mut s, "keyframe.set", json!({ "track": 0, "target": target, "time_seconds": 1.0, "value": 3.0 }));
        let mut ev = s.doc().evaluator().unwrap();
        assert!(ev.pose(0, 45).is_ok());
    }

    #[test]
    fn the_camera_is_a_command() {
        let mut s = session();
        run(&mut s, "camera.set", json!({ "zoom": 2.0, "position_x": 30.0, "zoom_on_face": { "track": 1, "amount": 0.5, "zoom": 3.0 } }));
        let c = &s.doc().scene.camera;
        assert_eq!((c.zoom, c.position.x), (2.0, 30.0));
        assert_eq!(c.zoom_on_face.unwrap().track, 1);
        run(&mut s, "camera.set", json!({ "zoom_on_face": null }));
        assert!(s.doc().scene.camera.zoom_on_face.is_none());
        assert!(s.execute("camera.set", &json!({ "zoom_on_face": { "track": 7 } })).is_err());
        run(&mut s, "camera.set", json!({ "reset": true }));
        assert!(s.doc().scene.camera.is_neutral());
    }

    #[test]
    fn audio_clips_move_and_split() {
        let mut s = session();
        let mut clip = AudioClip::new("v", "v", Tick::from_seconds_f64(1.0));
        clip.duration = Some(Tick::from_seconds_f64(4.0));
        s.edit("add clip", None, |d| d.scene.add_clip(0, clip)).unwrap();
        run(&mut s, "audio.add_track", json!({ "name": "Music" }));
        run(&mut s, "audio.move_clip", json!({ "clip": 0, "to_track": 1, "start_seconds": 2.0 }));
        assert!(s.doc().scene.audio[0].clips.is_empty());
        assert_eq!(s.doc().scene.audio[1].clips[0].start, Tick::from_seconds_f64(2.0));
        run(&mut s, "audio.split_clip", json!({ "track": 1, "clip": 0, "time_seconds": 3.0 }));
        let clips = &s.doc().scene.audio[1].clips;
        assert_eq!(clips.len(), 2);
        assert_eq!(
            (clips[0].duration, clips[1].start, clips[1].trim_in),
            (Some(Tick::from_seconds_f64(1.0)), Tick::from_seconds_f64(3.0), Tick::from_seconds_f64(1.0))
        );
        assert_eq!(clips[1].duration, Some(Tick::from_seconds_f64(3.0)));
        assert!(s.execute("audio.split_clip", &json!({ "track": 1, "clip": 0, "time_seconds": 9.0 })).is_err());
        run(&mut s, "audio.remove_track", json!({ "track": 1 }));
        assert_eq!(s.doc().scene.audio.len(), 1);
    }

    #[test]
    fn lip_sync_comes_from_a_clip_and_timing_edits_it() {
        let mut s = session();
        let mut clip = AudioClip::new("v", "v", Tick::from_seconds_f64(1.0));
        clip.duration = Some(Tick::from_seconds_f64(2.0));
        s.edit("add clip", None, |d| d.scene.add_clip(0, clip)).unwrap();
        assert!(s.execute("lipsync.from_clip", &json!({ "clip": 0 })).is_err(), "unanalysed clips are refused");
        let done = run(
            &mut s,
            "lipsync.from_clip",
            json!({ "clip": 0, "keys": [
            { "time_seconds": 0.0, "viseme": "Neutral" }, { "time_seconds": 0.5, "viseme": "Ah" }, { "time_seconds": 1.0, "viseme": "Oh" } ] }),
        );
        assert_eq!(done["take"]["start_seconds"], 1.0);
        assert_eq!(s.doc().scene.audio[0].clips[0].visemes.len(), 3, "the analysis stays with the clip");
        run(&mut s, "lipsync.from_clip", json!({ "clip": 0 }));
        let timing = run(&mut s, "timing.list", json!({ "track": 0, "take": 0 }));
        assert_eq!(timing["voice"].as_array().unwrap().len(), 3);
        run(&mut s, "timing.set_viseme", json!({ "track": 0, "take": 0, "index": 1, "viseme": "Ee", "start_seconds": 1.25 }));
        let timing = run(&mut s, "timing.list", json!({ "track": 0, "take": 0 }));
        assert_eq!(timing["voice"][1]["viseme"], "Ee");
        assert_eq!(timing["voice"][1]["start_seconds"], 1.25);
        run(&mut s, "timing.insert_viseme", json!({ "track": 0, "take": 0, "start_seconds": 2.0, "end_seconds": 2.25, "viseme": "M" }));
        run(&mut s, "timing.remove_viseme", json!({ "track": 0, "take": 0, "index": 1 }));
        assert!(s.execute("timing.set_viseme", &json!({ "track": 0, "take": 0, "index": 1, "viseme": "bogus" })).is_err());
    }

    #[test]
    fn key_timing_is_editable() {
        let mut s = session();
        let mut take = Take::new(0, [ParamGroup::Keys].into_iter().collect(), Tick::ZERO, Tick::from_seconds_f64(4.0));
        let held = |keys: &[&str]| crate::group::Frame { keys: keys.iter().map(|k| (*k).to_owned()).collect(), ..Default::default() };
        for (t, keys) in [(0.0, vec![]), (1.0, vec!["a"]), (2.0, vec![])] {
            take.samples.push(crate::take::Sample { t: Tick::from_seconds_f64(t), frame: held(&keys) });
        }
        s.edit("add take", None, |d| d.scene.add_take(0, take)).unwrap();
        run(&mut s, "timing.set_key", json!({ "track": 0, "take": 0, "index": 0, "start_seconds": 0.5, "end_seconds": 3.0 }));
        run(&mut s, "timing.add_key", json!({ "track": 0, "take": 0, "key": "b", "start_seconds": 1.0, "end_seconds": 1.5 }));
        let timing = run(&mut s, "timing.list", json!({ "track": 0, "take": 0 }));
        assert_eq!(timing["keys"].as_array().unwrap().len(), 2);
        run(&mut s, "timing.remove_key", json!({ "track": 0, "take": 0, "index": 0 }));
        assert_eq!(run(&mut s, "timing.list", json!({ "track": 0, "take": 0 }))["keys"].as_array().unwrap().len(), 1);
    }
}
