//! The face networks on ONNX Runtime (`ort`).
//!
//! [`OnnxRuntime`] owns the process's ONNX Runtime environment (which can only be created once) and the model store;
//! [`OnnxRuntime::tracker`] fetches the two models on first use (verified by SHA-256), opens a session for each and
//! returns a [`FaceTracker`] over them. Sessions are cheap to drop and recreate, so stopping and restarting the camera
//! does not re-create the environment.

use ort::session::Session;
use ort::value::Tensor;

use crate::error::{Error, Result};
use crate::landmarks::MESH_POINTS_WITH_IRIS;
use crate::model::{BODY_POSE, FACE_DETECTOR, FACE_LANDMARKS, ModelStore};
use crate::pipeline::{ANCHORS, DETECTOR_SIZE, DetectorOutput, FaceNet, FaceTracker, MESH_SIZE, MeshOutput, REGRESSION_STRIDE};
use crate::pose::{POSE_SIZE, PoseNet, PoseTracker, output_len};

fn inference(e: impl std::fmt::Display) -> Error {
    Error::Inference(e.to_string())
}

/// Threads ONNX Runtime may use per session: the models are small, so leave the other cores to the app.
const THREADS: usize = 2;

/// The environment builder. A runtime-loaded library is found by [`crate::runtime_lib`] and loaded here, so a missing
/// or broken library is an error for the status line instead of a panic inside `ort`.
#[cfg(feature = "onnx-dynamic")]
fn builder() -> Result<ort::environment::EnvironmentBuilder> {
    let located = crate::runtime_lib::locate();
    ort::init_from(located.path()).map_err(|e| Error::Inference(crate::runtime_lib::explain(&located, &e.to_string())))
}

#[cfg(not(feature = "onnx-dynamic"))]
#[allow(clippy::unnecessary_wraps, reason = "same signature as the runtime-loaded variant")]
fn builder() -> Result<ort::environment::EnvironmentBuilder> {
    Ok(ort::init())
}

/// The loaded runtime's own description of itself (version and build), for status lines.
#[must_use]
pub fn runtime_info() -> String {
    ort::info().to_owned()
}

/// The ONNX Runtime environment and the store the models come from.
///
/// Cloning gives another handle on the same environment, which is how the face tracker and the body tracker, each
/// on its own thread, share the process's single ONNX Runtime.
#[derive(Debug, Clone)]
pub struct OnnxRuntime {
    // Kept alive for the sessions; ONNX Runtime allows one environment per process.
    environment: ort::environment::Environment,
    store: ModelStore,
}

impl OnnxRuntime {
    /// Initialises ONNX Runtime (once per process) with models taken from `store`.
    ///
    /// # Errors
    /// [`Error::Inference`] when the runtime cannot start or was already started in this process.
    pub fn new(store: ModelStore) -> Result<Self> {
        let environment = builder()?.with_name("charcraft-tracking").build().map_err(inference)?;
        Ok(Self { environment, store })
    }

    /// The model store this runtime reads from.
    #[must_use]
    pub const fn store(&self) -> &ModelStore {
        &self.store
    }

    /// Like [`OnnxRuntime::new`] with the default model store.
    ///
    /// # Errors
    /// As [`OnnxRuntime::new`], plus [`Error::NoCacheDir`].
    pub fn with_default_store() -> Result<Self> {
        Self::new(ModelStore::default_location()?)
    }

    /// Downloads (if needed), verifies and opens both models.
    ///
    /// # Errors
    /// Download, verification and session errors.
    pub fn net(&self) -> Result<OnnxNet> {
        let detector = self.session(&FACE_DETECTOR)?;
        let mesh = self.session(&FACE_LANDMARKS)?;
        Ok(OnnxNet { detector, mesh })
    }

    /// A ready face tracker.
    ///
    /// # Errors
    /// As [`OnnxRuntime::net`].
    pub fn tracker(&self) -> Result<FaceTracker<OnnxNet>> {
        Ok(FaceTracker::new(self.net()?))
    }

    /// Downloads (if needed), verifies and opens the body pose model.
    ///
    /// # Errors
    /// Download, verification and session errors.
    pub fn pose_net(&self) -> Result<OnnxPoseNet> {
        let session = self.session(&BODY_POSE)?;
        let whole_numbers = session.inputs().first().and_then(|i| i.dtype().tensor_type()) == Some(ort::value::TensorElementType::Int32);
        Ok(OnnxPoseNet { session, whole_numbers })
    }

