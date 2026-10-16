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
    /// A group whose children the Cycle Layers behavior shows one after the other.
    pub const CYCLE: &str = "Adobe.CycleLayers.Cycle";
    /// Head Turner views.
    pub const LEFT_PROFILE: &str = "Adobe.Face.LeftProfile";
    pub const LEFT_QUARTER: &str = "Adobe.Face.LeftQuarter";
    pub const FRONT: &str = "Adobe.Face.Front";
    pub const RIGHT_QUARTER: &str = "Adobe.Face.RightQuarter";
    pub const RIGHT_PROFILE: &str = "Adobe.Face.RightProfile";
    pub const UPWARD: &str = "Adobe.Face.Upward";
    pub const DOWNWARD: &str = "Adobe.Face.Downward";
    /// Motion Trigger states.
    pub const AT_REST: &str = "Adobe.MotionTrigger.AtRest";
    pub const MOVING_LEFT: &str = "Adobe.MotionTrigger.MovingLeft";
    pub const MOVING_RIGHT: &str = "Adobe.MotionTrigger.MovingRight";
    pub const MOVING_UP: &str = "Adobe.MotionTrigger.MovingUp";
    pub const MOVING_DOWN: &str = "Adobe.MotionTrigger.MovingDown";
    pub const IN_TRANSITION: &str = "Adobe.MotionTrigger.InTransition";
    /// Layers that dangling handles bounce off.
    pub const COLLIDE: &str = "Adobe.Physics.Collide";
    pub const DRAGGABLE: &str = "Adobe.Dragger.Draggable";
    pub const FIXED: &str = "Adobe.HandleFixer.Fixed";
    pub const DANGLE: &str = "Adobe.RigDynamics.Dangle";
    pub const TRANSFORM: &str = "Adobe.Transform.Transform";
    /// The torso (this program's convenience tag).
    pub const BODY: &str = "Charcraft.Body";
    pub const LEFT_ARM: &str = "Charcraft.LeftArm";
    pub const RIGHT_ARM: &str = "Charcraft.RightArm";
    pub const BREATHE: &str = "Charcraft.Breathe";
    pub const LEFT_CHEEK: &str = "Adobe.Face.LeftCheek";
    pub const RIGHT_CHEEK: &str = "Adobe.Face.RightCheek";
    pub const LEFT_EAR: &str = "Charcraft.LeftEar";
    pub const RIGHT_EAR: &str = "Charcraft.RightEar";
    pub const HAIR: &str = "Charcraft.Hair";
    /// The artwork the Particles behavior throws, and where it throws it from (this program's convenience tag).
    pub const PARTICLE: &str = "Charcraft.Particle";
    /// The artwork the Shadow behavior casts on the floor (this program's convenience tag).
    pub const SHADOW: &str = "Charcraft.Shadow";
    /// Groups the Wiggler behavior shakes (this program's convenience tag).
    pub const WIGGLE: &str = "Charcraft.Wiggle";
    /// Layers the Fader behavior fades (this program's convenience tag).
    pub const FADER: &str = "Charcraft.Fader";
    /// The handle tag of a magnet group.
    pub const MAGNET: &str = "Adobe.Magnets.Magnet";
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
    handle(ids::LEFT_EYEBROW, &["lefteyebrow", "leftbrow"]),
    handle(ids::RIGHT_EYEBROW, &["righteyebrow", "rightbrow"]),
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
    layer("Adobe.Face.LeftProfile", &["leftprofile", "profileleft"]),
    layer("Adobe.Face.LeftQuarter", &["leftquarter", "leftthreequarter", "left34"]),
    layer("Adobe.Face.Front", &["frontal", "front", "frontview"]),
    layer("Adobe.Face.RightQuarter", &["rightquarter", "rightthreequarter", "right34"]),
    layer("Adobe.Face.RightProfile", &["rightprofile"]),
    layer("Adobe.Face.Upward", &["upward", "up", "lookup", "lookingup"]),
    layer("Adobe.Face.Downward", &["downward", "down", "lookdown", "lookingdown"]),
    layer(ids::CYCLE, &["cyclelayers", "cycle"]),
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
    handle(ids::MAGNET, &["magnet"]),
    layer(ids::PARTICLE, &["particle", "particles", "emitter"]),
    layer(ids::SHADOW, &["shadow"]),
    layer(ids::FADER, &["fader"]),
    handle(ids::WIGGLE, &["wiggle", "wiggler"]),
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
    handle(ids::LEFT_ARM, &["leftarm", "armleft"]),
    handle(ids::RIGHT_ARM, &["rightarm", "armright"]),
    handle(ids::BREATHE, &["breathe"]),
    layer(ids::LEFT_CHEEK, &["leftcheek"]),
    layer(ids::RIGHT_CHEEK, &["rightcheek"]),
    layer("Charcraft.Cheeks", &["cheek", "cheeks", "blush"]),
    layer(ids::LEFT_EAR, &["leftear"]),
    layer(ids::RIGHT_EAR, &["rightear"]),
    layer("Charcraft.Ears", &["ear", "ears"]),
    layer(ids::HAIR, &["hair", "hairback", "hairfront", "fringe", "bangs", "ponytail", "bun", "hairdo"]),
    layer("Charcraft.LeftEyelash", &["lefteyelash", "lefteyelashes", "leftlash", "leftlashes"]),
    layer("Charcraft.RightEyelash", &["righteyelash", "righteyelashes", "rightlash", "rightlashes"]),
    layer("Charcraft.Eyelashes", &["eyelash", "eyelashes", "lash", "lashes"]),
    layer("Charcraft.LeftMouthCorner", &["leftmouthcorner", "leftlipcorner", "leftsmile"]),
    layer("Charcraft.RightMouthCorner", &["rightmouthcorner", "rightlipcorner", "rightsmile"]),
    layer("Charcraft.Teeth", &["teeth", "tooth"]),
    layer("Charcraft.UpperTeeth", &["upperteeth", "topteeth"]),
    layer("Charcraft.LowerTeeth", &["lowerteeth", "bottomteeth"]),
    layer("Charcraft.Tongue", &["tongue"]),
    layer("Charcraft.Lips", &["lips", "lip"]),
    layer("Charcraft.Torso", &["torso", "chest", "trunk"]),
    layer("Charcraft.LeftLeg", &["leftleg"]),
    layer("Charcraft.RightLeg", &["rightleg"]),
    layer("Charcraft.Legs", &["leg", "legs"]),
    layer("Charcraft.LeftHand", &["lefthand"]),
    layer("Charcraft.RightHand", &["righthand"]),
    layer("Charcraft.LeftFoot", &["leftfoot"]),
    layer("Charcraft.RightFoot", &["rightfoot"]),
    layer("Charcraft.Neck", &["neck"]),
    layer("Charcraft.Shoulders", &["shoulder", "shoulders"]),
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

