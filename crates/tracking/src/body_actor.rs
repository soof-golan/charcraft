//! The body tracking actor.
//!
//! One thread owns the camera, the pose model and the solver, driven by commands and answering with events, exactly
//! like the face actor. Nothing is shared: the UI holds a [`BodyHandle`] with the two channel ends.

use std::thread::JoinHandle;
use std::time::Duration;

use anim_core::Tick;
use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded, unbounded};
use serde::{Deserialize, Serialize};

use crate::actor::CameraSpec;
use crate::body::{BodyCalibration, BodyInput, BodySmoothing, BodySolver, BodySource, ScriptedBodySource};
use crate::error::{Error, Result};

/// What the UI can ask of the body actor.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum BodyCommand {
    /// Open the camera and start tracking (replaces a running source).
    Start {
        /// The camera.
        camera: CameraSpec,
    },
    /// Stop tracking and release the camera.
    Stop,
    /// Capture the rest pose from the next `frames` frames that show both shoulders.
    SetRestPose {
        /// How many frames to average.
        frames: usize,
    },
    /// Abandon a running capture.
    CancelRestPose,
    /// Go back to the built-in rest pose.
    ResetRestPose,
    /// Use a stored rest pose (a saved profile).
    SetCalibration(BodyCalibration),
    /// Change the smoothing.
    SetSmoothing(BodySmoothing),
    /// Swap the performer's left and right.
    SetMirror(bool),
    /// Ask for a [`BodyEvent::Status`].
    Query,
    /// Stop and end the thread.
    Shutdown,
}

/// One tracked frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodySample {
    /// Capture time since the source started.
    pub time: Tick,
    /// The body.
    pub input: BodyInput,
}

/// A snapshot of the actor, for status displays and tools.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyStatus {
    /// Whether a source is running.
    pub running: bool,
    /// The running source's name.
    pub source: Option<String>,
    /// Frames processed since the last start.
    pub frames: u64,
    /// Frames in which a body was found.
    pub tracked_frames: u64,
    /// Events dropped because the consumer was too slow.
    pub dropped: u64,
    /// The rest pose in use.
    pub calibration: BodyCalibration,
    /// Rest-pose capture progress `(done, wanted)` while one runs.
    pub capturing: Option<(usize, usize)>,
    /// The smoothing in use.
    pub smoothing: BodySmoothing,
    /// Whether left and right are swapped.
    pub mirror: bool,
}

/// What the actor reports.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum BodyEvent {
    /// A source started.
    Started {
        /// Its name.
        source: String,
    },
    /// Tracking stopped on request.
    Stopped,
    /// The source ran out (a recording ended).
    SourceEnded,
    /// A tracked frame.
    Body(Box<BodySample>),
    /// A rest-pose capture advanced.
    RestPoseProgress {
        /// Frames captured.
        done: usize,
        /// Frames wanted.
        wanted: usize,
    },
    /// A rest-pose capture finished and is now in use.
    RestPoseSet(BodyCalibration),
    /// The answer to [`BodyCommand::Query`].
    Status(Box<BodyStatus>),
    /// Something failed; the source has been stopped if it was the cause.
    Error(String),
}

/// Builds a body source for a [`CameraSpec`], on the actor thread.
pub type BodySourceFactory = Box<dyn FnMut(&CameraSpec) -> Result<Box<dyn BodySource>> + Send>;

/// Capacity of the event queue. Body events beyond it are dropped (the newest frame is the one that matters).
const EVENT_QUEUE: usize = 256;

/// The UI's end of a running body actor.
#[derive(Debug)]
pub struct BodyHandle {
    commands: Sender<BodyCommand>,
    events: Receiver<BodyEvent>,
    thread: Option<JoinHandle<()>>,
}

impl BodyHandle {
    /// Sends a command.
    ///
    /// # Errors
    /// [`Error::ActorGone`] when the thread has ended.
    pub fn send(&self, command: BodyCommand) -> Result<()> {
        self.commands.send(command).map_err(|_| Error::ActorGone)
    }

    /// The next event if one is waiting.
    #[must_use]
    pub fn try_recv(&self) -> Option<BodyEvent> {
        self.events.try_recv().ok()
    }

    /// The next event, waiting up to `timeout`.
    #[must_use]
    pub fn recv_timeout(&self, timeout: Duration) -> Option<BodyEvent> {
        self.events.recv_timeout(timeout).ok()
    }

    /// The event channel, for `select!` or iteration.
    #[must_use]
    pub const fn events(&self) -> &Receiver<BodyEvent> {
        &self.events
    }

    /// Asks the actor to end and waits for its thread.
    pub fn shutdown(mut self) {
        self.finish();
    }

    fn finish(&mut self) {
        let _ = self.commands.send(BodyCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            while !thread.is_finished() {
                while self.events.try_recv().is_ok() {}
                std::thread::sleep(Duration::from_millis(1));
            }
            let _ = thread.join();
        }
    }
}