    /// A ready body tracker.
    ///
    /// # Errors
    /// As [`OnnxRuntime::pose_net`].
    pub fn body_tracker(&self) -> Result<PoseTracker<OnnxPoseNet>> {
        Ok(PoseTracker::new(self.pose_net()?))
    }

    fn session(&self, model: &crate::model::ModelInfo) -> Result<Session> {
        let path = self.store.ensure(model)?;
        let mut builder = Session::builder(&self.environment).map_err(inference)?;
        builder = builder.with_intra_threads(THREADS).map_err(inference)?;
        builder.commit_from_file(&path).map_err(inference)
    }
}

/// The detector and mesh models as sessions.
pub struct OnnxNet {
    detector: Session,
    mesh: Session,
}

impl std::fmt::Debug for OnnxNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnnxNet").finish_non_exhaustive()
    }
}

/// The pose model as a session. `MoveNet` converted from TensorFlow takes whole-number pixel values (`int32`);
/// other conversions take floats, so the input type is read from the model.
pub struct OnnxPoseNet {
    session: Session,
    whole_numbers: bool,
}

impl std::fmt::Debug for OnnxPoseNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnnxPoseNet").field("whole_numbers", &self.whole_numbers).finish_non_exhaustive()
    }
}

impl PoseNet for OnnxPoseNet {
    fn infer(&mut self, input: &[f32]) -> Result<Vec<f32>> {
        let side = i64::try_from(POSE_SIZE).map_err(inference)?;
        let shape = [1_i64, side, side, 3];
        let outputs = if self.whole_numbers {
            let pixels: Vec<i32> = input.iter().map(|v| anim_core::num::round_f64_to_i32(f64::from(*v))).collect();
            self.session.run(ort::inputs![Tensor::from_array((shape, pixels)).map_err(inference)?]).map_err(inference)?
        } else {
            self.session.run(ort::inputs![Tensor::from_array((shape, input.to_vec())).map_err(inference)?]).map_err(inference)?
        };
        let wanted = output_len();
        outputs
            .iter()
            .filter_map(|(_, v)| v.try_extract_tensor::<f32>().ok().map(|(_, d)| d.to_vec()))
            .find(|d| d.len() == wanted)
            .ok_or_else(|| Error::Inference(format!("the pose model has no output of {wanted} numbers")))
    }
}

/// A named output flattened to a vector.
type Named = (String, Vec<f32>);

/// Runs `session` on one NHWC image and returns every output, named, as a flat vector.
fn run(session: &mut Session, side: usize, data: &[f32]) -> Result<Vec<Named>> {
    let side = i64::try_from(side).map_err(inference)?;
    let tensor = Tensor::from_array(([1_i64, side, side, 3], data.to_vec())).map_err(inference)?;
    let outputs = session.run(ort::inputs![tensor]).map_err(inference)?;
    outputs.iter().map(|(name, v)| v.try_extract_tensor::<f32>().map(|(_, d)| (name.to_owned(), d.to_vec())).map_err(inference)).collect()
}

/// The output called `name` if it has `len` values, else the first output with `len` values.
fn pick(outputs: &[Named], name: &str, len: usize) -> Option<Vec<f32>> {
    outputs.iter().find(|(n, v)| n == name && v.len() == len).or_else(|| outputs.iter().find(|(_, v)| v.len() == len)).map(|(_, v)| v.clone())
}

fn shapes(outputs: &[Named]) -> Vec<(&str, usize)> {
    outputs.iter().map(|(n, v)| (n.as_str(), v.len())).collect()
}

impl FaceNet for OnnxNet {
    fn detect(&mut self, input: &[f32]) -> Result<DetectorOutput> {
        let outputs = run(&mut self.detector, DETECTOR_SIZE, input)?;
        let regressors = pick(&outputs, "regressors", ANCHORS * REGRESSION_STRIDE);
        let scores = pick(&outputs, "classificators", ANCHORS);
        regressors
            .zip(scores)
            .map(|(regressors, scores)| DetectorOutput { regressors, scores })
            .ok_or_else(|| Error::Inference(format!("face detector outputs are {:?}", shapes(&outputs))))
    }

    fn mesh(&mut self, input: &[f32]) -> Result<MeshOutput> {
        let outputs = run(&mut self.mesh, MESH_SIZE, input)?;
        let points = pick(&outputs, "Identity", MESH_POINTS_WITH_IRIS * 3);
        let presence = pick(&outputs, "Identity_1", 1).and_then(|v| v.first().copied());
        points
            .zip(presence)
            .map(|(points, presence)| MeshOutput { points, presence })
            .ok_or_else(|| Error::Inference(format!("face mesh outputs are {:?}", shapes(&outputs))))
    }
}
