//! The live input actor: the camera and face tracker, the microphone and its lip sync analyser.
//!
//! One thread owns the tracking actor's tool front ([`TrackingTools`]), the microphone ([`CaptureActor`]) and the
//! [`LiveLipSync`] analyser. The UI and the MCP commands talk to it over channels only: [`LiveLink`] carries requests
//! (tool calls with a reply channel), [`LiveEvent`]s stream back faces, mouth shapes and microphone audio. Nothing
//! here touches the document, so live input is never undoable and never part of a snapshot.

use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anim_audio::device::CpalInput;
use anim_audio::{CaptureActor, CaptureCommand, CaptureEvent, PcmBuffer};
use anim_core::Tick;
use anim_lipsync::{LiveLipSync, Viseme};
use charcraft_behaviors::{BodyInput, FaceInput};
use charcraft_tracking::actor::{Preview, Sample, TrackingHandle};
use charcraft_tracking::body::BodyInput as TrackedBody;
use charcraft_tracking::body_actor::{BodyHandle, BodySample};
use charcraft_tracking::input::FaceInput as TrackedFace;
use charcraft_tracking::tools::{TrackingTools, tools as tracking_tool_specs};
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use serde_json::{Value, json};

/// How long a caller waits for the actor to answer a request.
const ANSWER: Duration = Duration::from_secs(5);
/// How long the actor waits for the system's camera list before it reports a failure instead.
const CAMERA_TIMEOUT: Duration = Duration::from_secs(8);
/// How long a caller waits for the answer to a camera listing: the actor's own timeout plus a margin.
const CAMERA_ANSWER: Duration = Duration::from_secs(10);
/// The tool whose answer comes from a worker thread instead of the actor's own loop.
const LIST_CAMERAS: &str = "tracking_list_cameras";
/// The loop period of the actor.
const TICK: Duration = Duration::from_millis(8);
/// Events the UI may fall behind by before the actor drops face and voice frames.
const EVENT_QUEUE: usize = 512;

/// What the microphone panel can ask for.
#[derive(Debug, Clone, PartialEq)]
pub enum MicAction {
    /// Open the selected input device (the default one unless [`MicAction::SetDevice`] chose another).
    Start,
    /// Close it.
    Stop,
    /// Report the state.
    Status,
    /// Play a synthetic talking mouth when no microphone is on (demos, screenshots and tests).
    DemoVoice(bool),
    /// List the input devices.
    Devices,
    /// Choose the input device by name (`None` for the system default). A running microphone restarts on it.
    SetDevice(Option<String>),
    /// Set the input sensitivity: a gain applied before the level meter and the lip sync, clamped to 0.1 to 8.
    SetGain(f32),
}

/// Smallest and largest microphone input gain.
pub const GAIN_RANGE: (f32, f32) = (0.1, 8.0);

/// A request to the actor.
#[derive(Debug)]
pub enum LiveRequest {
    /// A `tracking_*` tool call.
    Tool { name: String, params: Value, reply: Sender<Result<Value, String>> },
    /// A microphone action.
    Mic { action: MicAction, reply: Sender<Result<Value, String>> },
    /// Forward microphone chunks as [`LiveEvent::Audio`] (while a take records sound).
    ForwardAudio(bool),
    /// Look for cameras; the answer arrives as [`LiveEvent::Cameras`], never as a reply, so the caller never waits.
    ListCameras,
    /// End the actor.
    Shutdown,
}

/// What the actor reports.
#[derive(Debug, Clone)]
pub enum LiveEvent {
    /// A new tracked face (`tracked` tells whether a face was found).
    Face(Tick, TrackedFace),
    /// A new tracked body (`tracked` tells whether a body was found).
    Body(Tick, Box<TrackedBody>),
    /// The mouth shape the voice makes, and the input level 0 to 1.
    Voice { viseme: Option<Viseme>, level: f32 },
    /// A microphone chunk (only while forwarding).
    Audio(PcmBuffer),
    /// The camera image and tracked points.
    Preview(Box<Preview>),
    /// The outcome of a camera search: the `tracking_list_cameras` answer, or why there is none (a timeout when the
    /// system's camera API is blocked, for example on a permission prompt).
    Cameras(Result<Value, String>),
}

