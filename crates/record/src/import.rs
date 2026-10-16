//! Reading a scene out of a Character Animator project: tracks, placements, recorded takes and sound clips.
//!
//! What imports is the stage (size, rate, length), one track per puppet clip with the Transform behavior's
//! placement and the clip's recorded takes, and the audio clips. The takes are the project's `.arf` event graphs
//! ([`crate::recorded`]): the face (Camera Input), the voice (Audio Input) and dragged handles (Handle), each
//! placed on the track the way the project's timeline has them.

use std::collections::BTreeMap;

use anim_core::{Affine, Size, Tick, Vec2};
use charcraft_arf::EventGraph;
use charcraft_ltab::LValue;
use charcraft_project::scene::TrackItem;
use charcraft_project::{Model, Project, PuppetItem};

use crate::audio::{MediaStore, valid_media_name};
use crate::error::{Error, Result};
use crate::recorded::{self, HandleBinding, Meta, Window};
use crate::scene::{AudioClip, AudioTrack, Placement, PuppetTrack, Scene};
use crate::stage::{origin_of, skin_of};
use crate::take::Take;

/// What to do with the takes of a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TakePolicy {
    /// Play every recorded take. A project can be saved with its takes muted (the news anchors sample is: all 42 of
    /// them), and a scene that opens silent is not what anyone who imports a recorded performance wants.
    #[default]
    Perform,
    /// Keep each take's own mute switch.
    AsStored,
    /// Leave the recorded takes out.
    Skip,
}

/// How to import a scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportOptions {
    /// What to do with the recorded takes.
    pub takes: TakePolicy,
}

/// A scene read from a project and what it needs from the rest of the project.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// The scene.
    pub scene: Scene,
    /// Store paths of the puppets the tracks use (the keys for [`Cast::from_project`](crate::Cast::from_project)).
    pub puppets: Vec<String>,
    /// `(media name, store path)` of the sound files the clips use.
    pub sounds: Vec<(String, String)>,
    /// Things that did not import.
    pub warnings: Vec<String>,
}

fn strip(path: &str) -> &str {
    path.trim_start_matches('/')
}

/// Reads a Transform behavior's parameters into a placement; absent ones keep their defaults.
#[must_use]
pub fn placement_from_params(params: &[(String, LValue)]) -> Placement {
    let get = |name: &str| params.iter().find(|(k, _)| k == name).and_then(|(_, v)| v.as_f64()).filter(|v| v.is_finite());
    let uniform = get("uniformScale").unwrap_or(100.0) / 100.0;
    Placement {
        position: Vec2::new(get("positionX").unwrap_or(0.0), get("positionY").unwrap_or(0.0)),
        anchor: Vec2::new(get("anchorPtX").unwrap_or(0.0), get("anchorPtY").unwrap_or(0.0)),
        scale: Vec2::new(uniform * get("scaleX").unwrap_or(100.0) / 100.0, uniform * get("scaleY").unwrap_or(100.0) / 100.0),
        rotation: get("rotation").unwrap_or(0.0),
        opacity: (get("opacity").unwrap_or(100.0) / 100.0).clamp(0.0, 1.0),
    }
}

/// A media name made from a file name: unsupported characters become `_`.
fn media_name(file_name: &str) -> String {
    let mut name: String = file_name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-') { c } else { '_' }).take(64).collect();
    if !name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) {
        name.insert(0, 'm');
        name = name.chars().take(64).collect();
    }
    name
}

/// The graphs read so far, by project path (`None`: the file could not be read, already reported).
type Graphs = BTreeMap<String, Option<EventGraph>>;

fn last_segment(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

/// The key of the rig handle that a recording's handle address names: `HandleTarget/../B2/B8/B4:/Draggable/` is
/// the `Draggable` handle of the puppet reached through bindings `B2`, `B8` and `B4` of the track's puppet, and the
/// rig keys its handles `<name of that puppet>/<handle>`.
fn handle_key(model: &Model, root: &PuppetItem, group: &str) -> Option<String> {
    let (chain, handle) = group.strip_prefix("HandleTarget/../")?.split_once(":/")?;
    let mut item = root;
    let mut label = None;
    let mut ids = chain.split('/').peekable();
    while let Some(id) = ids.next() {
        let ppath = item.binding(id)?.ppath.as_deref()?;
        label = Some(last_segment(ppath));
        if ids.peek().is_some() {
            item = model.puppet(strip(ppath))?;
        }
    }
    Some(format!("{}/{}", label?, handle.trim_end_matches('/')))
}

fn read_graph<'g>(project: &Project, model: &Model, graphs: &'g mut Graphs, ppath: &str, warnings: &mut Vec<String>) -> Option<&'g EventGraph> {
    if !graphs.contains_key(ppath) {
        let loaded = (|| {
            let file = model.recording(ppath).and_then(|r| r.file.as_ref()).ok_or_else(|| "the recording is not in the project".to_owned())?;
            let bytes = project.package().read(&file.path).map_err(|e| e.to_string())?;
            EventGraph::from_bytes(&bytes).map_err(|e| e.to_string())
        })();
        match loaded {
            Ok(graph) => {
                graphs.insert(ppath.to_owned(), Some(graph));
            }
            Err(e) => {
                warnings.push(format!("{ppath}: {e}"));
                graphs.insert(ppath.to_owned(), None);
            }
        }
    }
    graphs.get(ppath)?.as_ref()
}

