//! Stream output for `CharCraft`: where the performing puppet goes when it leaves the window.
//!
//! An [`actor::OutputHandle`] owns one running output at a time. The app sends it the puppet's pictures (RGBA, with
//! transparency) as they are rendered; the actor fits them to the output canvas, repeats the last one at the output's
//! frame rate so consumers never starve, and hands each to the [`Sink`] of the chosen [`Kind`]:
//!
//! | Kind | Carries | Works on | Needs |
//! |---|---|---|---|
//! | [`Kind::Ndi`] | RGBA with alpha | Windows, macOS, Linux | the NDI runtime, loaded when the output starts ([`charcraft_ndi`]) |
//! | [`Kind::VirtualCamera`] | YUYV, no alpha | Linux | the `v4l2loopback` kernel module |
//! | [`Kind::Web`] | MJPEG stream and PNG snapshot with alpha | everywhere | nothing; serves `127.0.0.1` only |
//! | [`Kind::Syphon`] | | macOS | not available in this build |
//! | [`Kind::Spout`] | | Windows | not available in this build |
//!
//! Outputs that cannot carry transparency show the puppet over a key colour ([`Background`]), so a chroma key still
//! works downstream. [`outputs`] reports what each kind needs on this machine, in words a person can act on.

#![allow(clippy::suboptimal_flops, reason = "no fused multiply-add: converted pixels must be identical on every CPU")]
#![cfg_attr(test, allow(clippy::arithmetic_side_effects, reason = "test code"))]

pub mod actor;
pub mod picture;
mod web;

#[cfg(target_os = "linux")]
mod vcam;

use serde::{Deserialize, Serialize};

pub use actor::{OutputHandle, OutputLink, Status};
pub use picture::{KEY_GREEN, Picture};

/// Why an output could not start or stopped.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The output is not available on this machine; the text says why and what to do.
    #[error("{0}")]
    Unavailable(String),
    /// A picture or canvas has an unusable size.
    #[error("a {width}x{height} picture with {got} bytes cannot be used")]
    BadPicture {
        /// Width.
        width: u32,
        /// Height.
        height: u32,
        /// Bytes given.
        got: usize,
    },
    /// Encoding a picture failed.
    #[error("encoding failed: {0}")]
    Encode(String),
    /// The output failed while running.
    #[error("{0}")]
    Output(String),
    /// The output thread is gone.
    #[error("the output thread has stopped")]
    Gone,
}

/// The ways out of the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// An NDI source on the network (OBS, vMix, Resolume, ...).
    Ndi,
    /// A webcam other programs can pick (Linux: v4l2loopback).
    VirtualCamera,
    /// A local web source: MJPEG video and PNG snapshots with alpha, for browser sources and media sources.
    Web,
    /// A Syphon server (macOS).
    Syphon,
    /// A Spout sender (Windows).
    Spout,
}

impl Kind {
    /// All kinds, in the order the UI lists them.
    pub const ALL: [Self; 5] = [Self::Ndi, Self::VirtualCamera, Self::Web, Self::Syphon, Self::Spout];

    /// The id used in commands.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Ndi => "ndi",
            Self::VirtualCamera => "virtual_camera",
            Self::Web => "web",
            Self::Syphon => "syphon",
            Self::Spout => "spout",
        }
    }

    /// The name shown to people.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ndi => "NDI",
            Self::VirtualCamera => "Virtual camera",
            Self::Web => "Web source (MJPEG / PNG)",
            Self::Syphon => "Syphon",
            Self::Spout => "Spout",
        }
    }

    /// Parses an id (any case; `-` and spaces count as `_`).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let key = text.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        Self::ALL.into_iter().find(|k| k.id() == key || (key == "webcam" && *k == Self::VirtualCamera) || (key == "mjpeg" && *k == Self::Web))
    }

    /// Whether this kind can carry transparency.
    #[must_use]
    pub const fn carries_alpha(self) -> bool {
        matches!(self, Self::Ndi | Self::Web | Self::Syphon | Self::Spout)
    }
}

/// What goes behind the puppet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Background {
    /// Nothing: outputs that carry alpha send it as it is, the others show [`KEY_GREEN`].
    Transparent,
    /// A solid colour, for every output.
    Colour([u8; 3]),
}

impl Background {
    /// The colour behind the picture for an output that carries alpha or not.
    #[must_use]
    pub const fn flatten_colour(self) -> [u8; 3] {
        match self {
            Self::Transparent => KEY_GREEN,
            Self::Colour(c) => c,
        }
    }
}

