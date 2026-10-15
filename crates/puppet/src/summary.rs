//! A JSON-friendly description of a puppet, for tools and the MCP server.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::{HandleKind, Puppet};

/// One handle in a summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandleSummary {
    /// The key for [`crate::PuppetPose::drag`].
    pub key: String,
    /// What it does.
    pub kind: HandleKind,
    /// Rest position in artwork pixels.
    pub position: [f64; 2],
}

/// One independent group in a summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupSummary {
    /// Layer path.
    pub path: String,
    /// Display name.
    pub label: String,
    /// Tag ids.
    pub tags: Vec<String>,
    /// Rotation and scale origin in artwork pixels.
    pub origin: [f64; 2],
    /// Bounds `[x0, y0, x1, y1]` of its artwork.
    pub bounds: Option<[f64; 4]>,
    /// Handles.
    pub handles: Vec<HandleSummary>,
    /// Has an ARAP mesh.
    pub warped: bool,
}

/// What a puppet contains.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PuppetSummary {
    /// Canvas size.
    pub size: [f64; 2],
    /// Number of layers.
    pub layers: usize,
    /// Independent groups.
    pub groups: Vec<GroupSummary>,
    /// Paths of the mouth groups.
    pub mouth_groups: Vec<String>,
    /// Mouth shapes found across mouth groups, by label.
    pub visemes: Vec<String>,
    /// Swap sets: name and member labels.
    pub swap_sets: Vec<(String, Vec<String>)>,
    /// How many layers carry each tag.
    pub tag_counts: BTreeMap<String, usize>,
    /// Warnings from import and meshing.
    pub warnings: Vec<String>,
}

impl Puppet {
    /// A serialisable description: groups with tags and handles, mouth shapes, swap sets and tag counts.
    #[must_use]
    pub fn summary(&self) -> PuppetSummary {
        let groups = self
            .independent_groups()
            .map(|l| GroupSummary {
                path: l.path.clone(),
                label: l.label.clone(),
                tags: l.tags.iter().cloned().collect(),
                origin: [l.origin.x, l.origin.y],
                bounds: l.bounds.map(|b| [b.x0, b.y0, b.x1, b.y1]),
                handles: l
                    .handles
                    .iter()
                    .map(|h| crate::summary::HandleSummary { key: h.key.clone(), kind: h.kind, position: [h.position.x, h.position.y] })
                    .collect(),
                warped: self.warps.iter().any(|w| w.group == l.id),
            })
            .collect();
        let mouths = self.mouth_groups();
        let mut visemes: Vec<_> = mouths.iter().flat_map(|&g| self.mouth_shapes(g)).map(|(v, _)| v).collect();
        visemes.sort();
        visemes.dedup();
        let mut tag_counts = BTreeMap::new();
        for l in &self.layers {
            for t in &l.tags {
                let n = tag_counts.entry(t.clone()).or_insert(0_usize);
                *n = n.saturating_add(1);
            }
        }
        PuppetSummary {
            size: [self.size.width, self.size.height],
            layers: self.layers.len(),
            groups,
            mouth_groups: mouths.iter().filter_map(|&g| self.layer(g)).map(|l| l.path.clone()).collect(),
            visemes: visemes.into_iter().map(|v| v.label().to_owned()).collect(),
            swap_sets: self
                .swap_sets
                .iter()
                .map(|s| (s.name.clone(), s.members.iter().filter_map(|&m| self.layer(m)).map(|l| l.label.clone()).collect()))
                .collect(),
            tag_counts,
            warnings: self.warnings.clone(),
        }
    }
}
