//! Exporting a scene: an MP4 (H.264 + AAC), a MOV (`ProRes` 4444 with alpha), an animated GIF, a folder of PNGs, one
//! PNG, the sound alone as a WAV, or an After Effects / Premiere package.
//!
//! An export owns copies of everything it needs ([`Job`]) and runs on its own thread, so the window and the MCP
//! session stay responsive. It reports progress and stops early when asked.
//!
//! ## Presets and the queue
//!
//! A [`Preset`] is a named format plus options (the Media Encoder idea). Every preset id is also accepted as a format
//! id by `export_run`, with the request's options merged over the preset's. The [`Queue`] holds exports waiting to run
//! one after another; the window drives it, and over MCP a client queues by simply calling `export_run` repeatedly.
//!
//! ## Dynamic Link
//!
//! Adobe Dynamic Link is proprietary and is not cloned. The closest legitimate workflow is [`Format::AfterEffects`]:
//! a folder with a PNG sequence (straight alpha), the scene's sound as a WAV, a `ProRes` 4444 MOV with alpha, an
//! `export.json` manifest and a `REEXPORT.txt` holding the command that makes the package again from the project.
//! After Effects and Premiere import all of these natively, and re-running the command refreshes the files in place.
//!
//! ## `WebM`
//!
//! `WebM` with alpha needs a VP9 (or VP8) encoder. There is no pure-Rust one, and bundling libvpx would add a C build to
//! every platform, so `WebM` is not offered. For transparent video use the `ProRes` 4444 MOV (full alpha), a PNG sequence
//! (full alpha) or the GIF (1-bit alpha).

use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anim_core::{FrameRate, Tick, TimeRange};
use anim_render::{Renderer, straight_rgba};
use anim_video::{Codec, H264Rate, VideoSettings};
use charcraft_record::{Cast, Evaluator, MediaStore, Scene, mix_scene, video_track, wav_bytes};
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
    /// An animated GIF (1-bit alpha).
    Gif,
    /// The scene's sound alone, as a 32-bit float WAV.
    Wav,
    /// A folder for After Effects or Premiere: PNG sequence, WAV, `ProRes` 4444 MOV, manifest and re-export command.
    AfterEffects,
}

impl Format {
    /// Every format.
    pub const ALL: [Self; 7] = [Self::Mp4, Self::Mov, Self::Gif, Self::PngSequence, Self::Png, Self::Wav, Self::AfterEffects];

    /// The id used by `export_run`.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::PngSequence => "png-sequence",
            Self::Png => "png",
            Self::Gif => "gif",
            Self::Wav => "wav",
            Self::AfterEffects => "after-effects",
        }
    }

    /// A name for menus.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "H.264 video (MP4)",
            Self::Mov => "ProRes 4444 with alpha (MOV)",
            Self::PngSequence => "PNG sequence",
            Self::Png => "One PNG",
            Self::Gif => "Animated GIF",
            Self::Wav => "Sound only (WAV)",
            Self::AfterEffects => "After Effects / Premiere package",
        }
    }

    /// The file extension, empty for folders.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Png => "png",
            Self::Gif => "gif",
            Self::Wav => "wav",
            Self::PngSequence | Self::AfterEffects => "",
        }
    }

    /// Whether the picture keeps the stage background out by default.
    #[must_use]
    pub const fn transparent_by_default(self) -> bool {
        matches!(self, Self::Mov | Self::PngSequence | Self::AfterEffects)
    }

    /// Whether this format makes pictures (options for size, range and alpha apply).
    #[must_use]
    pub const fn has_picture(self) -> bool {
        !matches!(self, Self::Wav)
    }

    /// Parses an id.
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == id)
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
    /// Leave the stage background out (default: for `mov`, `png-sequence` and `after-effects`).
    pub transparent: Option<bool>,
    /// Output frame rate (default: the scene's). Frames are picked by time, so the length stays the same.
    pub fps: Option<f64>,
}

impl Default for Options {
    fn default() -> Self {
        Self { scale: 1.0, start_frame: 0, frames: None, transparent: None, fps: None }
    }
}

impl Options {
    /// The size in pixels the export of a stage of this size comes out at.
    #[must_use]
    pub fn pixels(&self, width: f64, height: f64) -> (u32, u32) {
        (pixels(width, self.scale), pixels(height, self.scale))
    }
}

