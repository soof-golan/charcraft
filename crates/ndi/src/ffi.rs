//! The only module with `unsafe`: resolving and calling the NDI runtime's C functions.
#![allow(unsafe_code, reason = "calling a dynamically loaded C library; see the manifest")]

use std::ffi::{CStr, CString, c_char, c_int, c_void};

use libloading::Library;

use crate::{Error, Result, candidates};

/// `NDIlib_send_create_t`.
#[repr(C)]
struct SendCreate {
    ndi_name: *const c_char,
    groups: *const c_char,
    clock_video: bool,
    clock_audio: bool,
}

/// `NDIlib_video_frame_v2_t`.
#[repr(C)]
struct VideoFrameV2 {
    xres: c_int,
    yres: c_int,
    four_cc: u32,
    frame_rate_n: c_int,
    frame_rate_d: c_int,
    picture_aspect_ratio: f32,
    frame_format_type: c_int,
    timecode: i64,
    data: *const u8,
    line_stride_in_bytes: c_int,
    metadata: *const c_char,
    timestamp: i64,
}

/// `NDIlib_FourCC_video_type_RGBA`: the bytes "RGBA", little endian.
const FOURCC_RGBA: u32 = u32::from_le_bytes(*b"RGBA");
/// `NDIlib_frame_format_type_progressive`.
const FORMAT_PROGRESSIVE: c_int = 1;
/// `NDIlib_send_timecode_synthesize`: the runtime stamps the frame itself.
const TIMECODE_SYNTHESIZE: i64 = i64::MAX;
/// `NDIlib_send_timecode_synthesize` for the timestamp field is `0` ("undefined").
const TIMESTAMP_UNDEFINED: i64 = 0;

type Initialize = unsafe extern "C" fn() -> bool;
type Destroy = unsafe extern "C" fn();
type Version = unsafe extern "C" fn() -> *const c_char;
type CreateSender = unsafe extern "C" fn(*const SendCreate) -> *mut c_void;
type DestroySender = unsafe extern "C" fn(*mut c_void);
type SendVideo = unsafe extern "C" fn(*mut c_void, *const VideoFrameV2);

/// A running NDI source. Dropping it stops the source and releases the runtime.
pub struct Sender {
    handle: *mut c_void,
    release_source: DestroySender,
    send_video: SendVideo,
    destroy: Destroy,
    frame_rate: (c_int, c_int),
    version: String,
    path: String,
    // Declared last so the library is unloaded after everything above is done with it.
    _library: Library,
}

impl std::fmt::Debug for Sender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sender").field("library", &self.path).field("version", &self.version).finish_non_exhaustive()
    }
}

fn open_library(env: &dyn Fn(&str) -> Option<String>) -> Result<(Library, String)> {
    let tried = candidates(env);
    for path in &tried {
        // SAFETY: loading a shared library runs its initialisers; the NDI runtime's are the vendor's own.
        if let Ok(library) = unsafe { Library::new(path) } {
            return Ok((library, path.clone()));
        }
    }
    Err(Error::NotInstalled { tried: tried.join(", ") })
}

fn symbol<T: Copy>(library: &Library, path: &str, name: &'static str) -> Result<T> {
    let mut bytes = name.as_bytes().to_vec();
    bytes.push(0);
    // SAFETY: each call site names the exact C signature of the function it resolves.
    unsafe { library.get::<T>(&bytes) }.map(|s| *s).map_err(|_| Error::MissingSymbol { path: path.to_owned(), symbol: name })
}

