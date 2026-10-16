//! NDI output for `CharCraft`.
//!
//! The NDI runtime is not ours to ship or link: its vendor distributes it separately under its own licence. This crate
//! therefore *loads* it when a sender starts, from the places its installers use, and says exactly what to install
//! when it is not there ([`SETUP`]). Nothing else in the app depends on it.
//!
//! The protocol is the runtime's: the interface used here is the small, stable C one every NDI sender uses
//! (`NDIlib_initialize`, `NDIlib_send_create`, `NDIlib_send_send_video_v2`, `NDIlib_send_destroy`, `NDIlib_destroy`),
//! called through function pointers resolved by name. Frames are sent as straight-alpha RGBA, so a transparent
//! puppet arrives as a keyable source in OBS, Resolume, vMix and the like.

mod ffi;

pub use ffi::{Sender, loaded_version};

/// What to tell a person whose machine has no NDI runtime.
pub const SETUP: &str = "NDI output needs the free NDI runtime. Install \"NDI Tools\" or the \"NDI Runtime\" from https://ndi.video/tools/ \
(Windows and macOS), or on Linux install the NDI SDK's libndi.so.6 into /usr/local/lib, then start NDI output again. \
Set NDI_RUNTIME_DIR_V6 to the folder with the library if it is somewhere else.";

/// Why NDI output could not start.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// No NDI runtime library could be loaded from any of the usual places.
    #[error("the NDI runtime is not installed (looked for {tried}). {SETUP}")]
    NotInstalled {
        /// The names and paths tried, comma separated.
        tried: String,
    },
    /// The library loaded but does not export the interface used here.
    #[error("the NDI library found at {path} is missing {symbol}: install a current NDI runtime")]
    MissingSymbol {
        /// Where it was loaded from.
        path: String,
        /// The function that was not found.
        symbol: &'static str,
    },
    /// `NDIlib_initialize` returned false: the machine's CPU is unsupported.
    #[error("the NDI runtime refused to initialise (an unsupported CPU?)")]
    Initialise,
    /// The sender could not be created.
    #[error("could not create the NDI sender {0:?}")]
    Create(String),
    /// The name has a NUL byte or is empty.
    #[error("the NDI source name must be non-empty text without NUL bytes")]
    BadName,
    /// The frame does not fit the buffer.
    #[error("a {width}x{height} frame needs {needed} bytes, got {got}")]
    BadFrame {
        /// Frame width.
        width: u32,
        /// Frame height.
        height: u32,
        /// Bytes needed.
        needed: usize,
        /// Bytes given.
        got: usize,
    },
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;

/// The library names and paths to try, most specific first.
///
/// The environment variables are the ones the NDI installers set (`NDI_RUNTIME_DIR_V6` and older); then the platform's
/// library name searched by the system loader; then the folders installers use.
#[must_use]
pub fn candidates(env: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let names: &[&str] = if cfg!(target_os = "windows") {
        &["Processing.NDI.Lib.x64.dll"]
    } else if cfg!(target_os = "macos") {
        &["libndi.dylib"]
    } else {
        &["libndi.so.6", "libndi.so.5", "libndi.so"]
    };
    let mut out = Vec::new();
    for var in ["NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5", "NDI_RUNTIME_DIR_V4", "NDI_RUNTIME_DIR_V3"] {
        if let Some(dir) = env(var).filter(|d| !d.is_empty()) {
            out.extend(names.iter().map(|n| std::path::Path::new(&dir).join(n).display().to_string()));
        }
    }
    out.extend(names.iter().map(|n| (*n).to_owned()));
    if cfg!(target_os = "macos") {
        out.push("/usr/local/lib/libndi.dylib".to_owned());
        out.push("/Library/NDI SDK for Apple/lib/macOS/libndi.dylib".to_owned());
    } else if cfg!(target_os = "linux") {
        out.extend(["/usr/local/lib/libndi.so.6", "/usr/lib/libndi.so.6", "/usr/local/lib/libndi.so.5"].map(str::to_owned));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runtime_folder_variables_come_first() {
        let env = |name: &str| (name == "NDI_RUNTIME_DIR_V6").then(|| "/opt/ndi".to_owned());
        let list = candidates(&env);
        assert!(list.first().is_some_and(|p| p.starts_with("/opt/ndi") || p.starts_with("\\opt\\ndi")), "{list:?}");
        assert!(list.iter().any(|p| p.contains("ndi")));
        let none = candidates(&|_| None);
        assert!(!none.is_empty() && none.iter().all(|p| !p.starts_with("/opt/ndi")));
    }

    #[test]
    fn the_setup_message_names_where_to_get_the_runtime() {
        let e = Error::NotInstalled { tried: "libndi.so.6".into() };
        let text = e.to_string();
        assert!(text.contains("https://ndi.video/tools/") && text.contains("libndi.so.6"), "{text}");
    }

    #[test]
    fn a_machine_without_the_runtime_gets_the_setup_message_not_a_crash() {
        // CI has no NDI runtime; a machine that has one starts a sender, which is also fine.
        match Sender::new("CharCraft test", 30, 1, &|_| Some("/nonexistent".to_owned())) {
            Err(Error::NotInstalled { tried }) => assert!(tried.contains("nonexistent")),
            Err(other) => panic!("{other}"),
            Ok(_) => {}
        }
        assert!(matches!(Sender::new("", 30, 1, &|_| None), Err(Error::BadName)));
        assert!(matches!(Sender::new("a\0b", 30, 1, &|_| None), Err(Error::BadName)));
    }
}
