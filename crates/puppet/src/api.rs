//! The tool surface: edits and queries as plain serde values, addressed by layer path.
//!
//! An MCP server (or any scripting layer) needs operations it can describe in JSON and apply without holding
//! internal ids. [`PuppetOp`] is that vocabulary for editing a puppet, [`LayerInfo`] and
//! [`Puppet::summary`] answer questions about it, and [`PuppetPose`](crate::PuppetPose) is the argument of a
//! render. The application maps them to tools one to one:
//!
//! | tool | argument | result |
//! |---|---|---|
//! | `puppet.summary` | none | [`crate::PuppetSummary`] |
//! | `puppet.outline` | none | [`Puppet::outline`] text |
//! | `puppet.layer` | layer path or label | [`LayerInfo`] |
//! | `puppet.edit` | [`PuppetOp`] | [`OpOutcome`] |
//! | `puppet.render` | [`PuppetPose`](crate::PuppetPose) | PNG of the evaluated pose |
//!
//! Layers are named by path (`Head/mouth/Oh`, as in [`Puppet::find_path`]) or, when that is unambiguous enough
//! for the caller, by label (`mouth`).

use anim_core::Point;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{AttachStyle, CycleDef, HandleDef, HandleKind, LayerId, Puppet};

/// One edit of a puppet. Plain serde, tagged by `op`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PuppetOp {
    /// Gives a layer a tag.
    AddTag { layer: String, tag: String },
    /// Takes a tag off a layer.
    RemoveTag { layer: String, tag: String },
    /// Adds (or replaces) a handle of an independent group. A stick (`kind` Stick) also takes the far `end`.
    AddHandle {
        group: String,
        name: String,
        kind: HandleKind,
        position: [f64; 2],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<[f64; 2]>,
    },
    /// Moves a handle (a stick moves with both ends, or to a new `end` when given).
    MoveHandle {
        group: String,
        key: String,
        position: [f64; 2],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<[f64; 2]>,
    },
    /// Makes a layer an independent group or an ordinary layer.
    SetIndependent { layer: String, independent: bool },
    /// Chooses how an independent group attaches to its parent's mesh.
    SetAttachStyle { group: String, style: AttachStyle },
    /// Switches whether a group's own mesh ignores the groups attached to it.
    SetWarpIndependent { group: String, on: bool },
    /// Sets how finely a group's mesh is cut (0.5 to 4, 1 is the default).
    SetMeshDensity { group: String, density: f64 },
    /// Hides the siblings of a layer while its key trigger shows it.
    SetHideOthers { layer: String, on: bool },
    /// Makes a group show its children one after the other (`None` stops it).
    SetCycle { layer: String, cycle: Option<CycleDef> },
    /// Deletes a swap set.
    RemoveSwapSet { name: String },
    /// Renames a swap set.
    RenameSwapSet { name: String, new_name: String },
    /// Removes a handle by its key.
    RemoveHandle { group: String, key: String },
    /// Moves the origin of an independent group.
    SetOrigin { group: String, origin: [f64; 2] },
    /// Shows a layer only while a key trigger is on; no key removes the trigger.
    SetKeyTrigger { layer: String, key: Option<String> },
    /// Defines a swap set from layers.
    AddSwapSet { name: String, members: Vec<String> },
    /// Changes whether the artwork shows a layer by default.
    SetVisible { layer: String, visible: bool },
}

/// What an applied edit returns.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OpOutcome {
    /// The key of a handle the edit added.
    pub key: Option<String>,
    /// False when the edit changed nothing (a tag already there, a handle that did not exist).
    pub changed: bool,
}

/// What a tool shows about one layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerInfo {
    /// Layer path.
    pub path: String,
    /// The artwork's name.
    pub name: String,
    /// `group`, `raster` or `vector`.
    pub kind: String,
    /// Independent group.
    pub independent: bool,
    /// Shown by default.
    pub visible: bool,
    /// Tag ids.
    pub tags: Vec<String>,
    /// Paths of the children, bottom-most first.
    pub children: Vec<String>,
    /// Bounds `[x0, y0, x1, y1]` of everything below.
    pub bounds: Option<[f64; 4]>,
    /// Origin of an independent group.
    pub origin: Option<[f64; 2]>,
    /// Handles of an independent group.
    pub handles: Vec<HandleDef>,
    /// Key trigger.
    pub key_trigger: Option<String>,
    /// The key trigger hides the siblings while it shows the layer.
    pub hide_others: bool,
    /// How an independent group attaches to its parent's mesh.
    pub attach: Option<AttachStyle>,
    /// An independent group's mesh ignores the groups attached to it.
    pub warp_independent: Option<bool>,
    /// How finely an independent group's mesh is cut.
    pub mesh_density: Option<f64>,
    /// A group's layer cycle.
    pub cycle: Option<CycleDef>,
}