/// The recorded takes of one clip, lowest take number first.
#[allow(clippy::too_many_arguments, reason = "everything one clip's takes depend on, passed as it is")]
fn clip_takes(
    project: &Project,
    model: &Model,
    clip: &TrackItem,
    item: &PuppetItem,
    placement: &Placement,
    stage: Size,
    policy: TakePolicy,
    graphs: &mut Graphs,
    warnings: &mut Vec<String>,
) -> Vec<Take> {
    let mut takes: Vec<Take> = Vec::new();
    if policy == TakePolicy::Skip {
        return takes;
    }
    // Scene pixels (from the middle of the frame, y down) to the puppet's own space, for dragged handles.
    let centre = Vec2::new(stage.width / 2.0, stage.height / 2.0);
    let matrix = placement.matrix(centre, skin_of(item), origin_of(item));
    let to_puppet = (matrix.determinant().abs() > 1e-9).then(|| matrix.inverse() * Affine::translate(centre));
    for behavior in &clip.behaviors {
        for recorded in &behavior.takes {
            let Some(path) = recorded.source.as_deref() else { continue };
            let label = recorded.name.clone().unwrap_or_else(|| format!("take {}", recorded.take_id));
            let Some(graph) = read_graph(project, model, graphs, path, warnings) else { continue };
            let time = recorded.time.as_ref();
            let window = Window {
                offset: time.map_or(0.0, |t| t.parent_offset),
                trim_in: time.map_or(0.0, |t| t.trim_in),
                duration: time.map(|t| t.duration).filter(|d| *d > 0.0).unwrap_or(graph.end - graph.start),
            };
            let meta = Meta {
                number: u32::try_from(recorded.take_id.max(0)).unwrap_or(0),
                name: label.clone(),
                enabled: policy == TakePolicy::Perform || recorded.enabled,
                fade_in: recorded.fade_in,
                fade_out: recorded.fade_out,
            };
            let made = match behavior.behavior.as_str() {
                "Adobe.FaceTracker2.js" => recorded::face_take(graph, behavior.instance_id.as_deref(), &meta, window),
                "Adobe.LipSync2.js" => recorded::voice_take(graph, &meta, window),
                "Adobe.Dragger3.js" => {
                    let binding = recorded.group_id.as_deref().and_then(|group| {
                        Some(HandleBinding { instance: behavior.instance_id.clone()?, group: group.to_owned(), key: handle_key(model, item, group)? })
                    });
                    if let (Some(binding), Some(to_puppet)) = (binding, to_puppet.as_ref()) {
                        recorded::handle_take(graph, &binding, to_puppet, &meta, window)
                    } else {
                        warnings.push(format!("{label}: cannot tell which handle of {} it drags", item.display_name()));
                        continue;
                    }
                }
                other => {
                    warnings.push(format!("{label}: takes of {other} are not supported"));
                    continue;
                }
            };
            match made {
                Some(take) => takes.push(take),
                None => warnings.push(format!("{label}: the recording holds nothing for it inside the clip")),
            }
        }
    }
    takes.sort_by_key(|t| t.number);
    takes
}

/// Reads scene `name` (the first scene when `None`) of a project, with its recorded takes performed.
///
/// # Errors
/// [`Error::Invalid`] when the project has no such scene or the scene's settings are out of range.
pub fn import_scene(project: &Project, model: &Model, name: Option<&str>) -> Result<Imported> {
    import_scene_with(project, model, name, &ImportOptions::default())
}