/// A named format with options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    /// A stable id, also usable as a format id in `export_run`.
    pub id: String,
    /// A name for menus.
    pub label: String,
    /// What it is for.
    pub description: String,
    /// The format.
    pub format: Format,
    /// The options.
    pub options: Options,
}

/// The built-in presets.
#[must_use]
pub fn presets() -> Vec<Preset> {
    let make = |id: &str, label: &str, description: &str, format: Format, options: Options| Preset {
        id: id.to_owned(),
        label: label.to_owned(),
        description: description.to_owned(),
        format,
        options,
    };
    vec![
        make("h264-match-source", "H.264 - match source", "MP4 at the stage size and frame rate, with sound.", Format::Mp4, Options::default()),
        make("h264-half", "H.264 - half size", "MP4 at half the stage size, for previews.", Format::Mp4, Options { scale: 0.5, ..Options::default() }),
        make("h264-double", "H.264 - double size", "MP4 at twice the stage size.", Format::Mp4, Options { scale: 2.0, ..Options::default() }),
        make("prores-4444-alpha", "ProRes 4444 with alpha", "MOV with a transparent background, for editing.", Format::Mov, Options::default()),
        make(
            "gif-web",
            "Animated GIF - web",
            "Half-size GIF at 15 fps with a transparent background.",
            Format::Gif,
            Options { scale: 0.5, fps: Some(15.0), transparent: Some(true), ..Options::default() },
        ),
        make(
            "gif-opaque",
            "Animated GIF - opaque",
            "Half-size GIF at 15 fps on the stage background.",
            Format::Gif,
            Options { scale: 0.5, fps: Some(15.0), transparent: Some(false), ..Options::default() },
        ),
        make("png-sequence-alpha", "PNG sequence with alpha", "One PNG per frame, transparent background.", Format::PngSequence, Options::default()),
        make("still-png", "Still frame (PNG)", "The frame at the start frame, on the stage background.", Format::Png, Options { transparent: Some(false), ..Options::default() }),
        make("audio-wav", "Sound only (WAV)", "The mixed sound of the scene.", Format::Wav, Options::default()),
        make(
            "after-effects-package",
            "After Effects / Premiere package",
            "PNG sequence, WAV, ProRes 4444 MOV and the command that exports it again.",
            Format::AfterEffects,
            Options::default(),
        ),
    ]
}

/// What an export id names: a format or a preset, with the request's options (JSON) merged over the preset's.
///
/// # Errors
/// A message when the id names neither or the options do not parse.
pub fn resolve(id: &str, options: &Value) -> Result<(Format, Options), String> {
    let (format, base) = if let Some(format) = Format::parse(id) {
        (format, Options::default())
    } else {
        let preset = presets().into_iter().find(|p| p.id == id).ok_or_else(|| format!("no export format or preset {id:?}"))?;
        (preset.format, preset.options)
    };
    if options.is_null() {
        return Ok((format, base));
    }
    let mut merged = serde_json::to_value(&base).map_err(|e| e.to_string())?;
    if let (Some(into), Some(from)) = (merged.as_object_mut(), options.as_object()) {
        for (key, value) in from {
            into.insert(key.clone(), value.clone());
        }
    }
    let parsed = serde_json::from_value(merged).map_err(|e| format!("export options: {e}"))?;
    Ok((format, parsed))
}

/// One export waiting to run.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueItem {
    /// What to write.
    pub format: Format,
    /// Where.
    pub path: PathBuf,
    /// Options.
    pub options: Options,
    /// How it is doing.
    pub state: ItemState,
}

/// Where a queued export stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemState {
    /// Not started.
    Waiting,
    /// Running now.
    Running,
    /// Finished.
    Done,
    /// Stopped with this message.
    Failed(String),
}

/// Exports that run one after another.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Queue {
    /// The items in order, with the finished ones kept until cleared.
    pub items: Vec<QueueItem>,
    /// Whether the queue is running (it starts the next waiting item whenever one ends).
    pub running: bool,
}

impl Queue {
    /// Adds an export at the end.
    pub fn push(&mut self, format: Format, path: PathBuf, options: Options) {
        self.items.push(QueueItem { format, path, options, state: ItemState::Waiting });
    }