/// How to start an output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Spec {
    /// Which output.
    pub kind: Kind,
    /// The NDI source name or the camera label.
    pub name: String,
    /// Canvas width in pixels (even).
    pub width: u32,
    /// Canvas height in pixels.
    pub height: u32,
    /// Frames per second.
    pub fps: u32,
    /// What goes behind the puppet.
    pub background: Background,
    /// The virtual camera device (`/dev/videoN`); the first v4l2loopback device when unset.
    pub device: Option<String>,
    /// The port of the web source; 0 picks a free one.
    pub port: u16,
}

impl Default for Spec {
    fn default() -> Self {
        Self { kind: Kind::Ndi, name: "CharCraft".to_owned(), width: 1280, height: 720, fps: 30, background: Background::Transparent, device: None, port: 0 }
    }
}

impl Spec {
    /// Checks the numbers.
    ///
    /// # Errors
    /// [`Error::Unavailable`] naming the first value out of range.
    pub fn check(&self) -> Result<(), Error> {
        let bad = |what: &str| Err(Error::Unavailable(format!("{what} is out of range")));
        if !(16..=picture::MAX_SIDE).contains(&self.width) || !self.width.is_multiple_of(2) {
            return bad("the width (an even number from 16 to 8192)");
        }
        if !(16..=picture::MAX_SIDE).contains(&self.height) {
            return bad("the height (16 to 8192)");
        }
        if !(1..=120).contains(&self.fps) {
            return bad("the frame rate (1 to 120)");
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 128 || self.name.contains('\0') {
            return bad("the name (1 to 128 characters)");
        }
        Ok(())
    }
}

/// What one kind needs on this machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputInfo {
    /// The kind.
    pub kind: Kind,
    /// Its name.
    pub label: String,
    /// Whether it can start now.
    pub available: bool,
    /// Whether it carries transparency.
    pub alpha: bool,
    /// What it is, and when it is not available what to install or enable.
    pub note: String,
    /// Devices to choose from (virtual cameras).
    pub devices: Vec<String>,
}

/// Something a picture can be sent to.
pub trait Sink {
    /// A line about where the output goes (a URL, a device, a source name).
    fn describe(&self) -> String;

    /// Sends one picture, already the size of the canvas.
    ///
    /// # Errors
    /// [`Error::Output`] when the output failed; the actor then stops it.
    fn send(&mut self, picture: &Picture) -> Result<(), Error>;
}

