//! Tags: how layer names turn into rigging roles.
//!
//! Character Animator reads names case- and space-insensitively (`Left Eye`, `left eye`, `+LeftEye` all mean the
//! left eye), and a leading `+` makes the group independent. The table below lists the tag ids the stock behaviors
//! define together with the artwork names they match, as seen in the behavior files of real projects. Ids that
//! start with `Charcraft.` are conveniences of this program (`Body`, `Left Arm`, ...) that Character Animator does
//! not know; they are never written to project files.

use serde::{Deserialize, Serialize};

/// Well-known tag ids.
pub mod ids {
    pub const HEAD: &str = "Adobe.Face.Head";
    pub const LEFT_EYE: &str = "Adobe.Face.LeftEye";
    pub const RIGHT_EYE: &str = "Adobe.Face.RightEye";
    pub const LEFT_EYEBROW: &str = "Adobe.Face.LeftEyebrow";
    pub const RIGHT_EYEBROW: &str = "Adobe.Face.RightEyebrow";
    pub const LEFT_PUPIL: &str = "Adobe.Face.LeftPupil";
    pub const RIGHT_PUPIL: &str = "Adobe.Face.RightPupil";
    pub const LEFT_PUPIL_SIZE: &str = "Adobe.Face.LeftPupilSize";
    pub const RIGHT_PUPIL_SIZE: &str = "Adobe.Face.RightPupilSize";
    pub const LEFT_BLINK: &str = "Adobe.Face.LeftBlink";
    pub const RIGHT_BLINK: &str = "Adobe.Face.RightBlink";
    pub const NOSE: &str = "Adobe.Face.Nose";
    /// The handle tag of a mouth group.
    pub const MOUTH: &str = "Adobe.Face.Mouth";
    /// The layer tag of the group that holds the mouth shapes.
    pub const MOUTHS_PARENT: &str = "Adobe.Face.MouthsParent";
    pub const NEUTRAL_MOUTH: &str = "Adobe.Face.NeutralMouth";
    pub const SMILE_MOUTH: &str = "Adobe.Face.SmileMouth";
    pub const SURPRISED_MOUTH: &str = "Adobe.Face.SurprisedMouth";
    pub const JAW: &str = "Adobe.FlappyChin.Jaw";
    pub const DRAGGABLE: &str = "Adobe.Dragger.Draggable";
    pub const FIXED: &str = "Adobe.HandleFixer.Fixed";
    pub const DANGLE: &str = "Adobe.RigDynamics.Dangle";
    pub const TRANSFORM: &str = "Adobe.Transform.Transform";
    /// The torso (this program's convenience tag).
    pub const BODY: &str = "Charcraft.Body";
    pub const LEFT_ARM: &str = "Charcraft.LeftArm";
    pub const RIGHT_ARM: &str = "Charcraft.RightArm";
    pub const BREATHE: &str = "Charcraft.Breathe";
    /// Prefix of the walk-cycle handle tags (`Adobe.Walk.LeftHip`, ...).
    pub const WALK_PREFIX: &str = "Adobe.Walk.";
    /// Prefix of the lip sync layer tags.
    pub const LIPSYNC_PREFIX: &str = "Adobe.LipSynch.";
}

/// What a tag attaches to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TagKind {
    /// A handle tag: it marks a handle (an independent group's origin or one of its attach points).
    Handle,
    /// A layer tag: it marks a layer or group of the artwork.
    Layer,
}

/// One entry of the automatic tag table.
#[derive(Debug, Clone, Copy)]
pub struct AutoTag {
    /// The tag id.
    pub id: &'static str,
    /// Handle or layer tag.
    pub kind: TagKind,
    /// Normalised names (see [`normalize`]) that get the tag.
    pub matches: &'static [&'static str],
}