/// Words that name a side. A lone `l` or `r` is a mouth shape, so single letters only count next to other words.
const SIDE_WORDS: [(&str, &str); 4] = [("left", "left"), ("l", "left"), ("right", "right"), ("r", "right")];

/// The words of a name: split at blanks, `_`, `-` and camelCase humps, lower case, digits-only words dropped.
fn words(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for c in name.trim_start().trim_start_matches('+').chars() {
        if c.is_whitespace() || matches!(c, '_' | '-' | '.' | '/') {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            previous_lower = false;
            continue;
        }
        if c.is_uppercase() && previous_lower && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        previous_lower = c.is_lowercase();
        current.extend(c.to_lowercase());
    }
    if !current.is_empty() {
        out.push(current);
    }
    out.retain(|w| !w.chars().all(|c| c.is_ascii_digit()));
    out
}

/// The key a loosely written name stands for: `Eye Left`, `eye_L` and `LeftEyes` all give `lefteye`; a side word
/// anywhere moves to the front and a plural loses its `s`. `None` when the name has no side word and no plural, so
/// it has no other reading than the plain one.
fn guessed_key(name: &str) -> Option<String> {
    let mut parts = words(name);
    if parts.len() < 2 && parts.iter().all(|w| w.chars().count() <= 3) {
        return None;
    }
    let at = if parts.len() > 1 { parts.iter().position(|w| SIDE_WORDS.iter().any(|(s, _)| s == w)) } else { None };
    let side = at.and_then(|at| {
        let word = parts.remove(at);
        SIDE_WORDS.iter().find(|(s, _)| *s == word).map(|(_, full)| *full)
    });
    if let Some(last) = parts.last_mut()
        && last.chars().count() > 3
        && last.ends_with('s')
        && !last.ends_with("ss")
    {
        last.pop();
    }
    let rest: String = parts.concat();
    Some(format!("{}{rest}", side.unwrap_or_default()))
}

/// The table key a name is looked up by: its plain normal form when that is in the table, otherwise the guessed
/// form (`Eye Left` -> `lefteye`), otherwise the plain form.
fn lookup_key(name: &str) -> String {
    let plain = normalize(name);
    if AUTO_TAGS.iter().any(|t| t.matches.contains(&plain.as_str())) {
        return plain;
    }
    guessed_key(name).filter(|k| AUTO_TAGS.iter().any(|t| t.matches.contains(&k.as_str()))).unwrap_or(plain)
}

/// Every table entry a name gives. Handle tags only make sense on independent groups, so callers decide which
/// kinds to apply.
pub fn auto_tags(name: &str) -> impl Iterator<Item = &'static AutoTag> {
    let key = lookup_key(name);
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
    fn loose_names_get_the_tag_adobe_would_guess() {
        for (name, want) in [
            ("Eye Left", ids::LEFT_EYE),
            ("eye_R", ids::RIGHT_EYE),
            ("LeftEyes", ids::LEFT_EYE),
            ("Brow Right", ids::RIGHT_EYEBROW),
            ("L Eyebrow", ids::LEFT_EYEBROW),
            ("Right Cheek", ids::RIGHT_CHEEK),
            ("Cheek L", ids::LEFT_CHEEK),
            ("Hair 2", ids::HAIR),
            ("Ears", "Charcraft.Ears"),
            ("Ear Left", ids::LEFT_EAR),
            ("Eyelashes", "Charcraft.Eyelashes"),
            ("Mouth Corner Left", "Charcraft.LeftMouthCorner"),
            ("Profile Left", "Adobe.Face.LeftProfile"),
            ("Teeth", "Charcraft.Teeth"),
            ("Left Leg", "Charcraft.LeftLeg"),
        ] {
            assert!(ids_of(name).contains(&want), "{name} should give {want}, got {:?}", ids_of(name));
        }
        assert!(ids_of("R").contains(&"Adobe.LipSynch.R"), "a lone R stays the mouth shape");
        assert!(ids_of("Banana Left").is_empty());
    }

    #[test]
    fn no_duplicate_ids_with_the_same_match_and_kind() {
        let mut seen = std::collections::BTreeSet::new();
        for t in AUTO_TAGS {
            assert!(seen.insert((t.id, t.kind)), "{} listed twice", t.id);
        }
    }
}