impl Sender {
    /// Loads the runtime and starts a source called `name` that expects `fps_n / fps_d` frames per second. `env`
    /// reads environment variables (the app passes the process environment; tests pass their own).
    ///
    /// # Errors
    /// [`Error::NotInstalled`] with the setup instructions when the runtime cannot be found, other [`Error`]s when
    /// it is unusable.
    pub fn new(name: &str, fps_n: u32, fps_d: u32, env: &dyn Fn(&str) -> Option<String>) -> Result<Self> {
        if name.trim().is_empty() || name.contains('\0') {
            return Err(Error::BadName);
        }
        let c_name = CString::new(name).map_err(|_| Error::BadName)?;
        let (library, path) = open_library(env)?;
        let initialize: Initialize = symbol(&library, &path, "NDIlib_initialize")?;
        let destroy: Destroy = symbol(&library, &path, "NDIlib_destroy")?;
        let create: CreateSender = symbol(&library, &path, "NDIlib_send_create")?;
        let destroy_sender: DestroySender = symbol(&library, &path, "NDIlib_send_destroy")?;
        let send_video: SendVideo = symbol(&library, &path, "NDIlib_send_send_video_v2")?;
        let version_fn: Option<Version> = symbol(&library, &path, "NDIlib_version").ok();

        // SAFETY: plain C calls with the signatures resolved above; `create_info` and `c_name` outlive the call.
        let (handle, version) = unsafe {
            if !initialize() {
                return Err(Error::Initialise);
            }
            let create_info = SendCreate { ndi_name: c_name.as_ptr(), groups: std::ptr::null(), clock_video: false, clock_audio: false };
            let handle = create(&raw const create_info);
            if handle.is_null() {
                destroy();
                return Err(Error::Create(name.to_owned()));
            }
            let version = version_fn.map(|f| f()).filter(|p| !p.is_null()).map(|p| CStr::from_ptr(p).to_string_lossy().into_owned()).unwrap_or_default();
            (handle, version)
        };
        let frame_rate = (c_int::try_from(fps_n.max(1)).unwrap_or(30), c_int::try_from(fps_d.max(1)).unwrap_or(1));
        Ok(Self { handle, release_source: destroy_sender, send_video, destroy, frame_rate, version, path, _library: library })
    }

    /// The runtime's version text, when it reports one.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Where the runtime was loaded from.
    #[must_use]
    pub fn library_path(&self) -> &str {
        &self.path
    }

    /// Sends one frame of straight-alpha RGBA, `width * 4` bytes per row, rows top to bottom.
    ///
    /// # Errors
    /// [`Error::BadFrame`] when `rgba` is not `width * height * 4` bytes or the size does not fit the runtime's
    /// integers.
    pub fn send_rgba(&mut self, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
        let needed = usize::try_from(width).ok().and_then(|w| w.checked_mul(usize::try_from(height).ok()?)).and_then(|p| p.checked_mul(4));
        let bad = || Error::BadFrame { width, height, needed: needed.unwrap_or(usize::MAX), got: rgba.len() };
        if needed != Some(rgba.len()) || width == 0 || height == 0 {
            return Err(bad());
        }
        let (xres, yres) = (c_int::try_from(width).map_err(|_| bad())?, c_int::try_from(height).map_err(|_| bad())?);
        let stride = xres.checked_mul(4).ok_or_else(bad)?;
        #[expect(clippy::cast_precision_loss, clippy::as_conversions, reason = "an aspect ratio; small integers")]
        let aspect = xres as f32 / yres as f32;
        let frame = VideoFrameV2 {
            xres,
            yres,
            four_cc: FOURCC_RGBA,
            frame_rate_n: self.frame_rate.0,
            frame_rate_d: self.frame_rate.1,
            picture_aspect_ratio: aspect,
            frame_format_type: FORMAT_PROGRESSIVE,
            timecode: TIMECODE_SYNTHESIZE,
            data: rgba.as_ptr(),
            line_stride_in_bytes: stride,
            metadata: std::ptr::null(),
            timestamp: TIMESTAMP_UNDEFINED,
        };
        // SAFETY: `handle` came from NDIlib_send_create and is live until drop; `frame` points at `rgba`, whose length
        // was checked against the stride and height above; `send_video` copies the pixels before it returns.
        unsafe { (self.send_video)(self.handle, &raw const frame) };
        Ok(())
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        // SAFETY: `handle` is live and destroyed exactly once; `destroy` balances the `initialize` of `new`.
        unsafe {
            (self.release_source)(self.handle);
            (self.destroy)();
        }
    }
}

/// The version of the NDI runtime that can be loaded, or `None` when there is none (without starting a source).
#[must_use]
pub fn loaded_version(env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let (library, path) = open_library(env).ok()?;
    let version: Version = symbol(&library, &path, "NDIlib_version").ok()?;
    // SAFETY: `NDIlib_version` takes no arguments and returns a static NUL-terminated string.
    let text = unsafe {
        let p = version();
        if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
    };
    Some(text)
}