const fn handle(id: &'static str, matches: &'static [&'static str]) -> AutoTag {
    AutoTag { id, kind: TagKind::Handle, matches }
}

const fn layer(id: &'static str, matches: &'static [&'static str]) -> AutoTag {
    AutoTag { id, kind: TagKind::Layer, matches }
}

/// Every tag a name can give. Names are stored normalised (no spaces, lower case).
pub const AUTO_TAGS: &[AutoTag] = &[
    handle(ids::HEAD, &["head"]),
    handle(ids::LEFT_EYE, &["lefteye"]),
    handle(ids::RIGHT_EYE, &["righteye"]),
    handle(ids::LEFT_EYEBROW, &["lefteyebrow"]),
    handle(ids::RIGHT_EYEBROW, &["righteyebrow"]),
    handle(ids::LEFT_PUPIL, &["leftpupil"]),
    handle(ids::RIGHT_PUPIL, &["rightpupil"]),
    handle("Adobe.Face.LeftEyelidTop", &["lefteyelidtop"]),
    handle("Adobe.Face.RightEyelidTop", &["righteyelidtop"]),
    handle("Adobe.Face.LeftEyelidBottom", &["lefteyelidbottom"]),
    handle("Adobe.Face.RightEyelidBottom", &["righteyelidbottom"]),
    handle(ids::NOSE, &["nose"]),
    handle(ids::MOUTH, &["mouth"]),
    handle(ids::JAW, &["jaw"]),
    layer(ids::LEFT_PUPIL_SIZE, &["leftpupil", "leftpupilsize"]),
    layer(ids::RIGHT_PUPIL_SIZE, &["rightpupil", "rightpupilsize"]),
    layer("Adobe.Face.LeftPupilRange", &["lefteyeball", "leftpupilrange"]),
    layer("Adobe.Face.RightPupilRange", &["righteyeball", "rightpupilrange"]),
    layer("Adobe.Face.LeftEyelidTopLayer", &["lefteyelidtop"]),
    layer("Adobe.Face.RightEyelidTopLayer", &["righteyelidtop"]),
    layer("Adobe.Face.LeftEyelidBottomLayer", &["lefteyelidbottom"]),
    layer("Adobe.Face.RightEyelidBottomLayer", &["righteyelidbottom"]),
    layer(ids::LEFT_BLINK, &["leftblink"]),
    layer(ids::RIGHT_BLINK, &["rightblink"]),
    layer(ids::MOUTHS_PARENT, &["mouth"]),
    layer(ids::NEUTRAL_MOUTH, &["neutral"]),
    layer(ids::SMILE_MOUTH, &["smile"]),
    layer(ids::SURPRISED_MOUTH, &["surprised"]),
    layer("Adobe.LipSynch.Ah", &["ah", "aa", "a"]),
    layer("Adobe.LipSynch.D", &["d"]),
    layer("Adobe.LipSynch.Ee", &["ee"]),
    layer("Adobe.LipSynch.F", &["f"]),
    layer("Adobe.LipSynch.L", &["l", "th"]),
    layer("Adobe.LipSynch.M", &["m"]),
    layer("Adobe.LipSynch.Oh", &["oh", "oh-uh"]),
    layer("Adobe.LipSynch.R", &["r"]),
    layer("Adobe.LipSynch.S", &["s", "k"]),
    layer("Adobe.LipSynch.Uh", &["uh"]),
    layer("Adobe.LipSynch.W-Oo", &["w-oo", "woo"]),
    layer("Adobe.Face.LeftProfile", &["leftprofile"]),
    layer("Adobe.Face.LeftQuarter", &["leftquarter"]),
    layer("Adobe.Face.Front", &["frontal"]),
    layer("Adobe.Face.RightQuarter", &["rightquarter"]),
    layer("Adobe.Face.RightProfile", &["rightprofile"]),
    layer("Adobe.Face.Upward", &["upward"]),
    layer("Adobe.Face.Downward", &["downward"]),
    layer("Adobe.MotionTrigger.AtRest", &["atrest"]),
    layer("Adobe.MotionTrigger.MovingLeft", &["movingleft"]),
    layer("Adobe.MotionTrigger.MovingRight", &["movingright"]),
    layer("Adobe.MotionTrigger.MovingUp", &["movingup"]),
    layer("Adobe.MotionTrigger.MovingDown", &["movingdown"]),
    layer("Adobe.MotionTrigger.InTransition", &["intransition"]),
    handle(ids::FIXED, &["fixed"]),
    handle(ids::DRAGGABLE, &["draggable", "mousetrack"]),
    handle(ids::DANGLE, &["dangle"]),
    layer("Adobe.Physics.Collide", &["collide"]),
    layer("Adobe.Physics.Dynamic", &["dynamic"]),
    handle("Adobe.Magnets.Magnet", &["magnet"]),
    handle("Adobe.MotionLines.Trails", &["trails"]),
    handle(ids::TRANSFORM, &["transform"]),
    handle("Adobe.LeaderFollower.Leader", &["leader"]),
    handle("Adobe.LeaderFollower.Follower", &["follower"]),
    handle("Adobe.Walk.RightHip", &["righthip"]),
    handle("Adobe.Walk.LeftHip", &["lefthip"]),
    handle("Adobe.Walk.RightKnee", &["rightknee"]),
    handle("Adobe.Walk.LeftKnee", &["leftknee"]),
    handle("Adobe.Walk.RightAnkle", &["rightankle"]),
    handle("Adobe.Walk.LeftAnkle", &["leftankle"]),
    handle("Adobe.Walk.RightHeel", &["rightheel"]),
    handle("Adobe.Walk.LeftHeel", &["leftheel"]),
    handle("Adobe.Walk.RightToe", &["righttoe"]),
    handle("Adobe.Walk.LeftToe", &["lefttoe"]),
    handle("Adobe.Walk.Waist", &["waist"]),
    handle("Adobe.Walk.Neck", &["neck"]),
    handle("Adobe.Walk.RightShoulder", &["rightshoulder"]),
    handle("Adobe.Walk.LeftShoulder", &["leftshoulder"]),
    handle("Adobe.Walk.RightElbow", &["rightelbow"]),
    handle("Adobe.Walk.LeftElbow", &["leftelbow"]),
    handle("Adobe.Walk.RightWrist", &["rightwrist"]),
    handle("Adobe.Walk.LeftWrist", &["leftwrist"]),
    handle(ids::BODY, &["body"]),
    handle(ids::LEFT_ARM, &["leftarm"]),
    handle(ids::RIGHT_ARM, &["rightarm"]),
    handle(ids::BREATHE, &["breathe"]),
];

/// Groups whose name matches one of these tags are independent even without a `+` (Character Animator makes the
/// face parts and the torso into parts of their own).
pub const AUTO_INDEPENDENT: &[&str] = &[
    ids::HEAD,
    ids::LEFT_EYE,
    ids::RIGHT_EYE,
    ids::LEFT_EYEBROW,
    ids::RIGHT_EYEBROW,
    ids::LEFT_PUPIL,
    ids::RIGHT_PUPIL,
    ids::MOUTH,
    ids::BODY,
    ids::LEFT_ARM,
    ids::RIGHT_ARM,
];

/// The form of a name used for matching: the leading `+` marker and all white space removed, lower case
/// (Unicode aware, so Hebrew and Arabic names pass through unchanged).
#[must_use]
pub fn normalize(name: &str) -> String {
    name.trim_start().trim_start_matches('+').chars().filter(|c| !c.is_whitespace() && *c != '_').flat_map(char::to_lowercase).collect()
}

/// `+` as first non-blank character marks an independent group.
#[must_use]
pub fn is_independent(name: &str) -> bool {
    name.trim_start().starts_with('+')
}

/// The name without the independence marker and surrounding blanks (what Character Animator shows).
#[must_use]
pub fn label(name: &str) -> &str {
    name.trim_start().trim_start_matches('+').trim()
}

/// Every table entry a name gives. Handle tags only make sense on independent groups, so callers decide which
/// kinds to apply.
pub fn auto_tags(name: &str) -> impl Iterator<Item = &'static AutoTag> {
    let key = normalize(name);
    AUTO_TAGS.iter().filter(move |t| t.matches.contains(&key.as_str()))
}

/// A mouth shape: the twelve lip sync visemes plus the two expression shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Viseme {
    Neutral,
    Ah,
    D,
    Ee,
    F,
    L,
    M,
    Oh,
    R,
    S,
    Uh,
    /// `W-Oo`.
    WOo,
    Smile,
    Surprised,
}