/// Where the actor gets its devices from: the real system in the app, deterministic stand-ins in tests.
///
/// Plain function pointers keep it `Copy` and free of shared state.
#[derive(Debug, Clone, Copy)]
pub struct Sources {
    /// Starts the face and body tracking actors.
    pub tracking: fn() -> (TrackingHandle, BodyHandle),
    /// Lists the cameras (the `tracking_list_cameras` answer). Runs on a worker thread and may block.
    pub cameras: fn() -> Result<Value, String>,
    /// Lists the microphone names.
    pub microphones: fn() -> Vec<String>,
    /// How long a camera search may take before it is reported as failed.
    pub camera_timeout: Duration,
}

impl Sources {
    /// The real camera and microphone APIs of this machine (the camera and face models only with the `live` feature).
    #[must_use]
    pub const fn system() -> Self {
        Self { tracking: spawn_tracking, cameras: system_cameras, microphones: input_devices, camera_timeout: CAMERA_TIMEOUT }
    }

    /// No devices: the scripted demo face and one fixed microphone. Never touches the system, so it is what unit tests use.
    #[must_use]
    pub const fn demo() -> Self {
        Self { tracking: demo_tracking, cameras: demo_cameras, microphones: demo_microphones, camera_timeout: CAMERA_TIMEOUT }
    }
}

fn system_cameras() -> Result<Value, String> {
    charcraft_tracking::tools::camera_list().map_err(|e| e.to_string())
}

fn demo_tracking() -> (TrackingHandle, BodyHandle) {
    (charcraft_tracking::actor::TrackingActor::spawn_demo_only(), charcraft_tracking::body_actor::BodyActor::spawn_demo_only())
}

#[expect(clippy::unnecessary_wraps, reason = "it stands in for the system listing, which can fail")]
fn demo_cameras() -> Result<Value, String> {
    Ok(json!({ "cameras": [{ "index": 0, "name": "Demo face", "description": "a scripted talking face" }], "demo": true }))
}

fn demo_microphones() -> Vec<String> {
    vec!["Demo microphone".to_owned()]
}

/// The sending half, cheap to clone: document commands hold one.
#[derive(Debug, Clone)]
pub struct LiveLink {
    requests: Sender<LiveRequest>,
}

impl LiveLink {
    /// A link to nothing: every call fails with "no live input". For documents built without an actor.
    #[must_use]
    pub fn disconnected() -> Self {
        let (requests, _) = unbounded();
        Self { requests }
    }

    /// Runs a `tracking_*` tool.
    ///
    /// # Errors
    /// The tool's failure, or a message when the actor is gone or silent.
    pub fn tool(&self, name: &str, params: &Value) -> Result<Value, String> {
        let (reply, answer) = bounded(1);
        self.requests.send(LiveRequest::Tool { name: name.to_owned(), params: params.clone(), reply }).map_err(|_| "live input is not running".to_owned())?;
        let wait = if name == LIST_CAMERAS { CAMERA_ANSWER } else { ANSWER };
        answer.recv_timeout(wait).map_err(|_| "live input did not answer".to_owned())?
    }

    /// Starts a camera search without waiting; the outcome arrives as [`LiveEvent::Cameras`].
    pub fn search_cameras(&self) {
        let _ = self.requests.send(LiveRequest::ListCameras);
    }

    /// Runs a microphone action.
    ///
    /// # Errors
    /// A message when the actor is gone or silent.
    pub fn mic(&self, action: MicAction) -> Result<Value, String> {
        let (reply, answer) = bounded(1);
        self.requests.send(LiveRequest::Mic { action, reply }).map_err(|_| "live input is not running".to_owned())?;
        answer.recv_timeout(ANSWER).map_err(|_| "live input did not answer".to_owned())?
    }

    /// Starts or stops forwarding microphone audio to the UI.
    pub fn forward_audio(&self, on: bool) {
        let _ = self.requests.send(LiveRequest::ForwardAudio(on));
    }
}

/// The actor's owner-side handle: the link plus the event stream. Dropping it ends the actor.
#[derive(Debug)]
pub struct LiveHandle {
    link: LiveLink,
    events: Receiver<LiveEvent>,
    thread: Option<JoinHandle<()>>,
}

