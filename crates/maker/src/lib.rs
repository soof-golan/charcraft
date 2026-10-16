//! Puppet Maker: original characters drawn in code and rigged by naming.
//!
//! A [`Spec`] says what the character looks like (head shape, colours, hair, eyes, ears, cheeks, body). [`artwork`]
//! draws it with our own pixel primitives, naming the layers the way Character Animator reads them (`Head`,
//! `Left Eye`, `+ Left Pupil`, `Left Blink`, `mouth` with all fourteen mouth shapes, `Body`), and [`puppet`] builds the
//! rigged puppet from that artwork. The [`templates`] are ready-made specs. Nothing here is derived from any
//! Adobe artwork.
//!
//! ```
//! let spec = charcraft_maker::template("square-bot").map(|t| t.spec).unwrap_or_default();
//! let puppet = charcraft_maker::puppet(&spec)?;
//! assert_eq!(puppet.mouth_groups().len(), 1);
//! # Ok::<(), charcraft_puppet::Error>(())
//! ```

mod art;

use anim_artwork::ArtworkTree;
use charcraft_puppet::Puppet;
use serde::{Deserialize, Serialize};

pub use art::{HEIGHT, WIDTH};

/// A colour as red, green and blue bytes.
pub type Rgb = [u8; 3];

/// The shape of the head.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadShape {
    /// Nearly a circle.
    #[default]
    Round,
    /// Narrower and taller.
    Oval,
    /// A rounded box.
    Square,
}

/// The hair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HairStyle {
    /// None.
    Bald,
    /// A cap of hair behind the face.
    Short,
    /// A cap with a fringe over the forehead.
    #[default]
    Fringe,
    /// Falls to the shoulders behind the face, with a fringe.
    Long,
    /// A cap with a bun on top.
    Bun,
}

/// The eyes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EyeStyle {
    /// Medium eyes.
    #[default]
    Round,
    /// Large eyes with large irises.
    Big,
    /// Small eyes.
    Small,
}

/// The torso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyShape {
    /// Narrow.
    Slim,
    /// Average.
    #[default]
    Regular,
    /// Broad.
    Wide,
}

/// What a generated character looks like. Every field has a default (the classic character), so a partial JSON
/// object is a valid spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Spec {
    /// The character's name (it names the puppet in the cast).
    pub name: String,
    /// Head shape.
    pub head: HeadShape,
    /// Skin colour.
    pub skin: Rgb,
    /// Hair style.
    pub hair_style: HairStyle,
    /// Hair and eyebrow colour.
    pub hair: Rgb,
    /// Eye style.
    pub eyes: EyeStyle,
    /// Iris colour.
    pub iris: Rgb,
    /// Draw eyelashes.
    pub lashes: bool,
    /// Draw ears.
    pub ears: bool,
    /// Draw rosy cheeks.
    pub cheeks: bool,
    /// Lip colour.
    pub lips: Rgb,
    /// Shirt colour.
    pub shirt: Rgb,
    /// Torso width.
    pub body: BodyShape,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            name: "Classic".to_owned(),
            head: HeadShape::Round,
            skin: [244, 196, 160],
            hair_style: HairStyle::Fringe,
            hair: [74, 44, 30],
            eyes: EyeStyle::Round,
            iris: [60, 120, 90],
            lashes: false,
            ears: true,
            cheeks: true,
            lips: [176, 64, 76],
            shirt: [58, 112, 200],
            body: BodyShape::Regular,
        }
    }
}

/// A starter puppet: a named spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// Short id (`round-pal`).
    pub id: &'static str,
    /// Name for lists.
    pub title: &'static str,
    /// One line about it.
    pub about: &'static str,
    /// What it looks like.
    pub spec: Spec,
}

