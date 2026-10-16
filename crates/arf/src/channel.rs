//! What a channel is, from its name.
//!
//! The channel table of an event graph names every recorded signal with a path. The paths seen in recordings:
//!
//! ```text
//! Viseme/LookaheadIndex  Viseme/InputEnabled  Viseme/Index  Audio/Amplitude        Lip Sync
//! BehaviorOutput/Adobe.FaceTracker2.js/cameraInput/TIB47/Filtered/Head/Head/DX      Face (and Eye Gaze) output
//! BehaviorOutput/Adobe.Dragger3.js/Input/TIB45/HandleTarget/../B2/B8/B4:/Draggable/Translation/scene/X
//! ```
//!
//! `TIB47` is the id of the behavior instance on the track item that recorded the channel; `B2/B8/B4:` is the chain
//! of binding ids from the puppet down to the layer that carries the handle.

use std::fmt;

/// One of the twenty values the face tracker reports, after Character Animator's filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FaceParam {
    /// Head position, sideways (fraction of the camera frame, 0 at the calibrated centre).
    Dx,
    /// Head position, vertical.
    Dy,
    /// Head position, depth.
    Dz,
    /// Head turn about the horizontal axis (nod), radians.
    OrientX,
    /// Head turn about the vertical axis (shake), radians.
    OrientY,
    /// Head tilt, radians.
    OrientZ,
    /// Left eyebrow height relative to rest (1 at rest).
    LeftEyebrow,
    /// Right eyebrow height relative to rest.
    RightEyebrow,
    /// Left eye opening relative to rest (1 at rest; Face triggers its blink layer at 0.45 and below).
    LeftEyelid,
    /// Right eye opening relative to rest.
    RightEyelid,
    /// Mouth width relative to rest.
    MouthSx,
    /// Mouth height (opening) relative to rest.
    MouthSy,
    /// Mouth centre, sideways.
    MouthDx,
    /// Mouth centre, vertical.
    MouthDy,
    /// Head size relative to the calibrated distance (1 at rest).
    Scale,
    /// Which mouth the face makes: 0 neutral, 1 surprised, 2 smile.
    MouthShape,
    /// Left pupil, sideways.
    LeftEyeGazeX,
    /// Left pupil, vertical.
    LeftEyeGazeY,
    /// Right pupil, sideways.
    RightEyeGazeX,
    /// Right pupil, vertical.
    RightEyeGazeY,
}

impl FaceParam {
    /// Every parameter, in the order the recorder lists them.
    pub const ALL: [Self; 20] = [
        Self::Dx,
        Self::Dy,
        Self::Dz,
        Self::OrientX,
        Self::OrientY,
        Self::OrientZ,
        Self::LeftEyebrow,
        Self::RightEyebrow,
        Self::LeftEyelid,
        Self::RightEyelid,
        Self::MouthSx,
        Self::MouthSy,
        Self::MouthDx,
        Self::MouthDy,
        Self::Scale,
        Self::MouthShape,
        Self::LeftEyeGazeX,
        Self::LeftEyeGazeY,
        Self::RightEyeGazeX,
        Self::RightEyeGazeY,
    ];

    /// The path segment after `Head/Head/`.
    #[must_use]
    pub const fn leaf(self) -> &'static str {
        match self {
            Self::Dx => "DX",
            Self::Dy => "DY",
            Self::Dz => "DZ",
            Self::OrientX => "Orient/X",
            Self::OrientY => "Orient/Y",
            Self::OrientZ => "Orient/Z",
            Self::LeftEyebrow => "LeftEyebrow",
            Self::RightEyebrow => "RightEyebrow",
            Self::LeftEyelid => "LeftEyelid",
            Self::RightEyelid => "RightEyelid",
            Self::MouthSx => "MouthSX",
            Self::MouthSy => "MouthSY",
            Self::MouthDx => "MouthDX",
            Self::MouthDy => "MouthDY",
            Self::Scale => "Scale",
            Self::MouthShape => "MouthShape",
            Self::LeftEyeGazeX => "LeftEyeGazeX",
            Self::LeftEyeGazeY => "LeftEyeGazeY",
            Self::RightEyeGazeX => "RightEyeGazeX",
            Self::RightEyeGazeY => "RightEyeGazeY",
        }
    }

    /// The parameter named by a path segment.
    #[must_use]
    pub fn from_leaf(leaf: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.leaf() == leaf)
    }
}

/// What a handle channel measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HandleSpace {
    /// The handle's position in its layer's artwork pixels (`Translation/Layer`; equal to `mesh` in every sample).
    Layer,
    /// The same position in the layer's mesh (`Translation/mesh`).
    Mesh,
    /// In scene pixels, origin at the centre of the frame, y down (`Translation/scene`).
    Scene,
    /// In the Dragger behavior's own space (`Translation/behavior`).
    Behavior,
}

impl HandleSpace {
    const fn leaf(self) -> &'static str {
        match self {
            Self::Layer => "Layer",
            Self::Mesh => "mesh",
            Self::Scene => "scene",
            Self::Behavior => "behavior",
        }
    }

    fn from_leaf(leaf: &str) -> Option<Self> {
        [Self::Layer, Self::Mesh, Self::Scene, Self::Behavior].into_iter().find(|s| s.leaf() == leaf)
    }
}

/// An axis of a handle position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Axis {
    /// Horizontal.
    X,
    /// Vertical.
    Y,
}

/// A value of a draggable handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HandleValue {
    /// 1 while the handle is held (`Active`).
    Active,
    /// 1 while the handle has an angle (`Angle/IsSet`; 0 in every sample).
    AngleIsSet,
    /// Where the handle is.
    Translation(HandleSpace, Axis),
}

