//! MIDI in the app: the device, learn mode, note triggers and the Controls sliders a controller drives.
//!
//! [`MidiBank`] lives in the hooks and is the only owner of the MIDI actor. Commands and the window ask for things by
//! sending a [`MidiOp`] over [`crate::session::Links`]; the hooks hand the ops to the bank on their next tick and apply
//! the [`Effect`]s that come back (a rig edit for a learned note, a changed slider). Held notes are trigger tokens that
//! the window adds to the keys the performer holds, so a note shows a layer exactly like its key does.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use anim_session::{CommandError, CommandSpec, param, param_opt};
use charcraft_midi::{CcBinding, Event, MidiHandle, MidiMessage, cc_of, note_name, scale_cc};
use charcraft_puppet::MidiTrigger;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::live::MicAction;
use crate::session::{AppDoc, Links};

/// How many received messages the monitor keeps.
const HISTORY: usize = 8;
/// Longest control id or device name accepted.
const MAX_TEXT: usize = 256;

/// A slider of the Controls panel that a MIDI controller can move.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlDef {
    /// The id commands use.
    pub id: &'static str,
    /// The label on the slider.
    pub label: &'static str,
    /// Lowest value.
    pub min: f64,
    /// Highest value.
    pub max: f64,
    /// Whether the slider is logarithmic (gains).
    pub log: bool,
    /// The value at start.
    pub default: f64,
    /// What the slider does.
    pub hint: &'static str,
}

/// The Controls sliders.
pub const CONTROLS: [ControlDef; 3] = [
    ControlDef {
        id: "mic_gain",
        label: "Mic sensitivity",
        min: 0.1,
        max: 8.0,
        log: true,
        default: 1.0,
        hint: "Raise it for a quiet voice. The mouth shapes and the level meter follow it.",
    },
    ControlDef {
        id: "mouth_open",
        label: "Mouth opening",
        min: 0.1,
        max: 1.0,
        log: false,
        default: 0.45,
        hint: "How far your lips must part to open the puppet's mouth fully: lower is more sensitive.",
    },
    ControlDef {
        id: "smile",
        label: "Smile",
        min: 0.04,
        max: 0.4,
        log: false,
        default: 0.12,
        hint: "How far your mouth corners must lift for a full smile: lower is more sensitive.",
    },
];

/// The slider with this id.
#[must_use]
pub fn control(id: &str) -> Option<&'static ControlDef> {
    CONTROLS.iter().find(|c| c.id == id)
}

/// What the next message from the device is learned as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Learn {
    /// The next note binds to this layer's trigger.
    Trigger {
        /// The puppet's cast key.
        puppet: String,
        /// The layer path.
        layer: String,
    },
    /// The next controller binds to this slider.
    Control {
        /// The control id.
        control: String,
    },
}

/// A request to the bank.
#[derive(Debug, Clone, PartialEq)]
pub enum MidiOp {
    /// Look for devices again.
    Refresh,
    /// Open a device, or close the open one.
    Connect(Option<String>),
    /// Handle raw bytes as if the device sent them.
    Send(Vec<u8>),
    /// Start or cancel learn mode.
    Learn(Option<Learn>),
    /// Bind a controller to a slider.
    Bind {
        /// The control id.
        control: String,
        /// The controller.
        binding: CcBinding,
    },
    /// Forget a slider's controller.
    Unbind(String),
    /// A slider has this value now (set by the window or a command).
    Value {
        /// The control id.
        control: String,
        /// The value.
        value: f64,
    },
}

/// What the hooks must do after the bank handled something.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Give a layer a MIDI trigger (a rig edit).
    BindTrigger {
        /// The puppet's cast key.
        puppet: String,
        /// The layer path.
        layer: String,
        /// The note.
        trigger: MidiTrigger,
    },
    /// Move a slider: apply the value to the microphone or the tracker.
    Control {
        /// The control id.
        control: String,
        /// The new value.
        value: f64,
    },
}

