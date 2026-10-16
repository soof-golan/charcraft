//! The import report: what looks wrong with the way a puppet's artwork is named and structured.
//!
//! Character Animator lists problems after an import (two layers with the same name, a face without eyes, handle
//! tags on layers that cannot take them). [`Puppet::import_report`] does the same from the layer tree alone, so it
//! works for any puppet however it was loaded.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::{LayerKind, Puppet};
use crate::tags::{self, TagKind, ids};

/// What kind of problem an [`ImportIssue`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    /// Two layers of one group share a name, so the rig cannot tell them apart.
    DuplicateName,
    /// A part a face needs is missing (eyes, a neutral mouth).
    MissingPart,
    /// A name asks for a role that only an independent group can have.
    NotIndependent,
    /// A group has no name, so no tag can be given by name.
    UnnamedGroup,
    /// Something the loader or the project data could not carry over.
    Carried,
}

impl IssueKind {
    /// A short label for lists.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::DuplicateName => "Duplicate name",
            Self::MissingPart => "Missing part",
            Self::NotIndependent => "Not independent",
            Self::UnnamedGroup => "Unnamed group",
            Self::Carried => "Not carried over",
        }
    }
}

/// One problem found in a puppet.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ImportIssue {
    /// What kind of problem.
    pub kind: IssueKind,
    /// The layer it is about (a path), when it is about one.
    pub layer: Option<String>,
    /// A sentence for the person.
    pub message: String,
}

impl Puppet {
    /// The problems with the puppet's naming and structure, plus the warnings the loader collected, in a stable
    /// order. A tidy puppet gives an empty list.
    #[must_use]
    pub fn import_report(&self) -> Vec<ImportIssue> {
        let mut out = Vec::new();
        self.duplicate_names(&mut out);
        self.face_parts(&mut out);
        self.handle_tags_on_plain_groups(&mut out);
        let unnamed = self.layers().iter().filter(|l| l.kind == LayerKind::Group && l.name.is_empty() && l.parent.is_some()).count();
        if unnamed > 0 {
            out.push(ImportIssue {
                kind: IssueKind::UnnamedGroup,
                layer: None,
                message: format!("{unnamed} group{} ha{} no name, so names cannot give {} tags", plural(unnamed), if unnamed == 1 { "s" } else { "ve" }, if unnamed == 1 { "it" } else { "them" }),
            });
        }
        for w in &self.warnings {
            out.push(ImportIssue { kind: IssueKind::Carried, layer: None, message: w.clone() });
        }
        out.sort();
        out.dedup();
        out
    }

    fn duplicate_names(&self, out: &mut Vec<ImportIssue>) {
        for parent in self.layers() {
            let mut seen: BTreeMap<String, usize> = BTreeMap::new();
            for child in parent.children.iter().filter_map(|&c| self.layer(c)) {
                if child.name.is_empty() {
                    continue;
                }
                let key = tags::normalize(&child.name);
                let count = seen.entry(key).or_insert(0);
                *count = count.saturating_add(1);
                if *count == 2 {
                    out.push(ImportIssue {
                        kind: IssueKind::DuplicateName,
                        layer: Some(child.path.clone()),
                        message: format!("more than one layer is called \"{}\" in \"{}\"", child.label, if parent.name.is_empty() { "the artwork" } else { parent.label.as_str() }),
                    });
                }
            }
        }
    }

    fn face_parts(&self, out: &mut Vec<ImportIssue>) {
        for head in self.layers().iter().filter(|l| l.is_head()) {
            let inside = |tag: &str| self.with_tag(tag).into_iter().any(|id| self.is_inside(id, head.id));
            for (tag, what) in [(ids::LEFT_EYE, "Left Eye"), (ids::RIGHT_EYE, "Right Eye"), (ids::MOUTH, "Mouth")] {
                if !inside(tag) {
                    out.push(ImportIssue {
                        kind: IssueKind::MissingPart,
                        layer: Some(head.path.clone()),
                        message: format!("\"{}\" has no {what}, so the face cannot be tracked there", head.label),
                    });
                }
            }
        }
        for group in self.mouth_groups() {
            let Some(layer) = self.layer(group) else { continue };
            let shapes = self.mouth_shapes(group);
            if !shapes.iter().any(|(v, _)| *v == tags::Viseme::Neutral) {
                out.push(ImportIssue {
                    kind: IssueKind::MissingPart,
                    layer: Some(layer.path.clone()),
                    message: format!("the mouth \"{}\" has no Neutral shape to rest on", layer.label),
                });
            }
        }
    }

    fn handle_tags_on_plain_groups(&self, out: &mut Vec<ImportIssue>) {
        for layer in self.layers().iter().filter(|l| l.kind == LayerKind::Group && !l.independent && l.parent.is_some()) {
            if let Some(tag) = tags::auto_tags(&layer.name).find(|t| t.kind == TagKind::Handle) {
                out.push(ImportIssue {
                    kind: IssueKind::NotIndependent,
                    layer: Some(layer.path.clone()),
                    message: format!("\"{}\" would be a {} but is not an independent group: start its name with +", layer.label, tag.id.rsplit('.').next().unwrap_or(tag.id)),
                });
            }
        }
    }

    /// True when `inner` is `outer` or lies somewhere below it.
    fn is_inside(&self, inner: crate::model::LayerId, outer: crate::model::LayerId) -> bool {
        let mut at = Some(inner);
        let mut guard = 0_u32;
        while let Some(current) = at {
            if current == outer {
                return true;
            }
            guard = guard.saturating_add(1);
            if guard > 4096 {
                return false;
            }
            at = self.layer(current).and_then(|l| l.parent);
        }
        false
    }
}

const fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use anim_artwork::{ArtNode, ArtworkTree};
    use anim_core::Size;

    use super::*;

    fn tree(children: Vec<ArtNode>) -> ArtworkTree {
        ArtworkTree { size: Size::new(100.0, 100.0), root: ArtNode::group("", children), warnings: Vec::new() }
    }

    #[test]
    fn duplicates_and_headless_faces_are_reported() {
        let puppet = Puppet::from_artwork(&tree(vec![
            ArtNode::group("Head", vec![ArtNode::group("Left Eye", vec![]), ArtNode::group("Hair", vec![]), ArtNode::group("hair", vec![])]),
            ArtNode::group("Draggable", vec![]),
        ]))
        .unwrap();
        let report = puppet.import_report();
        assert!(report.iter().any(|i| i.kind == IssueKind::DuplicateName && i.layer.as_deref() == Some("Head/hair")), "{report:?}");
        assert!(report.iter().any(|i| i.kind == IssueKind::MissingPart && i.message.contains("Right Eye")), "{report:?}");
        assert!(report.iter().any(|i| i.kind == IssueKind::MissingPart && i.message.contains("Mouth")), "{report:?}");
        assert!(!report.iter().any(|i| i.message.contains("Left Eye,")), "the left eye is there");
    }

    #[test]
    fn a_tidy_face_is_quiet() {
        let puppet = Puppet::from_artwork(&tree(vec![ArtNode::group(
            "Head",
            vec![ArtNode::group("Left Eye", vec![]), ArtNode::group("Right Eye", vec![]), ArtNode::group("Mouth", vec![])],
        )]))
        .unwrap();
        let report = puppet.import_report();
        assert!(report.iter().all(|i| i.kind != IssueKind::MissingPart || i.message.contains("Neutral") || i.message.contains("Mouth")), "{report:?}");
        assert!(!report.iter().any(|i| i.message.contains("Eye")), "{report:?}");
    }
}