/// Reads scene `name` (the first scene when `None`) of a project, as `options` ask.
///
/// # Errors
/// [`Error::Invalid`] when the project has no such scene or the scene's settings are out of range.
pub fn import_scene_with(project: &Project, model: &Model, name: Option<&str>, options: &ImportOptions) -> Result<Imported> {
    let mut graphs = Graphs::new();
    let source = match name {
        Some(n) => model.scene(n).ok_or_else(|| Error::invalid(format!("no scene named {n:?}")))?,
        None => model.scenes.first().ok_or_else(|| Error::invalid("the project has no scenes"))?,
    };
    let fps = if source.fps.is_finite() && source.fps > 0.0 { anim_core::FrameRate::from_f64(source.fps) } else { anim_core::FrameRate::FPS_24 };
    let width = f64::from(u32::try_from(source.width.max(1)).unwrap_or(1));
    let height = f64::from(u32::try_from(source.height.max(1)).unwrap_or(1));
    let mut scene = Scene::new(&source.name, Size::new(width, height), fps, Tick::from_seconds_f64(source.duration.max(0.0)));
    scene.background = None;
    let mut warnings = Vec::new();
    let mut puppets: Vec<String> = Vec::new();
    let mut sounds: Vec<(String, String)> = Vec::new();
    let mut tracks: Vec<_> = source.tracks.iter().collect();
    tracks.sort_by_key(|t| t.index);
    let mut audio_tracks = Vec::new();
    for track in tracks {
        match track.kind.as_deref() {
            Some("trackKind_puppet") => {
                for clip in &track.items {
                    let Some(item) = clip.source.as_deref().map(strip).and_then(|s| model.puppet(s)).filter(|p| !p.is_part) else { continue };
                    let mut t = PuppetTrack::new(item.display_name(), &item.path);
                    t.enabled = track.enabled;
                    if let Some(transform) = clip.behaviors.iter().find(|b| b.behavior == "Adobe.Transform.js" && b.enabled) {
                        t.placement = placement_from_params(&transform.params);
                    }
                    t.takes = clip_takes(project, model, clip, item, &t.placement, scene.size, options.takes, &mut graphs, &mut warnings);
                    if !puppets.contains(&item.path) {
                        puppets.push(item.path.clone());
                    }
                    scene.add_track(t)?;
                }
            }
            Some("trackKind_audio") => {
                let mut audio = AudioTrack::new(&format!("Audio {}", audio_tracks.len().saturating_add(1)));
                audio.enabled = track.enabled;
                for clip in &track.items {
                    let Some(item) = clip.source.as_deref().map(strip).and_then(|s| model.media.iter().find(|m| m.path == s)) else { continue };
                    let Some(file) = item.file.as_ref() else { continue };
                    let stem = media_name(item.file_name.as_deref().unwrap_or("sound"));
                    let existing = sounds.iter().find(|(_, p)| *p == file.path).map(|(n, _)| n.clone());
                    let media = existing.unwrap_or_else(|| {
                        let mut unique = stem.clone();
                        let mut n = 2_u32;
                        while sounds.iter().any(|(k, _)| *k == unique) {
                            unique = format!("{stem} {n}");
                            n = n.saturating_add(1);
                        }
                        sounds.push((unique.clone(), file.path.clone()));
                        unique
                    });
                    let time = clip.time.as_ref();
                    let mut c = AudioClip::new(&media, &media, Tick::from_seconds_f64(time.map_or(0.0, |t| t.parent_offset)));
                    c.trim_in = Tick::from_seconds_f64(time.map_or(0.0, |t| t.trim_in).max(0.0));
                    c.duration = time.map(|t| t.duration).filter(|d| *d > 0.0 && d.is_finite()).map(Tick::from_seconds_f64);
                    audio.clips.push(c);
                }
                audio_tracks.push(audio);
            }
            _ => {}
        }
    }
    scene.audio = audio_tracks;
    scene.sanitize()?;
    Ok(Imported { scene, puppets, sounds, warnings })
}

/// Decodes the sound files of an imported scene into `media`. Returns what could not be read.
#[must_use]
pub fn load_sounds(project: &Project, imported: &Imported, media: &mut MediaStore) -> Vec<String> {
    let mut warnings = Vec::new();
    for (name, path) in &imported.sounds {
        if !valid_media_name(name) {
            continue;
        }
        let decoded = project.package().read(path).map_err(Error::from).and_then(|bytes| Ok(anim_audio::decode_bytes(&bytes, Some(name))?));
        match decoded.and_then(|buffer| media.insert(name, buffer)) {
            Ok(()) => {}
            Err(e) => warnings.push(format!("{path}: {e}")),
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_parameters_become_a_placement() {
        let params = vec![("anchorPtX".to_owned(), LValue::Int(-516)), ("uniformScale".to_owned(), LValue::Int(50)), ("scaleX".to_owned(), LValue::Int(200))];
        let p = placement_from_params(&params);
        assert_eq!(p.anchor, Vec2::new(-516.0, 0.0));
        assert_eq!(p.scale, Vec2::new(1.0, 0.5));
        assert_eq!(placement_from_params(&[]), Placement::default());
    }

    #[test]
    fn media_names_are_made_file_safe() {
        for input in ["voice.wav", "ü/../x", "", ".x", &"y".repeat(200), "a b-c_d.mp3"] {
            assert!(valid_media_name(&media_name(input)), "{input:?} -> {:?}", media_name(input));
        }
    }
}
