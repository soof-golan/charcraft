//! The Linux virtual camera: frames written to a `v4l2loopback` device, which other programs open as a webcam.

use std::io::Write;

use v4l::capability::Flags;
use v4l::video::Output;
use v4l::{Device, Format, FourCC};

use crate::picture::Picture;
use crate::{Error, Sink, Spec};

/// What to tell a person with no loopback device.
pub const SETUP: &str = "The virtual camera needs the v4l2loopback kernel module. Install it (Debian and Ubuntu: sudo apt install v4l2loopback-dkms; \
Fedora: akmod-v4l2loopback) and load it with: sudo modprobe v4l2loopback exclusive_caps=1 card_label=\"CharCraft\" video_nr=10. \
Then start the virtual camera again; Zoom, browsers and OBS list it as \"CharCraft\".";

/// The driver name v4l2loopback reports.
const LOOPBACK_DRIVER: &str = "v4l2 loopback";

/// The paths of the loopback devices, sorted.
pub fn loopback_devices() -> Vec<String> {
    let mut paths: Vec<String> = v4l::context::enum_devices()
        .into_iter()
        .filter_map(|node| {
            let path = node.path().to_path_buf();
            let device = Device::with_path(&path).ok()?;
            let caps = device.query_caps().ok()?;
            (caps.driver == LOOPBACK_DRIVER).then(|| path.display().to_string())
        })
        .collect();
    paths.sort();
    paths
}

/// A loopback device opened for writing.
pub struct VirtualCamera {
    device: Device,
    path: String,
    format: Format,
    backdrop: [u8; 3],
}

impl VirtualCamera {
    /// Opens the device the spec names (or the first loopback device) and sets the YUYV format of the canvas.
    pub fn open(spec: &Spec) -> Result<Self, Error> {
        let path = match &spec.device {
            Some(path) => path.clone(),
            None => loopback_devices().into_iter().next().ok_or_else(|| Error::Unavailable(SETUP.to_owned()))?,
        };
        let device = Device::with_path(&path).map_err(|e| Error::Output(format!("{path}: {e}")))?;
        let caps = device.query_caps().map_err(|e| Error::Output(format!("{path}: {e}")))?;
        if caps.driver != LOOPBACK_DRIVER && !caps.capabilities.contains(Flags::VIDEO_OUTPUT) {
            return Err(Error::Unavailable(format!("{path} is {} ({}), a camera and not a loopback device. {SETUP}", caps.card, caps.driver)));
        }
        let wanted = Format::new(spec.width, spec.height, FourCC::new(b"YUYV"));
        let format = Output::set_format(&device, &wanted).map_err(|e| Error::Output(format!("{path}: cannot set {}x{} YUYV: {e}", spec.width, spec.height)))?;
        if format.fourcc != wanted.fourcc || format.width != spec.width || format.height != spec.height {
            return Err(Error::Output(format!("{path} insists on {}x{} {:?}", format.width, format.height, format.fourcc)));
        }
        Ok(Self { device, path, format, backdrop: spec.background.flatten_colour() })
    }
}

impl Sink for VirtualCamera {
    fn describe(&self) -> String {
        format!("virtual camera {} ({}x{} YUYV)", self.path, self.format.width, self.format.height)
    }

    fn send(&mut self, picture: &Picture) -> Result<(), Error> {
        if (picture.width(), picture.height()) != (self.format.width, self.format.height) {
            return Err(Error::Output("the picture is not the size of the camera format".to_owned()));
        }
        let yuyv = picture.to_yuyv(self.backdrop)?;
        self.device.write_all(&yuyv).map_err(|e| Error::Output(format!("{}: {e}", self.path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_without_loopback_devices_gets_the_setup_text() {
        // Nothing here loads the module, so the list is normally empty; where it is not, opening works instead.
        let spec = Spec { kind: crate::Kind::VirtualCamera, device: Some("/dev/nonexistent-video".into()), ..Spec::default() };
        assert!(matches!(VirtualCamera::open(&spec), Err(Error::Output(m)) if m.contains("nonexistent-video")));
        if loopback_devices().is_empty() {
            let any = Spec { kind: crate::Kind::VirtualCamera, ..Spec::default() };
            assert!(matches!(VirtualCamera::open(&any), Err(Error::Unavailable(m)) if m.contains("modprobe v4l2loopback")));
        }
    }
}
