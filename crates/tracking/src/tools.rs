//! A JSON tool surface over the tracking actor, for MCP and other control interfaces.
//!
//! The app's MCP layer (`anim-mcp`) turns commands and queries into tools; live input is not document state, so
//! tracking offers its own small vocabulary instead: [`tools`] describes it (name, description, JSON Schema for the
//! parameters, whether it only reads) and [`TrackingTools::call`] runs one tool with JSON parameters and returns JSON.
//! Hooking it up is three lines in the app: list [`tools`] among the tools, route the names to `call`, and put
//! [`TrackingTools::status_json`] into the status report.
//!
//! | Tool | Does |
//! |---|---|
//! | `tracking_status` | running, source, frame counts, calibration, ranges, smoothing |
//! | `tracking_latest` | the most recent face frame as [`crate::input::FaceInput`], optionally mirrored |
//! | `tracking_start` | open a camera (`index`, `width`, `height`, `fps`) |
//! | `tracking_stop` | release the camera |
//! | `tracking_calibrate` | capture the neutral face from the next `frames` frames |
//! | `tracking_reset_calibration` | back to the built-in typical face |
//! | `tracking_set_sensitivity` | change any of the output ranges |
//! | `tracking_set_smoothing` | change the smoothing |
//! | `tracking_list_cameras` | the cameras the system reports (feature `camera`) |

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use crate::actor::{CameraSpec, Command, Event, Sample, Status, TrackingHandle};
use crate::solver::{Sensitivity, Smoothing};

/// How long a tool waits for the actor to answer a status query.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);
/// Frames averaged when `tracking_calibrate` names none (about one second of video).
const DEFAULT_CALIBRATION_FRAMES: usize = 30;
/// Most frames a calibration may average.
const MAX_CALIBRATION_FRAMES: usize = 600;

/// A tool failed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolError {
    /// No tool has this name.
    #[error("unknown tool {0:?}")]
    Unknown(String),
    /// The parameters do not fit.
    #[error("invalid parameters: {0}")]
    InvalidParams(String),
    /// The tool ran and failed.
    #[error("{0}")]
    Failed(String),
}

/// One tool's description.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolSpec {
    /// Tool name.
    pub name: &'static str,
    /// What it does, for the model reading the tool list.
    pub description: &'static str,
    /// Whether it changes nothing.
    pub read_only: bool,
    /// JSON Schema of the parameter object.
    pub params: Value,
}

const fn spec(name: &'static str, description: &'static str, read_only: bool, params: Value) -> ToolSpec {
    ToolSpec { name, description, read_only, params }
}

fn object(properties: Value) -> Value {
    let fields = [("type".to_owned(), json!("object")), ("properties".to_owned(), properties), ("additionalProperties".to_owned(), json!(false))];
    Value::Object(fields.into_iter().collect())
}

/// The tools, in a stable order.
#[must_use]
pub fn tools() -> Vec<ToolSpec> {
    let all = vec![
        spec(
            "tracking_status",
            "Whether face tracking is running, with frame counts, the calibration in use, output ranges and smoothing.",
            true,
            object(json!({})),
        ),
        spec(
            "tracking_latest",
            "The most recent tracked face as puppet-ready values: head pose in radians, eyebrows -1..1, eyes 0..1, gaze -1..1, mouth open 0..1, mouth width and smile -1..1.",
            true,
            object(json!({ "mirrored": { "type": "boolean", "description": "Swap sides so a puppet moves like a mirror image of the performer." } })),
        ),
        spec(
            "tracking_start",
            "Open a camera and start tracking the face in it. Downloads the face models on first use.",
            false,
            object(json!({
                "index": { "type": "integer", "minimum": 0, "description": "Camera index (see tracking_list_cameras); 0 is the default camera." },
                "width": { "type": "integer", "minimum": 1, "description": "Requested frame width in pixels (default 640)." },
                "height": { "type": "integer", "minimum": 1, "description": "Requested frame height in pixels (default 480)." },
                "fps": { "type": "integer", "minimum": 1, "description": "Requested frames per second (default 30)." }
            })),
        ),
        spec("tracking_stop", "Stop tracking and release the camera.", false, object(json!({}))),
        spec(
            "tracking_calibrate",
            "Capture the performer's neutral face (relaxed, looking at the camera) from the next frames; every expression is then measured against it.",
            false,
            object(
                json!({ "frames": { "type": "integer", "minimum": 1, "maximum": MAX_CALIBRATION_FRAMES, "description": "Frames to average (default 30)." } }),
            ),
        ),
        spec("tracking_reset_calibration", "Forget the captured neutral face and use the built-in typical one.", false, object(json!({}))),
        spec(
            "tracking_set_sensitivity",
            "Change how far each measurement must move from neutral to reach the end of its range. Only the named fields change.",
            false,
            object(json!({
                "brow": { "type": "number", "exclusiveMinimum": 0 },
                "eye_shut": { "type": "number" },
                "eye_open": { "type": "number" },
                "mouth_open": { "type": "number", "exclusiveMinimum": 0 },
                "mouth_wide": { "type": "number", "exclusiveMinimum": 0 },
                "smile": { "type": "number", "exclusiveMinimum": 0 },
                "gaze": { "type": "number", "exclusiveMinimum": 0 }
            })),
        ),
        spec(
            "tracking_set_smoothing",
            "Switch smoothing on or off and tune its filters (cutoff in Hz, beta for speed). Only the named fields change.",
            false,
            object(json!({
                "enabled": { "type": "boolean" },
                "head": filter_schema(),
                "brows": filter_schema(),
                "eyes": filter_schema(),
                "gaze": filter_schema(),
                "mouth": filter_schema()
            })),
        ),
    ];
    #[cfg(feature = "camera")]
    let all = {
        let mut all = all;
        all.push(spec("tracking_list_cameras", "List the cameras the system reports, with the index to pass to tracking_start.", true, object(json!({}))));
        all
    };
    all
}