impl Puppet {
    /// The layer a path or label names: the path wins, then the first label match.
    #[must_use]
    pub fn resolve(&self, layer: &str) -> Option<LayerId> {
        if layer.is_empty() {
            return Some(LayerId::ROOT);
        }
        self.find_path(layer).or_else(|| self.find_label(layer))
    }

    fn need(&self, layer: &str) -> Result<LayerId> {
        self.resolve(layer).ok_or_else(|| Error::NoLayer(layer.to_owned()))
    }

    fn need_group(&self, group: &str) -> Result<LayerId> {
        let id = self.need(group)?;
        if self.layer(id).is_some_and(|l| l.independent) { Ok(id) } else { Err(Error::NotIndependent(group.to_owned())) }
    }

    /// What a tool shows about a layer.
    #[must_use]
    pub fn layer_info(&self, layer: &str) -> Option<LayerInfo> {
        let l = self.layer(self.resolve(layer)?)?;
        Some(LayerInfo {
            path: l.path.clone(),
            name: l.name.clone(),
            kind: match l.kind {
                crate::LayerKind::Group => "group",
                crate::LayerKind::Raster => "raster",
                crate::LayerKind::Vector => "vector",
            }
            .to_owned(),
            independent: l.independent,
            visible: l.visible,
            tags: l.tags.iter().cloned().collect(),
            children: l.children.iter().filter_map(|&c| self.layer(c)).map(|c| c.path.clone()).collect(),
            bounds: l.bounds.map(|b| [b.x0, b.y0, b.x1, b.y1]),
            origin: l.independent.then_some([l.origin.x, l.origin.y]),
            handles: l.handles.clone(),
            key_trigger: l.key_trigger.clone(),
            hide_others: l.hide_others,
            attach: l.independent.then_some(l.attach),
            warp_independent: l.independent.then_some(l.warp_independent),
            mesh_density: l.independent.then_some(l.mesh_density),
            cycle: l.cycle,
        })
    }

