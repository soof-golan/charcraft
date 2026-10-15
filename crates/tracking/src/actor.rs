//! The tracking actor: one thread that owns the camera, the model and the solver, driven by commands and
//! answering with events. Nothing is shared: the UI holds a [`TrackingHandle`] with the two channel ends.
//!
//! ```text
//!  UI thread                         tracking thread (owns source + solver)
//!  ─────────                         ───────────────────────────────────────
//!  Command::Start(camera) ─────────► factory(&camera) builds a FaceSource on this thread
//!  Command::Calibrate{frames} ─────► solver captures the neutral face
//!  Command::Stop / Shutdown ───────► drops the source (releases the camera)
//!            ◄───────── Event::Face(Sample) per frame, Calibrated, Status, Error, ...
//! ```

use std::thread::JoinHandle;
use std::time::Duration;

use anim_core::Tick;
use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded, unbounded};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::input::FaceInput;
use crate::solver::{Calibration, FaceSolver, Sensitivity, Smoothing};
use crate::source::{FaceSource, ScriptedSource};

/// Which camera to open and how.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraSpec {
    /// Device index from [`crate::camera::list_cameras`] (0 is the system default).
    pub index: u32,
    /// Requested frame width in pixels.
    pub width: u32,
    /// Requested frame height in pixels.
    pub height: u32,
    /// Requested frames per second.
    pub fps: u32,
}

impl Default for CameraSpec {
    fn default() -> Self {
        Self { index: 0, width: 640, height: 480, fps: 30 }
    }
}

/// What the UI can ask of the actor.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Command {
    /// Open the camera and start tracking (replaces a running source).
    Start {
        /// The camera.
        camera: CameraSpec,
    },
    /// Stop tracking and release the camera.
    Stop,
    /// Capture the neutral face from the next `frames` tracked frames.
    Calibrate {
        /// How many frames to average.
        frames: usize,
    },
    /// Abandon a running calibration capture.
    CancelCalibration,
    /// Go back to the built-in typical face.
    ResetCalibration,
    /// Use a stored calibration (a saved profile).
    SetCalibration(Calibration),
    /// Change the output ranges.
    SetSensitivity(Sensitivity),
    /// Change the smoothing.
    SetSmoothing(Smoothing),
    /// Ask for a [`Event::Status`].
    Query,
    /// Stop and end the thread.
    Shutdown,
}

/// One tracked frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// Capture time since the source started.
    pub time: Tick,
    /// The face.
    pub input: FaceInput,
}

/// A snapshot of the actor, for status displays and tools.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Status {
    /// Whether a source is running.
    pub running: bool,
    /// The running source's name.
    pub source: Option<String>,
    /// Frames processed since the last start.
    pub frames: u64,
    /// Frames in which a face was found.
    pub tracked_frames: u64,
    /// Face events dropped because the consumer was too slow.
    pub dropped: u64,
    /// The calibration in use.
    pub calibration: Calibration,
    /// Calibration capture progress `(done, wanted)` while one runs.
    pub calibrating: Option<(usize, usize)>,
}

/// What the actor reports.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Event {
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
    Face(Sample),
    /// A calibration capture advanced.
    CalibrationProgress {
        /// Frames captured.
        done: usize,
        /// Frames wanted.
        wanted: usize,
    },
    /// A calibration capture finished and is now in use.
    Calibrated(Calibration),
    /// The answer to [`Command::Query`].
    Status(Box<Status>),
    /// Something failed; the source has been stopped if it was the cause.
    Error(String),
}

/// Builds a source for a [`CameraSpec`]. It runs on the actor thread, so the source it returns (camera handles are
/// often not `Send`) never crosses threads.
pub type SourceFactory = Box<dyn FnMut(&CameraSpec) -> Result<Box<dyn FaceSource>> + Send>;

/// Capacity of the event queue. Face events beyond it are dropped (the newest frame is the one that matters).
const EVENT_QUEUE: usize = 256;

/// The UI's end of a running actor.
#[derive(Debug)]
pub struct TrackingHandle {
    commands: Sender<Command>,
    events: Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

impl TrackingHandle {
    /// Sends a command.
    ///
    /// # Errors
    /// [`Error::ActorGone`] when the thread has ended.
    pub fn send(&self, command: Command) -> Result<()> {
        self.commands.send(command).map_err(|_| Error::ActorGone)
    }

    /// The next event if one is waiting.
    #[must_use]
    pub fn try_recv(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// The next event, waiting up to `timeout`.
    #[must_use]
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }

