//! Where a runtime-loaded ONNX Runtime library lives.
//!
//! Release packages ship the library next to the program (Linux and Windows: beside the executable, in `lib/`
//! or `../lib/`; macOS: in the app's `Contents/Frameworks`), so users never set `ORT_DYLIB_PATH`. The variable still
//! wins when it is set (Jetson's `JetPack` runtime, a newer build, a GPU build). The search yields absolute paths only,
//! so a stale copy somewhere on the system library path (an old `onnxruntime.dll` in `System32`, say) is never
//! picked over the bundled one.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The file name of the library on this platform: `libonnxruntime.so`, `libonnxruntime.dylib` or `onnxruntime.dll`.
#[must_use]
pub fn file_name() -> String {
    format!("{}onnxruntime{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX)
}

/// The places a bundled library may be, nearest first, for a program in `exe_dir`.
#[must_use]
pub fn bundled_candidates(exe_dir: &Path, name: &str) -> Vec<PathBuf> {
    vec![
        exe_dir.join(name),
        exe_dir.join("lib").join(name),
        exe_dir.join("..").join("lib").join(name),
        // macOS: CharCraft.app/Contents/MacOS/charcraft with the library in Contents/Frameworks.
        exe_dir.join("..").join("Frameworks").join(name),
    ]
}

/// How the library was chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Located {
    /// `ORT_DYLIB_PATH` named it.
    Environment(PathBuf),
    /// Found in the package next to the program.
    Bundled(PathBuf),
    /// Nothing found: the operating system's library search will be asked for the bare file name.
    System(PathBuf),
}

impl Located {
    /// The path (or bare name) to load.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Environment(p) | Self::Bundled(p) | Self::System(p) => p,
        }
    }
}

/// Chooses the library: `variable` (the value of `ORT_DYLIB_PATH`), else the first bundled candidate under `exe_dir`
/// that exists, else the bare file name.
#[must_use]
pub fn locate_in(variable: Option<&OsStr>, exe_dir: Option<&Path>) -> Located {
    if let Some(path) = variable.filter(|v| !v.is_empty()) {
        return Located::Environment(PathBuf::from(path));
    }
    let name = file_name();
    if let Some(dir) = exe_dir
        && let Some(found) = bundled_candidates(dir, &name).into_iter().find(|p| p.is_file())
    {
        return Located::Bundled(found.canonicalize().unwrap_or(found));
    }
    Located::System(PathBuf::from(name))
}

/// [`locate_in`] for this process: the real environment variable and the directory of the running executable.
#[must_use]
pub fn locate() -> Located {
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    locate_in(std::env::var_os("ORT_DYLIB_PATH").as_deref(), exe_dir.as_deref())
}

/// Why the library could not be loaded, in words for a status line.
#[must_use]
pub fn explain(located: &Located, error: &str) -> String {
    let name = file_name();
    match located {
        Located::Environment(p) => format!("ONNX Runtime: ORT_DYLIB_PATH points at {}, which cannot be loaded ({error})", p.display()),
        Located::Bundled(p) => format!("ONNX Runtime: the bundled {} cannot be loaded ({error})", p.display()),
        Located::System(_) => {
            format!(
                "ONNX Runtime: {name} was not found next to the program or on the system library path ({error}); reinstall CharCraft or set ORT_DYLIB_PATH to a copy of it"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_variable_wins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(file_name()), b"x").unwrap();
        let got = locate_in(Some(OsStr::new("/opt/ort/libonnxruntime.so")), Some(dir.path()));
        assert_eq!(got, Located::Environment(PathBuf::from("/opt/ort/libonnxruntime.so")));
    }

    #[test]
    fn an_empty_variable_counts_as_unset() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(file_name()), b"x").unwrap();
        assert!(matches!(locate_in(Some(OsStr::new("")), Some(dir.path())), Located::Bundled(_)));
    }

    #[test]
    fn a_library_beside_the_program_is_found() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(file_name()), b"x").unwrap();
        let got = locate_in(None, Some(dir.path()));
        assert!(matches!(&got, Located::Bundled(p) if p.is_absolute() && p.ends_with(file_name())), "{got:?}");
    }

    #[test]
    fn the_macos_frameworks_folder_is_searched() {
        let app = tempfile::tempdir().unwrap();
        let macos = app.path().join("Contents").join("MacOS");
        let frameworks = app.path().join("Contents").join("Frameworks");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::create_dir_all(&frameworks).unwrap();
        std::fs::write(frameworks.join(file_name()), b"x").unwrap();
        assert!(matches!(locate_in(None, Some(&macos)), Located::Bundled(_)));
    }

    #[test]
    fn a_missing_library_falls_back_to_the_system_name_and_explains_itself() {
        let dir = tempfile::tempdir().unwrap();
        let got = locate_in(None, Some(dir.path()));
        assert_eq!(got, Located::System(PathBuf::from(file_name())));
        assert!(explain(&got, "no such file").contains("reinstall CharCraft"));
    }
}
