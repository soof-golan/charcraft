//! Persistence: scenes and their recorded sound beside the project, as plain serde JSON and WAV files.
//!
//! A Character Animator project stores takes as `.arf` event graphs and `KeyframeBucket` files inside the
//! project's own database, in a form `charcraft-project` reads but does not write back (the recorded event
//! streams are opaque to it). Rather than write files Character Animator might misread, the takes live in a
//! sidecar next to the project:
//!
//! ```text
//! news anchors.chproj                     the project (never touched)
//! news anchors.chproj.record.json         the scenes: tracks, takes, audio clips
//! news anchors.chproj.record.media/       recorded or imported sound as 32-bit float WAV files
//! ```
//!
//! Reading treats the files as hostile: size caps, [`Scene::sanitize`], file-safe media names.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::audio::{MediaStore, valid_media_name, wav_bytes};
use crate::error::{Error, Result};
use crate::scene::Scene;

/// The `format` field of a scene file.
pub const FORMAT: &str = "charcraft-record";
/// The current version of the scene file.
pub const VERSION: u32 = 1;
/// Largest scene file read, in bytes.
pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
/// Largest sound file read, in bytes.
pub const MAX_SOUND_BYTES: u64 = 1024 * 1024 * 1024;

/// What the JSON file holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneFile {
    /// Always [`FORMAT`].
    pub format: String,
    /// The file version.
    pub version: u32,
    /// The scene.
    pub scene: Scene,
}

impl SceneFile {
    /// Wraps a scene for writing.
    #[must_use]
    pub fn new(scene: Scene) -> Self {
        Self { format: FORMAT.to_owned(), version: VERSION, scene }
    }
}

/// Where the sidecar of a project lives: beside the `.chproj`, `.puppet` or project folder.
#[must_use]
pub fn sidecar_path(project: &Path) -> PathBuf {
    let name = project.file_name().map_or_else(|| "project".to_owned(), |n| n.to_string_lossy().into_owned());
    project.parent().unwrap_or_else(|| Path::new("")).join(format!("{name}.record.json"))
}

/// The folder of a sidecar's sound files.
#[must_use]
pub fn media_dir(sidecar: &Path) -> PathBuf {
    let mut name = sidecar.file_stem().map_or_else(|| "record".to_owned(), |n| n.to_string_lossy().into_owned());
    // `x.chproj.record.json` has the stem `x.chproj.record`; the `.media` folder sits next to it.
    if !name.ends_with(".record") {
        name.push_str(".record");
    }
    name.push_str(".media");
    sidecar.parent().unwrap_or_else(|| Path::new("")).join(name)
}

/// Scene JSON text.
///
/// # Errors
/// [`Error::Json`].
pub fn to_json(scene: &Scene) -> Result<String> {
    Ok(serde_json::to_string_pretty(&SceneFile::new(scene.clone()))?)
}

/// Reads scene JSON text and checks it.
///
/// # Errors
/// [`Error::Json`] for malformed text, [`Error::Invalid`] for another format or a newer version, and whatever
/// [`Scene::sanitize`] rejects.
pub fn from_json(text: &str) -> Result<Scene> {
    let file: SceneFile = serde_json::from_str(text)?;
    if file.format != FORMAT {
        return Err(Error::invalid(format!("not a charcraft-record file (format {:?})", file.format)));
    }
    if file.version > VERSION {
        return Err(Error::invalid(format!("file version {} is newer than this program reads ({VERSION})", file.version)));
    }
    let mut scene = file.scene;
    scene.sanitize()?;
    Ok(scene)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, bytes).map_err(|e| Error::io(&tmp, e))?;
    fs::rename(&tmp, path).map_err(|e| Error::io(path, e))
}

/// Writes the scene and its sound to the sidecar `path` (and its media folder). Sound files that no longer
/// belong to the store are removed.
///
/// # Errors
/// [`Error::Io`], [`Error::Json`].
pub fn save(path: &Path, scene: &Scene, media: &MediaStore) -> Result<()> {
    let dir = media_dir(path);
    if !media.is_empty() {
        fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    }
    for name in media.names() {
        if let Some(buffer) = media.get(name) {
            write_atomic(&dir.join(format!("{name}.wav")), &wav_bytes(buffer))?;
        }
    }
    if dir.is_dir() {
        for entry in fs::read_dir(&dir).map_err(|e| Error::io(&dir, e))?.flatten() {
            let file = entry.path();
            let stale = file.extension().is_some_and(|e| e == "wav") && file.file_stem().is_some_and(|s| media.get(&s.to_string_lossy()).is_none());
            if stale {
                fs::remove_file(&file).map_err(|e| Error::io(&file, e))?;
            }
        }
    }
    write_atomic(path, to_json(scene)?.as_bytes())
}

