//! The model registry and the on-disk model store.
//!
//! Every model is open source under a licence compatible with ours, listed with its authors, source, size and
//! SHA-256. Weights are never committed: they are fetched on first use into the platform cache directory, verified
//! against the registry and loaded from there. Anything that does not match its hash is rejected and never written.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// What a model is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    /// Finds faces in a frame (a box and six keypoints each).
    FaceDetection,
    /// Places 478 landmarks on a face found in a crop.
    FaceLandmarks,
}

/// A model in the registry: what it is, who made it, its licence and where the weights come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    /// Stable identifier.
    pub id: &'static str,
    /// What the model does.
    pub task: Task,
    /// Display name.
    pub name: &'static str,
    /// Who made the weights (kept with the model and in `ATTRIBUTION.md`).
    pub authors: &'static str,
    /// SPDX identifier of the licence of the weights.
    pub licence: &'static str,
    /// Where the licence can be read.
    pub licence_url: &'static str,
    /// The project page.
    pub homepage: &'static str,
    /// Direct download of the weights (pinned to a revision).
    pub url: &'static str,
    /// File name inside the store.
    pub file_name: &'static str,
    /// Size in bytes.
    pub size: u64,
    /// SHA-256 of the file, lower-case hex.
    pub sha256: &'static str,
}

/// Licences a model may carry to be listed (open source and compatible with MIT OR BSD-3-Clause).
pub const ALLOWED_LICENCES: &[&str] = &["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "CC0-1.0"];

/// `MediaPipe` `BlazeFace` (short range) face detector, converted to ONNX.
pub const FACE_DETECTOR: ModelInfo = ModelInfo {
    id: "blazeface-short-onnx",
    task: Task::FaceDetection,
    name: "MediaPipe BlazeFace short-range face detector (ONNX)",
    authors: "Google LLC (MediaPipe); ONNX conversion by Fernando Tonon (QtMeshEditor)",
    licence: "Apache-2.0",
    licence_url: "https://github.com/google-ai-edge/mediapipe/blob/master/LICENSE",
    homepage: "https://huggingface.co/fernandotonon/QtMeshEditor-blazeface-onnx",
    url: "https://huggingface.co/fernandotonon/QtMeshEditor-blazeface-onnx/resolve/50f2c66ffbdf84beae8c267df2b49e5c5a5162e9/face_detector.onnx",
    file_name: "blazeface_short_range.onnx",
    size: 418_437,
    sha256: "02a04d5d37c3558dc4d5274f7f8f0f0f01ac94e46c5ffb2cee82395d47e23181",
};

/// `MediaPipe` Face Mesh V2 (478 landmarks including the irises), converted to ONNX.
pub const FACE_LANDMARKS: ModelInfo = ModelInfo {
    id: "mediapipe-face-mesh-v2-onnx",
    task: Task::FaceLandmarks,
    name: "MediaPipe Face Mesh V2, 478 landmarks (ONNX)",
    authors: "Google LLC (MediaPipe); ONNX conversion by naklitechie",
    licence: "Apache-2.0",
    licence_url: "https://github.com/google-ai-edge/mediapipe/blob/master/LICENSE",
    homepage: "https://huggingface.co/naklitechie/face-landmarks-onnx",
    url: "https://huggingface.co/naklitechie/face-landmarks-onnx/resolve/575c33816c840c5b156398adb485e4f8d138adc2/face_landmarks.onnx",
    file_name: "face_mesh_v2_478.onnx",
    size: 4_920_995,
    sha256: "f38c3321ceffbc9e95103480ad38cc3f52e7e1bde2bcee7cd9355d0b9138ac0c",
};

/// Every registered model.
pub const MODELS: &[ModelInfo] = &[FACE_DETECTOR, FACE_LANDMARKS];

/// Looks a model up by id.
#[must_use]
pub fn info(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id == id)
}