impl Viseme {
    /// All shapes in display order.
    pub const ALL: [Self; 14] =
        [Self::Neutral, Self::Ah, Self::D, Self::Ee, Self::F, Self::L, Self::M, Self::Oh, Self::R, Self::S, Self::Uh, Self::WOo, Self::Smile, Self::Surprised];

    /// The tag id of the layer showing this shape.
    #[must_use]
    pub const fn tag_id(self) -> &'static str {
        match self {
            Self::Neutral => ids::NEUTRAL_MOUTH,
            Self::Ah => "Adobe.LipSynch.Ah",
            Self::D => "Adobe.LipSynch.D",
            Self::Ee => "Adobe.LipSynch.Ee",
            Self::F => "Adobe.LipSynch.F",
            Self::L => "Adobe.LipSynch.L",
            Self::M => "Adobe.LipSynch.M",
            Self::Oh => "Adobe.LipSynch.Oh",
            Self::R => "Adobe.LipSynch.R",
            Self::S => "Adobe.LipSynch.S",
            Self::Uh => "Adobe.LipSynch.Uh",
            Self::WOo => "Adobe.LipSynch.W-Oo",
            Self::Smile => ids::SMILE_MOUTH,
            Self::Surprised => ids::SURPRISED_MOUTH,
        }
    }

    /// The name Character Animator shows (`W-Oo`).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Neutral => "Neutral",
            Self::Ah => "Ah",
            Self::D => "D",
            Self::Ee => "Ee",
            Self::F => "F",
            Self::L => "L",
            Self::M => "M",
            Self::Oh => "Oh",
            Self::R => "R",
            Self::S => "S",
            Self::Uh => "Uh",
            Self::WOo => "W-Oo",
            Self::Smile => "Smile",
            Self::Surprised => "Surprised",
        }
    }

    /// The shape a tag id stands for.
    #[must_use]
    pub fn from_tag(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.tag_id() == id)
    }

    /// The shape a name stands for (`"ah"`, `"W-Oo"`, `"Neutral"`), matched like the table does.
    #[must_use]
    pub fn from_label(name: &str) -> Option<Self> {
        let key = normalize(name);
        Self::ALL.into_iter().find(|v| normalize(v.label()) == key).or_else(|| auto_tags(name).find_map(|t| Self::from_tag(t.id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids_of(name: &str) -> Vec<&'static str> {
        auto_tags(name).map(|t| t.id).collect()
    }

    #[test]
    fn matching_ignores_case_spaces_and_the_plus() {
        for n in ["Left Eye", "left eye", "+LeftEye", "  + left_eye ", "LEFT EYE"] {
            assert!(ids_of(n).contains(&ids::LEFT_EYE), "{n}");
        }
        assert!(ids_of("Right Eyebrow").contains(&ids::RIGHT_EYEBROW));
        assert!(ids_of("Unrelated").is_empty());
    }

    #[test]
    fn mouth_names_give_both_kinds() {
        let tags: Vec<_> = auto_tags("Mouth").collect();
        assert!(tags.iter().any(|t| t.kind == TagKind::Handle && t.id == ids::MOUTH));
        assert!(tags.iter().any(|t| t.kind == TagKind::Layer && t.id == ids::MOUTHS_PARENT));
    }

    #[test]
    fn every_viseme_is_reachable_by_name() {
        for (name, want) in [
            ("Neutral", Viseme::Neutral),
            ("Aa", Viseme::Ah),
            ("Ah", Viseme::Ah),
            ("D", Viseme::D),
            ("ee", Viseme::Ee),
            ("F", Viseme::F),
            ("L", Viseme::L),
            ("M", Viseme::M),
            ("Oh", Viseme::Oh),
            ("R", Viseme::R),
            ("S", Viseme::S),
            ("Uh", Viseme::Uh),
            ("W-Oo", Viseme::WOo),
            ("Smile", Viseme::Smile),
            ("Surprised", Viseme::Surprised),
        ] {
            assert_eq!(Viseme::from_label(name), Some(want), "{name}");
        }
        assert_eq!(Viseme::from_label("Banana"), None);
    }

    #[test]
    fn viseme_tags_round_trip() {
        for v in Viseme::ALL {
            assert_eq!(Viseme::from_tag(v.tag_id()), Some(v));
        }
    }

    #[test]
    fn independence_and_labels() {
        assert!(is_independent("+Head"));
        assert!(is_independent("  + left hand"));
        assert!(!is_independent("Head"));
        assert_eq!(label("+ left hand"), "left hand");
        assert_eq!(label("  +Head "), "Head");
    }

    #[test]
    fn right_to_left_names_are_kept() {
        assert_eq!(normalize("+ ראש"), "ראש");
        assert!(is_independent("+שלום"));
        assert_eq!(label("+ مرحبا"), "مرحبا");
    }

    #[test]
    fn no_duplicate_ids_with_the_same_match_and_kind() {
        let mut seen = std::collections::BTreeSet::new();
        for t in AUTO_TAGS {
            assert!(seen.insert((t.id, t.kind)), "{} listed twice", t.id);
        }
    }
}
