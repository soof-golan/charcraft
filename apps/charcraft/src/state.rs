//! Plain data the window and the MCP commands share: the working mode and the view settings an agent can change.

use serde::{Deserialize, Serialize};

/// The three ways of working with a puppet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Look at the puppet tree, tags, handles and behaviors.
    #[default]
    Rig,
    /// Scene timeline with takes, camera and microphone panels, arm and record.
    Record,
    /// Full-window performance view.
    Stream,
}

impl Mode {
    /// All modes in tab order.
    pub const ALL: [Self; 3] = [Self::Rig, Self::Record, Self::Stream];

    /// The tab label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rig => "Rig",
            Self::Record => "Record",
            Self::Stream => "Stream",
        }
    }

    /// Parses `rig`, `record` or `stream` (any case).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.label().eq_ignore_ascii_case(text.trim()))
    }
}

/// A change to the view the window makes on request (every field optional). The MCP command `ui.set` sends this.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSet {
    /// Switch mode.
    pub mode: Option<Mode>,
    /// Select a puppet track by index.
    pub track: Option<usize>,
    /// Select a layer of the selected puppet by path.
    pub layer: Option<String>,
    /// Move the playhead, in seconds.
    pub seek_seconds: Option<f64>,
    /// Start or stop playback.
    pub playing: Option<bool>,
    /// Mirror the performer (the puppet moves like a reflection).
    pub mirror: Option<bool>,
    /// Dark (true) or light (false) theme.
    pub dark: Option<bool>,
    /// Play a synthetic talking mouth when no microphone is on.
    pub demo_voice: Option<bool>,
    /// Hold exactly these trigger keys down (the Triggers panel's buttons); an empty list lets go of all of them.
    pub triggers: Option<Vec<String>>,
    /// Choose swap set members (set name to layer label), as the Triggers panel does.
    pub swaps: Option<std::collections::BTreeMap<String, String>>,
    /// Show (true) or hide (false) the Controls, Triggers and scene panels of the stream view.
    pub stream_panels: Option<bool>,
    /// Make the stream view transparent: no backdrop in the window or in the pictures sent to outputs.
    pub transparent: Option<bool>,
    /// Show a chroma key green backdrop in the stream view.
    pub green_screen: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_parse_by_name() {
        assert_eq!(Mode::parse("Record"), Some(Mode::Record));
        assert_eq!(Mode::parse(" stream "), Some(Mode::Stream));
        assert_eq!(Mode::parse("edit"), None);
    }

    #[test]
    fn a_partial_ui_set_parses() {
        let set: UiSet = serde_json::from_str(r#"{"mode":"stream","seek_seconds":1.5}"#).unwrap();
        assert_eq!(set.mode, Some(Mode::Stream));
        assert_eq!(set.seek_seconds, Some(1.5));
        assert!(set.track.is_none());
    }
}