    /// The event channel, for `select!` or iteration.
    #[must_use]
    pub const fn events(&self) -> &Receiver<Event> {
        &self.events
    }

    /// Asks the actor to end and waits for its thread.
    pub fn shutdown(mut self) {
        self.finish();
    }

    fn finish(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            // The actor may be blocked sending a control event into a full queue: keep emptying it until it ends.
            while !thread.is_finished() {
                while self.events.try_recv().is_ok() {}
                std::thread::sleep(Duration::from_millis(1));
            }
            let _ = thread.join();
        }
    }
}

impl Drop for TrackingHandle {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Starts tracking actors.
#[derive(Debug)]
pub struct TrackingActor;

impl TrackingActor {
    /// Spawns the actor thread. `factory` builds a source on that thread whenever [`Command::Start`] arrives.
    #[must_use]
    pub fn spawn(factory: SourceFactory) -> TrackingHandle {
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = bounded(EVENT_QUEUE);
        let thread = std::thread::Builder::new()
            .name("charcraft-tracking".into())
            .spawn(move || Worker::new(factory, command_rx, event_tx).run())
            .ok();
        TrackingHandle { commands: command_tx, events: event_rx, thread }
    }

    /// An actor whose every [`Command::Start`] replays a scripted source, for tests and for machines without a
    /// camera.
    #[must_use]
    pub fn spawn_scripted(make: impl Fn() -> ScriptedSource + Send + 'static) -> TrackingHandle {
        Self::spawn(Box::new(move |_| {
            let source: Box<dyn FaceSource> = Box::new(make());
            Ok(source)
        }))
    }
}

struct Worker {
    factory: SourceFactory,
    commands: Receiver<Command>,
    events: Sender<Event>,
    source: Option<Box<dyn FaceSource>>,
    solver: FaceSolver,
    frames: u64,
    tracked_frames: u64,
    dropped: u64,
    last_progress: Option<usize>,
}

/// Whether the worker should keep going.
type Alive = bool;

impl Worker {
    fn new(factory: SourceFactory, commands: Receiver<Command>, events: Sender<Event>) -> Self {
        Self { factory, commands, events, source: None, solver: FaceSolver::default(), frames: 0, tracked_frames: 0, dropped: 0, last_progress: None }
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
    fn emit(&self, event: Event) -> Alive {
        self.events.send(event).is_ok()
    }

    fn status(&self) -> Status {
        Status {
            running: self.source.is_some(),
            source: self.source.as_ref().map(|s| s.name()),
            frames: self.frames,
            tracked_frames: self.tracked_frames,
            dropped: self.dropped,
            calibration: self.solver.calibration,
            calibrating: self.solver.calibration_progress(),
        }
    }

    fn handle(&mut self, command: Command) -> Alive {
        match command {
            Command::Start { camera } => {
                self.source = None;
                match (self.factory)(&camera) {
                    Ok(source) => {
                        let name = source.name();
                        self.source = Some(source);
                        self.frames = 0;
                        self.tracked_frames = 0;
                        self.last_progress = None;
                        self.solver.reset_smoothing();
                        self.emit(Event::Started { source: name })
                    }
                    Err(e) => self.emit(Event::Error(e.to_string())),
                }
            }
            Command::Stop => {
                let was_running = self.source.take().is_some();
                self.solver.cancel_calibration();
                !was_running || self.emit(Event::Stopped)
            }
            Command::Calibrate { frames } => {
                self.solver.begin_calibration(frames);
                self.last_progress = None;
                true
            }
            Command::CancelCalibration => {
                self.solver.cancel_calibration();
                true
            }
            Command::ResetCalibration => {
                self.solver.reset_calibration();
                true
            }
            Command::SetCalibration(c) => {
                self.solver.calibration = c;
                true
            }
            Command::SetSensitivity(s) => {
                self.solver.sensitivity = s;
                true
            }
            Command::SetSmoothing(s) => {
                self.solver.smoothing = s;
                true
            }
            Command::Query => self.emit(Event::Status(Box::new(self.status()))),
            Command::Shutdown => false,
        }
    }

    /// Reads and solves one observation.
    fn step(&mut self) -> Alive {
        let Some(source) = self.source.as_mut() else { return true };
        let observation = match source.next() {
            Ok(Some(o)) => o,
            Ok(None) => {
                self.source = None;
                return self.emit(Event::SourceEnded);
            }
            Err(e) => {
                self.source = None;
                return self.emit(Event::Error(e.to_string()));
            }
        };
        self.frames = self.frames.saturating_add(1);
        let solved = self.solver.solve(observation.time, observation.landmarks.as_ref());
        if solved.input.tracked {
            self.tracked_frames = self.tracked_frames.saturating_add(1);
        }
        if let Some((done, wanted)) = self.solver.calibration_progress().filter(|(done, _)| self.last_progress != Some(*done)) {
            self.last_progress = Some(done);
            if !self.emit(Event::CalibrationProgress { done, wanted }) {
                return false;
            }
        }
        if let Some(c) = solved.finished_calibration {
            self.last_progress = None;
            if !self.emit(Event::Calibrated(c)) {
                return false;
            }
        }
        match self.events.try_send(Event::Face(Sample { time: observation.time, input: solved.input })) {
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
    use crate::synthetic::{SyntheticFace, demo_faces};

    const WAIT: Duration = Duration::from_secs(5);

    fn start() -> CameraSpec {
        CameraSpec::default()
    }

    /// Reads events until `pick` accepts one, failing after the timeout.
    fn wait_for<T>(h: &TrackingHandle, mut pick: impl FnMut(&Event) -> Option<T>) -> T {
        loop {
            let e = h.recv_timeout(WAIT).expect("an event arrives in time");
            if let Some(v) = pick(&e) {
                return v;
            }
        }
    }

    fn script() -> ScriptedSource {
        ScriptedSource::from_faces("script", &demo_faces(30), 30)
    }

    #[test]
    fn start_streams_faces_until_the_script_ends() {
        let h = TrackingActor::spawn_scripted(script);
        h.send(Command::Start { camera: start() }).unwrap();
        assert_eq!(wait_for(&h, |e| matches!(e, Event::Started { .. }).then_some(())), ());
        let mut faces = vec![];
        wait_for(&h, |e| match e {
            Event::Face(s) => {
                faces.push(*s);
                None
            }
            Event::SourceEnded => Some(()),
            _ => None,
        });
        assert_eq!(faces.len(), 30);
        assert!(faces.iter().all(|s| s.input.tracked && s.input.is_valid()));
        assert!(faces.windows(2).all(|w| w[0].time < w[1].time));
        let moved = faces.iter().map(|s| s.input.head.yaw).fold(0.0_f32, |m, y| m.max(y.abs()));
        assert!(moved > 0.1, "the demo head turns: {moved}");
        h.shutdown();
    }

    #[test]
    fn stop_releases_the_source_and_reports() {
        let h = TrackingActor::spawn_scripted(|| script().looping());
        h.send(Command::Start { camera: start() }).unwrap();
        wait_for(&h, |e| matches!(e, Event::Face(_)).then_some(()));
        h.send(Command::Stop).unwrap();
        wait_for(&h, |e| matches!(e, Event::Stopped).then_some(()));
        h.send(Command::Query).unwrap();
        let status = wait_for(&h, |e| if let Event::Status(s) = e { Some(s.clone()) } else { None });
        assert!(!status.running && status.source.is_none());
        assert!(status.frames > 0);
        h.shutdown();
    }

    #[test]
    fn calibrate_reports_progress_and_adopts_the_result() {
        let rest = SyntheticFace { mouth_open: 0.1, yaw: 0.2, ..SyntheticFace::default() };
        let h = TrackingActor::spawn_scripted(move || ScriptedSource::from_faces("rest", &[rest; 40], 30));
        // Calibration is requested before the frames flow, so it covers the first frames.
        h.send(Command::Calibrate { frames: 5 }).unwrap();
        h.send(Command::Start { camera: start() }).unwrap();
        let mut progress = vec![];
        let calibration = wait_for(&h, |e| match e {
            Event::CalibrationProgress { done, wanted } => {
                assert_eq!(*wanted, 5);
                progress.push(*done);
                None
            }
            Event::Calibrated(c) => Some(*c),
            _ => None,
        });
        assert!(calibration.captured);
        assert_eq!(progress, [1, 2, 3, 4]);
        // After calibrating, the held pose reads as neutral.
        let after = wait_for(&h, |e| if let Event::Face(s) = e { Some(*s) } else { None });
        assert!(after.input.head.yaw.abs() < 0.05 && after.input.mouth_open < 0.05, "{after:?}");
        h.send(Command::Query).unwrap();
        let status = wait_for(&h, |e| if let Event::Status(s) = e { Some(s.clone()) } else { None });
        assert_eq!(status.calibration, calibration);
        assert_eq!(status.calibrating, None);
        h.shutdown();
    }

    #[test]
    fn a_source_that_fails_stops_with_an_error_and_the_actor_survives() {
        struct Failing;
        impl FaceSource for Failing {
            fn name(&self) -> String {
                "failing".into()
            }
            fn next(&mut self) -> Result<Option<crate::source::Observation>> {
                Err(Error::Camera("unplugged".into()))
            }
        }
        let mut n = 0;
        let h = TrackingActor::spawn(Box::new(move |_| {
            n += 1;
            let source: Box<dyn FaceSource> = if n == 1 { Box::new(Failing) } else { Box::new(script()) };
            Ok(source)
        }));
        h.send(Command::Start { camera: start() }).unwrap();
        let message = wait_for(&h, |e| if let Event::Error(m) = e { Some(m.clone()) } else { None });
        assert!(message.contains("unplugged"));
        h.send(Command::Start { camera: start() }).unwrap();
        wait_for(&h, |e| matches!(e, Event::SourceEnded).then_some(()));
        h.shutdown();
    }

    #[test]
    fn a_factory_error_is_reported() {
        let h = TrackingActor::spawn(Box::new(|c| Err(Error::Camera(format!("no camera {}", c.index)))));
        h.send(Command::Start { camera: CameraSpec { index: 3, ..start() } }).unwrap();
        let message = wait_for(&h, |e| if let Event::Error(m) = e { Some(m.clone()) } else { None });
        assert!(message.contains("no camera 3"));
        h.shutdown();
    }

    #[test]
    fn frames_without_a_face_are_reported_untracked() {
        use crate::source::Observation;
        let frames = vec![
            Observation { time: Tick(0), landmarks: Some(SyntheticFace::default().landmarks()) },
            Observation { time: Tick(1), landmarks: None },
        ];
        let h = TrackingActor::spawn_scripted(move || ScriptedSource::new("gap", frames.clone()));
        h.send(Command::Start { camera: start() }).unwrap();
        let mut tracked = vec![];
        wait_for(&h, |e| match e {
            Event::Face(s) => {
                tracked.push(s.input.tracked);
                None
            }
            Event::SourceEnded => Some(()),
            _ => None,
        });
        assert_eq!(tracked, [true, false]);
        h.send(Command::Query).unwrap();
        let status = wait_for(&h, |e| if let Event::Status(s) = e { Some(s.clone()) } else { None });
        assert_eq!((status.frames, status.tracked_frames), (2, 1));
        h.shutdown();
    }

    #[test]
    fn a_slow_consumer_drops_faces_instead_of_stalling_the_actor() {
        let h = TrackingActor::spawn_scripted(|| ScriptedSource::from_faces("many", &demo_faces(1000), 30));
        h.send(Command::Start { camera: start() }).unwrap();
        // Do not read events; the script must still run to its end without blocking.
        std::thread::sleep(Duration::from_millis(300));
        h.send(Command::Query).unwrap();
        let status = wait_for(&h, |e| if let Event::Status(s) = e { Some(s.clone()) } else { None });
        assert!(status.dropped > 0, "{}", status.dropped);
        h.shutdown();
    }

    #[test]
    fn settings_commands_apply() {
        let h = TrackingActor::spawn_scripted(script);
        let sens = Sensitivity { brow: 0.5, ..Sensitivity::default() };
        h.send(Command::SetSensitivity(sens)).unwrap();
        h.send(Command::SetSmoothing(Smoothing::off())).unwrap();
        h.send(Command::SetCalibration(Calibration::default())).unwrap();
        h.send(Command::ResetCalibration).unwrap();
        h.send(Command::CancelCalibration).unwrap();
        h.send(Command::Query).unwrap();
        let status = wait_for(&h, |e| if let Event::Status(s) = e { Some(s.clone()) } else { None });
        assert_eq!(status.calibration, Calibration::default());
        h.shutdown();
    }

    #[test]
    fn commands_after_shutdown_fail_cleanly() {
        let h = TrackingActor::spawn_scripted(script);
        h.send(Command::Shutdown).unwrap();
        // Wait for the thread to notice.
        for _ in 0..100 {
            if h.send(Command::Query).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the actor never stopped");
    }

    #[test]
    fn dropping_the_handle_stops_the_thread() {
        let h = TrackingActor::spawn_scripted(|| script().looping());
        h.send(Command::Start { camera: start() }).unwrap();
        wait_for(&h, |e| matches!(e, Event::Face(_)).then_some(()));
        drop(h);
    }
}
