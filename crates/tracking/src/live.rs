//! The real thing: a webcam through nokhwa, face landmarks through ONNX Runtime.
//!
//! [`factory`] is the [`SourceFactory`] a [`TrackingActor`] needs; [`spawn`] starts an actor with it.

use crate::actor::{SourceFactory, TrackingActor, TrackingHandle};
use crate::camera::CameraSource;
use crate::onnx::OnnxRuntime;
use crate::source::{FaceSource, TrackedCamera};

/// A factory that opens the requested camera and the face models.
///
/// The ONNX Runtime environment is created on the first start and kept for later ones. Models are downloaded into
/// the platform cache on first use.
pub fn factory() -> SourceFactory {
    let mut runtime: Option<OnnxRuntime> = None;
    Box::new(move |spec| {
        if runtime.is_none() {
            runtime = Some(OnnxRuntime::with_default_store()?);
        }
        let tracker = match &runtime {
            Some(runtime) => runtime.tracker()?,
            None => return Err(crate::Error::Inference("ONNX Runtime did not start".into())),
        };
        let camera = CameraSource::open(spec)?;
        let source: Box<dyn FaceSource> = Box::new(TrackedCamera::new(camera, tracker));
        Ok(source)
    })
}

/// Starts a tracking actor on the real camera and models.
#[must_use]
pub fn spawn() -> TrackingHandle {
    TrackingActor::spawn(factory())
}