fn filter_schema() -> Value {
    json!({ "type": "object", "properties": { "min_cutoff": { "type": "number", "exclusiveMinimum": 0 }, "beta": { "type": "number", "minimum": 0 } }, "additionalProperties": false })
}

/// Merges `patch` over `base` field by field, objects recursively.
fn merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (key, value) in patch {
                merge(base.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (base, patch) => *base = patch.clone(),
    }
}

fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ToolError> {
    serde_json::from_value(value).map_err(|e| ToolError::InvalidParams(e.to_string()))
}

/// Runs the tools against a tracking actor, keeping the latest face and the last problem it saw.
#[derive(Debug)]
pub struct TrackingTools {
    handle: TrackingHandle,
    latest: Option<Sample>,
    last_error: Option<String>,
    calibrated: bool,
    status: Option<Status>,
}

impl TrackingTools {
    /// Tools over `handle`.
    #[must_use]
    pub const fn new(handle: TrackingHandle) -> Self {
        Self { handle, latest: None, last_error: None, calibrated: false, status: None }
    }

    /// The actor's handle, for the app's own use.
    #[must_use]
    pub const fn handle(&self) -> &TrackingHandle {
        &self.handle
    }

    fn absorb(&mut self, event: Event) {
        match event {
            Event::Face(sample) => self.latest = Some(sample),
            Event::Error(message) => self.last_error = Some(message),
            Event::Calibrated(_) => self.calibrated = true,
            Event::Status(status) => self.status = Some(*status),
            Event::Started { .. } => self.last_error = None,
            _ => {}
        }
    }

    /// Takes in the events waiting.
    pub fn pump(&mut self) {
        while let Some(event) = self.handle.try_recv() {
            self.absorb(event);
        }
    }

    /// Asks the actor for its status and waits for the answer.
    fn fresh_status(&mut self) -> Result<Status, ToolError> {
        self.pump();
        self.status = None;
        self.handle.send(Command::Query).map_err(|e| ToolError::Failed(e.to_string()))?;
        let deadline = Instant::now().checked_add(ANSWER_TIMEOUT).ok_or_else(|| ToolError::Failed("clock overflow".into()))?;
        while self.status.is_none() {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else { break };
            if let Some(event) = self.handle.recv_timeout(left) {
                self.absorb(event);
            }
        }
        self.status.take().ok_or_else(|| ToolError::Failed("the tracking actor did not answer".into()))
    }

    /// The status for an app's status report: what [`tracking_status`](tools) returns.
    ///
    /// # Errors
    /// [`ToolError::Failed`] when the actor is gone or silent.
    pub fn status_json(&mut self) -> Result<Value, ToolError> {
        let status = self.fresh_status()?;
        let mut value = serde_json::to_value(&status).map_err(|e| ToolError::Failed(e.to_string()))?;
        if let Value::Object(map) = &mut value {
            map.insert("last_error".into(), self.last_error.clone().map_or(Value::Null, Value::String));
            map.insert("calibrated_this_session".into(), Value::Bool(self.calibrated));
        }
        Ok(value)
    }

    fn send(&self, command: Command) -> Result<(), ToolError> {
        self.handle.send(command).map_err(|e| ToolError::Failed(e.to_string()))
    }