    /// Marks the running item finished with the outcome of its export.
    pub fn finish(&mut self, outcome: Result<(), String>) {
        if let Some(item) = self.items.iter_mut().find(|i| i.state == ItemState::Running) {
            item.state = match outcome {
                Ok(()) => ItemState::Done,
                Err(message) => ItemState::Failed(message),
            };
        }
    }

    /// Whether an item of the queue is running.
    #[must_use]
    pub fn has_running(&self) -> bool {
        self.items.iter().any(|i| i.state == ItemState::Running)
    }

    /// Takes the next waiting item and marks it running; stops the queue when none is left.
    pub fn take_next(&mut self) -> Option<(Format, PathBuf, Options)> {
        if !self.running {
            return None;
        }
        let Some(item) = self.items.iter_mut().find(|i| i.state == ItemState::Waiting) else {
            self.running = false;
            return None;
        };
        item.state = ItemState::Running;
        Some((item.format, item.path.clone(), item.options.clone()))
    }

    /// Removes the finished and failed items.
    pub fn clear_finished(&mut self) {
        self.items.retain(|i| matches!(i.state, ItemState::Waiting | ItemState::Running));
    }

    /// Waiting items.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.items.iter().filter(|i| i.state == ItemState::Waiting).count()
    }
}

/// The export dialog while it is open.
#[derive(Debug, Clone, PartialEq)]
pub struct Dialog {
    /// The preset last picked (the options below may have been changed since).
    pub preset: Option<String>,
    /// What to write.
    pub format: Format,
    /// Where, as typed.
    pub path: String,
    /// The options.
    pub options: Options,
}

impl Dialog {
    /// A dialog on the first preset, saving as `dir` + the scene's name.
    #[must_use]
    pub fn new(scene_name: &str, dir: &str) -> Self {
        let name: String = scene_name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        let name = if name.is_empty() { "scene".to_owned() } else { name };
        let (format, options, preset) = presets().into_iter().next().map_or_else(|| (Format::Mp4, Options::default(), None), |p| (p.format, p.options, Some(p.id)));
        Self { preset, format, path: with_extension(&format!("{dir}{name}"), format), options }
    }
}

/// The window's export state: the dialog and the queue.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExportUi {
    /// The dialog, when open.
    pub dialog: Option<Dialog>,
    /// The queue.
    pub queue: Queue,
    /// Whether the queue window is open.
    pub queue_open: bool,
}

/// `path` with the extension of `format` (a folder format drops it).
#[must_use]
pub fn with_extension(path: &str, format: Format) -> String {
    Path::new(path).with_extension(format.extension()).display().to_string()
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
    /// The project file the scene came from, for the re-export command of the After Effects package.
    pub project: Option<PathBuf>,
}

fn pixels(extent: f64, scale: f64) -> u32 {
    anim_core::num::round_f64_to_u32((extent * scale).max(1.0))
}