/// The settings that survive restarts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MidiPrefs {
    /// The controller each slider follows.
    pub bindings: BTreeMap<String, CcBinding>,
    /// The device that was open when the app closed.
    pub device: Option<String>,
}

/// What the window and the `midi_state` query show.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MidiSnapshot {
    /// Devices that are plugged in.
    pub ports: Vec<String>,
    /// The open device.
    pub connected: Option<String>,
    /// Learn mode, when on.
    pub learn: Option<Learn>,
    /// The newest messages, newest first, as text.
    pub history: Vec<String>,
    /// The trigger tokens of the notes held now.
    pub held: Vec<String>,
    /// The controller each slider follows, as text.
    pub bindings: BTreeMap<String, String>,
    /// The slider values the bank knows.
    pub controls: BTreeMap<String, f64>,
    /// The last problem.
    pub error: Option<String>,
}

/// The MIDI state and its actor.
pub struct MidiBank {
    handle: MidiHandle,
    ports: Vec<String>,
    connected: Option<String>,
    learn: Option<Learn>,
    history: VecDeque<String>,
    held: BTreeSet<String>,
    bindings: BTreeMap<String, CcBinding>,
    values: BTreeMap<String, f64>,
    error: Option<String>,
    wanted_device: Option<String>,
    dirty: bool,
}

impl std::fmt::Debug for MidiBank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidiBank").field("connected", &self.connected).field("learn", &self.learn).finish_non_exhaustive()
    }
}

/// The text of a message for the monitor.
#[must_use]
pub fn describe(message: &MidiMessage) -> String {
    match *message {
        MidiMessage::NoteOn { channel, note, velocity } => format!("Note on {} ch {channel} vel {velocity}", note_name(note)),
        MidiMessage::NoteOff { channel, note } => format!("Note off {} ch {channel}", note_name(note)),
        MidiMessage::ControlChange { channel, controller, value } => format!("CC {controller} = {value} ch {channel}"),
    }
}

/// The trigger tokens a note holds: one for its channel, one for any channel.
fn tokens(channel: u8, note: u8) -> [String; 2] {
    [MidiTrigger::token_for(Some(channel), note), MidiTrigger::token_for(None, note)]
}

impl MidiBank {
    /// A bank around a running actor.
    #[must_use]
    pub fn new(handle: MidiHandle) -> Self {
        let values = CONTROLS.iter().map(|c| (c.id.to_owned(), c.default)).collect();
        Self {
            handle,
            ports: Vec::new(),
            connected: None,
            learn: None,
            history: VecDeque::new(),
            held: BTreeSet::new(),
            bindings: BTreeMap::new(),
            values,
            error: None,
            wanted_device: None,
            dirty: false,
        }
    }

    /// Takes the saved settings; the saved device opens as soon as it shows up in the device list.
    pub fn restore(&mut self, prefs: &MidiPrefs) {
        self.bindings.clone_from(&prefs.bindings);
        self.wanted_device.clone_from(&prefs.device);
        self.handle.list();
    }

    /// The settings to save, when they changed since the last call.
    pub fn take_prefs(&mut self) -> Option<MidiPrefs> {
        std::mem::take(&mut self.dirty)
            .then(|| MidiPrefs { bindings: self.bindings.clone(), device: self.connected.clone().or_else(|| self.wanted_device.clone()) })
    }

    /// The trigger tokens of the notes held now.
    pub fn held(&self) -> impl Iterator<Item = &String> {
        self.held.iter()
    }

    /// A copy of the state for the window and the query.
    #[must_use]
    pub fn snapshot(&self) -> MidiSnapshot {
        MidiSnapshot {
            ports: self.ports.clone(),
            connected: self.connected.clone(),
            learn: self.learn.clone(),
            history: self.history.iter().cloned().collect(),
            held: self.held.iter().cloned().collect(),
            bindings: self.bindings.iter().map(|(k, b)| (k.clone(), b.label())).collect(),
            controls: self.values.clone(),
            error: self.error.clone(),
        }
    }

