//! Media items: artwork skins and audio files the project refers to, and where they live on disk.

use std::collections::BTreeMap;

use charcraft_ltab::LValue;
use serde::Serialize;

use super::util::{boolean, last_segment, number, string};

/// What kind of media an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MediaKind {
    /// Artwork (`Skin`): a PSD, AI, PNG or similar image the puppet is built from.
    Artwork,
    /// Audio (`AudioFile_000`).
    Audio,
    /// Another `...File_nnn` class.
    Other,
}

/// A rectangle in artwork pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Bounds {
    /// Left edge.
    pub left: f64,
    /// Top edge.
    pub top: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// A file of `Ch Media/` or `Ch Data/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MediaFile {
    /// Path inside the package (`Ch Media/Gathered Media/Boy.psd`).
    pub path: String,
    /// Size in bytes.
    pub size: u64,
}

/// A media item recorded in the project store.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MediaItem {
    /// The store path.
    pub path: String,
    /// The class (`Skin`, `AudioFile_000`).
    pub class: String,
    /// Artwork or audio.
    pub kind: MediaKind,
    /// The file name the project remembers (from the file alias).
    pub file_name: Option<String>,
    /// The full path the alias recorded on the author's machine.
    pub alias_path: Option<String>,
    /// Character Animator flagged the file as missing.
    pub missing: bool,
    /// The file as found inside the package, when it is there.
    pub file: Option<MediaFile>,
    /// Audio length in seconds.
    pub duration: Option<f64>,
    /// Audio channel count.
    pub channels: Option<i64>,
    /// Audio sample rate.
    pub sample_rate: Option<i64>,
    /// Audio format description.
    pub format: Option<String>,
    /// Lip sync transcript text.
    pub transcript: Option<String>,
    /// Artwork content bounds.
    pub content_bounds: Option<Bounds>,
    /// Artwork document bounds.
    pub document_bounds: Option<Bounds>,
    /// Artwork render oversample factor.
    pub oversample: Option<i64>,
    /// The artwork reloads when the file changes on disk.
    pub auto_update: bool,
}

fn bounds(value: Option<&LValue>) -> Option<Bounds> {
    let value = value?;
    Some(Bounds { left: number(value, "left"), top: number(value, "top"), width: number(value, "width"), height: number(value, "height") })
}

/// The `fullpath` of a file alias (`Alias{ aliasJSON = "{...}" }`).
fn alias_path(alias: Option<&LValue>) -> Option<String> {
    let json = string(alias?, "aliasJSON")?;
    let parsed: serde_json::Value = serde_json::from_str(&json).ok()?;
    parsed.get("fullpath")?.as_str().map(str::to_owned)
}

/// The kind a class name stands for, or `None` when it is not media.
#[must_use]
pub fn media_kind(class: &str) -> Option<MediaKind> {
    match class {
        "Skin" => Some(MediaKind::Artwork),
        "AudioFile_000" => Some(MediaKind::Audio),
        other if other.contains("File_") && !other.starts_with("EventGraph") => Some(MediaKind::Other),
        _ => None,
    }
}

/// Decodes a media item. `on_disk` maps file names to the matching file in the package.
#[must_use]
pub fn decode(path: &str, class: &str, kind: MediaKind, value: &LValue, on_disk: &BTreeMap<String, MediaFile>) -> MediaItem {
    let alias = value.get("_alias").or_else(|| value.get("_fileAlias"));
    let alias_path = alias_path(alias);
    let file_name = alias_path.as_deref().map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p).to_owned()).or_else(|| Some(last_segment(path).to_owned()));
    let file = file_name.as_deref().and_then(|n| on_disk.get(n)).cloned();
    let duration = value.get("_duration_serializableRatioTime").and_then(|r| {
        let den = number(r, "denominator");
        (den > 0.0).then(|| number(r, "numerator") / den)
    });
    MediaItem {
        path: path.to_owned(),
        class: class.to_owned(),
        kind,
        file_name,
        alias_path,
        missing: alias.is_some_and(|a| boolean(a, "bMissing")),
        file,
        duration,
        channels: value.get("_channelCount").and_then(LValue::as_i64),
        sample_rate: value.get("_sSampleRate").and_then(LValue::as_i64),
        format: string(value, "_sFormatDescription"),
        transcript: string(value, "_sTranscriptText"),
        content_bounds: bounds(value.get("_contentBounds")),
        document_bounds: bounds(value.get("_documentBounds")),
        oversample: value.get("_imageOversample").and_then(LValue::as_i64),
        auto_update: boolean(value, "_bAutoUpdateFromFilesystem"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_classes_are_recognised() {
        assert_eq!(media_kind("Skin"), Some(MediaKind::Artwork));
        assert_eq!(media_kind("AudioFile_000"), Some(MediaKind::Audio));
        assert_eq!(media_kind("EventGraphFile_000"), None);
        assert_eq!(media_kind("Puppet"), None);
    }

    #[test]
    fn alias_file_names_handle_both_separators() {
        let alias = charcraft_ltab::parse("{ aliasJSON = \"{\\\"fullpath\\\":\\\"C:\\\\\\\\Users\\\\\\\\a\\\\\\\\Boy.psd\\\"}\" }").expect("parses");
        assert_eq!(alias_path(Some(&alias)).as_deref(), Some("C:\\Users\\a\\Boy.psd"));
    }
}