fn make_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    Ok(())
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<u64, String> {
    make_parent(path)?;
    std::fs::write(path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
}

fn write_png(path: &Path, picture: &crate::compose::Picture) -> Result<u64, String> {
    let bytes = anim_export::encode_png(picture.width, picture.height, &picture.rgba).map_err(|e| format!("encoding the PNG: {e}"))?;
    write_file(path, &bytes)
}

/// Which frames an export renders: the range of the scene, and how output frames map onto it.
struct Plan {
    first: u32,
    source_count: u32,
    out_count: u32,
    out_rate: FrameRate,
}

impl Plan {
    fn new(scene: &Scene, format: Format, options: &Options) -> Result<Self, String> {
        let total = scene.frame_count();
        if total == 0 {
            return Err("the scene has no frames".to_owned());
        }
        let first = options.start_frame.min(total.saturating_sub(1));
        let source_count = match format {
            Format::Png => 1,
            _ => options.frames.unwrap_or_else(|| total.saturating_sub(first)).clamp(1, total.saturating_sub(first)),
        };
        let out_rate = match options.fps {
            None => FrameRate { num: scene.fps.num.max(1), den: scene.fps.den.max(1) },
            Some(fps) if fps.is_finite() && fps > 0.0 && fps <= 240.0 => FrameRate::from_f64(fps),
            Some(fps) => return Err(format!("fps must be between 0 and 240, got {fps}")),
        };
        let out_count = if options.fps.is_none() || format == Format::Png {
            source_count
        } else {
            let length = scene.frame_time(i64::from(first.saturating_add(source_count))).0.saturating_sub(scene.frame_time(i64::from(first)).0);
            u32::try_from(out_rate.frame_at(Tick(length.saturating_sub(1).max(0))).saturating_add(1)).unwrap_or(1).max(1)
        };
        if out_count > MAX_FRAMES {
            return Err(format!("{out_count} frames is more than the limit of {MAX_FRAMES}"));
        }
        Ok(Self { first, source_count, out_count, out_rate })
    }

    /// The scene frame shown by output frame `n`.
    fn source_frame(&self, scene: &Scene, n: u32) -> u32 {
        if self.out_rate.num == scene.fps.num && self.out_rate.den == scene.fps.den {
            return self.first.saturating_add(n.min(self.source_count.saturating_sub(1)));
        }
        let offset = u32::try_from(scene.frame_at(self.out_rate.tick_of(i64::from(n))).max(0)).unwrap_or(u32::MAX);
        self.first.saturating_add(offset.min(self.source_count.saturating_sub(1)))
    }
}

/// Gif frame delays in hundredths of a second for `count` frames at `fps`.
fn gif_delays(count: u32, fps: f64) -> Vec<u16> {
    anim_export::frame_delays_cs(count, fps, 2).unwrap_or_else(|_| vec![4; anim_core::num::u32_to_usize(count)])
}

/// The sound of the part of the scene being exported, or `None` when nothing sounds.
fn sound_of(scene: &Scene, media: &MediaStore, plan: &Plan) -> Result<Option<anim_audio::PcmBuffer>, String> {
    let has_sound = scene.audio.iter().filter(|t| t.enabled).flat_map(|t| &t.clips).any(|c| c.enabled && media.get(&c.media).is_some());
    if !has_sound {
        return Ok(None);
    }
    let (mixed, _missing) = mix_scene(scene, media, anim_audio::MixFormat::STEREO_48K).map_err(|e| e.to_string())?;
    let from = scene.frame_time(i64::from(plan.first));
    let length = Tick(scene.frame_time(i64::from(plan.first.saturating_add(plan.source_count))).0.saturating_sub(from.0));
    Ok(Some(mixed.slice_ticks(TimeRange::new(from, length))))
}

/// Runs the export. `progress(fraction, message)` returns false to cancel.
///
/// # Errors
/// A message naming the problem (bad options, a render or encoder failure, a write failure, cancellation).
#[expect(clippy::too_many_lines, reason = "one arm per format")]
pub fn run(job: Job, progress: &mut dyn FnMut(f32, &str) -> bool) -> Result<Value, String> {
    let Job { cast, scene, media, format, path, options, project } = job;
    if !options.scale.is_finite() || options.scale <= 0.0 || options.scale > 8.0 {
        return Err("scale must be a number between 0 and 8".to_owned());
    }
    let plan = Plan::new(&scene, format, &options)?;
    let started = std::time::Instant::now();
    if format == Format::Wav {
        progress(0.0, "mixing");
        let sound = sound_of(&scene, &media, &plan)?.ok_or_else(|| "the scene has no sound to export".to_owned())?;
        let bytes = write_file(&path, &wav_bytes(&sound))?;
        return Ok(json!({ "format": "wav", "path": path.display().to_string(), "bytes": bytes, "seconds": started.elapsed().as_secs_f64() }));
    }
    let transparent = options.transparent.unwrap_or_else(|| format.transparent_by_default());
    let stage = Stage::new(cast);
    let mut evaluator = Evaluator::new(&scene, &stage.cast().templates()).map_err(|e| e.to_string())?;
    let mut renderer = Renderer::new();
    let (mut width, mut height) = options.pixels(scene.size.width, scene.size.height);
    if matches!(format, Format::Mp4 | Format::Mov | Format::AfterEffects) {
        width = width.saturating_add(width % 2);
        height = height.saturating_add(height % 2);
    }
    let count = plan.out_count;
    let mut render_at = |n: u32| -> Result<anim_display::Image, String> {
        let poses = evaluator.poses(i64::from(plan.source_frame(&scene, n)));
        stage.render_scene_image(&mut renderer, &scene, &poses, options.scale, Some((width, height)), transparent).map_err(|e| e.to_string())
    };
    let picture_of = |image: &anim_display::Image| crate::compose::Picture { width: image.width, height: image.height, rgba: straight_rgba(image) };
    let advance = |progress: &mut dyn FnMut(f32, &str) -> bool, n: u32| {
        if progress(fraction(n, count), &format!("frame {} of {count}", n.saturating_add(1))) { Ok(()) } else { Err("cancelled".to_owned()) }
    };
    match format {
        Format::Png => {
            progress(0.0, "rendering");
            let image = render_at(0)?;
            let picture = picture_of(&image);
            let bytes = write_png(&path, &picture)?;
            Ok(
                json!({ "format": "png", "path": path.display().to_string(), "frame": plan.first, "width": picture.width, "height": picture.height, "bytes": bytes }),
            )
        }
        Format::PngSequence => {
            let mut bytes = 0_u64;
            for n in 0..count {
                advance(progress, n)?;
                let picture = picture_of(&render_at(n)?);
                bytes = bytes.saturating_add(write_png(&path.join(format!("frame_{:05}.png", n.saturating_add(1))), &picture)?);
            }
            Ok(
                json!({ "format": "png-sequence", "path": path.display().to_string(), "frames": count, "width": width, "height": height, "bytes": bytes, "seconds": started.elapsed().as_secs_f64() }),
            )
        }
        Format::Gif => {
            make_parent(&path)?;
            let (Ok(w16), Ok(h16)) = (u16::try_from(width), u16::try_from(height)) else {
                return Err(format!("{width}x{height} does not fit a GIF (65535 pixels per side)"));
            };
            let file = std::fs::File::create(&path).map_err(|e| format!("creating {}: {e}", path.display()))?;
            let mut encoder = gif::Encoder::new(BufWriter::new(file), w16, h16, &[]).map_err(|e| format!("starting the GIF: {e}"))?;
            encoder.set_repeat(gif::Repeat::Infinite).map_err(|e| format!("starting the GIF: {e}"))?;
            let delays = gif_delays(count, plan.out_rate.as_f64());
            let written = (|| -> Result<(), String> {
                for (n, delay) in (0..count).zip(delays) {
                    advance(progress, n)?;
                    let mut pixels = straight_rgba(&render_at(n)?);
                    let mut any_transparent = false;
                    let (chunks, _) = pixels.as_chunks_mut::<4>();
                    for [_, _, _, alpha] in chunks {
                        if *alpha < 128 {
                            *alpha = 0;
                            any_transparent = true;
                        } else {
                            *alpha = 255;
                        }
                    }
                    let mut frame = gif::Frame::from_rgba_speed(w16, h16, &mut pixels, 10);
                    frame.delay = delay;
                    frame.dispose = if any_transparent { gif::DisposalMethod::Background } else { gif::DisposalMethod::Keep };
                    encoder.write_frame(&frame).map_err(|e| format!("encoding a frame: {e}"))?;
                }
                Ok(())
            })();
            drop(encoder);
            if let Err(message) = written {
                let _ = std::fs::remove_file(&path);
                return Err(message);
            }
            let bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
            Ok(
                json!({ "format": "gif", "path": path.display().to_string(), "frames": count, "width": width, "height": height, "bytes": bytes, "seconds": started.elapsed().as_secs_f64() }),
            )
        }
        Format::Mp4 | Format::Mov => {
            let codec = if format == Format::Mp4 { Codec::H264 { rate: H264Rate::default() } } else { Codec::ProRes4444 { alpha: true } };
            let mut settings = VideoSettings::new(width, height, plan.out_rate, codec);
            if let Some(sound) = sound_of(&scene, &media, &plan)? {
                settings = settings.with_audio(video_track(&sound));
            }
            make_parent(&path)?;
            let mut encoder = anim_video::start(&path, settings).map_err(|e| format!("starting the encoder: {e}"))?;
            for n in 0..count {
                advance(progress, n)?;
                encoder.send_frame(render_at(n)?).map_err(|e| format!("encoding a frame: {e}"))?;
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
        Format::AfterEffects => {
            let stem = path.file_name().map_or_else(|| "scene".to_owned(), |n| n.to_string_lossy().into_owned());
            let mov_path = path.join(format!("{stem}.mov"));
            let wav_path = path.join("audio.wav");
            let sound = sound_of(&scene, &media, &plan)?;
            let mut settings = VideoSettings::new(width, height, plan.out_rate, Codec::ProRes4444 { alpha: true });
            let mut bytes = 0_u64;
            if let Some(sound) = &sound {
                settings = settings.with_audio(video_track(sound));
                bytes = bytes.saturating_add(write_file(&wav_path, &wav_bytes(sound))?);
            }
            make_parent(&mov_path)?;
            let mut encoder = anim_video::start(&mov_path, settings).map_err(|e| format!("starting the encoder: {e}"))?;
            for n in 0..count {
                advance(progress, n)?;
                let image = render_at(n)?;
                let picture = picture_of(&image);
                bytes = bytes.saturating_add(write_png(&path.join("frames").join(format!("frame_{:05}.png", n.saturating_add(1))), &picture)?);
                encoder.send_frame(image).map_err(|e| format!("encoding a frame: {e}"))?;
            }
            let summary = encoder.finish().map_err(|e| format!("finishing the video: {e}"))?;
            bytes = bytes.saturating_add(summary.bytes);
            let command = reexport_command(&path, &stem, project.as_deref(), &scene, &plan, &options);
            let manifest = json!({
                "format": "charcraft-after-effects-package",
                "scene": scene.name,
                "width": width,
                "height": height,
                "fps": plan.out_rate.as_f64(),
                "frames": count,
                "alpha": true,
                "png_sequence": "frames/frame_00001.png",
                "movie": format!("{stem}.mov"),
                "audio": sound.is_some().then_some("audio.wav"),
                "reexport": command,
            });
            let manifest_text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
            write_file(&path.join("export.json"), manifest_text.as_bytes())?;
            let readme = format!(
                "CharCraft package for After Effects and Premiere Pro\n\n\
                 {stem}.mov     ProRes 4444 with alpha{}\n\
                 frames/        PNG sequence, straight alpha: import as a sequence ({count} frames at {} fps)\n\
                 audio.wav      the scene's sound{}\n\
                 export.json    what is in this folder\n\n\
                 Adobe Dynamic Link is proprietary and is not available. This folder is the supported way in:\n\
                 import the MOV, or the PNG sequence plus the WAV, and run the command below to refresh the files\n\
                 after the project changes (the paths are relative to the folder that holds this one).\n\n{command}\n",
                if sound.is_some() { " and the sound" } else { "" },
                plan.out_rate.as_f64(),
                if sound.is_some() { "" } else { " (none: the scene has no sound, so there is no file)" },
            );
            write_file(&path.join("REEXPORT.txt"), readme.as_bytes())?;
            Ok(json!({
                "format": "after-effects",
                "path": path.display().to_string(),
                "movie": mov_path.display().to_string(),
                "frames": count,
                "width": width,
                "height": height,
                "bytes": bytes,
                "audio": sound.is_some(),
                "reexport": command,
                "seconds": started.elapsed().as_secs_f64(),
            }))
        }
        Format::Wav => Err("the sound is written before the picture is set up".to_owned()),
    }
}

/// The `charcraft export` command that makes the movie of a package again, with the project relative to the folder
/// that holds the package when it lies below it.
fn reexport_command(package: &Path, stem: &str, project: Option<&Path>, scene: &Scene, plan: &Plan, options: &Options) -> String {
    let base = package.parent().filter(|p| !p.as_os_str().is_empty());
    let shown = |p: &Path| base.and_then(|b| p.strip_prefix(b).ok()).unwrap_or(p).display().to_string();
    let project_text = project.map_or_else(|| "<project.chproj>".to_owned(), shown);
    let start = scene.frame_time(i64::from(plan.first)).seconds();
    let length = scene.frame_time(i64::from(plan.first.saturating_add(plan.source_count))).seconds() - start;
    format!(
        "charcraft export \"{project_text}\" --scene \"{}\" --start {start:.4} --seconds {length:.4} --scale {} -o \"{}/{stem}.mov\"",
        scene.name,
        options.scale,
        shown(package)
    )
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
        Job { cast, scene, media: MediaStore::new(), format, path, options: Options::default(), project: None }
    }

    fn with_sound(job: &mut Job) {
        let samples: Vec<f32> = (0..9600_u32).map(|i| anim_core::num::f64_to_f32((f64::from(i) * 0.05).sin() * 0.3)).collect();
        job.media.insert("tone", anim_audio::PcmBuffer::new(48_000, 1, samples)).unwrap();
        let mut track = charcraft_record::AudioTrack::new("Sound");
        track.clips.push(charcraft_record::AudioClip::new("tone", "tone", Tick::ZERO));
        job.scene.audio.push(track);
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

    #[test]
    fn a_gif_has_the_requested_frames_and_a_lower_rate_drops_frames() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.gif");
        let done = run(job(Format::Gif, file.clone()), &mut |_, _| true).unwrap();
        assert_eq!(done["frames"], 6);
        let bytes = std::fs::read(&file).unwrap();
        assert_eq!(&bytes[..6], b"GIF89a");
        let mut slow = job(Format::Gif, dir.path().join("b.gif"));
        slow.options.fps = Some(15.0);
        slow.options.transparent = Some(true);
        let done = run(slow, &mut |_, _| true).unwrap();
        assert_eq!(done["frames"], 3);
        let mut decoder = gif::DecodeOptions::new();
        decoder.set_color_output(gif::ColorOutput::RGBA);
        let mut reader = decoder.read_info(std::fs::File::open(dir.path().join("b.gif")).unwrap()).unwrap();
        let mut frames = 0;
        while reader.read_next_frame().unwrap().is_some() {
            frames += 1;
        }
        assert_eq!(frames, 3);
    }

    #[test]
    fn sound_alone_is_a_wav_and_no_sound_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut with = job(Format::Wav, dir.path().join("a.wav"));
        with_sound(&mut with);
        let done = run(with, &mut |_, _| true).unwrap();
        assert!(done["bytes"].as_u64().unwrap() > 1000);
        let bytes = std::fs::read(dir.path().join("a.wav")).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        let none = run(job(Format::Wav, dir.path().join("b.wav")), &mut |_, _| true);
        assert!(none.unwrap_err().contains("no sound"));
    }

    #[test]
    fn the_after_effects_package_holds_frames_sound_movie_and_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("pkg");
        let mut package = job(Format::AfterEffects, out.clone());
        with_sound(&mut package);
        package.project = Some(dir.path().join("show.chproj"));
        let done = run(package, &mut |_, _| true).unwrap();
        assert_eq!(done["frames"], 6);
        assert!(out.join("frames/frame_00006.png").is_file());
        assert!(out.join("audio.wav").is_file());
        assert!(out.join("pkg.mov").is_file());
        let readme = std::fs::read_to_string(out.join("REEXPORT.txt")).unwrap();
        assert!(readme.contains("charcraft export \"show.chproj\""), "{readme}");
        let manifest: Value = serde_json::from_str(&std::fs::read_to_string(out.join("export.json")).unwrap()).unwrap();
        assert_eq!(manifest["frames"], 6);
    }

    #[test]
    fn presets_resolve_as_formats_with_options_merged() {
        let (format, options) = resolve("gif-web", &Value::Null).unwrap();
        assert_eq!((format, options.fps), (Format::Gif, Some(15.0)));
        let (_, options) = resolve("gif-web", &json!({ "scale": 1.0 })).unwrap();
        assert_eq!((options.scale, options.fps), (1.0, Some(15.0)));
        assert_eq!(resolve("mp4", &Value::Null).unwrap().0, Format::Mp4);
        assert!(resolve("nope", &Value::Null).is_err());
        assert!(presets().iter().all(|p| Format::parse(&p.id).is_none()));
    }

    #[test]
    fn the_queue_runs_items_in_order() {
        let mut queue = Queue::default();
        queue.push(Format::Mp4, "a.mp4".into(), Options::default());
        queue.push(Format::Gif, "b.gif".into(), Options::default());
        assert!(queue.take_next().is_none());
        queue.running = true;
        assert_eq!(queue.take_next().unwrap().1, PathBuf::from("a.mp4"));
        queue.finish(Err("boom".to_owned()));
        assert_eq!(queue.take_next().unwrap().1, PathBuf::from("b.gif"));
        queue.finish(Ok(()));
        assert!(queue.take_next().is_none() && !queue.running);
        queue.clear_finished();
        assert!(queue.items.is_empty());
    }
}
