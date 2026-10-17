//! Where MIDI messages come from: the system's devices, or a stand-in for tests and demos.

/// Receives the raw bytes of each message a connected device sends. It runs on the device's own thread.
pub type Sink = Box<dyn FnMut(&[u8]) + Send>;

/// An open device. Dropping it disconnects.
pub struct Connection {
    _device: Box<dyn Send>,
}

impl Connection {
    /// Wraps whatever keeps the device open.
    #[must_use]
    pub fn new(device: impl Send + 'static) -> Self {
        Self { _device: Box::new(device) }
    }
}

/// A source of MIDI input devices.
pub trait Backend: Send {
    /// The names of the input devices that are plugged in now.
    fn ports(&mut self) -> Vec<String>;

    /// Opens the device called `name`; messages arrive at `sink`.
    ///
    /// # Errors
    /// A sentence saying why the device could not be opened.
    fn connect(&mut self, name: &str, sink: Sink) -> Result<Connection, String>;
}

/// Devices that do not exist: a fixed list of names that connect to nothing. Messages reach the app through
/// [`crate::MidiHandle::inject`] only. Used by tests and by the demo mode.
#[derive(Debug, Clone, Default)]
pub struct MockBackend {
    names: Vec<String>,
}

impl MockBackend {
    /// A backend that lists these devices.
    #[must_use]
    pub fn new(names: &[&str]) -> Self {
        Self { names: names.iter().map(|n| (*n).to_owned()).collect() }
    }
}

impl Backend for MockBackend {
    fn ports(&mut self) -> Vec<String> {
        self.names.clone()
    }

    fn connect(&mut self, name: &str, _sink: Sink) -> Result<Connection, String> {
        if self.names.iter().any(|n| n == name) { Ok(Connection::new(())) } else { Err(format!("no MIDI device called \"{name}\"")) }
    }
}

/// The backend of this system: `midir` where it exists, otherwise one that sees no devices.
#[must_use]
pub fn system_backend() -> Box<dyn Backend> {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        Box::new(system::Midir)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Box::new(MockBackend::default())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod system {
    use midir::{Ignore, MidiInput};

    use super::{Backend, Connection, Sink};

    /// The system's MIDI inputs through `midir`. A fresh `MidiInput` per call: connecting consumes it, and the
    /// platform APIs want one client per connection anyway.
    pub struct Midir;

    fn client() -> Result<MidiInput, String> {
        let mut input = MidiInput::new("CharCraft").map_err(|e| format!("MIDI is not available: {e}"))?;
        input.ignore(Ignore::All);
        Ok(input)
    }

    impl Backend for Midir {
        fn ports(&mut self) -> Vec<String> {
            let Ok(input) = client() else { return Vec::new() };
            input.ports().iter().filter_map(|p| input.port_name(p).ok()).collect()
        }

        fn connect(&mut self, name: &str, mut sink: Sink) -> Result<Connection, String> {
            let input = client()?;
            let port = input
                .ports()
                .into_iter()
                .find(|p| input.port_name(p).is_ok_and(|n| n == name))
                .ok_or_else(|| format!("no MIDI device called \"{name}\""))?;
            let connection = input.connect(&port, "charcraft-input", move |_stamp, bytes, ()| sink(bytes), ()).map_err(|e| format!("cannot open \"{name}\": {e}"))?;
            Ok(Connection::new(connection))
        }
    }
}
