//! The real thing: a webcam through nokhwa, face landmarks through ONNX Runtime.
//!
//! [`factory`] is the [`SourceFactory`] a [`TrackingActor`] needs; [`spawn`] starts an actor with it. A start is one of
//! three explicit things, chosen by the [`crate::actor::CameraSpec`]: the webcam (the default), the pictures of a
//! file or folder standing in for one (`file`), or the scripted demo face (`demo`). A failure of the first two is
//! reported as it is (no camera, access denied, library or model missing) and never answered with the demo face.

use crate::actor::{CameraSpec, SourceFactory, TrackingActor, TrackingHandle};
use crate::camera::CameraSource;
use crate::files::ImageFiles;
use crate::onnx::OnnxRuntime;
use crate::source::{FaceSource, FrameSource, ScriptedSource, TrackedCamera};

/// A factory that opens the requested camera (or picture file, or the demo face) and the face models.
///
/// The ONNX Runtime environment is created on the first start and kept for later ones. Models are downloaded into
/// the platform cache on first use.
pub fn factory() -> SourceFactory {
    let mut runtime: Option<OnnxRuntime> = None;
    Box::new(move |spec| {
        if spec.demo {
            let source: Box<dyn FaceSource> = Box::new(ScriptedSource::demo(240, 30));
            return Ok(source);
        }
        // The camera is opened first: with none attached the answer comes at once, before any model is fetched.
        let frames: Box<dyn FrameSource> = open_frames(spec)?;
        if runtime.is_none() {
            runtime = Some(OnnxRuntime::with_default_store()?);
        }
        let tracker = match &runtime {
            Some(runtime) => runtime.tracker()?,
            None => return Err(crate::Error::Inference("ONNX Runtime did not start".into())),
        };
        let source: Box<dyn FaceSource> = Box::new(TrackedCamera::new(frames, tracker));
        Ok(source)
    })
}

fn open_frames(spec: &CameraSpec) -> crate::Result<Box<dyn FrameSource>> {
    Ok(match &spec.file {
        Some(path) => Box::new(ImageFiles::open(path, spec.fps)?),
        None => Box::new(CameraSource::open(spec)?),
    })
}

/// Checks everything live input needs except a camera.
///
/// It describes what it did, one line per fact: loads ONNX Runtime from the bundled or configured library, fetches
/// and verifies the face models (SHA-256), opens a session for each and runs them once on a blank frame. Release
/// packages run this after packaging.
///
/// # Errors
/// The first thing that does not work: a missing library, a model that cannot be fetched or does not match its
/// checksum, a session that does not open.
pub fn self_check() -> crate::Result<Vec<String>> {
    use crate::frame::Frame;
    use crate::model::{FACE_DETECTOR, FACE_LANDMARKS};
    use crate::source::LandmarkBackend;

    let mut lines = Vec::new();
    #[cfg(feature = "onnx-dynamic")]
    lines.push(format!("ONNX Runtime library: {}", crate::runtime_lib::locate().path().display()));
    let runtime = OnnxRuntime::with_default_store()?;
    lines.push(format!("ONNX Runtime: {}", crate::onnx::runtime_info()));
    for model in [&FACE_DETECTOR, &FACE_LANDMARKS] {
        let path = runtime.store().ensure(model)?;
        lines.push(format!("model {} verified (SHA-256 {}) at {}", model.name, model.sha256, path.display()));
    }
    let mut tracker = runtime.tracker()?;
    let blank = Frame::solid(640, 480, [128, 128, 128])?;
    let found = tracker.landmarks(&blank)?;
    lines.push(format!("models ran on a blank frame: {}", if found.is_none() { "no face, as expected" } else { "a face where there is none" }));
    Ok(lines)
}

/// Starts a tracking actor on the real camera and models.
#[must_use]
pub fn spawn() -> TrackingHandle {
    TrackingActor::spawn(factory())
}