/// What a channel is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    /// `Viseme/Index`: the mouth shape the voice makes, a Lip Sync viseme number (0 Neutral, 1 Ah, 2 D, 3 Ee, 4 F,
    /// 5 L, 6 M, 7 Oh, 8 R, 9 S, 10 Uh, 11 W-Oo); negative before any speech.
    VisemeIndex,
    /// `Viseme/LookaheadIndex`: the viseme the voice is about to make (the transcript recordings hold only this).
    VisemeLookahead,
    /// `Viseme/InputEnabled`: 1 while the voice input was on.
    VisemeInputEnabled,
    /// `Audio/Amplitude`: the loudness of the voice.
    AudioAmplitude,
    /// A face tracker value.
    Face {
        /// The behavior whose output it is (`Adobe.FaceTracker2.js`, or `Adobe.EyeGaze.js` for the gaze-filtered copy).
        behavior: String,
        /// The behavior instance (`TIB47`).
        instance: String,
        /// Which value.
        param: FaceParam,
    },
    /// A value of a draggable handle.
    Handle {
        /// The behavior instance (`TIB45`).
        instance: String,
        /// The handle's address, `HandleTarget/../B2/B8/B4:/Draggable/` (the take's `_takeGroupID`).
        group: String,
        /// Which value.
        value: HandleValue,
    },
    /// Anything else, kept by name.
    Other,
}

impl Role {
    /// Reads a channel name.
    #[must_use]
    pub fn parse(name: &str) -> Self {
        match name {
            "Viseme/Index" => return Self::VisemeIndex,
            "Viseme/LookaheadIndex" => return Self::VisemeLookahead,
            "Viseme/InputEnabled" => return Self::VisemeInputEnabled,
            "Audio/Amplitude" => return Self::AudioAmplitude,
            _ => {}
        }
        let Some(rest) = name.strip_prefix("BehaviorOutput/") else { return Self::Other };
        let Some((behavior, rest)) = rest.split_once('/') else { return Self::Other };
        if let Some(rest) = rest.strip_prefix("cameraInput/") {
            let Some((instance, rest)) = rest.split_once('/') else { return Self::Other };
            let Some(leaf) = rest.strip_prefix("Filtered/Head/Head/") else { return Self::Other };
            return FaceParam::from_leaf(leaf).map_or(Self::Other, |param| Self::Face { behavior: behavior.to_owned(), instance: instance.to_owned(), param });
        }
        if let Some(rest) = rest.strip_prefix("Input/") {
            let Some((instance, rest)) = rest.split_once('/') else { return Self::Other };
            let Some(at) = rest.rfind("Draggable/") else { return Self::Other };
            let (group, value) = rest.split_at(at.saturating_add("Draggable/".len()));
            let value = match value {
                "Active" => HandleValue::Active,
                "Angle/IsSet" => HandleValue::AngleIsSet,
                other => {
                    let Some(path) = other.strip_prefix("Translation/") else { return Self::Other };
                    let Some((space, axis)) = path.split_once('/') else { return Self::Other };
                    let axis = match axis {
                        "X" => Axis::X,
                        "Y" => Axis::Y,
                        _ => return Self::Other,
                    };
                    let Some(space) = HandleSpace::from_leaf(space) else { return Self::Other };
                    HandleValue::Translation(space, axis)
                }
            };
            return Self::Handle { instance: instance.to_owned(), group: group.to_owned(), value };
        }
        Self::Other
    }
}

/// The channel name of a face value.
#[must_use]
pub fn face_channel(behavior: &str, instance: &str, param: FaceParam) -> String {
    format!("BehaviorOutput/{behavior}/cameraInput/{instance}/Filtered/Head/Head/{}", param.leaf())
}

/// The channel name of a handle value.
#[must_use]
pub fn handle_channel(instance: &str, group: &str, value: HandleValue) -> String {
    let tail = match value {
        HandleValue::Active => "Active".to_owned(),
        HandleValue::AngleIsSet => "Angle/IsSet".to_owned(),
        HandleValue::Translation(space, axis) => format!("Translation/{}/{}", space.leaf(), if axis == Axis::X { 'X' } else { 'Y' }),
    };
    format!("BehaviorOutput/Adobe.Dragger3.js/Input/{instance}/{group}{tail}")
}

impl fmt::Display for FaceParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.leaf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_parse_and_print_back() {
        for param in FaceParam::ALL {
            let name = face_channel("Adobe.FaceTracker2.js", "TIB47", param);
            assert_eq!(Role::parse(&name), Role::Face { behavior: "Adobe.FaceTracker2.js".into(), instance: "TIB47".into(), param });
        }
        let group = "HandleTarget/../B2/B8/B4:/Draggable/";
        for value in [
            HandleValue::Active,
            HandleValue::AngleIsSet,
            HandleValue::Translation(HandleSpace::Scene, Axis::X),
            HandleValue::Translation(HandleSpace::Layer, Axis::Y),
            HandleValue::Translation(HandleSpace::Mesh, Axis::X),
            HandleValue::Translation(HandleSpace::Behavior, Axis::Y),
        ] {
            let name = handle_channel("TIB45", group, value);
            assert_eq!(Role::parse(&name), Role::Handle { instance: "TIB45".into(), group: group.into(), value });
        }
        assert_eq!(Role::parse("Viseme/Index"), Role::VisemeIndex);
        assert_eq!(Role::parse("Audio/Amplitude"), Role::AudioAmplitude);
        assert_eq!(Role::parse("BehaviorOutput/Adobe.Whatever.js/Other/x"), Role::Other);
        assert_eq!(Role::parse(""), Role::Other);
        assert_eq!(Role::parse("BehaviorOutput/Adobe.Dragger3.js/Input/TIB45/nothing"), Role::Other);
    }
}
