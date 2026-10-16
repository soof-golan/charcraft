//! The real face models on a real photo.
//!
//! Skipped (passes) unless `CHARCRAFT_TRACKING_IMAGE` names a photo of a face and both models are already in the
//! model store (`CHARCRAFT_MODELS_DIR` or the platform cache; run the `probe` example once to download them).
//! With `onnx-dynamic`, `ORT_DYLIB_PATH` must point at libonnxruntime. No test ever touches the network.
#![cfg(feature = "onnx-runtime")]

use std::time::Duration;

use anim_core::Tick;
use charcraft_tracking::actor::{CameraSpec, Command, Event, TrackingActor};
use charcraft_tracking::frame::Frame;
use charcraft_tracking::model::{FACE_DETECTOR, FACE_LANDMARKS, ModelStore};
use charcraft_tracking::onnx::OnnxRuntime;
use charcraft_tracking::solver::{FaceSolver, Smoothing};
use charcraft_tracking::source::{FaceSource, LandmarkBackend, StillImage, TrackedCamera};

#[test]
fn finds_and_measures_a_face_in_a_photo() {
    let Some(path) = std::env::var_os("CHARCRAFT_TRACKING_IMAGE") else { return };
    let Ok(store) = ModelStore::default_location() else { return };
    if store.installed(&FACE_DETECTOR).is_none() || store.installed(&FACE_LANDMARKS).is_none() {
        return;
    }
    let picture = image::open(path).unwrap().to_rgb8();
    let frame = Frame::new(picture.width(), picture.height(), picture.into_raw()).unwrap();

    let runtime = OnnxRuntime::new(store).unwrap();
    let mut tracker = runtime.tracker().unwrap();
    let landmarks = tracker.landmarks(&frame).unwrap().expect("a face in the photo");
    assert!(landmarks.score > 0.9, "{}", landmarks.score);
    assert_eq!(landmarks.points.len(), 478);
    let [x0, y0, x1, y1] = landmarks.bounds().unwrap();
    assert!(
        x0 >= -20.0
            && y0 >= -20.0
            && x1 <= f32::from(u16::try_from(frame.width()).unwrap()) + 20.0
            && y1 <= f32::from(u16::try_from(frame.height()).unwrap()) + 20.0
    );

    let mut solver = FaceSolver::default().with_smoothing(Smoothing::off());
    let face = solver.solve(Tick::ZERO, Some(&landmarks)).input;
    assert!(face.tracked && face.is_valid(), "{face:?}");
    assert!(face.head.yaw.abs() < 0.5 && face.head.roll.abs() < 0.5, "a photo of a face looking at the camera: {face:?}");

    // The face is followed from its own landmarks on the next frame, without the detector.
    let again = tracker.landmarks(&frame).unwrap().expect("followed");
    let (a, b) = (landmarks.get(1).unwrap(), again.get(1).unwrap());
    assert!((a.x() - b.x()).abs() < 3.0 && (a.y() - b.y()).abs() < 3.0, "the nose tip stays put");

    // The whole stack through the actor, with the photo standing in for the camera. ONNX Runtime allows one
    // environment per process, so the runtime moves into the actor's factory.
    let handle = TrackingActor::spawn(Box::new(move |_| {
        let source: Box<dyn FaceSource> = Box::new(TrackedCamera::new(StillImage::new(frame.clone(), 5, 30), runtime.tracker()?));
        Ok(source)
    }));
    handle.send(Command::Start { camera: CameraSpec::default() }).unwrap();
    let mut faces = vec![];
    while let Some(event) = handle.recv_timeout(Duration::from_secs(30)) {
        match event {
            Event::Face(sample) => faces.push(sample),
            Event::SourceEnded => break,
            Event::Error(message) => panic!("{message}"),
            _ => {}
        }
    }
    assert_eq!(faces.len(), 5);
    assert!(faces.iter().all(|s| s.input.tracked && s.input.is_valid()));
    handle.shutdown();
}
