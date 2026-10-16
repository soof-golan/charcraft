//! One camera for several trackers.
//!
//! A webcam can usually be opened by one reader at a time, but a puppeteer wants the face tracker and the body
//! tracker on the same picture. [`CameraHub`] is an actor that owns the camera and hands every frame to each
//! subscriber over its own channel; a subscriber is a [`HubSource`], an ordinary [`FrameSource`] that a
//! [`crate::source::TrackedCamera`] or a [`crate::body::TrackedBodyCamera`] reads. The camera opens when the first
//! subscriber arrives and is released when the last one is dropped. Nothing is shared between threads except the
//! channels.

use std::thread::JoinHandle;

use anim_core::Tick;
use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded, unbounded};

use crate::actor::CameraSpec;
use crate::error::{Error, Result};
use crate::frame::Frame;
use crate::source::FrameSource;

/// Opens a camera on the hub's thread.
pub type FrameOpener = Box<dyn FnMut(&CameraSpec) -> Result<Box<dyn FrameSource>> + Send>;

/// Frames a slow subscriber may have waiting before the hub starts skipping frames for it.
const BACKLOG: usize = 2;

enum Delivery {
    Frame(Tick, Frame),
    Ended,
    Failed(String),
}

enum HubCommand {
    Subscribe { spec: CameraSpec, frames: Sender<Delivery>, reply: Sender<Result<String>> },
}

/// A handle on the camera actor. Clone it to give every tracker its own.
#[derive(Clone, Debug)]
pub struct CameraHub {
    commands: Sender<HubCommand>,
}

impl CameraHub {
    /// Starts the hub's thread. `opener` runs on that thread whenever the first subscriber arrives. The thread ends
    /// when every handle is dropped.
    #[must_use]
    pub fn spawn(opener: FrameOpener) -> (Self, Option<JoinHandle<()>>) {
        let (commands, receiver) = unbounded();
        let thread = std::thread::Builder::new().name("charcraft-camera".into()).spawn(move || Hub::new(opener, receiver).run()).ok();
        (Self { commands }, thread)
    }

    /// Starts the hub on a detached thread.
    #[must_use]
    pub fn start(opener: FrameOpener) -> Self {
        Self::spawn(opener).0
    }

    /// Joins the camera: opens it if nobody has, and returns a source of its frames.
    ///
    /// # Errors
    /// The opener's error when the camera cannot be opened, or [`Error::ActorGone`].
    pub fn subscribe(&self, spec: &CameraSpec) -> Result<HubSource> {
        let (frames_tx, frames) = bounded(BACKLOG);
        let (reply_tx, reply_rx) = bounded(1);
        self.commands.send(HubCommand::Subscribe { spec: spec.clone(), frames: frames_tx, reply: reply_tx }).map_err(|_| Error::ActorGone)?;
        let name = reply_rx.recv().map_err(|_| Error::ActorGone)??;
        Ok(HubSource { name, frames })
    }
}

/// A subscriber's view of the camera.
#[derive(Debug)]
pub struct HubSource {
    name: String,
    frames: Receiver<Delivery>,
}

impl FrameSource for HubSource {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn next_frame(&mut self) -> Result<Option<(Tick, Frame)>> {
        match self.frames.recv() {
            Ok(Delivery::Frame(time, frame)) => Ok(Some((time, frame))),
            Ok(Delivery::Ended) | Err(_) => Ok(None),
            Ok(Delivery::Failed(message)) => Err(Error::Camera(message)),
        }
    }
}

struct Hub {
    opener: FrameOpener,
    commands: Receiver<HubCommand>,
    camera: Option<Box<dyn FrameSource>>,
    subscribers: Vec<Sender<Delivery>>,
}

impl Hub {
    fn new(opener: FrameOpener, commands: Receiver<HubCommand>) -> Self {
        Self { opener, commands, camera: None, subscribers: Vec::new() }
    }

    fn run(mut self) {
        loop {
            let command = if self.camera.is_some() && !self.subscribers.is_empty() {
                match self.commands.try_recv() {
                    Ok(c) => Some(c),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => return,
                }
            } else {
                // Nobody is watching: let go of the camera and wait for the next subscriber.
                self.camera = None;
                match self.commands.recv() {
                    Ok(c) => Some(c),
                    Err(_) => return,
                }
            };
            match command {
                Some(HubCommand::Subscribe { spec, frames, reply }) => self.subscribe(&spec, frames, &reply),
                None => self.pump(),
            }
        }
    }