/// Lower-case hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().fold(String::with_capacity(64), |mut s, b| {
        // Writing to a String cannot fail.
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Checks `bytes` against the registry entry: licence on the allow list, size and hash as listed.
///
/// # Errors
/// [`Error::LicenceNotAllowed`] or [`Error::ChecksumMismatch`].
pub fn verify(model: &ModelInfo, bytes: &[u8]) -> Result<()> {
    if !ALLOWED_LICENCES.contains(&model.licence) {
        return Err(Error::LicenceNotAllowed { name: model.name, licence: model.licence });
    }
    let got = sha256_hex(bytes);
    if got != model.sha256 {
        return Err(Error::ChecksumMismatch { name: model.name, got, expected: model.sha256 });
    }
    Ok(())
}

/// The notice kept next to installed weights: what they are, who made them, their licence.
#[must_use]
pub fn notice(model: &ModelInfo) -> String {
    format!(
        "{}\n\nAuthors: {}\nLicence: {} (https://spdx.org/licenses/{}.html; as stated by the authors: {})\nSource: {}\nProject: {}\n\nDownloaded unmodified from the source above and verified against SHA-256 {}.\nCharCraft is not affiliated with or endorsed by the authors.\n",
        model.name, model.authors, model.licence, model.licence, model.licence_url, model.url, model.homepage, model.sha256
    )
}

/// The directory models live in. Defaults to `<platform cache dir>/charcraft/models`; `CHARCRAFT_MODELS_DIR`
/// overrides it (offline installs, tests).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelStore {
    dir: PathBuf,
}

impl ModelStore {
    /// A store in `dir`.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The default store (see the type docs).
    ///
    /// # Errors
    /// [`Error::NoCacheDir`] when the platform has no cache directory and the variable is unset.
    pub fn default_location() -> Result<Self> {
        if let Some(dir) = std::env::var_os("CHARCRAFT_MODELS_DIR").filter(|d| !d.is_empty()) {
            return Ok(Self::new(dir));
        }
        let cache = dirs::cache_dir().ok_or(Error::NoCacheDir)?;
        Ok(Self::new(cache.join("charcraft").join("models")))
    }

    /// The directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where `model` is (or would be) stored.
    #[must_use]
    pub fn path_of(&self, model: &ModelInfo) -> PathBuf {
        self.dir.join(model.file_name)
    }

    /// The path of `model` when it is installed and its hash still matches; `None` when absent or damaged.
    #[must_use]
    pub fn installed(&self, model: &ModelInfo) -> Option<PathBuf> {
        let path = self.path_of(model);
        let bytes = std::fs::read(&path).ok()?;
        verify(model, &bytes).ok().map(|()| path)
    }

    /// Verifies `bytes` and writes them into the store (through a temporary file, so a crash never leaves a
    /// half-written model), with a licence notice beside them.
    ///
    /// # Errors
    /// The verification errors of [`verify`], or [`Error::Io`].
    pub fn install(&self, model: &ModelInfo, bytes: &[u8]) -> Result<PathBuf> {
        verify(model, bytes)?;
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| Error::Io { path, source }
        };
        std::fs::create_dir_all(&self.dir).map_err(io(&self.dir))?;
        let path = self.path_of(model);
        let tmp = self.dir.join(format!("{}.part", model.file_name));
        std::fs::write(&tmp, bytes).map_err(io(&tmp))?;
        std::fs::rename(&tmp, &path).map_err(io(&path))?;
        let notice_path = self.dir.join(format!("{}.NOTICE.txt", model.file_name));
        std::fs::write(&notice_path, notice(model)).map_err(io(&notice_path))?;
        Ok(path)
    }

    /// The path of `model`, fetching it with `fetch(url)` first when it is not installed.
    ///
    /// # Errors
    /// Whatever `fetch` returns, the verification errors of [`verify`], or [`Error::Io`].
    pub fn ensure_with(&self, model: &ModelInfo, fetch: impl FnOnce(&str) -> Result<Vec<u8>>) -> Result<PathBuf> {
        if let Some(path) = self.installed(model) {
            return Ok(path);
        }
        let bytes = fetch(model.url)?;
        self.install(model, &bytes)
    }

    /// The path of `model`, downloading it on first use.
    ///
    /// # Errors
    /// See [`ModelStore::ensure_with`]; without the `download` feature a missing model is [`Error::ModelUnavailable`].
    pub fn ensure(&self, model: &ModelInfo) -> Result<PathBuf> {
        self.ensure_with(model, |url| http_get(url, model))
    }
}

