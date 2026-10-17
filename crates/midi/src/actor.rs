//! The MIDI actor: owns the backend and the open device, takes requests and reports events.

use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::MidiMessage;
use crate::backend::{Backend, Connection};

/// What the app can ask of the actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Look for devices; the answer is [`Event::Ports`].
    List,
    /// Open this device, or close the open one with `None`; the answer is [`Event::Connected`] or [`Event::Failed`].
    Connect(Option<String>),
    /// Handle these raw bytes as if the open device had sent them.
    Inject(Vec<u8>),
    /// End the actor.
    Shutdown,
}

/// What the actor reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The devices that are plugged in.
    Ports(Vec<String>),
    /// The device that is open now (`None` after closing).
    Connected(Option<String>),
    /// A note or controller message.
    Message(MidiMessage),
    /// A device could not be opened.
    Failed(String),
}

/// The app's end of the actor.
pub struct MidiHandle {
    requests: Sender<Request>,
    events: Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

impl MidiHandle {
    /// Starts the actor on `backend`. `wake` is called after every event so the window repaints.
    #[must_use]
    pub fn spawn(backend: Box<dyn Backend>, wake: impl Fn() + Clone + Send + 'static) -> Self {
        let (requests, request_rx) = unbounded();
        let (event_tx, events) = unbounded();
        let thread = std::thread::Builder::new().name("charcraft-midi".into()).spawn(move || run(backend, &request_rx, &event_tx, &wake)).ok();
        Self { requests, events, thread }
    }

    /// Asks for the device list (the answer arrives as an event).
    pub fn list(&self) {
        let _ = self.requests.send(Request::List);
    }

    /// Opens a device by name, or closes the open one.
    pub fn connect(&self, name: Option<String>) {
        let _ = self.requests.send(Request::Connect(name));
    }

    /// Feeds raw bytes in as if the open device had sent them (tests, the MCP `midi_send` command).
    pub fn inject(&self, bytes: Vec<u8>) {
        let _ = self.requests.send(Request::Inject(bytes));
    }

    /// The next event, if one is waiting.
    #[must_use]
    pub fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// Waits up to `timeout` for the next event (tests).
    #[must_use]
    pub fn wait_event(&self, timeout: std::time::Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }
}

impl Drop for MidiHandle {
    fn drop(&mut self) {
        let _ = self.requests.send(Request::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(mut backend: Box<dyn Backend>, requests: &Receiver<Request>, events: &Sender<Event>, wake: &(impl Fn() + Clone + Send + 'static)) {
    let send = |event: Event| {
        let _ = events.send(event);
        wake();
    };
    let mut open: Option<(String, Connection)> = None;
    while let Ok(request) = requests.recv() {
        match request {
            Request::List => send(Event::Ports(backend.ports())),
            Request::Connect(None) => {
                open = None;
                send(Event::Connected(None));
            }
            Request::Connect(Some(name)) => {
                let (tx, wake_in) = (events.clone(), wake.clone());
                let sink = Box::new(move |bytes: &[u8]| {
                    if let Some(message) = MidiMessage::parse(bytes) {
                        let _ = tx.send(Event::Message(message));
                        wake_in();
                    }
                });
                // The old device closes before the new one opens: some drivers cannot share.
                open = None;
                match backend.connect(&name, sink) {
                    Ok(connection) => {
                        open = Some((name.clone(), connection));
                        send(Event::Connected(Some(name)));
                    }
                    Err(reason) => {
                        send(Event::Failed(reason));
                        send(Event::Connected(None));
                    }
                }
            }
            Request::Inject(bytes) => {
                if let Some(message) = MidiMessage::parse(&bytes) {
                    send(Event::Message(message));
                }
            }
            Request::Shutdown => break,
        }
    }
    drop(open);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::MockBackend;

    const WAIT: Duration = Duration::from_secs(2);

    fn handle() -> MidiHandle {
        MidiHandle::spawn(Box::new(MockBackend::new(&["Keys 49", "Pad Controller"])), || {})
    }

    #[test]
    fn the_actor_lists_and_connects_devices() {
        let midi = handle();
        midi.list();
        assert_eq!(midi.wait_event(WAIT), Some(Event::Ports(vec!["Keys 49".into(), "Pad Controller".into()])));
        midi.connect(Some("Pad Controller".into()));
        assert_eq!(midi.wait_event(WAIT), Some(Event::Connected(Some("Pad Controller".into()))));
        midi.connect(Some("Gone".into()));
        assert!(matches!(midi.wait_event(WAIT), Some(Event::Failed(_))));
        assert_eq!(midi.wait_event(WAIT), Some(Event::Connected(None)));
        midi.connect(None);
        assert_eq!(midi.wait_event(WAIT), Some(Event::Connected(None)));
    }

    #[test]
    fn injected_bytes_become_events_and_junk_is_dropped() {
        let midi = handle();
        midi.inject(vec![0xf8]);
        midi.inject(vec![0x90, 60, 90]);
        midi.inject(vec![0xb0, 1, 64]);
        assert_eq!(midi.wait_event(WAIT), Some(Event::Message(MidiMessage::NoteOn { channel: 1, note: 60, velocity: 90 })));
        assert_eq!(midi.wait_event(WAIT), Some(Event::Message(MidiMessage::ControlChange { channel: 1, controller: 1, value: 64 })));
        assert_eq!(midi.try_event(), None);
    }

    #[test]
    fn dropping_the_handle_ends_the_actor() {
        drop(handle());
    }
}
