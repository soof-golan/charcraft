//! The body tracker's share of the tracking tool surface (see [`crate::tools`]).
//!
//! | Tool | Does |
//! |---|---|
//! | `tracking_body_status` | running, source, frame counts, rest pose, smoothing, mirror |
//! | `tracking_body_latest` | the most recent body: 17 joints in body units against the rest pose |
//! | `tracking_body_start` | open a camera and start tracking the body (`index`, `width`, `height`, `fps`) |
//! | `tracking_body_stop` | stop body tracking |
//! | `tracking_body_set_rest_pose` | capture the rest pose from the next `frames` frames: stand as the puppet stands, arms down |
//! | `tracking_reset_rest_pose` | back to the built-in typical body |
//! | `tracking_body_set_smoothing` | switch smoothing on or off and tune its filter |
//! | `tracking_body_mirror` | swap the performer's left and right |

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::actor::CameraSpec;
use crate::body::BodySmoothing;
use crate::body_actor::{BodyCommand, BodyEvent, BodyHandle, BodySample, BodyStatus};
use crate::tools::{ToolError, ToolSpec, merge, object, parse, spec};

/// How long a tool waits for the actor to answer a status query.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);
/// Frames averaged when `tracking_body_set_rest_pose` names none (about one second of video).
const DEFAULT_REST_FRAMES: usize = 30;
/// Most frames a rest-pose capture may average.
const MAX_REST_FRAMES: usize = 600;

const NAMES: [&str; 8] = [
    "tracking_body_status",
    "tracking_body_latest",
    "tracking_body_start",
    "tracking_body_stop",
    "tracking_body_set_rest_pose",
    "tracking_reset_rest_pose",
    "tracking_body_set_smoothing",
    "tracking_body_mirror",
];

/// Whether `name` is one of the body tools.
#[must_use]
pub fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

/// The body tools' descriptions.
#[must_use]
pub fn specs() -> Vec<ToolSpec> {
    vec![
        spec(
            "tracking_body_status",
            "Whether body tracking is running, with frame counts, the rest pose in use, smoothing and the mirror switch.",
            true,
            object(json!({})),
        ),
        spec(
            "tracking_body_latest",
            "The most recent tracked body: 17 joints (nose, eyes, ears, shoulders, elbows, wrists, hips, knees, ankles) in body units (the rest pose's torso length; x to the performer's left, y down) with the rest pose they are measured against. Joints the camera cannot see are null.",
            true,
            object(json!({})),
        ),
        spec(
            "tracking_body_start",
            "Open a camera and start tracking the body in it. Downloads the pose model on first use. The camera is shared with face tracking.",
            false,
            object(json!({
                "index": { "type": "integer", "minimum": 0, "description": "Camera index; 0 is the default camera." },
                "width": { "type": "integer", "minimum": 1, "description": "Requested frame width in pixels (default 640)." },
                "height": { "type": "integer", "minimum": 1, "description": "Requested frame height in pixels (default 480)." },
                "fps": { "type": "integer", "minimum": 1, "description": "Requested frames per second (default 30)." }
            })),
        ),
        spec("tracking_body_stop", "Stop body tracking (the camera is released once face tracking has stopped too).", false, object(json!({}))),
        spec(
            "tracking_body_set_rest_pose",
            "Capture the performer's rest pose (standing or sitting as the puppet does, arms hanging, shoulders in view) from the next frames; every movement is then measured against it.",
            false,
            object(json!({ "frames": { "type": "integer", "minimum": 1, "maximum": MAX_REST_FRAMES, "description": "Frames to average (default 30)." } })),
        ),
        spec("tracking_reset_rest_pose", "Forget the captured rest pose and use the built-in typical body.", false, object(json!({}))),
        spec(
            "tracking_body_set_smoothing",
            "Switch body smoothing on or off and tune its One-Euro filter (cutoff in Hz, beta for speed). Only the named fields change.",
            false,
            object(json!({
                "enabled": { "type": "boolean" },
                "filter": { "type": "object", "properties": { "min_cutoff": { "type": "number", "exclusiveMinimum": 0 }, "beta": { "type": "number", "minimum": 0 } }, "additionalProperties": false }
            })),
        ),
        spec(
            "tracking_body_mirror",
            "Swap the performer's left and right so the puppet moves like their reflection.",
            false,
            object(json!({ "on": { "type": "boolean", "description": "Mirror (default true)." } })),
        ),
    ]
}

/// Runs the body tools against a body actor, keeping the latest body and the last problem it saw.
#[derive(Debug)]
pub struct BodyTools {
    handle: BodyHandle,
    latest: Option<BodySample>,
    last_error: Option<String>,
    rest_set: bool,
    status: Option<BodyStatus>,
}

impl BodyTools {
    /// Tools over `handle`.
    #[must_use]
    pub const fn new(handle: BodyHandle) -> Self {
        Self { handle, latest: None, last_error: None, rest_set: false, status: None }
    }