    /// Applies one edit.
    ///
    /// # Errors
    ///
    /// [`Error::NoLayer`] for an unknown layer, [`Error::NotIndependent`] when a handle or origin is set on a
    /// layer that is not an independent group, [`Error::Rejected`] for values that cannot be used (non-finite
    /// positions, swap sets of fewer than two layers).
    pub fn apply(&mut self, op: &PuppetOp) -> Result<OpOutcome> {
        let finite = |p: &[f64; 2]| p.iter().all(|v| v.is_finite());
        match op {
            PuppetOp::AddTag { layer, tag } => {
                let id = self.need(layer)?;
                Ok(OpOutcome { changed: self.add_tag(id, tag), key: None })
            }
            PuppetOp::RemoveTag { layer, tag } => {
                let id = self.need(layer)?;
                Ok(OpOutcome { changed: self.remove_tag(id, tag), key: None })
            }
            PuppetOp::AddHandle { group, name, kind, position, end } => {
                if !finite(position) || end.as_ref().is_some_and(|e| !finite(e)) {
                    return Err(Error::Rejected("handle positions must be finite"));
                }
                let id = self.need_group(group)?;
                let at = Point::new(position[0], position[1]);
                let key = match end {
                    Some(e) if *kind == HandleKind::Stick => self.add_stick(id, name, at, Point::new(e[0], e[1])),
                    _ => self.add_handle(id, name, *kind, at),
                };
                Ok(OpOutcome { changed: key.is_some(), key })
            }
            PuppetOp::MoveHandle { group, key, position, end } => {
                if !finite(position) || end.as_ref().is_some_and(|e| !finite(e)) {
                    return Err(Error::Rejected("handle positions must be finite"));
                }
                let id = self.need_group(group)?;
                let moved = self.move_handle(id, key, Point::new(position[0], position[1]), end.map(|e| Point::new(e[0], e[1])));
                Ok(OpOutcome { changed: moved, key: None })
            }
            PuppetOp::SetIndependent { layer, independent } => {
                let id = self.need(layer)?;
                if id == LayerId::ROOT {
                    return Err(Error::Rejected("the puppet itself is always an independent group"));
                }
                Ok(OpOutcome { changed: self.set_independent(id, *independent), key: None })
            }
            PuppetOp::SetAttachStyle { group, style } => {
                let id = self.need_group(group)?;
                Ok(OpOutcome { changed: self.set_attach(id, *style), key: None })
            }
            PuppetOp::SetWarpIndependent { group, on } => {
                let id = self.need_group(group)?;
                Ok(OpOutcome { changed: self.set_warp_independent(id, *on), key: None })
            }
            PuppetOp::SetMeshDensity { group, density } => {
                if !density.is_finite() || *density <= 0.0 {
                    return Err(Error::Rejected("the mesh density must be a positive number"));
                }
                let id = self.need_group(group)?;
                Ok(OpOutcome { changed: self.set_mesh_density(id, *density), key: None })
            }
            PuppetOp::SetHideOthers { layer, on } => {
                let id = self.need(layer)?;
                Ok(OpOutcome { changed: self.set_hide_others(id, *on), key: None })
            }
            PuppetOp::SetCycle { layer, cycle } => {
                let id = self.need(layer)?;
                if self.layer(id).is_none_or(|l| l.kind != crate::LayerKind::Group || l.children.len() < 2) {
                    return Err(Error::Rejected("a layer cycle needs a group with at least two layers"));
                }
                Ok(OpOutcome { changed: self.set_cycle(id, *cycle), key: None })
            }
            PuppetOp::RemoveSwapSet { name } => Ok(OpOutcome { changed: self.remove_swap_set(name), key: None }),
            PuppetOp::RenameSwapSet { name, new_name } => {
                if new_name.trim().is_empty() {
                    return Err(Error::Rejected("a swap set needs a name"));
                }
                Ok(OpOutcome { changed: self.rename_swap_set(name, new_name), key: None })
            }
            PuppetOp::RemoveHandle { group, key } => {
                let id = self.need_group(group)?;
                Ok(OpOutcome { changed: self.remove_handle(id, key), key: None })
            }
            PuppetOp::SetOrigin { group, origin } => {
                if !finite(origin) {
                    return Err(Error::Rejected("origins must be finite"));
                }
                let id = self.need_group(group)?;
                Ok(OpOutcome { changed: self.set_origin(id, Point::new(origin[0], origin[1])), key: None })
            }
            PuppetOp::SetKeyTrigger { layer, key } => {
                let id = self.need(layer)?;
                Ok(OpOutcome { changed: self.set_key_trigger(id, key.as_deref()), key: None })
            }
            PuppetOp::AddSwapSet { name, members } => {
                let ids = members.iter().map(|m| self.need(m)).collect::<Result<Vec<_>>>()?;
                if !self.add_swap_set(name, ids) {
                    return Err(Error::Rejected("a swap set needs at least two layers"));
                }
                Ok(OpOutcome { changed: true, key: None })
            }
            PuppetOp::SetVisible { layer, visible } => {
                let id = self.need(layer)?;
                let changed = self.layer(id).is_some_and(|l| l.visible != *visible);
                if let Some(l) = self.layer_mut(id) {
                    l.visible = *visible;
                }
                Ok(OpOutcome { changed, key: None })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_are_tagged_json() {
        let op = PuppetOp::AddHandle { group: "Body".into(), name: "Fixed".into(), kind: HandleKind::Fixed, position: [1.0, 2.0], end: None };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains(r#""op":"add_handle""#), "{json}");
        let back: PuppetOp = serde_json::from_str(&json).unwrap();
        assert_eq!(back, op);
        let parsed: PuppetOp = serde_json::from_str(r#"{"op":"set_key_trigger","layer":"Star","key":null}"#).unwrap();
        assert_eq!(parsed, PuppetOp::SetKeyTrigger { layer: "Star".into(), key: None });
    }
}