    /// Runs the tool `name` with `params` (`null` counts as an empty object).
    ///
    /// # Errors
    /// [`ToolError`].
    pub fn call(&mut self, name: &str, params: &Value) -> Result<Value, ToolError> {
        let params = if params.is_null() { json!({}) } else { params.clone() };
        match name {
            "tracking_status" => self.status_json(),
            "tracking_latest" => {
                self.pump();
                let mirrored = params.get("mirrored").and_then(Value::as_bool).unwrap_or(false);
                let sample = self.latest.map(|s| Sample { input: if mirrored { s.input.mirrored() } else { s.input }, ..s });
                Ok(json!({ "available": sample.is_some(), "mirrored": mirrored, "sample": sample }))
            }
            "tracking_start" => {
                let camera: CameraSpec = {
                    let mut base = serde_json::to_value(CameraSpec::default()).map_err(|e| ToolError::Failed(e.to_string()))?;
                    merge(&mut base, &params);
                    parse(base)?
                };
                self.send(Command::Start { camera: camera.clone() })?;
                Ok(json!({ "requested": camera }))
            }
            "tracking_stop" => {
                self.send(Command::Stop)?;
                Ok(json!({ "stopping": true }))
            }
            "tracking_calibrate" => {
                let frames = match params.get("frames") {
                    None | Some(Value::Null) => DEFAULT_CALIBRATION_FRAMES,
                    Some(v) => parse::<usize>(v.clone())?,
                };
                if !(1..=MAX_CALIBRATION_FRAMES).contains(&frames) {
                    return Err(ToolError::InvalidParams(format!("frames must be 1 to {MAX_CALIBRATION_FRAMES}")));
                }
                self.calibrated = false;
                self.send(Command::Calibrate { frames })?;
                Ok(json!({ "capturing": frames, "hint": "ask the performer to hold a relaxed neutral face; poll tracking_status for progress" }))
            }
            "tracking_reset_calibration" => {
                self.send(Command::ResetCalibration)?;
                Ok(json!({ "reset": true }))
            }
            "tracking_set_sensitivity" => {
                let current = serde_json::to_value(self.fresh_status()?.sensitivity).map_err(|e| ToolError::Failed(e.to_string()))?;
                let sensitivity: Sensitivity = self.patched(current, &params)?;
                self.send(Command::SetSensitivity(sensitivity))?;
                Ok(json!({ "sensitivity": sensitivity }))
            }
            "tracking_set_smoothing" => {
                let current = serde_json::to_value(self.fresh_status()?.smoothing).map_err(|e| ToolError::Failed(e.to_string()))?;
                let smoothing: Smoothing = self.patched(current, &params)?;
                self.send(Command::SetSmoothing(smoothing))?;
                Ok(json!({ "smoothing": smoothing }))
            }
            #[cfg(feature = "camera")]
            "tracking_list_cameras" => {
                let cameras = crate::camera::list_cameras().map_err(|e| ToolError::Failed(e.to_string()))?;
                Ok(json!({ "cameras": cameras }))
            }
            other => Err(ToolError::Unknown(other.to_owned())),
        }
    }

    #[allow(clippy::unused_self, reason = "kept a method so tools read uniformly")]
    fn patched<T: serde::de::DeserializeOwned>(&self, mut current: Value, patch: &Value) -> Result<T, ToolError> {
        merge(&mut current, patch);
        parse(current)
    }

