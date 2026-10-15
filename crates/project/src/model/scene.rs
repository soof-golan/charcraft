//! Scenes: settings, tracks, track items with their behavior instances and takes, and markers.

use charcraft_ltab::LValue;
use serde::Serialize;

use super::util::{boolean, boolean_or, id_items, int, number, opt_number, ppath, string};

/// A time span on a timeline. All times are seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct TimeItem {
    /// Length on the timeline.
    pub duration: f64,
    /// Offset from the parent's start.
    pub parent_offset: f64,
    /// Start of the used part of the source.
    pub trim_in: f64,
    /// End of the used part of the source.
    pub trim_out: f64,
}

#[must_use]
pub fn time_item(value: Option<&LValue>) -> Option<TimeItem> {
    let value = value?;
    Some(TimeItem {
        duration: number(value, "_durationTime"),
        parent_offset: number(value, "_parentOffsetTime"),
        trim_in: number(value, "_trimInTime"),
        trim_out: number(value, "_trimOutTime"),
    })
}

/// One recorded take of a behavior parameter (an `EventGraphPileItem_`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Take {
    /// Id in the pile (`egPile2`).
    pub id: String,
    /// Display name (`take 1 - Audio Input`).
    pub name: Option<String>,
    /// The parameter recorded.
    pub param: Option<String>,
    /// The take number.
    pub take_id: i64,
    /// Whether it plays.
    pub enabled: bool,
    /// Solo.
    pub solo: bool,
    /// Placement on the timeline.
    pub time: Option<TimeItem>,
    /// The `.arf` event graph the data comes from.
    pub source: Option<String>,
}

/// A behavior on a track item with the values this scene gives its parameters.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BehaviorInstance {
    /// Key in the behavior bag (`BB1`).
    pub key: String,
    /// The behavior (`Adobe.LipSync2.js`).
    pub behavior: String,
    /// The bound behavior of the puppet this belongs to.
    pub bound_id: Option<String>,
    /// Instance id (`TIB33`).
    pub instance_id: Option<String>,
    /// Switched on.
    pub enabled: bool,
    /// Armed for recording.
    pub armed: bool,
    /// Parameter overrides, in file order.
    pub params: Vec<(String, LValue)>,
    /// Parameters that carry keyframes.
    pub keyframed: Vec<String>,
    /// Recorded takes.
    pub takes: Vec<Take>,
}

/// A clip on a track: a puppet instance or an audio file placed on the timeline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackItem {
    /// Id on the track (`idaTrackItems1`).
    pub id: String,
    /// The project item it plays (`/ProjectItems/desk`).
    pub source: Option<String>,
    /// Placement.
    pub time: Option<TimeItem>,
    /// The behaviors of the instance.
    pub behaviors: Vec<BehaviorInstance>,
    /// How many child instances it has.
    pub child_count: usize,
}

/// A track of a scene.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Track {
    /// Id (`idaTracks2`).
    pub id: String,
    /// Position in the list.
    pub index: i64,
    /// The kind (`trackKind_puppet`, `trackKind_audio`, ...).
    pub kind: Option<String>,
    /// Enabled.
    pub enabled: bool,
    /// Armed for recording.
    pub armed: bool,
    /// Visible in the UI.
    pub visible: bool,
    /// Label colour as stored.
    pub color: Option<i64>,
    /// Most items the track holds (`None` for unlimited).
    pub max_items: Option<f64>,
    /// The clips.
    pub items: Vec<TrackItem>,
}

/// A timeline marker.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Marker {
    /// Id (`idaMarkers1`).
    pub id: String,
    /// The text.
    pub name: Option<String>,
    /// Stops playback.
    pub stop: bool,
    /// Placement.
    pub time: Option<TimeItem>,
}

/// A link from a track item's take to its stored keyframes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordingRef {
    /// Store path of the `recordingRef` file.
    pub path: String,
    /// The bound behavior.
    pub bound_behavior: Option<String>,
    /// The take in the pile.
    pub pile_item: Option<String>,
    /// The track item.
    pub track_item: Option<String>,
    /// How many `KeyframeBucket` files sit beside it.
    pub buckets: usize,
}