    /// The actor's handle.
    #[must_use]
    pub const fn handle(&self) -> &BodyHandle {
        &self.handle
    }

    /// The most recent body.
    #[must_use]
    pub const fn latest(&self) -> Option<&BodySample> {
        self.latest.as_ref()
    }

    fn absorb(&mut self, event: BodyEvent) {
        match event {
            BodyEvent::Body(sample) => self.latest = Some(*sample),
            BodyEvent::Error(message) => self.last_error = Some(message),
            BodyEvent::RestPoseSet(_) => self.rest_set = true,
            BodyEvent::Status(status) => self.status = Some(*status),
            BodyEvent::Started { .. } => self.last_error = None,
            _ => {}
        }
    }

    /// Takes in the events waiting.
    pub fn pump(&mut self) {
        while let Some(event) = self.handle.try_recv() {
            self.absorb(event);
        }
    }

    fn fresh_status(&mut self) -> Result<BodyStatus, ToolError> {
        self.pump();
        self.status = None;
        self.handle.send(BodyCommand::Query).map_err(|e| ToolError::Failed(e.to_string()))?;
        let deadline = Instant::now().checked_add(ANSWER_TIMEOUT).ok_or_else(|| ToolError::Failed("clock overflow".into()))?;
        while self.status.is_none() {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else { break };
            if let Some(event) = self.handle.recv_timeout(left) {
                self.absorb(event);
            }
        }
        self.status.take().ok_or_else(|| ToolError::Failed("the body tracking actor did not answer".into()))
    }

    /// The status for an app's status report.
    ///
    /// # Errors
    /// [`ToolError::Failed`] when the actor is gone or silent.
    pub fn status_json(&mut self) -> Result<Value, ToolError> {
        let status = self.fresh_status()?;
        let mut value = serde_json::to_value(&status).map_err(|e| ToolError::Failed(e.to_string()))?;
        if let Value::Object(map) = &mut value {
            map.insert("last_error".into(), self.last_error.clone().map_or(Value::Null, Value::String));
            map.insert("rest_pose_set_this_session".into(), Value::Bool(self.rest_set));
        }
        Ok(value)
    }

    fn send(&self, command: BodyCommand) -> Result<(), ToolError> {
        self.handle.send(command).map_err(|e| ToolError::Failed(e.to_string()))
    }

    /// Runs the tool `name` with `params`.
    ///
    /// # Errors
    /// [`ToolError`].
    pub fn call(&mut self, name: &str, params: &Value) -> Result<Value, ToolError> {
        let params = &if params.is_null() { json!({}) } else { params.clone() };
        match name {
            "tracking_body_status" => self.status_json(),
            "tracking_body_latest" => {
                self.pump();
                Ok(json!({ "available": self.latest.is_some(), "sample": self.latest }))
            }
            "tracking_body_start" => {
                let camera: CameraSpec = {
                    let mut base = serde_json::to_value(CameraSpec::default()).map_err(|e| ToolError::Failed(e.to_string()))?;
                    merge(&mut base, params);
                    parse(base)?
                };
                self.send(BodyCommand::Start { camera: camera.clone() })?;
                Ok(json!({ "requested": camera }))
            }
            "tracking_body_stop" => {
                self.send(BodyCommand::Stop)?;
                Ok(json!({ "stopping": true }))
            }
            "tracking_body_set_rest_pose" => {
                let frames = match params.get("frames") {
                    None | Some(Value::Null) => DEFAULT_REST_FRAMES,
                    Some(v) => parse::<usize>(v.clone())?,
                };
                if !(1..=MAX_REST_FRAMES).contains(&frames) {
                    return Err(ToolError::InvalidParams(format!("frames must be 1 to {MAX_REST_FRAMES}")));
                }
                self.rest_set = false;
                self.send(BodyCommand::SetRestPose { frames })?;
                Ok(
                    json!({ "capturing": frames, "hint": "ask the performer to stand or sit as the puppet does, arms hanging, shoulders in view; poll tracking_body_status for progress" }),
                )
            }
            "tracking_reset_rest_pose" => {
                self.send(BodyCommand::ResetRestPose)?;
                Ok(json!({ "reset": true }))
            }
            "tracking_body_set_smoothing" => {
                let mut current = serde_json::to_value(self.fresh_status()?.smoothing).map_err(|e| ToolError::Failed(e.to_string()))?;
                merge(&mut current, params);
                let smoothing: BodySmoothing = parse(current)?;
                self.send(BodyCommand::SetSmoothing(smoothing))?;
                Ok(json!({ "smoothing": smoothing }))
            }
            "tracking_body_mirror" => {
                let on = params.get("on").and_then(Value::as_bool).unwrap_or(true);
                self.send(BodyCommand::SetMirror(on))?;
                Ok(json!({ "mirror": on }))
            }
            other => Err(ToolError::Unknown(other.to_owned())),
        }
    }