/// The process environment, for the NDI runtime lookup.
fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// What each output needs here, as of now.
#[must_use]
pub fn outputs() -> Vec<OutputInfo> {
    Kind::ALL
        .into_iter()
        .map(|kind| {
            let (available, note, devices) = match kind {
                Kind::Ndi => charcraft_ndi::loaded_version(&process_env).map_or_else(
                    || (false, charcraft_ndi::SETUP.to_owned(), Vec::new()),
                    |version| {
                        let found = if version.is_empty() { String::new() } else { format!(" ({version})") };
                        (true, format!("An NDI source other programs on the network can pick. NDI runtime found{found}."), Vec::new())
                    },
                ),
                Kind::VirtualCamera => virtual_camera_info(),
                Kind::Web => (true, "A local web source: video at /stream.mjpg and a PNG with transparency at /frame.png, on 127.0.0.1. Add it in OBS as a Browser or Media source.".to_owned(), Vec::new()),
                Kind::Syphon => (false, "Syphon is not available in this build. Use NDI (Resolume, VDMX, OBS and MadMapper all take it on macOS) or the web source.".to_owned(), Vec::new()),
                Kind::Spout => (false, "Spout is not available in this build. Use NDI (Resolume, OBS and TouchDesigner all take it on Windows) or the web source.".to_owned(), Vec::new()),
            };
            OutputInfo { kind, label: kind.label().to_owned(), available, alpha: kind.carries_alpha(), note, devices }
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn virtual_camera_info() -> (bool, String, Vec<String>) {
    let devices = vcam::loopback_devices();
    if devices.is_empty() {
        (false, vcam::SETUP.to_owned(), Vec::new())
    } else {
        (
            true,
            "A webcam named by the loopback device that Zoom, browsers and OBS can pick. It carries no transparency: the puppet is shown over the key colour."
                .to_owned(),
            devices,
        )
    }
}

#[cfg(not(target_os = "linux"))]
fn virtual_camera_info() -> (bool, String, Vec<String>) {
    (
        false,
        "A system webcam needs a driver this build does not ship. Send NDI or the web source to OBS and start OBS Virtual Camera there: every video call app then sees it as a webcam.".to_owned(),
        Vec::new(),
    )
}

/// Opens the output `spec` asks for.
///
/// # Errors
/// [`Error::Unavailable`] with setup instructions when the machine lacks something, [`Error::Output`] when the output
/// fails to start.
pub fn open(spec: &Spec) -> Result<Box<dyn Sink>, Error> {
    spec.check()?;
    match spec.kind {
        Kind::Ndi => {
            let sender = charcraft_ndi::Sender::new(&spec.name, spec.fps, 1, &process_env).map_err(|e| match e {
                charcraft_ndi::Error::NotInstalled { .. } => Error::Unavailable(e.to_string()),
                other => Error::Output(other.to_string()),
            })?;
            Ok(Box::new(NdiSink { sender }))
        }
        Kind::VirtualCamera => open_virtual_camera(spec),
        Kind::Web => Ok(Box::new(web::WebSink::start(spec)?)),
        Kind::Syphon | Kind::Spout => Err(Error::Unavailable(outputs().into_iter().find(|o| o.kind == spec.kind).map(|o| o.note).unwrap_or_default())),
    }
}

#[cfg(target_os = "linux")]
fn open_virtual_camera(spec: &Spec) -> Result<Box<dyn Sink>, Error> {
    Ok(Box::new(vcam::VirtualCamera::open(spec)?))
}

#[cfg(not(target_os = "linux"))]
fn open_virtual_camera(_: &Spec) -> Result<Box<dyn Sink>, Error> {
    Err(Error::Unavailable(virtual_camera_info().1))
}

struct NdiSink {
    sender: charcraft_ndi::Sender,
}

impl Sink for NdiSink {
    fn describe(&self) -> String {
        format!("NDI source ({})", self.sender.library_path())
    }

    fn send(&mut self, picture: &Picture) -> Result<(), Error> {
        self.sender.send_rgba(picture.width(), picture.height(), picture.rgba()).map_err(|e| Error::Output(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_parse_by_id_and_by_common_names() {
        assert_eq!(Kind::parse("NDI"), Some(Kind::Ndi));
        assert_eq!(Kind::parse(" virtual-camera "), Some(Kind::VirtualCamera));
        assert_eq!(Kind::parse("webcam"), Some(Kind::VirtualCamera));
        assert_eq!(Kind::parse("mjpeg"), Some(Kind::Web));
        assert_eq!(Kind::parse("hdmi"), None);
        assert!(Kind::ALL.iter().all(|k| Kind::parse(k.id()) == Some(*k)));
    }

    #[test]
    fn a_spec_is_checked() {
        assert!(Spec::default().check().is_ok());
        assert!(Spec { width: 1281, ..Spec::default() }.check().is_err());
        assert!(Spec { height: 4, ..Spec::default() }.check().is_err());
        assert!(Spec { fps: 0, ..Spec::default() }.check().is_err());
        assert!(Spec { name: "  ".into(), ..Spec::default() }.check().is_err());
    }

    #[test]
    fn every_output_reports_what_it_needs() {
        let all = outputs();
        assert_eq!(all.len(), Kind::ALL.len());
        for info in &all {
            assert!(!info.note.is_empty(), "{:?}", info.kind);
            assert_eq!(info.alpha, info.kind.carries_alpha());
        }
        assert!(all.iter().any(|o| o.kind == Kind::Web && o.available));
        let spout = all.iter().find(|o| o.kind == Kind::Spout).unwrap();
        assert!(!spout.available && spout.note.contains("NDI"));
    }

    #[test]
    fn unavailable_outputs_fail_with_their_setup_text() {
        let err = open(&Spec { kind: Kind::Syphon, ..Spec::default() }).err().unwrap();
        assert!(matches!(&err, Error::Unavailable(t) if t.contains("Syphon")), "{err}");
        let bad = open(&Spec { fps: 0, ..Spec::default() }).err().unwrap();
        assert!(matches!(bad, Error::Unavailable(_)));
    }

    #[test]
    fn a_background_picks_the_flatten_colour() {
        assert_eq!(Background::Transparent.flatten_colour(), KEY_GREEN);
        assert_eq!(Background::Colour([1, 2, 3]).flatten_colour(), [1, 2, 3]);
    }
}
