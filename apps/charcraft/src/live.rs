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
use charcraft_behaviors::FaceInput;
use charcraft_tracking::actor::{Preview, Sample, TrackingHandle};
use charcraft_tracking::input::FaceInput as TrackedFace;
use charcraft_tracking::tools::{TrackingTools, tools as tracking_tool_specs};
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use serde_json::{Value, json};

/// How long a caller waits for the actor to answer a request.
const ANSWER: Duration = Duration::from_secs(5);
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
    /// End the actor.
    Shutdown,
}

/// What the actor reports.
#[derive(Debug, Clone)]
pub enum LiveEvent {
    /// A new tracked face (`tracked` tells whether a face was found).
    Face(Tick, TrackedFace),
    /// The mouth shape the voice makes, and the input level 0 to 1.
    Voice { viseme: Option<Viseme>, level: f32 },
    /// A microphone chunk (only while forwarding).
    Audio(PcmBuffer),
    /// The camera image and tracked points.
    Preview(Box<Preview>),
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
        answer.recv_timeout(ANSWER).map_err(|_| "live input did not answer".to_owned())?
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
        let (requests, request_rx) = unbounded();
        let (event_tx, events) = bounded(EVENT_QUEUE);
        let thread = std::thread::Builder::new().name("charcraft-live".into()).spawn(move || Actor::new(event_tx, Box::new(wake)).run(&request_rx)).ok();
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

#[cfg(feature = "live")]
fn spawn_tracking() -> TrackingHandle {
    charcraft_tracking::live::spawn()
}

#[cfg(not(feature = "live"))]
fn spawn_tracking() -> TrackingHandle {
    charcraft_tracking::actor::TrackingActor::spawn_demo_only()
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

struct Actor {
    events: Sender<LiveEvent>,
    wake: Box<dyn Fn() + Send>,
    tracking: TrackingTools,
    mic: Mic,
    last_face: Option<Tick>,
    started: Instant,
    last_voice: Option<(Option<Viseme>, u32)>,
}

impl Actor {
    fn new(events: Sender<LiveEvent>, wake: Box<dyn Fn() + Send>) -> Self {
        Self { events, wake, tracking: TrackingTools::new(spawn_tracking()), mic: Mic::default(), last_face: None, started: Instant::now(), last_voice: None }
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
                    MicAction::Devices => Ok(json!({ "devices": input_devices(), "selected": self.mic.device })),
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let handle = LiveHandle::spawn(|| {});
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
        let handle = LiveHandle::spawn(|| {});
        let link = handle.link();
        let cameras = link.tool("tracking_list_cameras", &Value::Null).unwrap();
        assert!(cameras["cameras"].as_array().is_some_and(|c| !c.is_empty()) || cfg!(feature = "live"));
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
        let handle = LiveHandle::spawn(|| {});
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
}