    /// Ends the actor and waits for its thread.
    pub fn shutdown(self) {
        self.handle.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::TrackingActor;
    use crate::source::ScriptedSource;
    use crate::synthetic::demo_faces;

    fn tools_over_demo() -> TrackingTools {
        TrackingTools::new(TrackingActor::spawn_scripted(|| ScriptedSource::from_faces("demo", &demo_faces(60), 30).looping().paced(Duration::from_millis(2))))
    }

    fn wait_for_face(t: &mut TrackingTools) -> Value {
        for _ in 0..500 {
            let v = t.call("tracking_latest", &Value::Null).unwrap();
            if v["available"] == json!(true) {
                return v;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no face arrived");
    }

    #[test]
    fn every_tool_has_a_schema_and_a_unique_name() {
        let all = tools();
        let mut names: Vec<_> = all.iter().map(|t| t.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len());
        for t in &all {
            assert!(t.name.starts_with("tracking_") && !t.description.is_empty(), "{}", t.name);
            assert_eq!(t.params["type"], "object", "{}", t.name);
        }
        assert!(serde_json::to_string(&all).is_ok());
    }

    #[test]
    fn start_latest_and_status_round_trip() {
        let mut t = tools_over_demo();
        assert_eq!(t.call("tracking_latest", &Value::Null).unwrap()["available"], json!(false));
        assert_eq!(t.call("tracking_status", &Value::Null).unwrap()["running"], json!(false));
        let started = t.call("tracking_start", &json!({ "width": 320, "height": 240 })).unwrap();
        assert_eq!(started["requested"]["width"], json!(320));
        assert_eq!(started["requested"]["fps"], json!(30), "unnamed fields keep their defaults");
        let face = wait_for_face(&mut t);
        assert_eq!(face["sample"]["input"]["tracked"], json!(true));
        let status = t.call("tracking_status", &Value::Null).unwrap();
        assert_eq!(status["running"], json!(true));
        assert_eq!(status["source"], json!("demo"));
        assert!(status["frames"].as_u64().unwrap() > 0);
        t.call("tracking_stop", &Value::Null).unwrap();
        t.shutdown();
    }

    #[test]
    fn mirrored_flips_the_sides() {
        let mut t = tools_over_demo();
        t.call("tracking_start", &Value::Null).unwrap();
        wait_for_face(&mut t);
        t.call("tracking_stop", &Value::Null).unwrap();
        // Stopped: the latest sample no longer changes, so both views describe the same frame.
        std::thread::sleep(Duration::from_millis(50));
        t.pump();
        let plain = t.call("tracking_latest", &json!({ "mirrored": false })).unwrap();
        let mirrored = t.call("tracking_latest", &json!({ "mirrored": true })).unwrap();
        let (a, b) = (&plain["sample"]["input"], &mirrored["sample"]["input"]);
        assert_eq!(a["brow_l"], b["brow_r"]);
        assert_eq!(a["eye_open_r"], b["eye_open_l"]);
        assert_eq!(mirrored["mirrored"], json!(true));
        t.shutdown();
    }

    #[test]
    fn calibrate_reports_and_validates() {
        let mut t = tools_over_demo();
        assert!(matches!(t.call("tracking_calibrate", &json!({ "frames": 0 })), Err(ToolError::InvalidParams(_))));
        assert!(matches!(t.call("tracking_calibrate", &json!({ "frames": 100_000 })), Err(ToolError::InvalidParams(_))));
        assert!(matches!(t.call("tracking_calibrate", &json!({ "frames": "many" })), Err(ToolError::InvalidParams(_))));
        t.call("tracking_start", &Value::Null).unwrap();
        let r = t.call("tracking_calibrate", &json!({ "frames": 5 })).unwrap();
        assert_eq!(r["capturing"], json!(5));
        for _ in 0..500 {
            let s = t.call("tracking_status", &Value::Null).unwrap();
            if s["calibration"]["captured"] == json!(true) {
                assert_eq!(s["calibrated_this_session"], json!(true));
                t.call("tracking_reset_calibration", &Value::Null).unwrap();
                let s = t.call("tracking_status", &Value::Null).unwrap();
                assert_eq!(s["calibration"]["captured"], json!(false));
                t.shutdown();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("calibration never finished");
    }

    #[test]
    fn settings_merge_over_the_current_values() {
        let mut t = tools_over_demo();
        let r = t.call("tracking_set_sensitivity", &json!({ "brow": 0.5 })).unwrap();
        assert_eq!(r["sensitivity"]["brow"], json!(0.5));
        assert_eq!(r["sensitivity"]["smile"], json!(Sensitivity::default().smile), "other fields are kept");
        let r = t.call("tracking_set_smoothing", &json!({ "enabled": false, "eyes": { "beta": 2.0 } })).unwrap();
        assert_eq!(r["smoothing"]["enabled"], json!(false));
        assert_eq!(r["smoothing"]["eyes"]["beta"], json!(2.0));
        assert_eq!(r["smoothing"]["eyes"]["min_cutoff"], json!(Smoothing::default().eyes.min_cutoff), "nested fields merge too");
        let status = t.call("tracking_status", &Value::Null).unwrap();
        assert_eq!(status["sensitivity"]["brow"], json!(0.5));
        assert!(matches!(t.call("tracking_set_sensitivity", &json!({ "brow": "wide" })), Err(ToolError::InvalidParams(_))));
        assert!(matches!(t.call("tracking_set_sensitivity", &json!({ "nonsense": 1 })), Ok(_) | Err(ToolError::InvalidParams(_))));
        t.shutdown();
    }

    #[test]
    fn unknown_tools_and_bad_start_parameters_are_errors() {
        let mut t = tools_over_demo();
        assert_eq!(t.call("tracking_dance", &Value::Null), Err(ToolError::Unknown("tracking_dance".into())));
        assert!(matches!(t.call("tracking_start", &json!({ "width": -4 })), Err(ToolError::InvalidParams(_))));
        assert!(matches!(t.call("tracking_start", &json!({ "index": "front" })), Err(ToolError::InvalidParams(_))));
        t.shutdown();
    }

    #[test]
    fn a_failing_camera_is_visible_in_the_status() {
        let mut t = TrackingTools::new(TrackingActor::spawn(Box::new(|_| Err(crate::Error::Camera("no camera".into())))));
        t.call("tracking_start", &Value::Null).unwrap();
        for _ in 0..200 {
            let s = t.call("tracking_status", &Value::Null).unwrap();
            if s["last_error"].as_str().is_some_and(|e| e.contains("no camera")) {
                t.shutdown();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the error never showed");
    }
}
