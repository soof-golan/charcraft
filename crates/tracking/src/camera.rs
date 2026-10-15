//! Webcam capture through nokhwa: `Video4Linux` on Linux, `AVFoundation` on macOS, Media Foundation on Windows.
//!
//! [`list_cameras`] enumerates devices; [`CameraSource::open`] opens one at a requested resolution and delivers RGB
//! [`Frame`]s as a [`FrameSource`]. Cameras are asked for raw YUYV first (cheap to convert, no JPEG artefacts) and
//! for Motion-JPEG when the device has no raw mode at that size; Motion-JPEG is decoded with the pure-Rust `image`
//! crate, so no C JPEG library is involved.

use std::time::Instant;

use anim_core::Tick;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution};
use serde::{Deserialize, Serialize};

use crate::actor::CameraSpec;
use crate::error::{Error, Result};
use crate::frame::Frame;
use crate::source::FrameSource;

fn camera_error(e: impl std::fmt::Display) -> Error {
    Error::Camera(e.to_string())
}

/// A camera the system reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraDevice {
    /// The index to put in [`CameraSpec::index`].
    pub index: u32,
    /// The device's name.
    pub name: String,
    /// The driver's description (bus, backend).
    pub description: String,
}

/// Asks the system for permission to use cameras. Only macOS asks; elsewhere this succeeds at once.
///
/// # Errors
/// [`Error::Camera`] when access is denied or the question is not answered within 30 seconds.
pub fn request_access() -> Result<()> {
    if nokhwa::nokhwa_check() {
        return Ok(());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    nokhwa::nokhwa_initialize(move |granted| {
        let _ = tx.send(granted);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok(true) => Ok(()),
        Ok(false) => Err(Error::Camera("camera access was denied".into())),
        // Platforms without a permission prompt never call back.
        Err(_) => nokhwa::nokhwa_check().then_some(()).ok_or_else(|| Error::Camera("camera access was not granted".into())),
    }
}

/// Lists the cameras attached to the system.
///
/// # Errors
/// [`Error::Camera`] when the platform's camera API fails.
pub fn list_cameras() -> Result<Vec<CameraDevice>> {
    request_access()?;
    let infos = nokhwa::query(ApiBackend::Auto).map_err(camera_error)?;
    Ok(infos
        .into_iter()
        .enumerate()
        .map(|(position, info)| CameraDevice {
            index: info.index().as_index().unwrap_or_else(|_| u32::try_from(position).unwrap_or(u32::MAX)),
            name: info.human_name(),
            description: info.description().to_owned(),
        })
        .collect())
}

/// An open webcam delivering RGB frames.
pub struct CameraSource {
    camera: nokhwa::Camera,
    started: Instant,
    name: String,
}

impl std::fmt::Debug for CameraSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CameraSource").field("name", &self.name).finish_non_exhaustive()
    }
}

impl CameraSource {
    /// Opens the camera `spec` names, as close to its resolution and frame rate as the device allows.
    ///
    /// # Errors
    /// [`Error::Camera`] when access is denied, the device is missing or busy, or no format fits.
    pub fn open(spec: &CameraSpec) -> Result<Self> {
        request_access()?;
        let resolution = Resolution::new(spec.width, spec.height);
        let mut last = Error::Camera("the camera has no usable format".into());
        for format in [FrameFormat::YUYV, FrameFormat::MJPEG, FrameFormat::NV12] {
            let wanted = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(CameraFormat::new(resolution, format, spec.fps)));
            match nokhwa::Camera::new(CameraIndex::Index(spec.index), wanted) {
                Ok(mut camera) => {
                    camera.open_stream().map_err(camera_error)?;
                    let name = camera.info().human_name();
                    return Ok(Self { camera, started: Instant::now(), name });
                }
                Err(e) => last = camera_error(e),
            }
        }
        Err(last)
    }

    /// The format the device settled on, as `WIDTHxHEIGHT @ FPS FORMAT`.
    #[must_use]
    pub fn describe_format(&self) -> String {
        let f = self.camera.camera_format();
        format!("{}x{} @ {} {:?}", f.resolution().width(), f.resolution().height(), f.frame_rate(), f.format())
    }
}

impl FrameSource for CameraSource {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn next_frame(&mut self) -> Result<Option<(Tick, Frame)>> {
        let buffer = self.camera.frame().map_err(camera_error)?;
        let time = Tick::from_seconds_f64(self.started.elapsed().as_secs_f64());
        let frame = if buffer.source_frame_format() == FrameFormat::MJPEG {
            let decoded = image::load_from_memory_with_format(buffer.buffer(), image::ImageFormat::Jpeg).map_err(camera_error)?.to_rgb8();
            Frame::new(decoded.width(), decoded.height(), decoded.into_raw())?
        } else {
            let resolution = buffer.resolution();
            let rgb = buffer.decode_image::<RgbFormat>().map_err(camera_error)?;
            Frame::new(resolution.width(), resolution.height(), rgb.into_raw())?
        };
        Ok(Some((time, frame)))
    }
}

impl Drop for CameraSource {
    fn drop(&mut self) {
        // Release the device so another application (or the next Start) can open it.
        let _ = self.camera.stop_stream();
    }
}