/// One keyframe of a recorded parameter.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Keyframe {
    /// Time in seconds, relative to the bucket's take.
    pub time: f64,
    /// The value (a number for most parameters).
    pub value: LValue,
}

/// One stored chunk of keyframes (`KeyframeBucket{...}`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeyframeBucket {
    /// Store path of the bucket file.
    pub path: String,
    /// The bucket number (the file name).
    pub index: i64,
    /// Edited by the user after recording.
    pub edited: bool,
    /// Keyframes in time order as stored.
    pub keyframes: Vec<Keyframe>,
}

/// Decodes a `KeyframeBucket{...}` file.
#[must_use]
pub fn decode_keyframe_bucket(path: &str, index: i64, value: &LValue) -> KeyframeBucket {
    KeyframeBucket {
        path: path.to_owned(),
        index,
        edited: boolean(value, "_bEdited"),
        keyframes: value
            .get("_aKeyframes")
            .map(|list| list.iter().filter_map(|(_, k)| Some(Keyframe { time: k.get("t")?.as_f64()?, value: k.get("v")?.clone() })).collect())
            .unwrap_or_default(),
    }
}

/// A scene: size, rate and length, its tracks and markers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Scene {
    /// Folder in the store (`ProjectItems/anchors_01`).
    pub path: String,
    /// Display name.
    pub name: String,
    /// Guid.
    pub guid: Option<String>,
    /// Frames per second.
    pub fps: f64,
    /// Width in pixels.
    pub width: i64,
    /// Height in pixels.
    pub height: i64,
    /// Length in seconds.
    pub duration: f64,
    /// Origin at the centre.
    pub centered_origin: bool,
    /// Playback speed factor.
    pub playback_speed: f64,
    /// Recording speed factor.
    pub recording_speed: f64,
    /// Physics simulation rate factor.
    pub simulation_rate_factor: f64,
    /// Smart replays on.
    pub smart_replays: bool,
    /// Work area in point, seconds.
    pub work_area_in: f64,
    /// Work area out point, seconds.
    pub work_area_out: f64,
    /// Work area active.
    pub work_area_active: bool,
    /// Next take number.
    pub take_counter: i64,
    /// The scene puppet's track item.
    pub scene_puppet: Option<String>,
    /// Tracks.
    pub tracks: Vec<Track>,
    /// Markers.
    pub markers: Vec<Marker>,
    /// Links to recorded streams.
    pub recordings: Vec<RecordingRef>,
}

impl Scene {
    /// Length in whole frames, rounded up.
    #[must_use]
    pub fn frame_count(&self) -> i64 {
        if self.fps <= 0.0 || !self.duration.is_finite() || self.duration < 0.0 {
            return 0;
        }
        anim_core::num::round_f64_to_i64((self.duration * self.fps).ceil())
    }

    /// Every behavior instance across all tracks.
    pub fn behavior_instances(&self) -> impl Iterator<Item = &BehaviorInstance> {
        self.tracks.iter().flat_map(|t| &t.items).flat_map(|i| &i.behaviors)
    }
}

/// Decodes `SceneData{...}` for the scene stored in folder `path`.
#[must_use]
pub fn decode_scene(path: &str, data: &LValue) -> Scene {
    Scene {
        path: path.to_owned(),
        name: string(data, "_sUiName").unwrap_or_else(|| super::util::last_segment(path).to_owned()),
        guid: string(data, "_sGuid"),
        fps: number(data, "_framerate"),
        width: int(data, "_pixWidth"),
        height: int(data, "_pixHeight"),
        duration: number(data, "_sceneDurationSec"),
        centered_origin: boolean(data, "_centeredOrigin"),
        playback_speed: opt_number(data, "_playbackSpeed").unwrap_or(1.0),
        recording_speed: opt_number(data, "_recordingSpeed").unwrap_or(1.0),
        simulation_rate_factor: opt_number(data, "_simulationRateFactor").unwrap_or(1.0),
        smart_replays: boolean(data, "_bSmartReplays"),
        work_area_in: number(data, "_workAreaIn"),
        work_area_out: number(data, "_workAreaOut"),
        work_area_active: boolean(data, "_bWorkAreaActive"),
        take_counter: int(data, "_takeCounter"),
        scene_puppet: ppath(data, "_scenePuppetTrackItemCPath"),
        tracks: Vec::new(),
        markers: Vec::new(),
        recordings: Vec::new(),
    }
}

