//! Writing rendered frames as a video file or a PNG sequence.

use std::path::{Path, PathBuf};

use anim_audio::PcmBuffer;
use anim_core::{FrameRate, Tick};
use anim_display::Image;
use anim_video::{AudioTrack, Codec, H264Rate, VideoSettings};
use color_eyre::eyre::{Result, WrapErr, bail};

use crate::frames::save_png;

/// Most frames one command renders; a guard against a mistyped duration or frame rate.
pub const MAX_FRAMES: i64 = 2_000_000;

/// What gets written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// H.264 in an MP4.
    Mp4,
    /// `ProRes` 4444 with alpha in a MOV.
    Mov,
    /// One PNG per frame in a folder.
    PngSequence,
    /// An animated GIF.
    Gif,
    /// The sound alone, as a WAV (no frames are rendered).
    Wav,
}

impl Format {
    /// By the output name: `.mp4`, `.mov`, `.gif`, `.wav`, anything else is a folder of PNGs.
    #[must_use]
    pub fn of(output: &Path) -> Self {
        match output.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref() {
            Some("mp4") => Self::Mp4,
            Some("mov") => Self::Mov,
            Some("gif") => Self::Gif,
            Some("wav") => Self::Wav,
            _ => Self::PngSequence,
        }
    }

    /// Whether frames should keep their transparency.
    #[must_use]
    pub const fn transparent(self) -> bool {
        matches!(self, Self::Mov | Self::PngSequence)
    }
}

/// The shape of the output.
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub frames: i64,
}

impl Plan {
    /// Plans an output of about `width` x `height` pixels (rounded to even sizes for video).
    pub fn new(output: &Path, width: u32, height: u32, fps: f64, duration: Tick) -> Result<Self> {
        if !fps.is_finite() || fps <= 0.0 || fps > 240.0 {
            bail!("the frame rate must be between 0 and 240, got {fps}");
        }
        let format = Format::of(output);
        let fps = FrameRate::from_f64(fps);
        let frames = fps.frame_at(Tick(duration.0.saturating_sub(1).max(0))).saturating_add(1);
        if duration.0 <= 0 {
            bail!("there is nothing to render: the duration is zero");
        }
        if frames > MAX_FRAMES {
            bail!("{frames} frames is more than the limit of {MAX_FRAMES}");
        }
        let even = |n: u32| if format == Format::PngSequence { n } else { n.saturating_add(n % 2) };
        Ok(Self { format, width: even(width), height: even(height), fps, frames })
    }

    /// The start time of frame `index`.
    #[must_use]
    pub fn time_of(&self, index: i64) -> Tick {
        self.fps.tick_of(index)
    }
}

/// Audio for the encoder: the part of `audio` from `start` for `length`, in a layout AAC accepts.
pub fn audio_track(audio: &PcmBuffer, start: Tick, length: Tick) -> Result<AudioTrack> {
    let mut buffer = audio.slice_ticks(anim_core::TimeRange::new(start, length));
    if buffer.channels > 2 {
        buffer = buffer.remix(2);
    }
    if !matches!(buffer.sample_rate, 44_100 | 48_000) {
        buffer = anim_audio::resample(&buffer, 48_000).wrap_err("resampling the audio")?;
    }
    buffer.sanitize();
    Ok(AudioTrack { samples: buffer.samples, sample_rate: buffer.sample_rate, channels: buffer.channels })
}