    /// Handles the requests and whatever the actor reported; returns what the hooks must do.
    pub fn pump(&mut self, ops: impl IntoIterator<Item = MidiOp>) -> Vec<Effect> {
        let mut effects = Vec::new();
        for op in ops {
            match op {
                MidiOp::Refresh => self.handle.list(),
                MidiOp::Connect(name) => {
                    self.wanted_device.clone_from(&name);
                    self.handle.connect(name);
                }
                MidiOp::Send(bytes) => self.handle.inject(bytes),
                MidiOp::Learn(learn) => self.learn = learn,
                MidiOp::Bind { control, binding } => {
                    self.bindings.insert(control, binding);
                    self.dirty = true;
                }
                MidiOp::Unbind(control) => {
                    self.dirty |= self.bindings.remove(&control).is_some();
                }
                MidiOp::Value { control, value } => {
                    self.values.insert(control, value);
                }
            }
        }
        while let Some(event) = self.handle.try_event() {
            match event {
                Event::Ports(ports) => {
                    self.ports = ports;
                    if self.connected.is_none()
                        && let Some(wanted) = self.wanted_device.clone().filter(|w| self.ports.contains(w))
                    {
                        self.handle.connect(Some(wanted));
                    }
                }
                Event::Connected(name) => {
                    if name != self.connected {
                        self.dirty = true;
                    }
                    self.held.clear();
                    self.connected = name;
                }
                Event::Failed(reason) => self.error = Some(reason),
                Event::Message(message) => {
                    self.error = None;
                    self.history.push_front(describe(&message));
                    self.history.truncate(HISTORY);
                    self.message(message, &mut effects);
                }
            }
        }
        effects
    }

    fn message(&mut self, message: MidiMessage, effects: &mut Vec<Effect>) {
        match message {
            MidiMessage::NoteOn { channel, note, .. } => {
                if let Some(Learn::Trigger { puppet, layer }) = self.learn.clone() {
                    self.learn = None;
                    effects.push(Effect::BindTrigger { puppet, layer, trigger: MidiTrigger { channel: Some(channel), note } });
                    return;
                }
                self.held.extend(tokens(channel, note));
            }
            MidiMessage::NoteOff { channel, note } => {
                for token in tokens(channel, note) {
                    self.held.remove(&token);
                }
            }
            MidiMessage::ControlChange { channel, controller, .. } => {
                if let Some(Learn::Control { control }) = self.learn.clone() {
                    self.learn = None;
                    self.bindings.insert(control, CcBinding { channel: Some(channel), controller });
                    self.dirty = true;
                    return;
                }
                for (id, binding) in &self.bindings {
                    let (Some(value), Some(def)) = (binding.value_of(&message), control(id)) else { continue };
                    let value = scale_cc(value, def.min, def.max, def.log);
                    self.values.insert(id.clone(), value);
                    effects.push(Effect::Control { control: id.clone(), value });
                }
            }
        }
    }
}

/// Applies a slider value to the microphone or the tracker.
///
/// # Errors
/// A sentence when the control is unknown, the value is out of range, or the actor refuses.
pub fn apply_control(links: &Links, id: &str, value: f64) -> Result<f64, String> {
    let def = control(id).ok_or_else(|| format!("no control {id:?} (controls: {})", CONTROLS.map(|c| c.id).join(", ")))?;
    if !value.is_finite() || value < def.min || value > def.max {
        return Err(format!("{} is {} to {}", def.id, def.min, def.max));
    }
    match id {
        "mic_gain" => links.live.mic(MicAction::SetGain(anim_core::num::f64_to_f32(value))).map(|_| ()),
        other => links.live.tool("tracking_set_sensitivity", &json!({ other: value })).map(|_| ()),
    }?;
    Ok(value)
}

// --- commands ----------------------------------------------------------------------------------------------------------

fn send(doc: &AppDoc, op: MidiOp) -> Result<(), CommandError> {
    doc.links.midi.send(op).map_err(|_| CommandError::failed("MIDI is not running"))
}