impl Drop for BodyHandle {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Starts body tracking actors.
#[derive(Debug)]
pub struct BodyActor;

impl BodyActor {
    /// Spawns the actor thread. `factory` builds a source on that thread whenever [`BodyCommand::Start`] arrives.
    #[must_use]
    pub fn spawn(factory: BodySourceFactory) -> BodyHandle {
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = bounded(EVENT_QUEUE);
        let thread = std::thread::Builder::new().name("charcraft-body".into()).spawn(move || Worker::new(factory, command_rx, event_tx).run()).ok();
        BodyHandle { commands: command_tx, events: event_rx, thread }
    }

    /// An actor for builds without camera support: it plays the demo body when [`CameraSpec::demo`] asks for it and
    /// fails every other start with a message saying so (it never substitutes the demo body unasked).
    #[must_use]
    pub fn spawn_demo_only() -> BodyHandle {
        Self::spawn(Box::new(|spec| {
            if spec.demo {
                let source: Box<dyn BodySource> = Box::new(ScriptedBodySource::demo(240, 30));
                Ok(source)
            } else {
                Err(Error::Camera("this build has no camera or pose-model support; start the demo body instead".into()))
            }
        }))
    }

    /// An actor whose every start replays a scripted source, for tests and for machines without a camera.
    #[must_use]
    pub fn spawn_scripted(make: impl Fn() -> ScriptedBodySource + Send + 'static) -> BodyHandle {
        Self::spawn(Box::new(move |_| {
            let source: Box<dyn BodySource> = Box::new(make());
            Ok(source)
        }))
    }
}

struct Worker {
    factory: BodySourceFactory,
    commands: Receiver<BodyCommand>,
    events: Sender<BodyEvent>,
    source: Option<Box<dyn BodySource>>,
    solver: BodySolver,
    frames: u64,
    tracked_frames: u64,
    dropped: u64,
    last_progress: Option<usize>,
}

impl Worker {
    fn new(factory: BodySourceFactory, commands: Receiver<BodyCommand>, events: Sender<BodyEvent>) -> Self {
        Self { factory, commands, events, source: None, solver: BodySolver::default(), frames: 0, tracked_frames: 0, dropped: 0, last_progress: None }
    }

    fn run(mut self) {
        loop {
            let command = if self.source.is_some() {
                match self.commands.try_recv() {
                    Ok(c) => Some(c),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => return,
                }
            } else {
                match self.commands.recv() {
                    Ok(c) => Some(c),
                    Err(_) => return,
                }
            };
            if let Some(command) = command {
                if !self.handle(command) {
                    return;
                }
                continue;
            }
            if !self.step() {
                return;
            }
        }
    }

    /// Sends a control event, waiting for room. `false` when the UI is gone.
    fn emit(&self, event: BodyEvent) -> bool {
        self.events.send(event).is_ok()
    }

    fn status(&self) -> BodyStatus {
        BodyStatus {
            running: self.source.is_some(),
            source: self.source.as_ref().map(|s| s.name()),
            frames: self.frames,
            tracked_frames: self.tracked_frames,
            dropped: self.dropped,
            calibration: self.solver.calibration,
            capturing: self.solver.capture_progress(),
            smoothing: self.solver.smoothing,
            mirror: self.solver.mirror,
        }
    }

    fn handle(&mut self, command: BodyCommand) -> bool {
        match command {
            BodyCommand::Start { camera } => {
                self.source = None;
                match (self.factory)(&camera) {
                    Ok(source) => {
                        let name = source.name();
                        self.source = Some(source);
                        self.frames = 0;
                        self.tracked_frames = 0;
                        self.last_progress = None;
                        self.solver.reset_smoothing();
                        self.emit(BodyEvent::Started { source: name })
                    }
                    Err(e) => self.emit(BodyEvent::Error(e.to_string())),
                }
            }
            BodyCommand::Stop => {
                let was_running = self.source.take().is_some();
                self.solver.cancel_rest_capture();
                !was_running || self.emit(BodyEvent::Stopped)
            }
            BodyCommand::SetRestPose { frames } => {
                self.solver.begin_rest_capture(frames);
                self.last_progress = None;
                true
            }
            BodyCommand::CancelRestPose => {
                self.solver.cancel_rest_capture();
                true
            }
            BodyCommand::ResetRestPose => {
                self.solver.reset_calibration();
                true
            }
            BodyCommand::SetCalibration(c) => {
                self.solver.calibration = c;
                true
            }
            BodyCommand::SetSmoothing(s) => {
                self.solver.smoothing = s;
                true
            }
            BodyCommand::SetMirror(on) => {
                self.solver.mirror = on;
                true
            }
            BodyCommand::Query => self.emit(BodyEvent::Status(Box::new(self.status()))),
            BodyCommand::Shutdown => false,
        }
    }

