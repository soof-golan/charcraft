//! Reading a scene out of a Character Animator project: tracks, placements and sound clips.
//!
//! The project's recorded takes (`.arf` event graphs) are not decoded, so the imported scene has puppet tracks
//! without takes; record new ones or load a sidecar. What does import is the stage (size, rate, length), one
//! track per puppet clip with the Transform behavior's placement, and the audio clips.

use anim_core::{Size, Tick, Vec2};
use charcraft_ltab::LValue;
use charcraft_project::{Model, Project};

use crate::audio::{MediaStore, valid_media_name};
use crate::error::{Error, Result};
use crate::scene::{AudioClip, AudioTrack, Placement, PuppetTrack, Scene};

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

/// Reads scene `name` (the first scene when `None`) of a project model.
///
/// # Errors
/// [`Error::Invalid`] when the project has no such scene or the scene's settings are out of range.
pub fn import_scene(model: &Model, name: Option<&str>) -> Result<Imported> {
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
    let project_takes: usize = source.behavior_instances().map(|b| b.takes.len()).sum();
    if project_takes > 0 {
        warnings.push(format!("{project_takes} recorded takes of the project (.arf event graphs) are not decoded"));
    }
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