fn text(what: &str, value: String) -> Result<String, CommandError> {
    if value.trim().is_empty() || value.chars().count() > MAX_TEXT || value.chars().any(char::is_control) {
        return Err(CommandError::invalid_params(format!("{what} must be 1 to {MAX_TEXT} characters without control characters")));
    }
    Ok(value)
}

fn midi_refresh(doc: &mut AppDoc, _: &Value) -> Result<Value, CommandError> {
    send(doc, MidiOp::Refresh)?;
    Ok(json!({ "requested": "device list; read it with the midi_state query" }))
}

fn midi_connect(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let device: Option<String> = param_opt(params, "device")?;
    let device = device.map(|d| text("device", d)).transpose()?;
    send(doc, MidiOp::Connect(device.clone()))?;
    Ok(json!({ "device": device }))
}

fn midi_send(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let message: MidiMessage = serde_json::from_value(params.clone()).map_err(|e| CommandError::invalid_params(e.to_string()))?;
    let ok = match message {
        MidiMessage::NoteOn { channel, note, velocity } => (1..=16).contains(&channel) && note < 128 && (1..128).contains(&velocity),
        MidiMessage::NoteOff { channel, note } => (1..=16).contains(&channel) && note < 128,
        MidiMessage::ControlChange { channel, controller, value } => (1..=16).contains(&channel) && controller < 128 && value < 128,
    };
    if !ok {
        return Err(CommandError::invalid_params("channel is 1 to 16, notes, controllers and values are 0 to 127 (velocity 1 to 127)"));
    }
    send(doc, MidiOp::Send(message.to_bytes().to_vec()))?;
    Ok(json!({ "sent": describe(&message) }))
}

fn midi_learn(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let layer: Option<String> = param_opt(params, "layer")?;
    let control_id: Option<String> = param_opt(params, "control")?;
    let learn = match (layer, control_id) {
        (Some(layer), None) => {
            let puppet: Option<String> = param_opt(params, "puppet")?;
            let puppet = match puppet {
                Some(p) => p,
                None => doc.rig.skeletons.keys().next().cloned().ok_or_else(|| CommandError::failed("the project has no puppet"))?,
            };
            let skeleton = doc.rig.skeletons.get(&puppet).ok_or_else(|| CommandError::invalid_params(format!("no puppet {puppet:?}")))?;
            if !skeleton.layers.iter().any(|l| l.path == layer) {
                return Err(CommandError::invalid_params(format!("no layer {layer:?} in {puppet:?} (rig_layers lists them)")));
            }
            Some(Learn::Trigger { puppet, layer })
        }
        (None, Some(id)) => {
            control(&id).ok_or_else(|| CommandError::invalid_params(format!("no control {id:?}")))?;
            Some(Learn::Control { control: id })
        }
        (None, None) => None,
        (Some(_), Some(_)) => return Err(CommandError::invalid_params("learn a layer or a control, not both")),
    };
    send(doc, MidiOp::Learn(learn.clone()))?;
    Ok(json!({ "learning": learn }))
}

fn midi_bind(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let id: String = param(params, "control")?;
    control(&id).ok_or_else(|| CommandError::invalid_params(format!("no control {id:?}")))?;
    let controller: Option<u8> = param_opt(params, "controller")?;
    let channel: Option<u8> = param_opt(params, "channel")?;
    let Some(controller) = controller else {
        send(doc, MidiOp::Unbind(id.clone()))?;
        return Ok(json!({ "control": id, "binding": null }));
    };
    if controller > 127 || channel.is_some_and(|c| !(1..=16).contains(&c)) {
        return Err(CommandError::invalid_params("controllers are 0 to 127, channels 1 to 16"));
    }
    let binding = CcBinding { channel, controller };
    send(doc, MidiOp::Bind { control: id.clone(), binding })?;
    Ok(json!({ "control": id, "binding": binding.label() }))
}

