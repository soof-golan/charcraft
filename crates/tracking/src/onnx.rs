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
use crate::model::{FACE_DETECTOR, FACE_LANDMARKS, ModelStore};
use crate::pipeline::{ANCHORS, DETECTOR_SIZE, DetectorOutput, FaceNet, FaceTracker, MESH_SIZE, MeshOutput, REGRESSION_STRIDE};

fn inference(e: impl std::fmt::Display) -> Error {
    Error::Inference(e.to_string())
}

/// Threads ONNX Runtime may use per session: the models are small, so leave the other cores to the app.
const THREADS: usize = 2;

/// The ONNX Runtime environment and the store the models come from.
#[derive(Debug)]
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
        let environment = ort::init().with_name("charcraft-tracking").build().map_err(inference)?;
        Ok(Self { environment, store })
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