    /// Reads and solves one observation.
    fn step(&mut self) -> bool {
        let Some(source) = self.source.as_mut() else { return true };
        let observation = match source.next() {
            Ok(Some(o)) => o,
            Ok(None) => {
                self.source = None;
                return self.emit(BodyEvent::SourceEnded);
            }
            Err(e) => {
                self.source = None;
                return self.emit(BodyEvent::Error(e.to_string()));
            }
        };
        self.frames = self.frames.saturating_add(1);
        let solved = self.solver.solve(observation.time, observation.keypoints.as_ref());
        if solved.input.tracked {
            self.tracked_frames = self.tracked_frames.saturating_add(1);
        }
        if let Some((done, wanted)) = self.solver.capture_progress().filter(|(done, _)| self.last_progress != Some(*done)) {
            self.last_progress = Some(done);
            if !self.emit(BodyEvent::RestPoseProgress { done, wanted }) {
                return false;
            }
        }
        if let Some(c) = solved.finished_rest {
            self.last_progress = None;
            if !self.emit(BodyEvent::RestPoseSet(c)) {
                return false;
            }
        }
        match self.events.try_send(BodyEvent::Body(Box::new(BodySample { time: observation.time, input: solved.input }))) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.dropped = self.dropped.saturating_add(1);
                true
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{Joint, demo_bodies};

    const WAIT: Duration = Duration::from_secs(5);

    /// Reads events until `pick` accepts one, failing after the timeout.
    fn wait_for<T>(h: &BodyHandle, mut pick: impl FnMut(&BodyEvent) -> Option<T>) -> T {
        loop {
            let e = h.recv_timeout(WAIT).expect("an event arrives in time");
            if let Some(v) = pick(&e) {
                return v;
            }
        }
    }

    fn script() -> ScriptedBodySource {
        ScriptedBodySource::from_bodies("script", &demo_bodies(30), 30)
    }

    #[test]
    fn start_streams_bodies_until_the_script_ends() {
        let h = BodyActor::spawn_scripted(script);
        h.send(BodyCommand::Start { camera: CameraSpec::default() }).unwrap();
        wait_for(&h, |e| matches!(e, BodyEvent::Started { .. }).then_some(()));
        let mut bodies = vec![];
        wait_for(&h, |e| match e {
            BodyEvent::Body(s) => {
                bodies.push(**s);
                None
            }
            BodyEvent::SourceEnded => Some(()),
            _ => None,
        });
        assert_eq!(bodies.len(), 30);
        assert!(bodies.iter().all(|s| s.input.tracked && s.input.is_valid()));
        assert!(bodies.windows(2).all(|w| w[0].time < w[1].time));
        assert!(bodies.iter().all(|s| s.input.joint(Joint::LeftWrist).is_some()));
        h.shutdown();
    }

    #[test]
    fn the_rest_pose_is_captured_on_command_and_reported() {
        let h = BodyActor::spawn_scripted(script);
        h.send(BodyCommand::SetRestPose { frames: 5 }).unwrap();
        h.send(BodyCommand::Start { camera: CameraSpec::default() }).unwrap();
        let set = wait_for(&h, |e| match e {
            BodyEvent::RestPoseSet(c) => Some(*c),
            _ => None,
        });
        assert!(set.captured);
        h.send(BodyCommand::Query).unwrap();
        let status = wait_for(&h, |e| match e {
            BodyEvent::Status(s) => Some(s.clone()),
            _ => None,
        });
        assert!(status.calibration.captured && status.capturing.is_none());
        h.send(BodyCommand::ResetRestPose).unwrap();
        h.send(BodyCommand::SetMirror(true)).unwrap();
        h.send(BodyCommand::Query).unwrap();
        let after = wait_for(&h, |e| match e {
            BodyEvent::Status(s) => Some(s.clone()),
            _ => None,
        });
        assert!(!after.calibration.captured && after.mirror);
        h.shutdown();
    }

    #[test]
    fn a_failing_factory_reports_an_error_and_stop_ends_a_run() {
        let h = BodyActor::spawn(Box::new(|_| Err(Error::Camera("no camera".into()))));
        h.send(BodyCommand::Start { camera: CameraSpec::default() }).unwrap();
        let msg = wait_for(&h, |e| match e {
            BodyEvent::Error(m) => Some(m.clone()),
            _ => None,
        });
        assert!(msg.contains("no camera"), "{msg}");
        h.shutdown();
        let h = BodyActor::spawn_scripted(|| ScriptedBodySource::demo(30, 30));
        h.send(BodyCommand::Start { camera: CameraSpec::default() }).unwrap();
        wait_for(&h, |e| matches!(e, BodyEvent::Body(_)).then_some(()));
        h.send(BodyCommand::Stop).unwrap();
        wait_for(&h, |e| matches!(e, BodyEvent::Stopped).then_some(()));
        h.shutdown();
    }
}