impl LiveHandle {
    /// Starts the actor. `wake` is called whenever events are waiting (to repaint the window).
    #[must_use]
    pub fn spawn(wake: impl Fn() + Send + 'static) -> Self {
        Self::spawn_with(wake, Sources::system())
    }

    /// Starts the actor on the given device sources.
    #[must_use]
    pub fn spawn_with(wake: impl Fn() + Send + 'static, sources: Sources) -> Self {
        let (requests, request_rx) = unbounded();
        let (event_tx, events) = bounded(EVENT_QUEUE);
        let thread =
            std::thread::Builder::new().name("charcraft-live".into()).spawn(move || Actor::new(event_tx, Box::new(wake), sources).run(&request_rx)).ok();
        Self { link: LiveLink { requests }, events, thread }
    }

    /// A link for commands.
    #[must_use]
    pub fn link(&self) -> LiveLink {
        self.link.clone()
    }

    /// The next waiting event.
    #[must_use]
    pub fn try_event(&self) -> Option<LiveEvent> {
        self.events.try_recv().ok()
    }
}

impl Drop for LiveHandle {
    fn drop(&mut self) {
        let _ = self.link.requests.send(LiveRequest::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Names and descriptions of the `tracking_*` tools, for listings.
#[must_use]
pub fn tracking_tools() -> Vec<(&'static str, &'static str, bool)> {
    tracking_tool_specs().into_iter().map(|t| (t.name, t.description, t.read_only)).collect()
}

/// The face and body tracking actors. With the `live` feature they read one shared webcam and one ONNX Runtime.
#[cfg(feature = "live")]
fn spawn_tracking() -> (TrackingHandle, BodyHandle) {
    use charcraft_tracking::actor::TrackingActor;
    use charcraft_tracking::body_actor::BodyActor;
    use charcraft_tracking::live;
    use charcraft_tracking::onnx::OnnxRuntime;
    let hub = live::camera_hub();
    match OnnxRuntime::with_default_store() {
        Ok(runtime) => (TrackingActor::spawn(live::shared_face_factory(hub.clone(), runtime.clone())), live::spawn_body(hub, runtime)),
        Err(e) => {
            let message = e.to_string();
            let again = message.clone();
            (
                TrackingActor::spawn(Box::new(move |_| Err(charcraft_tracking::Error::Inference(message.clone())))),
                BodyActor::spawn(Box::new(move |_| Err(charcraft_tracking::Error::Inference(again.clone())))),
            )
        }
    }
}

#[cfg(not(feature = "live"))]
fn spawn_tracking() -> (TrackingHandle, BodyHandle) {
    (charcraft_tracking::actor::TrackingActor::spawn_demo_only(), charcraft_tracking::body_actor::BodyActor::spawn_demo_only())
}

/// The mouth shapes the demo voice cycles through, each held for a moment.
const DEMO_CYCLE: [Viseme; 10] =
    [Viseme::Ah, Viseme::Neutral, Viseme::Oh, Viseme::Ee, Viseme::D, Viseme::Neutral, Viseme::Uh, Viseme::M, Viseme::Ah, Viseme::S];

struct Mic {
    actor: Option<CaptureActor>,
    analyser: Option<LiveLipSync>,
    format: Option<(u32, u16)>,
    running: bool,
    error: Option<String>,
    level: f32,
    viseme: Option<Viseme>,
    demo: bool,
    forward: bool,
    device: Option<String>,
    gain: f32,
}

impl Default for Mic {
    fn default() -> Self {
        Self {
            actor: None,
            analyser: None,
            format: None,
            running: false,
            error: None,
            level: 0.0,
            viseme: None,
            demo: false,
            forward: false,
            device: None,
            gain: 1.0,
        }
    }
}

impl Mic {
    fn start(&mut self) -> Result<(), String> {
        if self.actor.is_none() {
            let device = self.device.clone();
            let open = move || device.as_ref().map_or_else(CpalInput::open_default, |name| CpalInput::open_named(name));
            self.actor = Some(CaptureActor::spawn_with(open).map_err(|e| e.to_string())?);
        }
        self.error = None;
        self.actor.as_ref().ok_or_else(|| "no capture actor".to_owned())?.send(CaptureCommand::Start).map_err(|e| e.to_string())
    }

    fn stop(&mut self) {
        if let Some(actor) = &self.actor {
            let _ = actor.send(CaptureCommand::Stop);
        }
        self.running = false;
        self.level = 0.0;
        self.viseme = None;
    }

    /// Chooses the input device; a running microphone restarts on it.
    fn select(&mut self, device: Option<String>) -> Result<(), String> {
        let was_running = self.running;
        if self.device == device && self.actor.is_some() {
            return Ok(());
        }
        self.stop();
        // Dropping the actor closes the old device; the next start opens the new one.
        self.actor = None;
        self.device = device;
        if was_running { self.start() } else { Ok(()) }
    }

    /// Takes in the capture events; returns the chunks to forward.
    fn pump(&mut self) -> Vec<PcmBuffer> {
        let mut forwarded = Vec::new();
        let Some(actor) = &self.actor else { return forwarded };
        while let Ok(event) = actor.events().try_recv() {
            match event {
                CaptureEvent::Started(format) => {
                    self.running = true;
                    self.format = Some((format.sample_rate, format.channels));
                    self.analyser = Some(LiveLipSync::new(format.sample_rate));
                }
                CaptureEvent::Chunk(chunk) => {
                    let mut mono = chunk.to_mono();
                    for sample in &mut mono.samples {
                        *sample = (*sample * self.gain).clamp(-1.0, 1.0);
                    }
                    let peak = mono.samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
                    self.level = (self.level * 0.6).max((peak * 3.0).min(1.0));
                    if let Some(analyser) = self.analyser.as_mut()
                        && (analyser.push(&mono.samples).is_some() || self.viseme.is_none())
                    {
                        self.viseme = Some(analyser.current());
                    }
                    if self.forward {
                        forwarded.push(chunk);
                    }
                }
                CaptureEvent::Overrun { .. } => {}
                CaptureEvent::Stopped => {
                    self.running = false;
                    self.level = 0.0;
                }
                CaptureEvent::Error(message) => self.error = Some(message),
            }
        }
        self.level *= 0.97;
        forwarded
    }

    fn status(&self) -> Value {
        json!({
            "running": self.running,
            "sample_rate": self.format.map(|f| f.0),
            "channels": self.format.map(|f| f.1),
            "level": self.level,
            "viseme": self.viseme.map(|v| format!("{v:?}")),
            "demo_voice": self.demo,
            "error": self.error,
            "device": self.device,
            "gain": self.gain,
        })
    }
}

/// The names of the input devices the system reports.
fn input_devices() -> Vec<String> {
    anim_audio::device::list_input_devices()
}

/// A camera search in flight on a worker thread, with everyone waiting for its outcome.
struct CameraSearch {
    started: Instant,
    outcome: Receiver<Result<Value, String>>,
    replies: Vec<Sender<Result<Value, String>>>,
}

struct Actor {
    events: Sender<LiveEvent>,
    wake: Box<dyn Fn() + Send>,
    sources: Sources,
    tracking: TrackingTools,
    mic: Mic,
    cameras: Option<CameraSearch>,
    last_face: Option<Tick>,
    last_body: Option<Tick>,
    started: Instant,
    last_voice: Option<(Option<Viseme>, u32)>,
}

impl Actor {
    fn new(events: Sender<LiveEvent>, wake: Box<dyn Fn() + Send>, sources: Sources) -> Self {
        let (face, body) = (sources.tracking)();
        Self {
            events,
            wake,
            sources,
            tracking: TrackingTools::new(face).with_body(body),
            mic: Mic::default(),
            cameras: None,
            last_face: None,
            last_body: None,
            started: Instant::now(),
            last_voice: None,
        }
    }

    /// Starts a camera search on its own thread (or joins the one running). The platform's camera API may block for a
    /// long time, for example on a permission prompt, and the actor must keep answering meanwhile; [`Self::poll_cameras`]
    /// collects the outcome or gives up at the timeout.
    fn search_cameras(&mut self, reply: Option<Sender<Result<Value, String>>>) {
        if self.cameras.is_none() {
            let (sender, outcome) = bounded(1);
            let list = self.sources.cameras;
            match std::thread::Builder::new().name("charcraft-cameras".into()).spawn(move || {
                let _ = sender.send(list());
            }) {
                Ok(_) => self.cameras = Some(CameraSearch { started: Instant::now(), outcome, replies: Vec::new() }),
                Err(error) => {
                    let outcome = Err(format!("cannot look for cameras: {error}"));
                    if let Some(reply) = reply {
                        let _ = reply.send(outcome.clone());
                    }
                    self.send(LiveEvent::Cameras(outcome));
                    return;
                }
            }
        }
        if let (Some(search), Some(reply)) = (self.cameras.as_mut(), reply) {
            search.replies.push(reply);
        }
    }

    /// Reports a finished or overdue camera search.
    fn poll_cameras(&mut self) {
        let Some(search) = &self.cameras else { return };
        let outcome = match search.outcome.try_recv() {
            Ok(outcome) => outcome,
            Err(crossbeam_channel::TryRecvError::Empty) if search.started.elapsed() < self.sources.camera_timeout => return,
            Err(crossbeam_channel::TryRecvError::Empty) => Err(format!(
                "looking for cameras took longer than {:?}; if the system asked for camera access, allow it and look again",
                self.sources.camera_timeout
            )),
            Err(crossbeam_channel::TryRecvError::Disconnected) => Err("the camera search stopped without an answer".to_owned()),
        };
        if let Some(search) = self.cameras.take() {
            for reply in search.replies {
                let _ = reply.send(outcome.clone());
            }
        }
        self.send(LiveEvent::Cameras(outcome));
    }

    fn run(mut self, requests: &Receiver<LiveRequest>) {
        loop {
            match requests.recv_timeout(TICK) {
                Ok(LiveRequest::Shutdown) | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                Ok(request) => self.handle(request),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            }
            self.poll();
        }
        self.mic.stop();
        self.tracking.shutdown();
    }

    fn handle(&mut self, request: LiveRequest) {
        match request {
            LiveRequest::Tool { name, reply, .. } if name == LIST_CAMERAS => self.search_cameras(Some(reply)),
            LiveRequest::ListCameras => self.search_cameras(None),
            LiveRequest::Tool { name, params, reply } => {
                let result = self.tracking.call(&name, &params).map_err(|e| e.to_string());
                let _ = reply.send(result);
            }
            LiveRequest::Mic { action, reply } => {
                let result = match &action {
                    MicAction::Start => self.mic.start().map(|()| json!({ "starting": true })),
                    MicAction::Stop => {
                        self.mic.stop();
                        Ok(json!({ "stopped": true }))
                    }
                    MicAction::DemoVoice(on) => {
                        self.mic.demo = *on;
                        Ok(json!({ "demo_voice": on }))
                    }
                    MicAction::Status => Ok(Value::Null),
                    MicAction::Devices => Ok(json!({ "devices": (self.sources.microphones)(), "selected": self.mic.device })),
                    MicAction::SetDevice(device) => self.mic.select(device.clone()).map(|()| Value::Null),
                    MicAction::SetGain(gain) => {
                        if gain.is_finite() {
                            self.mic.gain = gain.clamp(GAIN_RANGE.0, GAIN_RANGE.1);
                            Ok(Value::Null)
                        } else {
                            Err("gain must be a finite number".to_owned())
                        }
                    }
                };
                // Give the capture thread a moment to report a start before answering with the state.
                let restarted = matches!(action, MicAction::Start | MicAction::SetDevice(_)) && self.mic.actor.is_some();
                if restarted {
                    std::thread::sleep(Duration::from_millis(150));
                    let _ = self.mic.pump();
                }
                let _ = reply.send(result.map(|v| if v.is_null() || matches!(action, MicAction::Start) { self.mic.status() } else { v }));
            }
            LiveRequest::ForwardAudio(on) => self.mic.forward = on,
            LiveRequest::Shutdown => {}
        }
    }

    fn send(&self, event: LiveEvent) {
        if self.events.try_send(event).is_ok() {
            (self.wake)();
        }
    }

    fn poll(&mut self) {
        self.poll_cameras();
        self.tracking.pump();
        if let Ok(value) = self.tracking.call("tracking_latest", &Value::Null)
            && let Some(sample) = value.get("sample").cloned().and_then(|s| serde_json::from_value::<Sample>(s).ok())
            && self.last_face != Some(sample.time)
        {
            self.last_face = Some(sample.time);
            self.send(LiveEvent::Face(sample.time, sample.input));
        }
        if let Some(preview) = self.tracking.take_preview() {
            self.send(LiveEvent::Preview(Box::new(preview)));
        }
        if let Ok(value) = self.tracking.call("tracking_body_latest", &Value::Null)
            && let Some(sample) = value.get("sample").cloned().and_then(|s| serde_json::from_value::<BodySample>(s).ok())
            && self.last_body != Some(sample.time)
        {
            self.last_body = Some(sample.time);
            self.send(LiveEvent::Body(sample.time, Box::new(sample.input)));
        }
        for chunk in self.mic.pump() {
            self.send(LiveEvent::Audio(chunk));
        }
        let (viseme, level) = if self.mic.running {
            (self.mic.viseme, self.mic.level)
        } else if self.mic.demo {
            let step = usize::try_from(self.started.elapsed().as_millis() / 140).unwrap_or(0);
            (DEMO_CYCLE.get(step.checked_rem(DEMO_CYCLE.len()).unwrap_or(0)).copied(), 0.5)
        } else {
            (None, 0.0)
        };
        let level_key = anim_core::num::round_f64_to_u32(f64::from(level) * 50.0);
        if self.last_voice != Some((viseme, level_key)) {
            self.last_voice = Some((viseme, level_key));
            self.send(LiveEvent::Voice { viseme, level });
        }
    }
}

/// Maps a tracked face to the face the behaviors read.
///
/// With `mirror` the puppet moves like the performer's reflection ([`TrackedFace::mirrored`]: sides swap and the
/// horizontal movements flip). Tracking measures in radians with y up; the behaviors take degrees with y down.
#[must_use]
pub fn behavior_face(face: &TrackedFace, mirror: bool) -> FaceInput {
    let face = if mirror { face.mirrored() } else { *face };
    let deg = |rad: f32| f64::from(rad).to_degrees();
    let v = |x: f32| f64::from(x);
    FaceInput {
        head_yaw: deg(face.head.yaw),
        head_pitch: deg(face.head.pitch),
        head_roll: deg(face.head.roll),
        head_translation: anim_core::Vec2::new(v(face.head.x), -v(face.head.y)),
        head_scale: v(face.head.scale),
        eyebrow_left: v(face.brow_l),
        eyebrow_right: v(face.brow_r),
        eye_open_left: v(face.eye_open_l),
        eye_open_right: v(face.eye_open_r),
        gaze: anim_core::Vec2::new(v(face.gaze[0]), -v(face.gaze[1])),
        mouth_open: v(face.mouth_open),
        mouth_wide: v(face.mouth_wide).clamp(0.0, 1.0),
        smile: v(face.smile).max(0.0),
    }
    .sanitized()
}

/// Maps a tracked body to the body the behaviors read.
///
/// With `mirror` the puppet moves like the performer's reflection ([`TrackedBody::mirrored`]: sides swap and the
/// horizontal positions flip). Both sides measure in body units, x towards the performer's left and y down.
#[must_use]
pub fn behavior_body(body: &TrackedBody, mirror: bool) -> BodyInput {
    let body = if mirror { body.mirrored() } else { *body };
    let point = |p: [f32; 2]| anim_core::Vec2::new(f64::from(p[0]), f64::from(p[1]));
    let joints = charcraft_tracking::body::Joint::ALL
        .iter()
        .zip(charcraft_behaviors::Joint::ALL)
        .filter_map(|(j, target)| body.joint(*j).map(|p| (target, point(p))))
        .collect();
    let rest = charcraft_behaviors::Joint::ALL.iter().zip(body.rest).map(|(j, p)| (*j, point(p))).collect();
    BodyInput { joints, rest }.sanitized()
}

#[cfg(test)]
mod tests {
    use charcraft_tracking::body::{Joint as TrackedJoint, SyntheticBody};

    use super::*;

    fn solved(body: &SyntheticBody) -> TrackedBody {
        let mut solver = charcraft_tracking::body::BodySolver::default();
        solver.smoothing.enabled = false;
        solver.solve(Tick::ZERO, Some(&body.keypoints())).input
    }

    #[test]
    fn a_tracked_body_maps_joint_for_joint_and_mirroring_swaps_sides() {
        let body = solved(&SyntheticBody { left_arm: 90.0, ..SyntheticBody::default() });
        let plain = behavior_body(&body, false);
        let wrist = plain.joint(charcraft_behaviors::Joint::LeftWrist).unwrap();
        let tracked = body.joint(TrackedJoint::LeftWrist).unwrap();
        assert!((wrist.x - f64::from(tracked[0])).abs() < 1e-6 && (wrist.y - f64::from(tracked[1])).abs() < 1e-6);
        assert_eq!(plain.rest.len(), 17);
        let mirrored = behavior_body(&body, true);
        let flipped = mirrored.joint(charcraft_behaviors::Joint::RightWrist).unwrap();
        assert!((flipped.x + wrist.x).abs() < 1e-6 && (flipped.y - wrist.y).abs() < 1e-6);
    }

    #[test]
    fn missing_joints_stay_missing() {
        let body = solved(&SyntheticBody { legs: false, ..SyntheticBody::default() });
        let mapped = behavior_body(&body, false);
        assert!(mapped.joint(charcraft_behaviors::Joint::LeftKnee).is_none() && mapped.joint(charcraft_behaviors::Joint::LeftElbow).is_some());
    }

    #[test]
    fn the_actor_streams_demo_bodies_and_answers_body_tools() {
        let handle = LiveHandle::spawn_with(|| {}, Sources::demo());
        let link = handle.link();
        assert_eq!(link.tool("tracking_body_status", &Value::Null).unwrap()["running"], false);
        link.tool("tracking_body_start", &json!({ "demo": true })).unwrap();
        let mut bodies = 0;
        for _ in 0..400 {
            while let Some(event) = handle.try_event() {
                if matches!(event, LiveEvent::Body(..)) {
                    bodies += 1;
                }
            }
            if bodies > 3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(bodies > 3, "{bodies} bodies");
        let rest = link.tool("tracking_body_set_rest_pose", &json!({ "frames": 3 })).unwrap();
        assert_eq!(rest["capturing"], 3);
    }

    #[test]
    fn a_neutral_face_maps_to_a_neutral_face() {
        let mapped = behavior_face(&TrackedFace::default(), true);
        assert!(mapped.head_yaw.abs() < 1e-9 && mapped.mouth_open == 0.0 && mapped.eye_open_left > 0.9);
    }

    #[test]
    fn mirroring_turns_lateral_movement_around() {
        let mut face = TrackedFace::default();
        face.head.yaw = 0.2;
        face.head.x = 0.5;
        let mirrored = behavior_face(&face, true);
        let plain = behavior_face(&face, false);
        assert!(mirrored.head_yaw < 0.0 && plain.head_yaw > 0.0);
        assert!(mirrored.head_translation.x < 0.0 && plain.head_translation.x > 0.0);
    }

    #[test]
    fn the_actor_keeps_the_microphone_choices() {
        let handle = LiveHandle::spawn_with(|| {}, Sources::demo());
        let link = handle.link();
        assert!(link.mic(MicAction::Devices).unwrap()["devices"].is_array());
        assert_eq!(link.mic(MicAction::SetGain(100.0)).unwrap()["gain"].as_f64(), Some(8.0), "the gain is clamped");
        assert!((link.mic(MicAction::SetGain(0.0001)).unwrap()["gain"].as_f64().unwrap() - 0.1).abs() < 1e-6);
        assert!(link.mic(MicAction::SetGain(f32::NAN)).is_err());
        let chosen = link.mic(MicAction::SetDevice(Some("No such microphone".into()))).unwrap();
        assert_eq!(chosen["device"], "No such microphone");
        assert_eq!(chosen["running"], false, "a microphone that was off stays off");
        assert!(link.mic(MicAction::SetDevice(None)).unwrap()["device"].is_null());
    }

    #[test]
    fn the_actor_lists_cameras_sets_the_rest_pose_and_streams_previews() {
        let handle = LiveHandle::spawn_with(|| {}, Sources::demo());
        let link = handle.link();
        let cameras = link.tool("tracking_list_cameras", &Value::Null).unwrap();
        assert_eq!(cameras["cameras"][0]["name"], "Demo face");
        assert_eq!(cameras["demo"], true);
        assert!(link.tool("tracking_set_rest_pose", &json!({ "target": "legs" })).is_err());
        link.tool("tracking_start", &json!({ "demo": true })).unwrap();
        link.tool("tracking_set_rest_pose", &json!({ "target": "body", "frames": 3 })).unwrap();
        let mut previews = 0;
        for _ in 0..400 {
            while let Some(event) = handle.try_event() {
                if let LiveEvent::Preview(p) = event {
                    assert!(p.points.iter().all(|q| (0.0..=1.0).contains(&q[0])));
                    previews += 1;
                }
            }
            if previews > 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(previews > 1, "{previews} previews");
        assert_eq!(link.tool("tracking_preview", &Value::Null).unwrap()["available"], true);
    }

    #[test]
    fn the_actor_answers_tools_and_streams_demo_faces() {
        let handle = LiveHandle::spawn_with(|| {}, Sources::demo());
        let link = handle.link();
        let status = link.tool("tracking_status", &Value::Null).unwrap();
        assert_eq!(status["running"], false);
        link.tool("tracking_start", &json!({ "demo": true })).unwrap();
        let mut faces = 0;
        for _ in 0..400 {
            while let Some(event) = handle.try_event() {
                if matches!(event, LiveEvent::Face(..)) {
                    faces += 1;
                }
            }
            if faces > 3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(faces > 3, "{faces} faces");
        assert!(link.tool("nope", &Value::Null).is_err());
        let voice = link.mic(MicAction::DemoVoice(true)).unwrap();
        assert_eq!(voice["demo_voice"], true);
    }

    fn blocked_cameras() -> Result<Value, String> {
        std::thread::sleep(Duration::from_secs(3));
        Ok(json!({ "cameras": [], "demo": false }))
    }

    fn next_cameras(handle: &LiveHandle) -> Result<Value, String> {
        for _ in 0..1000 {
            while let Some(event) = handle.try_event() {
                if let LiveEvent::Cameras(outcome) = event {
                    return outcome;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Err("no camera event arrived".to_owned())
    }

    #[test]
    fn a_blocked_camera_api_times_out_and_never_freezes_the_actor() {
        let sources = Sources { cameras: blocked_cameras, camera_timeout: Duration::from_millis(150), ..Sources::demo() };
        let handle = LiveHandle::spawn_with(|| {}, sources);
        let link = handle.link();
        link.search_cameras();
        // The actor keeps answering while the search is stuck.
        let started = Instant::now();
        assert_eq!(link.tool("tracking_status", &Value::Null).unwrap()["running"], false);
        assert!(link.mic(MicAction::Devices).unwrap()["devices"].is_array());
        assert!(started.elapsed() < Duration::from_secs(1), "the actor waited for the camera search");
        let outcome = next_cameras(&handle);
        let message = outcome.unwrap_err();
        assert!(message.contains("took longer"), "{message}");
        // A tool call asking for the list gets the same clear error instead of a silent timeout.
        let err = link.tool("tracking_list_cameras", &Value::Null).unwrap_err();
        assert!(err.contains("took longer"), "{err}");
    }

    #[test]
    fn a_camera_search_reports_its_list_as_an_event() {
        let handle = LiveHandle::spawn_with(|| {}, Sources::demo());
        handle.link().search_cameras();
        let list = next_cameras(&handle).unwrap();
        assert_eq!(list["cameras"][0]["index"], 0);
    }

    /// Touches the real cameras and microphones of this machine; runs only with `CHARCRAFT_DEVICE_TESTS=1`.
    #[test]
    fn the_system_devices_answer_in_time() {
        if std::env::var("CHARCRAFT_DEVICE_TESTS").as_deref() != Ok("1") {
            return;
        }
        let handle = LiveHandle::spawn_with(|| {}, Sources::system());
        let link = handle.link();
        assert!(link.mic(MicAction::Devices).unwrap()["devices"].is_array());
        // Either the list or a clear error (access denied, timeout), but always an answer from the actor.
        match link.tool("tracking_list_cameras", &Value::Null) {
            Ok(list) => assert!(list["cameras"].is_array(), "{list}"),
            Err(message) => assert_ne!(message, "live input did not answer"),
        }
        assert_eq!(link.tool("tracking_status", &Value::Null).unwrap()["running"], false);
    }
}