/// Decodes a `ProjectUIData_001{ i = n, v = Track{...} }` file; `id` is the file name.
#[must_use]
pub fn decode_track(id: &str, doc: &LValue) -> Option<Track> {
    let track = doc.get("v")?;
    let items = track
        .get("_idaTrackItems")
        .map(|array| {
            id_items(array)
                .into_iter()
                .map(|(item_id, item)| {
                    let root = item.get("_rootTrackItemPuppet");
                    TrackItem {
                        id: item_id,
                        source: ppath(item, "_trackSource"),
                        time: time_item(item.get("_timeItem")),
                        behaviors: root
                            .and_then(|r| r.get("_behaviorBag"))
                            .map(|bag| bag.named().map(|(key, b)| behavior_instance(key, b)).collect())
                            .unwrap_or_default(),
                        child_count: root.and_then(|r| r.get("_children")).map_or(0, |c| c.iter().count()),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Track {
        id: id.to_owned(),
        index: int(doc, "i"),
        kind: string(track, "_kind"),
        enabled: boolean_or(track, "_bEnabled", true),
        armed: boolean(track, "_bArmedForRecord"),
        visible: boolean_or(track, "_bUserVisible", true),
        color: track.get("_uiColor").and_then(LValue::as_i64),
        max_items: opt_number(track, "_maxTrackItems").filter(|m| m.is_finite()),
        items,
    })
}

fn behavior_instance(key: &str, value: &LValue) -> BehaviorInstance {
    BehaviorInstance {
        key: key.to_owned(),
        behavior: string(value, "_behaviorID").unwrap_or_default(),
        bound_id: string(value, "_boundBehaviorArrayID"),
        instance_id: string(value, "_instanceID"),
        enabled: boolean_or(value, "_bEnabled", true),
        armed: boolean(value, "_bArmedForRecord"),
        params: value.get("_paramBag").map(|p| p.named().map(|(k, v)| (k.to_owned(), v.clone())).collect()).unwrap_or_default(),
        keyframed: value
            .get("_paramIsKeyframedBag")
            .map(|p| p.named().filter(|(_, on)| on.as_bool() == Some(true)).map(|(k, _)| k.to_owned()).collect())
            .unwrap_or_default(),
        takes: value
            .get("_egPile")
            .and_then(|p| p.get("_idaPileItems"))
            .map(|array| {
                id_items(array)
                    .into_iter()
                    .map(|(id, t)| Take {
                        id,
                        name: string(t, "_uiName"),
                        param: string(t, "_paramId"),
                        take_id: int(t, "_takeID"),
                        enabled: boolean_or(t, "_bEnabled", true),
                        solo: boolean(t, "_bSolo"),
                        time: time_item(t.get("_timeItem")),
                        source: ppath(t, "_sourceEventGraphPPath"),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Decodes a marker file `ProjectUIData_001{ i = n, v = Marker_001{...} }`.
#[must_use]
pub fn decode_marker(id: &str, doc: &LValue) -> Option<Marker> {
    let marker = doc.get("v")?;
    Some(Marker { id: id.to_owned(), name: string(marker, "_uiName"), stop: boolean(marker, "_bStop"), time: time_item(marker.get("_timeItem")) })
}

/// Decodes `RecordingRef_001{...}`.
#[must_use]
pub fn decode_recording_ref(path: &str, value: &LValue, buckets: usize) -> RecordingRef {
    RecordingRef {
        path: path.to_owned(),
        bound_behavior: string(value, "boundBehaviorArrayID"),
        pile_item: string(value, "pileItemID"),
        track_item: ppath(value, "trackItemCPath"),
        buckets,
    }
}