/// Renders every frame through `frame` and writes the result to `output`. Returns a one-line summary.
pub fn write(output: &Path, plan: &Plan, audio: Option<AudioTrack>, mut frame: impl FnMut(i64, Tick) -> Result<Image>) -> Result<String> {
    let started = std::time::Instant::now();
    let report = |done: i64| {
        if done % 30 == 0 {
            eprintln!("frame {done}/{}", plan.frames);
        }
    };
    match plan.format {
        Format::PngSequence => {
            std::fs::create_dir_all(output).wrap_err_with(|| format!("creating {}", output.display()))?;
            for index in 0..plan.frames {
                report(index);
                let image = frame(index, plan.time_of(index))?;
                let name: PathBuf = output.join(format!("frame_{:05}.png", index.saturating_add(1)));
                save_png(&name, &image)?;
            }
        }
        Format::Wav => bail!("a WAV has no picture"),
        Format::Gif => {
            let (Ok(w16), Ok(h16)) = (u16::try_from(plan.width), u16::try_from(plan.height)) else {
                bail!("{}x{} does not fit a GIF (65535 pixels per side)", plan.width, plan.height);
            };
            if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).wrap_err_with(|| format!("creating {}", parent.display()))?;
            }
            let file = std::fs::File::create(output).wrap_err_with(|| format!("creating {}", output.display()))?;
            let mut encoder = gif::Encoder::new(std::io::BufWriter::new(file), w16, h16, &[]).wrap_err("starting the GIF")?;
            encoder.set_repeat(gif::Repeat::Infinite).wrap_err("starting the GIF")?;
            let delays = anim_export::frame_delays_cs(u32::try_from(plan.frames).unwrap_or(u32::MAX), plan.fps.as_f64(), 2).wrap_err("the GIF frame delays")?;
            for (index, delay) in (0..plan.frames).zip(delays) {
                report(index);
                let mut pixels = anim_render::straight_rgba(&frame(index, plan.time_of(index))?);
                let mut any_transparent = false;
                let (chunks, _) = pixels.as_chunks_mut::<4>();
                for [_, _, _, alpha] in chunks {
                    any_transparent |= *alpha < 128;
                    *alpha = if *alpha < 128 { 0 } else { 255 };
                }
                let mut gif_frame = gif::Frame::from_rgba_speed(w16, h16, &mut pixels, 10);
                gif_frame.delay = delay;
                gif_frame.dispose = if any_transparent { gif::DisposalMethod::Background } else { gif::DisposalMethod::Keep };
                encoder.write_frame(&gif_frame).wrap_err("encoding a frame")?;
            }
            drop(encoder);
        }
        Format::Mp4 | Format::Mov => {
            let codec = if plan.format == Format::Mp4 { Codec::H264 { rate: H264Rate::default() } } else { Codec::ProRes4444 { alpha: true } };
            let mut settings = VideoSettings::new(plan.width, plan.height, plan.fps, codec);
            if let Some(track) = audio {
                settings = settings.with_audio(track);
            }
            if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).wrap_err_with(|| format!("creating {}", parent.display()))?;
            }
            let mut encoder = anim_video::start(output, settings).wrap_err("starting the encoder")?;
            for index in 0..plan.frames {
                report(index);
                let image = frame(index, plan.time_of(index))?;
                encoder.send_frame(image).wrap_err("encoding a frame")?;
            }
            let summary = encoder.finish().wrap_err("finishing the video")?;
            return Ok(format!(
                "{}: {} frames, {}x{} at {} fps, {} bytes, audio {} ({:.1} s to encode)",
                output.display(),
                summary.frames,
                summary.width,
                summary.height,
                plan.fps.as_f64(),
                summary.bytes,
                if summary.has_audio { "yes" } else { "no" },
                started.elapsed().as_secs_f64()
            ));
        }
    }
    let what = if plan.format == Format::Gif { "GIF" } else { "PNG" };
    Ok(format!("{}: {} {what} frames, {}x{} ({:.1} s)", output.display(), plan.frames, plan.width, plan.height, started.elapsed().as_secs_f64()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_output_name_picks_the_format() {
        for (name, format) in [("a.mp4", Format::Mp4), ("a.MOV", Format::Mov), ("a.gif", Format::Gif), ("a.wav", Format::Wav), ("frames", Format::PngSequence)]
        {
            assert_eq!(Format::of(Path::new(name)), format, "{name}");
        }
    }

    #[test]
    fn a_gif_has_the_planned_frames_and_a_lower_rate_means_fewer() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a.gif");
        let plan = Plan::new(&out, 8, 6, 12.0, Tick::from_seconds_f64(0.5)).unwrap();
        assert_eq!(plan.frames, 6);
        let summary = write(&out, &plan, None, |_, _| Ok(Image { width: 8, height: 6, premultiplied_rgba: vec![200; 8 * 6 * 4] })).unwrap();
        assert!(summary.contains("6 GIF frames"), "{summary}");
        assert_eq!(&std::fs::read(&out).unwrap()[..6], b"GIF89a");
    }
}