    fn subscribe(&mut self, spec: &CameraSpec, frames: Sender<Delivery>, reply: &Sender<Result<String>>) {
        if self.camera.is_none() {
            match (self.opener)(spec) {
                Ok(camera) => self.camera = Some(camera),
                Err(e) => {
                    let _ = reply.send(Err(e));
                    return;
                }
            }
        }
        let name = self.camera.as_ref().map(FrameSource::name).unwrap_or_default();
        if reply.send(Ok(name)).is_ok() {
            self.subscribers.push(frames);
        }
    }

    /// Reads one frame and hands it to everyone; a subscriber that has gone is dropped, and the camera with the last.
    fn pump(&mut self) {
        let Some(camera) = self.camera.as_mut() else { return };
        let delivery = match camera.next_frame() {
            Ok(Some((time, frame))) => Delivery::Frame(time, frame),
            Ok(None) => Delivery::Ended,
            Err(e) => Delivery::Failed(e.to_string()),
        };
        let finished = !matches!(delivery, Delivery::Frame(..));
        self.subscribers.retain(|s| {
            let item = match &delivery {
                Delivery::Frame(t, f) => Delivery::Frame(*t, f.clone()),
                Delivery::Ended => Delivery::Ended,
                Delivery::Failed(m) => Delivery::Failed(m.clone()),
            };
            // A full queue means a slow tracker: it skips this frame.
            !matches!(s.try_send(item), Err(TrySendError::Disconnected(_)))
        });
        if finished {
            self.subscribers.clear();
            self.camera = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A camera of `n` tiny frames that tells the test when it is dropped.
    struct Fake {
        left: u32,
        dropped: Sender<()>,
    }

    impl FrameSource for Fake {
        fn name(&self) -> String {
            "fake".into()
        }
        fn next_frame(&mut self) -> Result<Option<(Tick, Frame)>> {
            std::thread::sleep(Duration::from_millis(2));
            if self.left == 0 {
                return Ok(None);
            }
            self.left -= 1;
            Ok(Some((Tick(i64::from(self.left)), Frame::solid(2, 2, [1, 2, 3])?)))
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = self.dropped.send(());
        }
    }

    fn hub(frames: u32) -> (CameraHub, Receiver<()>, Receiver<()>) {
        let (dropped_tx, dropped_rx) = unbounded();
        let (opened_tx, opened_rx) = unbounded();
        let hub = CameraHub::start(Box::new(move |_| {
            let _ = opened_tx.send(());
            let camera: Box<dyn FrameSource> = Box::new(Fake { left: frames, dropped: dropped_tx.clone() });
            Ok(camera)
        }));
        (hub, opened_rx, dropped_rx)
    }

    #[test]
    fn two_subscribers_see_the_same_camera_and_it_opens_once() {
        let (hub, opened, _dropped) = hub(1000);
        let mut a = hub.subscribe(&CameraSpec::default()).unwrap();
        let mut b = hub.subscribe(&CameraSpec::default()).unwrap();
        assert_eq!(a.name(), "fake");
        for _ in 0..5 {
            assert!(a.next_frame().unwrap().is_some());
            assert!(b.next_frame().unwrap().is_some());
        }
        assert_eq!(opened.try_iter().count(), 1, "one camera, two readers");
    }

    #[test]
    fn the_camera_is_released_when_the_last_subscriber_goes() {
        let (hub, opened, dropped) = hub(100_000);
        let a = hub.subscribe(&CameraSpec::default()).unwrap();
        let mut b = hub.subscribe(&CameraSpec::default()).unwrap();
        drop(a);
        assert!(b.next_frame().unwrap().is_some());
        assert!(dropped.recv_timeout(Duration::from_millis(50)).is_err(), "still one reader");
        drop(b);
        assert!(dropped.recv_timeout(Duration::from_secs(5)).is_ok(), "the camera was dropped");
        // A later subscriber opens it again.
        let mut again = hub.subscribe(&CameraSpec::default()).unwrap();
        assert!(again.next_frame().unwrap().is_some());
        assert_eq!(opened.try_iter().count(), 2);
    }

    #[test]
    fn the_end_of_a_stream_reaches_everyone_and_a_failed_open_is_an_error() {
        let (hub, _o, _d) = hub(3);
        let mut a = hub.subscribe(&CameraSpec::default()).unwrap();
        let mut got = 0;
        while a.next_frame().unwrap().is_some() {
            got += 1;
        }
        assert!(got <= 3);
        let broken = CameraHub::start(Box::new(|_| Err(Error::Camera("busy".into()))));
        assert!(matches!(broken.subscribe(&CameraSpec::default()), Err(Error::Camera(m)) if m == "busy"));
    }
}