/// Reads a sidecar and the sound it refers to. A clip whose sound file is missing keeps its name; mixing reports
/// it as missing.
///
/// # Errors
/// [`Error::Io`], [`Error::Json`], [`Error::Invalid`] and [`Error::TooLarge`].
pub fn load(path: &Path) -> Result<(Scene, MediaStore)> {
    let size = fs::metadata(path).map_err(|e| Error::io(path, e))?.len();
    if size > MAX_FILE_BYTES {
        return Err(Error::TooLarge { what: "the scene file", limit: usize::try_from(MAX_FILE_BYTES).unwrap_or(usize::MAX) });
    }
    let text = fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    let scene = from_json(&text)?;
    let mut media = MediaStore::new();
    let dir = media_dir(path);
    let wanted: std::collections::BTreeSet<&str> = scene.audio.iter().flat_map(|t| &t.clips).map(|c| c.media.as_str()).collect();
    for name in wanted.into_iter().filter(|n| valid_media_name(n)) {
        let file = dir.join(format!("{name}.wav"));
        let Ok(meta) = fs::metadata(&file) else { continue };
        if meta.len() > MAX_SOUND_BYTES {
            continue;
        }
        let buffer = anim_audio::decode_path(&file)?;
        media.insert(name, buffer)?;
    }
    Ok((scene, media))
}

#[cfg(test)]
mod tests {
    use anim_audio::PcmBuffer;
    use anim_core::{FrameRate, Size, Tick};

    use super::*;
    use crate::group::{Frame, ParamGroup};
    use crate::scene::{AudioClip, PuppetTrack};
    use crate::take::{Sample, Take};

    fn scene() -> Scene {
        let mut scene = Scene::new("Anchors", Size::new(1280.0, 720.0), FrameRate::FPS_24, Tick::from_seconds_f64(5.0));
        let t = scene.add_track(PuppetTrack::new("Lead", "ProjectItems/Lead")).unwrap();
        let mut take = Take::new(0, [ParamGroup::Keys].into_iter().collect(), Tick::ZERO, Tick::from_seconds_f64(2.0));
        take.samples.push(Sample { t: Tick::ZERO, frame: Frame { keys: ["space".to_owned()].into_iter().collect(), ..Frame::default() } });
        scene.add_take(t, take).unwrap();
        scene.add_clip(0, AudioClip::new("voice", "voice", Tick::ZERO)).unwrap();
        scene
    }

    #[test]
    fn the_sidecar_sits_beside_the_project() {
        let p = sidecar_path(Path::new("/a/b/news anchors.chproj"));
        assert_eq!(p, Path::new("/a/b/news anchors.chproj.record.json"));
        assert_eq!(media_dir(&p), Path::new("/a/b/news anchors.chproj.record.media"));
    }

    #[test]
    fn save_and_load_round_trip_scene_and_sound() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar_path(&dir.path().join("show.chproj"));
        let mut media = MediaStore::new();
        media.insert("voice", PcmBuffer::new(16_000, 1, (0..1600_u16).map(|i| f32::from(i) / 1600.0).collect())).unwrap();
        media.insert("unused", PcmBuffer::new(16_000, 1, vec![0.0; 10])).unwrap();
        let s = scene();
        save(&path, &s, &media).unwrap();
        let mut smaller = MediaStore::new();
        smaller.insert("voice", media.get("voice").unwrap().clone()).unwrap();
        save(&path, &s, &smaller).unwrap();
        assert!(!media_dir(&path).join("unused.wav").exists(), "stale sound is removed");
        let (back, back_media) = load(&path).unwrap();
        assert_eq!(back, s);
        assert_eq!(back_media.get("voice"), media.get("voice"));
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn hostile_files_are_errors_not_panics() {
        assert!(from_json("").is_err());
        assert!(from_json("{}").is_err());
        assert!(from_json("[1,2,3]").is_err());
        let mut file = SceneFile::new(scene());
        file.format = "something-else".to_owned();
        assert!(from_json(&serde_json::to_string(&file).unwrap()).is_err());
        let mut file = SceneFile::new(scene());
        file.version = VERSION + 1;
        assert!(from_json(&serde_json::to_string(&file).unwrap()).is_err());
        let mut file = SceneFile::new(scene());
        file.scene.fps = FrameRate { num: 0, den: 0 };
        assert!(from_json(&serde_json::to_string(&file).unwrap()).is_err());
        let dir = tempfile::tempdir().unwrap();
        assert!(load(&dir.path().join("missing.json")).is_err());
    }

    #[test]
    fn media_names_from_a_file_cannot_escape_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.record.json");
        let mut s = scene();
        s.audio[0].clips[0].media = "../../etc/passwd".to_owned();
        fs::write(&path, to_json(&s).unwrap()).unwrap();
        let (_, media) = load(&path).unwrap();
        assert!(media.is_empty());
    }
}
