//! Picture files standing in for a webcam.
//!
//! A still photo, an animated GIF or a folder of images (a video turned into frames) is played back at a fixed rate
//! like a camera. Used on machines without a camera and to check the live pipeline end to end.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anim_core::Tick;
use image::{AnimationDecoder, DynamicImage, ImageFormat};

use crate::error::{Error, Result};
use crate::frame::Frame;
use crate::source::FrameSource;

/// Most frames read from one GIF or folder, so a hostile or huge file cannot exhaust memory.
pub const MAX_FILE_FRAMES: usize = 1800;
/// Longest side of a frame, in pixels: larger pictures are scaled down on load (the face networks work on small
/// crops anyway).
pub const MAX_SIDE: u32 = 1280;

fn io(path: &Path, source: std::io::Error) -> Error {
    Error::Io { path: path.to_path_buf(), source }
}

fn picture(path: &Path, e: impl std::fmt::Display) -> Error {
    Error::Camera(format!("{}: {e}", path.display()))
}

fn frame_of(image: DynamicImage) -> Result<Frame> {
    let image = if image.width() > MAX_SIDE || image.height() > MAX_SIDE { image.thumbnail(MAX_SIDE, MAX_SIDE) } else { image };
    let rgb = image.to_rgb8();
    Frame::new(rgb.width(), rgb.height(), rgb.into_raw())
}

/// A [`FrameSource`] over picture files, paced at a frame rate. One picture repeats forever; several loop.
#[derive(Clone, Debug)]
pub struct ImageFiles {
    name: String,
    frames: Vec<Frame>,
    next: usize,
    step: Duration,
    due: Option<Instant>,
    emitted: i64,
    tick: Tick,
}

impl ImageFiles {
    /// Reads `path`: an image, an animated GIF, or a directory whose images (sorted by name) are the frames.
    /// `fps` paces still images and folders; GIFs keep their own timing only for their frame count (every frame is
    /// played at `fps`, which keeps the playback rate steady).
    ///
    /// # Errors
    /// [`Error::Camera`] when the file or folder holds no readable picture; [`Error::Io`] when it cannot be read.
    pub fn open(path: &Path, fps: u32) -> Result<Self> {
        let frames = if path.is_dir() { read_folder(path)? } else { read_file(path)? };
        if frames.is_empty() {
            return Err(Error::Camera(format!("{}: no pictures to track", path.display())));
        }
        let fps = fps.clamp(1, 120);
        let name = format!("file {}", path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned()));
        Ok(Self {
            name,
            frames,
            next: 0,
            step: Duration::from_secs(1).checked_div(fps).unwrap_or_default(),
            due: None,
            emitted: 0,
            tick: Tick::from_units(1, i64::from(fps)),
        })
    }

    /// How many distinct pictures play before the loop repeats.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.frames.len()
    }

    /// Whether there are no pictures (never true for a source made by [`ImageFiles::open`]).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

fn read_file(path: &Path) -> Result<Vec<Frame>> {
    let bytes = std::fs::read(path).map_err(|e| io(path, e))?;
    let format = image::guess_format(&bytes).map_err(|e| picture(path, e))?;
    if format == ImageFormat::Gif {
        let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&bytes)).map_err(|e| picture(path, e))?;
        let mut frames = Vec::new();
        for frame in decoder.into_frames().take(MAX_FILE_FRAMES) {
            let frame = frame.map_err(|e| picture(path, e))?;
            frames.push(frame_of(DynamicImage::ImageRgba8(frame.into_buffer()))?);
        }
        return Ok(frames);
    }
    let image = image::load_from_memory_with_format(&bytes, format).map_err(|e| picture(path, e))?;
    Ok(vec![frame_of(image)?])
}

fn read_folder(dir: &Path) -> Result<Vec<Frame>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| io(dir, e))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && ImageFormat::from_path(p).is_ok())
        .collect();
    paths.sort();
    paths.truncate(MAX_FILE_FRAMES);
    let mut frames = Vec::with_capacity(paths.len());
    for path in &paths {
        frames.extend(read_file(path)?.into_iter().take(1));
    }
    Ok(frames)
}

impl FrameSource for ImageFiles {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn next_frame(&mut self) -> Result<Option<(Tick, Frame)>> {
        // Pace like a camera: frame n is due n steps after the first.
        let now = Instant::now();
        let due = self.due.unwrap_or(now);
        if let Some(wait) = due.checked_duration_since(now) {
            std::thread::sleep(wait);
        }
        self.due = Some(due.checked_add(self.step).unwrap_or(now));
        let index = self.next.checked_rem(self.frames.len()).unwrap_or(0);
        self.next = index.saturating_add(1);
        let time = Tick(self.tick.0.saturating_mul(self.emitted));
        self.emitted = self.emitted.saturating_add(1);
        Ok(self.frames.get(index).cloned().map(|f| (time, f)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(path: &Path, colour: [u8; 3]) {
        image::RgbImage::from_pixel(8, 6, image::Rgb(colour)).save(path).unwrap();
    }

    #[test]
    fn a_still_image_repeats_at_the_frame_rate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        write_png(&path, [9, 8, 7]);
        let mut files = ImageFiles::open(&path, 120).unwrap();
        assert_eq!(files.len(), 1);
        let (t0, f0) = files.next_frame().unwrap().unwrap();
        let (t1, f1) = files.next_frame().unwrap().unwrap();
        assert!(t1 > t0 && f0 == f1 && f0.width() == 8);
    }

    #[test]
    fn a_folder_plays_in_name_order_and_loops() {
        let dir = tempfile::tempdir().unwrap();
        write_png(&dir.path().join("2.png"), [0, 0, 255]);
        write_png(&dir.path().join("1.png"), [255, 0, 0]);
        std::fs::write(dir.path().join("notes.txt"), "not a picture").unwrap();
        let mut files = ImageFiles::open(dir.path(), 120).unwrap();
        let reds: Vec<u8> = (0..4).map(|_| files.next_frame().unwrap().unwrap().1.rgb()[0]).collect();
        assert_eq!(reds, [255, 0, 255, 0]);
    }

    #[test]
    fn missing_and_hostile_files_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ImageFiles::open(&dir.path().join("nope.png"), 30).is_err());
        let junk = dir.path().join("junk.png");
        std::fs::write(&junk, b"definitely not an image").unwrap();
        assert!(ImageFiles::open(&junk, 30).is_err());
        let empty = tempfile::tempdir().unwrap();
        assert!(ImageFiles::open(empty.path(), 30).is_err());
    }
}
