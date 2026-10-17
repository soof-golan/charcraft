//! MIDI input for `CharCraft`.
//!
//! A [`MidiHandle`] owns one actor thread that lists the MIDI input devices, connects to one and turns the raw messages
//! it sends into [`MidiMessage`]s: notes and controllers (CC), the two things Character Animator maps. The app binds
//! notes to triggers (in learn mode the next note is the one) and controllers to Controls sliders ([`CcBinding`]).
//!
//! The device layer sits behind the [`Backend`] trait. `midir` (ALSA, `CoreMIDI`, `WinMM`) implements it on Linux,
//! macOS and Windows; other systems get a backend that lists no devices. Tests and the MCP command `midi_send` push
//! raw messages through [`MidiHandle::inject`], so everything downstream of the device is testable without hardware.

mod actor;
mod backend;

use serde::{Deserialize, Serialize};

pub use actor::{Event, MidiHandle, Request};
pub use backend::{Backend, Connection, MockBackend, Sink, system_backend};

/// One MIDI message the app cares about. Channels are 1 to 16, as musicians count them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MidiMessage {
    /// A key went down. A note-on with velocity 0 is a [`MidiMessage::NoteOff`].
    NoteOn {
        /// 1 to 16.
        channel: u8,
        /// 0 to 127.
        note: u8,
        /// 1 to 127.
        velocity: u8,
    },
    /// A key came up.
    NoteOff {
        /// 1 to 16.
        channel: u8,
        /// 0 to 127.
        note: u8,
    },
    /// A knob, fader or pedal moved.
    ControlChange {
        /// 1 to 16.
        channel: u8,
        /// The controller number, 0 to 127.
        controller: u8,
        /// 0 to 127.
        value: u8,
    },
}

impl MidiMessage {
    /// Parses a raw message. Only note on, note off and control change are understood; anything else (clock, program
    /// change, pitch bend, system exclusive, truncated bytes) is `None`.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let status = *bytes.first()?;
        let channel = (status & 0x0f).checked_add(1)?;
        let data1 = *bytes.get(1)?;
        let data2 = *bytes.get(2)?;
        if data1 > 127 || data2 > 127 {
            return None;
        }
        match status & 0xf0 {
            0x80 => Some(Self::NoteOff { channel, note: data1 }),
            0x90 if data2 == 0 => Some(Self::NoteOff { channel, note: data1 }),
            0x90 => Some(Self::NoteOn { channel, note: data1, velocity: data2 }),
            0xb0 => Some(Self::ControlChange { channel, controller: data1, value: data2 }),
            _ => None,
        }
    }

    /// The raw three bytes of the message.
    #[must_use]
    pub fn to_bytes(self) -> [u8; 3] {
        let low = |channel: u8| channel.saturating_sub(1).min(15);
        match self {
            Self::NoteOn { channel, note, velocity } => [0x90 | low(channel), note.min(127), velocity.min(127)],
            Self::NoteOff { channel, note } => [0x80 | low(channel), note.min(127), 0],
            Self::ControlChange { channel, controller, value } => [0xb0 | low(channel), controller.min(127), value.min(127)],
        }
    }
}

/// The name of a note the way musicians write it (middle C is C4, note 60).
#[must_use]
pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let name = NAMES.get(usize::from(note % 12)).copied().unwrap_or("?");
    let octave = i16::from(note / 12).saturating_sub(1);
    format!("{name}{octave}")
}

/// A controller (CC) bound to something, on one channel or on any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CcBinding {
    /// The channel (1 to 16), or `None` for any.
    #[serde(default)]
    pub channel: Option<u8>,
    /// The controller number.
    pub controller: u8,
}

impl CcBinding {
    /// The value (0 to 127) this binding takes from a message, if the message is its controller.
    #[must_use]
    pub fn value_of(&self, message: &MidiMessage) -> Option<u8> {
        match *message {
            MidiMessage::ControlChange { channel, controller, value } if controller == self.controller && self.channel.is_none_or(|c| c == channel) => {
                Some(value)
            }
            _ => None,
        }
    }

    /// A short text for the UI, `CC 74` or `ch 2 CC 74`.
    #[must_use]
    pub fn label(&self) -> String {
        self.channel.map_or_else(|| format!("CC {}", self.controller), |c| format!("ch {c} CC {}", self.controller))
    }
}