fn controls_set(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let id: String = param(params, "control")?;
    let value: f64 = param(params, "value")?;
    let value = apply_control(&doc.links, &id, value).map_err(CommandError::invalid_params)?;
    send(doc, MidiOp::Value { control: id.clone(), value })?;
    Ok(json!({ "control": id, "value": value, "controller_value": control(&id).map(|c| cc_of(value, c.min, c.max, c.log)) }))
}

/// The MIDI commands.
#[must_use]
pub fn commands() -> Vec<CommandSpec<AppDoc>> {
    vec![
        CommandSpec::new("midi.refresh", "Look for MIDI devices", midi_refresh)
            .transient()
            .describe("Ask for the MIDI input devices that are plugged in. The list arrives in the midi_state query (ports).")
            .with_schema(r#"{"type":"object","properties":{},"additionalProperties":false}"#),
        CommandSpec::new("midi.connect", "Connect a MIDI device", midi_connect)
            .transient()
            .describe("Open a MIDI input device by name (see midi_state ports), or close the open one with no device. The device is opened again at the next start.")
            .with_schema(r#"{"type":"object","properties":{"device":{"type":["string","null"]}},"additionalProperties":false}"#),
        CommandSpec::new("midi.send", "Send a MIDI message", midi_send)
            .transient()
            .describe("Feed a MIDI message in as if the open device had sent it. 'type' is note_on (channel 1-16, note 0-127, velocity 1-127), note_off (channel, note) or control_change (channel, controller 0-127, value 0-127). Held notes show the layers that have that note as a trigger; controllers move the sliders they are bound to; in learn mode the message is learned.")
            .with_schema(r#"{"type":"object","required":["type"],"properties":{"type":{"enum":["note_on","note_off","control_change"]},"channel":{"type":"integer","minimum":1,"maximum":16},"note":{"type":"integer","minimum":0,"maximum":127},"velocity":{"type":"integer","minimum":1,"maximum":127},"controller":{"type":"integer","minimum":0,"maximum":127},"value":{"type":"integer","minimum":0,"maximum":127}},"additionalProperties":false}"#),
        CommandSpec::new("midi.learn", "MIDI learn", midi_learn)
            .transient()
            .describe("Start learn mode: the next note binds to the trigger of 'layer' (a layer path of 'puppet', default the first), or the next controller binds to the slider 'control' (mic_gain, mouth_open, smile). With neither, learn mode ends.")
            .with_schema(r#"{"type":"object","properties":{"puppet":{"type":"string"},"layer":{"type":"string"},"control":{"enum":["mic_gain","mouth_open","smile"]}},"additionalProperties":false}"#),
        CommandSpec::new("midi.bind_cc", "Bind a controller to a slider", midi_bind)
            .transient()
            .describe("Make a MIDI controller move a Controls slider (mic_gain, mouth_open, smile). Without 'controller' the binding is removed. Without 'channel' any channel works.")
            .with_schema(r#"{"type":"object","required":["control"],"properties":{"control":{"enum":["mic_gain","mouth_open","smile"]},"controller":{"type":"integer","minimum":0,"maximum":127},"channel":{"type":"integer","minimum":1,"maximum":16}},"additionalProperties":false}"#),
        CommandSpec::new("controls.set", "Set a Controls slider", controls_set)
            .transient()
            .describe("Move a slider of the Controls panel: mic_gain (0.1 to 8), mouth_open (0.1 to 1), smile (0.04 to 0.4). The same thing a MIDI controller bound to it does.")
            .with_schema(r#"{"type":"object","required":["control","value"],"properties":{"control":{"enum":["mic_gain","mouth_open","smile"]},"value":{"type":"number"}},"additionalProperties":false}"#),
    ]
}

#[cfg(test)]
mod tests {
    use charcraft_midi::MockBackend;

    use super::*;

    fn bank() -> MidiBank {
        MidiBank::new(MidiHandle::spawn(Box::new(MockBackend::new(&["Keys 49"])), || {}))
    }

    /// Pumps until `done` holds or a second passes.
    fn until(bank: &mut MidiBank, mut ops: Vec<MidiOp>, done: impl Fn(&MidiBank) -> bool) -> Vec<Effect> {
        let mut all = Vec::new();
        let start = std::time::Instant::now();
        loop {
            all.extend(bank.pump(std::mem::take(&mut ops)));
            if done(bank) || start.elapsed() > std::time::Duration::from_secs(2) {
                return all;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn note_on(channel: u8, note: u8) -> MidiOp {
        MidiOp::Send(MidiMessage::NoteOn { channel, note, velocity: 100 }.to_bytes().to_vec())
    }

    #[test]
    fn devices_are_listed_and_connected() {
        let mut b = bank();
        until(&mut b, vec![MidiOp::Refresh], |b| !b.ports.is_empty());
        assert_eq!(b.snapshot().ports, ["Keys 49"]);
        until(&mut b, vec![MidiOp::Connect(Some("Keys 49".into()))], |b| b.connected.is_some());
        assert_eq!(b.snapshot().connected.as_deref(), Some("Keys 49"));
        until(&mut b, vec![MidiOp::Connect(Some("Nope".into()))], |b| b.error.is_some());
        assert!(b.snapshot().error.is_some_and(|e| e.contains("Nope")));
    }

    #[test]
    fn held_notes_are_trigger_tokens_until_released() {
        let mut b = bank();
        until(&mut b, vec![note_on(2, 60)], |b| !b.held.is_empty());
        assert_eq!(b.held().cloned().collect::<Vec<_>>(), ["midi:2:60", "midi:60"]);
        until(&mut b, vec![MidiOp::Send(MidiMessage::NoteOff { channel: 2, note: 60 }.to_bytes().to_vec())], |b| b.held.is_empty());
        assert!(b.held.is_empty());
        assert!(b.snapshot().history[0].starts_with("Note off C4"));
    }

    #[test]
    fn learn_binds_the_next_note_to_the_layer_and_holds_nothing() {
        let mut b = bank();
        let learn = Learn::Trigger { puppet: "demo".into(), layer: "Head/Hat".into() };
        let effects = until(&mut b, vec![MidiOp::Learn(Some(learn)), note_on(3, 64)], |b| b.learn.is_none());
        assert_eq!(effects, [Effect::BindTrigger { puppet: "demo".into(), layer: "Head/Hat".into(), trigger: MidiTrigger { channel: Some(3), note: 64 } }]);
        assert!(b.held.is_empty(), "the learned note is not also a performance");
    }

    #[test]
    fn learn_binds_a_controller_and_the_controller_then_drives_the_slider() {
        let mut b = bank();
        let cc = |value: u8| MidiOp::Send(MidiMessage::ControlChange { channel: 1, controller: 74, value }.to_bytes().to_vec());
        until(&mut b, vec![MidiOp::Learn(Some(Learn::Control { control: "mouth_open".into() })), cc(10)], |b| b.learn.is_none());
        assert_eq!(b.snapshot().bindings["mouth_open"], "ch 1 CC 74");
        let prefs = b.take_prefs().expect("changed");
        assert_eq!(prefs.bindings.len(), 1);
        assert!(b.take_prefs().is_none(), "saved once");
        let effects = until(&mut b, vec![cc(127)], |b| b.history.len() >= 2);
        assert_eq!(effects, [Effect::Control { control: "mouth_open".into(), value: 1.0 }]);
        assert!((b.snapshot().controls["mouth_open"] - 1.0).abs() < 1e-9);
        let effects = until(&mut b, vec![MidiOp::Unbind("mouth_open".into()), cc(0)], |b| b.history.len() >= 3);
        assert!(effects.is_empty());
    }

    #[test]
    fn controls_validate_their_values() {
        let (ui, _rx) = crossbeam_channel::unbounded();
        let links = Links::new(crate::live::LiveLink::disconnected(), ui);
        assert!(apply_control(&links, "nope", 1.0).is_err());
        assert!(apply_control(&links, "mic_gain", 100.0).is_err());
        assert!(apply_control(&links, "mic_gain", f64::NAN).is_err());
    }
}