#[cfg(feature = "download")]
fn http_get(url: &str, model: &ModelInfo) -> Result<Vec<u8>> {
    let unavailable = |e: &dyn std::fmt::Display| Error::ModelUnavailable { name: model.name, reason: e.to_string() };
    // The registry knows the size; allow a little slack and refuse anything larger.
    let limit = model.size.saturating_add(1 << 20);
    let mut response = ureq::get(url).call().map_err(|e| unavailable(&e))?;
    response.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| unavailable(&e))
}

#[cfg(not(feature = "download"))]
fn http_get(_url: &str, model: &ModelInfo) -> Result<Vec<u8>> {
    Err(Error::ModelUnavailable { name: model.name, reason: format!("built without the `download` feature; place the file at {} yourself", model.file_name) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A registry entry for test bytes (never a real model).
    fn entry(bytes: &[u8]) -> ModelInfo {
        let sha256 = Box::leak(sha256_hex(bytes).into_boxed_str());
        ModelInfo { sha256, size: u64::try_from(bytes.len()).unwrap_or(0), file_name: "test.onnx", ..FACE_DETECTOR }
    }

    #[test]
    fn sha256_matches_the_standard_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn registry_entries_are_well_formed() {
        for m in MODELS {
            assert!(ALLOWED_LICENCES.contains(&m.licence), "{}", m.id);
            assert_eq!(m.sha256.len(), 64, "{}", m.id);
            assert!(m.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()), "{}", m.id);
            assert!(m.url.starts_with("https://"), "{}", m.id);
            assert!(m.size > 0);
            assert_eq!(info(m.id), Some(m));
        }
        let mut ids: Vec<_> = MODELS.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MODELS.len(), "ids are unique");
        assert!(info("nope").is_none());
    }

    #[test]
    fn verify_accepts_matching_bytes_and_rejects_others() {
        let m = entry(b"weights");
        assert!(verify(&m, b"weights").is_ok());
        let err = verify(&m, b"tampered").unwrap_err();
        assert!(matches!(err, Error::ChecksumMismatch { .. }), "{err}");
    }

    #[test]
    fn verify_rejects_disallowed_licences() {
        let m = ModelInfo { licence: "CC-BY-NC-4.0", ..entry(b"w") };
        assert!(matches!(verify(&m, b"w"), Err(Error::LicenceNotAllowed { .. })));
    }

    #[test]
    fn install_verifies_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path().join("models"));
        let m = entry(b"weights");
        assert!(store.installed(&m).is_none());
        assert!(store.install(&m, b"bad bytes").is_err());
        assert!(!store.dir().exists(), "a rejected file leaves nothing behind");
        let path = store.install(&m, b"weights").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"weights");
        assert_eq!(store.installed(&m), Some(path));
        let notice = std::fs::read_to_string(store.dir().join("test.onnx.NOTICE.txt")).unwrap();
        assert!(notice.contains("Apache-2.0") && notice.contains(m.sha256));
    }

    #[test]
    fn a_damaged_installed_file_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let m = entry(b"weights");
        let path = store.install(&m, b"weights").unwrap();
        std::fs::write(&path, b"weightz").unwrap();
        assert!(store.installed(&m).is_none());
    }

    #[test]
    fn ensure_fetches_once_and_then_uses_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let m = entry(b"weights");
        let mut fetches = 0;
        let first = store
            .ensure_with(&m, |url| {
                fetches += 1;
                assert_eq!(url, m.url);
                Ok(b"weights".to_vec())
            })
            .unwrap();
        let second = store.ensure_with(&m, |_| panic!("must not fetch again")).unwrap();
        assert_eq!((first, fetches), (second, 1));
    }

    #[test]
    fn ensure_rejects_a_bad_download() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let m = entry(b"weights");
        let err = store.ensure_with(&m, |_| Ok(b"<html>captive portal</html>".to_vec())).unwrap_err();
        assert!(matches!(err, Error::ChecksumMismatch { .. }));
        assert!(store.installed(&m).is_none());
    }

    #[test]
    fn a_failing_fetch_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let m = entry(b"weights");
        let err = store.ensure_with(&m, |_| Err(Error::ModelUnavailable { name: "x", reason: "offline".into() })).unwrap_err();
        assert!(err.to_string().contains("offline"));
    }
}