/// The starter puppets, the classic character first.
#[must_use]
pub fn templates() -> Vec<Template> {
    let base = Spec::default();
    vec![
        Template { id: "classic", title: "Classic", about: "Round face, brown fringe, blue shirt", spec: base.clone() },
        Template {
            id: "round-pal",
            title: "Round Pal",
            about: "A bald, cheerful yellow character with big eyes",
            spec: Spec {
                name: "Round Pal".to_owned(),
                skin: [255, 214, 102],
                hair_style: HairStyle::Bald,
                eyes: EyeStyle::Big,
                iris: [70, 70, 160],
                ears: false,
                lips: [200, 80, 70],
                shirt: [38, 160, 150],
                ..base.clone()
            },
        },
        Template {
            id: "square-bot",
            title: "Square Bot",
            about: "A boxy grey robot with glowing cyan eyes",
            spec: Spec {
                name: "Square Bot".to_owned(),
                head: HeadShape::Square,
                skin: [168, 184, 200],
                hair_style: HairStyle::Bald,
                hair: [70, 80, 96],
                iris: [60, 210, 235],
                ears: false,
                cheeks: false,
                lips: [60, 70, 90],
                shirt: [92, 98, 112],
                body: BodyShape::Wide,
                ..base.clone()
            },
        },
        Template {
            id: "long-hair",
            title: "Long Hair",
            about: "An oval face with long auburn hair and eyelashes",
            spec: Spec {
                name: "Long Hair".to_owned(),
                head: HeadShape::Oval,
                hair_style: HairStyle::Long,
                hair: [138, 62, 32],
                iris: [100, 80, 40],
                lashes: true,
                body: BodyShape::Slim,
                shirt: [170, 60, 140],
                ..base.clone()
            },
        },
        Template {
            id: "bun",
            title: "Bun",
            about: "Dark skin, black hair in a bun, orange top",
            spec: Spec {
                name: "Bun".to_owned(),
                head: HeadShape::Oval,
                skin: [150, 100, 70],
                hair_style: HairStyle::Bun,
                hair: [24, 20, 24],
                iris: [50, 36, 28],
                lashes: true,
                lips: [150, 56, 60],
                shirt: [232, 130, 40],
                ..base
            },
        },
    ]
}

/// The template with this id.
#[must_use]
pub fn template(id: &str) -> Option<Template> {
    templates().into_iter().find(|t| t.id.eq_ignore_ascii_case(id.trim()))
}

/// The artwork of a character.
#[must_use]
pub fn artwork(spec: &Spec) -> ArtworkTree {
    art::artwork(spec)
}

/// The rigged puppet of a character.
///
/// # Errors
/// Whatever building a puppet from the artwork reports (it cannot fail for generated artwork).
pub fn puppet(spec: &Spec) -> charcraft_puppet::Result<Puppet> {
    Puppet::from_artwork(&artwork(spec))
}

#[cfg(test)]
mod tests {
    use charcraft_puppet::{PuppetPose, Viseme};

    use super::*;

    #[test]
    fn every_template_makes_a_complete_rig() {
        for t in templates() {
            let p = puppet(&t.spec).unwrap();
            assert_eq!(p.mouth_groups().len(), 1, "{}", t.id);
            assert_eq!(p.mouth_shapes(p.mouth_groups()[0]).len(), 14, "{}", t.id);
            for path in ["Head", "Head/Left Eye", "Head/Right Eye", "Head/Left Eye/Left Blink", "Body"] {
                assert!(p.find_path(path).is_some(), "{} lacks {path}", t.id);
            }
            assert!(p.import_report().is_empty(), "{}: {:?}", t.id, p.import_report());
            let drawn = p.evaluate(&PuppetPose::neutral().with_viseme(Viseme::Oh)).nodes.len();
            assert!(drawn > 0);
        }
    }

    #[test]
    fn choices_change_the_layers() {
        let plain = puppet(&Spec { hair_style: HairStyle::Bald, ears: false, cheeks: false, lashes: false, ..Spec::default() }).unwrap();
        assert!(plain.find_path("Head/Hair back").is_none());
        assert!(plain.find_path("Head/Ears").is_none());
        assert!(plain.find_path("Head/Cheeks").is_none());
        let rich = puppet(&Spec { hair_style: HairStyle::Bun, lashes: true, ..Spec::default() }).unwrap();
        assert!(rich.find_path("Head/Bun").is_some());
        assert!(rich.find_path("Head/Left Eye/Left Eyelashes").is_some());
        let tags = |p: &Puppet, path: &str| p.find_path(path).and_then(|id| p.layer(id)).map(|l| l.tags.clone()).unwrap_or_default();
        assert!(tags(&rich, "Head/Bun").contains("Charcraft.Hair"));
        assert!(tags(&rich, "Head/Ears").contains("Charcraft.Ears"));
        assert!(tags(&rich, "Head/Cheeks").contains("Charcraft.Cheeks"));
    }

    #[test]
    fn a_spec_survives_json_and_partial_specs_fill_in() {
        let spec = template("long-hair").unwrap().spec;
        let text = serde_json::to_string(&spec).unwrap();
        assert_eq!(serde_json::from_str::<Spec>(&text).unwrap(), spec);
        let partial: Spec = serde_json::from_str(r#"{"head":"square","lashes":true}"#).unwrap();
        assert_eq!(partial.head, HeadShape::Square);
        assert_eq!(partial.skin, Spec::default().skin);
        assert!(serde_json::from_str::<Spec>(r#"{"hed":"square"}"#).is_err());
    }

    #[test]
    fn template_ids_are_unique_and_found_in_any_case() {
        let all = templates();
        let ids: std::collections::BTreeSet<_> = all.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), all.len());
        assert!(template("SQUARE-BOT").is_some() && template("nope").is_none());
    }
}