    /// Ends the actor and waits for its thread.
    pub fn shutdown(self) {
        self.handle.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{Joint, ScriptedBodySource, demo_bodies};
    use crate::body_actor::BodyActor;

    fn tools() -> BodyTools {
        BodyTools::new(BodyActor::spawn_scripted(|| ScriptedBodySource::from_bodies("demo", &demo_bodies(60), 30).looping().paced(Duration::from_millis(2))))
    }

    fn wait_for_body(t: &mut BodyTools) -> Value {
        for _ in 0..500 {
            let v = t.call("tracking_body_latest", &Value::Null).unwrap();
            if v["available"] == json!(true) {
                return v;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no body arrived");
    }

    #[test]
    fn the_tools_are_listed_with_schemas_and_every_name_is_handled() {
        let all = specs();
        assert_eq!(all.len(), NAMES.len());
        for s in &all {
            assert!(handles(s.name) && !s.description.is_empty(), "{}", s.name);
            assert_eq!(s.params["type"], "object");
        }
        assert!(!handles("tracking_start"));
        assert!(crate::tools::tools().iter().any(|t| t.name == "tracking_body_set_rest_pose"));
    }

    #[test]
    fn start_latest_and_status_round_trip() {
        let mut t = tools();
        assert_eq!(t.call("tracking_body_latest", &Value::Null).unwrap()["available"], json!(false));
        assert_eq!(t.call("tracking_body_status", &Value::Null).unwrap()["running"], json!(false));
        t.call("tracking_body_start", &json!({ "width": 320 })).unwrap();
        let body = wait_for_body(&mut t);
        assert_eq!(body["sample"]["input"]["tracked"], json!(true));
        assert!(body["sample"]["input"]["joints"][Joint::LeftWrist.index()].is_array());
        let status = t.call("tracking_body_status", &Value::Null).unwrap();
        assert_eq!(status["running"], json!(true));
        assert_eq!(status["source"], json!("demo"));
        t.call("tracking_body_stop", &Value::Null).unwrap();
        t.shutdown();
    }

    #[test]
    fn the_rest_pose_is_captured_validated_and_reset() {
        let mut t = tools();
        assert!(matches!(t.call("tracking_body_set_rest_pose", &json!({ "frames": 0 })), Err(ToolError::InvalidParams(_))));
        assert!(matches!(t.call("tracking_body_set_rest_pose", &json!({ "frames": 1_000_000 })), Err(ToolError::InvalidParams(_))));
        t.call("tracking_body_start", &Value::Null).unwrap();
        assert_eq!(t.call("tracking_body_set_rest_pose", &json!({ "frames": 5 })).unwrap()["capturing"], json!(5));
        for _ in 0..500 {
            let s = t.call("tracking_body_status", &Value::Null).unwrap();
            if s["calibration"]["captured"] == json!(true) {
                assert_eq!(s["rest_pose_set_this_session"], json!(true));
                t.call("tracking_reset_rest_pose", &Value::Null).unwrap();
                let s = t.call("tracking_body_status", &Value::Null).unwrap();
                assert_eq!(s["calibration"]["captured"], json!(false));
                t.shutdown();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the rest pose was never captured");
    }

    #[test]
    fn smoothing_merges_and_mirror_switches() {
        let mut t = tools();
        let r = t.call("tracking_body_set_smoothing", &json!({ "filter": { "beta": 2.0 } })).unwrap();
        assert_eq!(r["smoothing"]["filter"]["beta"], json!(2.0));
        assert_eq!(r["smoothing"]["filter"]["min_cutoff"], json!(BodySmoothing::default().filter.min_cutoff));
        assert!(matches!(t.call("tracking_body_set_smoothing", &json!({ "enabled": "yes" })), Err(ToolError::InvalidParams(_))));
        t.call("tracking_body_mirror", &json!({})).unwrap();
        assert_eq!(t.call("tracking_body_status", &Value::Null).unwrap()["mirror"], json!(true));
        t.call("tracking_body_mirror", &json!({ "on": false })).unwrap();
        assert_eq!(t.call("tracking_body_status", &Value::Null).unwrap()["mirror"], json!(false));
        assert_eq!(t.call("tracking_body_dance", &Value::Null), Err(ToolError::Unknown("tracking_body_dance".into())));
        t.shutdown();
    }

    #[test]
    fn the_face_tools_route_body_calls_and_refuse_without_a_body_tracker() {
        use crate::actor::TrackingActor;
        use crate::source::ScriptedSource;
        let face = || TrackingActor::spawn_scripted(|| ScriptedSource::demo(30, 30));
        let mut without = crate::tools::TrackingTools::new(face());
        assert!(matches!(without.call("tracking_body_status", &Value::Null), Err(ToolError::Failed(_))));
        without.shutdown();
        let body = BodyActor::spawn_scripted(|| ScriptedBodySource::demo(30, 30));
        let mut with = crate::tools::TrackingTools::new(face()).with_body(body);
        assert_eq!(with.call("tracking_body_status", &Value::Null).unwrap()["running"], json!(false));
        assert!(with.body().is_some());
        with.shutdown();
    }
}
