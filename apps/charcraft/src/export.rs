//! Exporting a scene: an MP4 (H.264 + AAC), a MOV (`ProRes` 4444 with alpha), a folder of PNGs or one PNG.
//!
//! An export owns copies of everything it needs ([`Job`]) and runs on its own thread, so the window and the MCP
//! session stay responsive. It reports progress and stops early when asked.

use std::path::{Path, PathBuf};

use anim_core::{FrameRate, Tick, TimeRange};
use anim_render::{Renderer, straight_rgba};
use anim_video::{Codec, H264Rate, VideoSettings};
use charcraft_record::{Cast, Evaluator, MediaStore, Scene, mix_scene, video_track};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::compose::Stage;

/// Most frames one export renders.
pub const MAX_FRAMES: u32 = 2_000_000;

/// What gets written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// H.264 in an MP4, with the scene's sound.
    Mp4,
    /// `ProRes` 4444 with alpha in a MOV, with the scene's sound.
    Mov,
    /// One PNG per frame in a folder.
    PngSequence,
    /// One PNG of one frame.
    Png,
}

impl Format {
    /// The id used by `export_run`.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::PngSequence => "png-sequence",
            Self::Png => "png",
        }
    }

    /// Parses an id.
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        [Self::Mp4, Self::Mov, Self::PngSequence, Self::Png].into_iter().find(|f| f.id() == id)
    }
}

/// Options of an export (`options` of `export_run`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    /// Pixels per stage pixel.
    pub scale: f64,
    /// First frame.
    pub start_frame: u32,
    /// Frames to export (all the rest when absent). For `png`, the frame is `start_frame`.
    pub frames: Option<u32>,
    /// Leave the stage background out (default: for `mov` and `png-sequence`).
    pub transparent: Option<bool>,
}

impl Default for Options {
    fn default() -> Self {
        Self { scale: 1.0, start_frame: 0, frames: None, transparent: None }
    }
}

/// Everything an export needs.
#[derive(Debug)]
pub struct Job {
    /// The puppets.
    pub cast: Cast,
    /// The scene.
    pub scene: Scene,
    /// The sound.
    pub media: MediaStore,
    /// What to write.
    pub format: Format,
    /// Where.
    pub path: PathBuf,
    /// Options.
    pub options: Options,
}

fn pixels(extent: f64, scale: f64) -> u32 {
    anim_core::num::round_f64_to_u32((extent * scale).max(1.0))
}

fn write_png(path: &Path, picture: &crate::compose::Picture) -> Result<u64, String> {
    let bytes = anim_export::encode_png(picture.width, picture.height, &picture.rgba).map_err(|e| format!("encoding the PNG: {e}"))?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    std::fs::write(path, &bytes).map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
}