/// Maps a controller value (0 to 127) onto `min..=max`, linearly, or exponentially when `log` (for gains).
#[must_use]
pub fn scale_cc(value: u8, min: f64, max: f64, log: bool) -> f64 {
    let t = f64::from(value.min(127)) / 127.0;
    if log && min > 0.0 && max > 0.0 { min * (max / min).powf(t) } else { (max - min).mul_add(t, min) }
}

/// The controller value (0 to 127) that puts a slider at `value` on `min..=max`; the inverse of [`scale_cc`].
#[must_use]
pub fn cc_of(value: f64, min: f64, max: f64, log: bool) -> u8 {
    let t = if log && min > 0.0 && max > 0.0 && value > 0.0 { (value / min).log(max / min) } else { (value - min) / (max - min) };
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    u8::try_from(anim_core::num::round_f64_to_u32(t * 127.0)).unwrap_or(127)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_and_controllers_parse() {
        assert_eq!(MidiMessage::parse(&[0x92, 60, 100]), Some(MidiMessage::NoteOn { channel: 3, note: 60, velocity: 100 }));
        assert_eq!(MidiMessage::parse(&[0x82, 60, 40]), Some(MidiMessage::NoteOff { channel: 3, note: 60 }));
        assert_eq!(MidiMessage::parse(&[0x90, 61, 0]), Some(MidiMessage::NoteOff { channel: 1, note: 61 }), "velocity 0 is a note off");
        assert_eq!(MidiMessage::parse(&[0xbf, 74, 127]), Some(MidiMessage::ControlChange { channel: 16, controller: 74, value: 127 }));
    }

    #[test]
    fn everything_else_is_ignored_and_nothing_panics() {
        assert_eq!(MidiMessage::parse(&[]), None);
        assert_eq!(MidiMessage::parse(&[0x90]), None);
        assert_eq!(MidiMessage::parse(&[0x90, 60]), None, "truncated");
        assert_eq!(MidiMessage::parse(&[0xf8]), None, "clock");
        assert_eq!(MidiMessage::parse(&[0xe0, 0, 64]), None, "pitch bend");
        assert_eq!(MidiMessage::parse(&[0x90, 200, 10]), None, "data bytes have no high bit");
        for status in 0..=255_u8 {
            let _ = MidiMessage::parse(&[status, 1, 2]);
        }
    }

    #[test]
    fn bytes_round_trip() {
        for m in [
            MidiMessage::NoteOn { channel: 1, note: 0, velocity: 1 },
            MidiMessage::NoteOff { channel: 16, note: 127 },
            MidiMessage::ControlChange { channel: 9, controller: 7, value: 64 },
        ] {
            assert_eq!(MidiMessage::parse(&m.to_bytes()), Some(m));
        }
    }

    #[test]
    fn notes_have_names() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(69), "A4");
        assert_eq!(note_name(0), "C-1");
        assert_eq!(note_name(127), "G9");
    }

    #[test]
    fn bindings_filter_by_channel_and_controller() {
        let any = CcBinding { channel: None, controller: 74 };
        let two = CcBinding { channel: Some(2), controller: 74 };
        let m = MidiMessage::ControlChange { channel: 1, controller: 74, value: 99 };
        assert_eq!(any.value_of(&m), Some(99));
        assert_eq!(two.value_of(&m), None);
        assert_eq!(any.value_of(&MidiMessage::NoteOn { channel: 1, note: 74, velocity: 1 }), None);
        assert_eq!(two.label(), "ch 2 CC 74");
    }

    #[test]
    fn scaling_is_linear_or_logarithmic_and_invertible() {
        assert!((scale_cc(0, 0.1, 8.0, true) - 0.1).abs() < 1e-9);
        assert!((scale_cc(127, 0.1, 8.0, true) - 8.0).abs() < 1e-9);
        assert!((scale_cc(64, 0.0, 1.0, false) - 64.0 / 127.0).abs() < 1e-9);
        for v in [0, 1, 40, 64, 100, 127] {
            assert_eq!(cc_of(scale_cc(v, 0.1, 8.0, true), 0.1, 8.0, true), v);
            assert_eq!(cc_of(scale_cc(v, 0.0, 2.0, false), 0.0, 2.0, false), v);
        }
        assert_eq!(cc_of(f64::NAN, 0.0, 1.0, false), 0);
    }
}