/// Runs the export. `progress(fraction, message)` returns false to cancel.
///
/// # Errors
/// A message naming the problem (bad options, a render or encoder failure, a write failure, cancellation).
pub fn run(job: Job, progress: &mut dyn FnMut(f32, &str) -> bool) -> Result<Value, String> {
    let Job { cast, scene, media, format, path, options } = job;
    if !options.scale.is_finite() || options.scale <= 0.0 || options.scale > 8.0 {
        return Err("scale must be a number between 0 and 8".to_owned());
    }
    let total = scene.frame_count();
    if total == 0 {
        return Err("the scene has no frames".to_owned());
    }
    let first = options.start_frame.min(total.saturating_sub(1));
    let count = match format {
        Format::Png => 1,
        _ => options.frames.unwrap_or_else(|| total.saturating_sub(first)).clamp(1, total.saturating_sub(first)),
    };
    if count > MAX_FRAMES {
        return Err(format!("{count} frames is more than the limit of {MAX_FRAMES}"));
    }
    let transparent = options.transparent.unwrap_or(matches!(format, Format::Mov | Format::PngSequence));
    let stage = Stage::new(cast);
    let mut evaluator = Evaluator::new(&scene, &stage.cast().templates()).map_err(|e| e.to_string())?;
    let mut renderer = Renderer::new();
    let (mut width, mut height) = (pixels(scene.size.width, options.scale), pixels(scene.size.height, options.scale));
    if matches!(format, Format::Mp4 | Format::Mov) {
        width = width.saturating_add(width % 2);
        height = height.saturating_add(height % 2);
    }
    let started = std::time::Instant::now();
    let mut render_at = |frame: u32| -> Result<anim_display::Image, String> {
        let poses = evaluator.poses(i64::from(frame));
        stage.render_scene_image(&mut renderer, &scene, &poses, options.scale, Some((width, height)), transparent).map_err(|e| e.to_string())
    };
    match format {
        Format::Png => {
            progress(0.0, "rendering");
            let image = render_at(first)?;
            let picture = crate::compose::Picture { width: image.width, height: image.height, rgba: straight_rgba(&image) };
            let bytes = write_png(&path, &picture)?;
            Ok(json!({ "format": "png", "path": path.display().to_string(), "frame": first, "width": picture.width, "height": picture.height, "bytes": bytes }))
        }
        Format::PngSequence => {
            let mut bytes = 0_u64;
            for n in 0..count {
                if !progress(fraction(n, count), &format!("frame {} of {count}", n.saturating_add(1))) {
                    return Err("cancelled".to_owned());
                }
                let image = render_at(first.saturating_add(n))?;
                let picture = crate::compose::Picture { width: image.width, height: image.height, rgba: straight_rgba(&image) };
                bytes = bytes.saturating_add(write_png(&path.join(format!("frame_{:05}.png", n.saturating_add(1))), &picture)?);
            }
            Ok(json!({ "format": "png-sequence", "path": path.display().to_string(), "frames": count, "width": width, "height": height, "bytes": bytes, "seconds": started.elapsed().as_secs_f64() }))
        }
        Format::Mp4 | Format::Mov => {
            let fps = FrameRate { num: scene.fps.num.max(1), den: scene.fps.den.max(1) };
            let codec = if format == Format::Mp4 { Codec::H264 { rate: H264Rate::default() } } else { Codec::ProRes4444 { alpha: true } };
            let mut settings = VideoSettings::new(width, height, fps, codec);
            let (mixed, _missing) = mix_scene(&scene, &media, anim_audio::MixFormat::STEREO_48K).map_err(|e| e.to_string())?;
            let has_sound = scene.audio.iter().filter(|t| t.enabled).flat_map(|t| &t.clips).any(|c| c.enabled && media.get(&c.media).is_some());
            if has_sound {
                let from = scene.frame_time(i64::from(first));
                let length = Tick(scene.frame_time(i64::from(first.saturating_add(count))).0.saturating_sub(from.0));
                settings = settings.with_audio(video_track(&mixed.slice_ticks(TimeRange::new(from, length))));
            }
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).map_err(|e| format!("creating {}: {e}", parent.display()))?;
            }
            let mut encoder = anim_video::start(&path, settings).map_err(|e| format!("starting the encoder: {e}"))?;
            for n in 0..count {
                if !progress(fraction(n, count), &format!("frame {} of {count}", n.saturating_add(1))) {
                    return Err("cancelled".to_owned());
                }
                encoder.send_frame(render_at(first.saturating_add(n))?).map_err(|e| format!("encoding a frame: {e}"))?;
            }
            let summary = encoder.finish().map_err(|e| format!("finishing the video: {e}"))?;
            Ok(json!({
                "format": format.id(),
                "path": path.display().to_string(),
                "frames": summary.frames,
                "width": summary.width,
                "height": summary.height,
                "bytes": summary.bytes,
                "audio": summary.has_audio,
                "seconds": started.elapsed().as_secs_f64(),
            }))
        }
    }
}

fn fraction(done: u32, total: u32) -> f32 {
    if total == 0 { 1.0 } else { anim_core::num::f64_to_f32(f64::from(done) / f64::from(total)) }
}

#[cfg(test)]
mod tests {
    use charcraft_record::CastMember;

    use super::*;

    fn job(format: Format, path: PathBuf) -> Job {
        let mut cast = Cast::new();
        cast.insert("demo", CastMember::new(crate::demo::puppet().unwrap()));
        let opened = crate::world::World::demo().unwrap();
        let mut scene = opened.rec.scene;
        scene.size = anim_core::Size::new(160.0, 90.0);
        scene.duration = Tick::from_seconds_f64(0.2);
        if let Some(track) = scene.tracks.first_mut() {
            track.placement.scale = anim_core::Vec2::new(0.12, 0.12);
        }
        Job { cast, scene, media: MediaStore::new(), format, path, options: Options::default() }
    }

    #[test]
    fn a_png_sequence_has_one_file_per_frame() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("frames");
        let mut seen = Vec::new();
        let done = run(job(Format::PngSequence, out.clone()), &mut |f, _| {
            seen.push(f);
            true
        })
        .unwrap();
        assert_eq!(done["frames"], 6);
        assert!(out.join("frame_00006.png").is_file());
        assert!(seen.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn one_png_and_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("one.png");
        run(job(Format::Png, file.clone()), &mut |_, _| true).unwrap();
        assert!(std::fs::metadata(&file).unwrap().len() > 100);
        let cancelled = run(job(Format::PngSequence, dir.path().join("x")), &mut |_, _| false);
        assert_eq!(cancelled.unwrap_err(), "cancelled");
        let mut bad = job(Format::Png, dir.path().join("y.png"));
        bad.options.scale = f64::NAN;
        assert!(run(bad, &mut |_, _| true).is_err());
    }

    #[test]
    fn a_video_has_the_requested_frames() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.mp4");
        let done = run(job(Format::Mp4, file.clone()), &mut |_, _| true).unwrap();
        assert_eq!(done["frames"], 6);
        assert!(std::fs::metadata(&file).unwrap().len() > 500);
        assert_eq!(Format::parse("png-sequence"), Some(Format::PngSequence));
    }
}
