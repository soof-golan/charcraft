//! Re-import: carrying a rig over to changed artwork by layer identity.
//!
//! When the artwork file changes (a layer renamed, moved into another group, added or deleted) the puppet is built
//! again from the new file and every rig edit made so far is replayed on it. An edit names layers by path, so each
//! one is looked up in the new artwork: the same path is the same layer; otherwise a layer with the same name (case
//! and blanks ignored) that is unique, or unique inside a group of the same name, is taken to be the same layer that
//! moved. Edits whose layers are gone are dropped and reported.

use std::collections::{BTreeMap, BTreeSet};

use charcraft_puppet::PuppetOp;
use charcraft_puppet::tags::normalize;

use crate::session::{RigEdit, Skeleton};

fn parent_label(path: &str) -> String {
    path.rsplit_once('/').and_then(|(parent, _)| parent.rsplit('/').next()).map(normalize).unwrap_or_default()
}

/// Which new layer each old layer became, by path. Old layers with no counterpart are not in the map.
#[must_use]
pub fn layer_map(old: &Skeleton, new: &Skeleton) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut taken: BTreeSet<&str> = BTreeSet::new();
    let new_paths: BTreeSet<&str> = new.layers.iter().map(|l| l.path.as_str()).collect();
    for layer in &old.layers {
        if let Some(path) = new_paths.get(layer.path.as_str()) {
            map.insert(layer.path.clone(), (*path).to_owned());
            taken.insert(path);
        }
    }
    let unmatched: Vec<_> = old.layers.iter().filter(|l| !map.contains_key(&l.path)).collect();
    for layer in unmatched {
        let key = normalize(&layer.label);
        if key.is_empty() {
            continue;
        }
        let candidates: Vec<&str> =
            new.layers.iter().filter(|n| !taken.contains(n.path.as_str()) && normalize(&n.label) == key).map(|n| n.path.as_str()).collect();
        let chosen = match candidates.as_slice() {
            [only] => Some(*only),
            [] => None,
            many => {
                let wanted = parent_label(&layer.path);
                let same_parent: Vec<&str> = many.iter().copied().filter(|p| parent_label(p) == wanted).collect();
                match same_parent.as_slice() {
                    [only] => Some(*only),
                    _ => None,
                }
            }
        };
        if let Some(path) = chosen {
            map.insert(layer.path.clone(), path.to_owned());
            taken.insert(path);
        }
    }
    map
}

/// The result of carrying a puppet's edits over.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Carried {
    /// The edits of the puppet, rewritten to the new paths.
    pub edits: Vec<RigEdit>,
    /// Why each dropped edit was dropped.
    pub dropped: Vec<String>,
    /// Old layers found again.
    pub matched: usize,
    /// Old layers with no counterpart.
    pub removed: usize,
    /// New layers with no old counterpart.
    pub added: usize,
}

fn resolve(old: &Skeleton, map: &BTreeMap<String, String>, layer: &str) -> Result<String, String> {
    let found = old.resolve(layer).ok_or_else(|| format!("layer {layer:?} was not in the old artwork"))?;
    map.get(&found.path).cloned().ok_or_else(|| format!("layer {:?} is gone from the new artwork", found.path))
}

fn resolve_group(old: &Skeleton, new: &Skeleton, map: &BTreeMap<String, String>, group: &str) -> Result<(String, String), String> {
    let path = resolve(old, map, group)?;
    let layer = new.resolve(&path).ok_or_else(|| format!("layer {path:?} is gone from the new artwork"))?;
    if layer.independent { Ok((path, layer.label.clone())) } else { Err(format!("{path:?} is no longer an independent group (its name must start with +)")) }
}

/// A handle key (`Group label/handle name`) in terms of the new group.
fn remap_handle_key(old: &Skeleton, new: &Skeleton, map: &BTreeMap<String, String>, group: &str, key: &str) -> Result<(String, String), String> {
    let old_label = old.resolve(group).map(|l| l.label.clone()).unwrap_or_default();
    let (path, label) = resolve_group(old, new, map, group)?;
    let name = key.strip_prefix(&format!("{old_label}/")).unwrap_or(key);
    Ok((path, format!("{label}/{name}")))
}

/// One edit in terms of the new artwork.
///
/// # Errors
/// Why the edit cannot be carried over.
pub fn remap_op(op: &PuppetOp, old: &Skeleton, new: &Skeleton, map: &BTreeMap<String, String>) -> Result<PuppetOp, String> {
    Ok(match op {
        PuppetOp::AddTag { layer, tag } => PuppetOp::AddTag { layer: resolve(old, map, layer)?, tag: tag.clone() },
        PuppetOp::RemoveTag { layer, tag } => PuppetOp::RemoveTag { layer: resolve(old, map, layer)?, tag: tag.clone() },
        PuppetOp::AddHandle { group, name, kind, position, end } => {
            PuppetOp::AddHandle { group: resolve_group(old, new, map, group)?.0, name: name.clone(), kind: *kind, position: *position, end: *end }
        }
        PuppetOp::MoveHandle { group, key, position, end } => {
            let (path, key) = remap_handle_key(old, new, map, group, key)?;
            PuppetOp::MoveHandle { group: path, key, position: *position, end: *end }
        }
        PuppetOp::RemoveHandle { group, key } => {
            let (path, key) = remap_handle_key(old, new, map, group, key)?;
            PuppetOp::RemoveHandle { group: path, key }
        }
        PuppetOp::SetIndependent { layer, independent } => PuppetOp::SetIndependent { layer: resolve(old, map, layer)?, independent: *independent },
        PuppetOp::SetAttachStyle { group, style } => PuppetOp::SetAttachStyle { group: resolve_group(old, new, map, group)?.0, style: *style },
        PuppetOp::SetWarpIndependent { group, on } => PuppetOp::SetWarpIndependent { group: resolve_group(old, new, map, group)?.0, on: *on },
        PuppetOp::SetMeshDensity { group, density } => PuppetOp::SetMeshDensity { group: resolve_group(old, new, map, group)?.0, density: *density },
        PuppetOp::SetHideOthers { layer, on } => PuppetOp::SetHideOthers { layer: resolve(old, map, layer)?, on: *on },
        PuppetOp::SetCycle { layer, cycle } => PuppetOp::SetCycle { layer: resolve(old, map, layer)?, cycle: *cycle },
        PuppetOp::RemoveSwapSet { name } => PuppetOp::RemoveSwapSet { name: name.clone() },
        PuppetOp::RenameSwapSet { name, new_name } => PuppetOp::RenameSwapSet { name: name.clone(), new_name: new_name.clone() },
        PuppetOp::SetOrigin { group, origin } => PuppetOp::SetOrigin { group: resolve_group(old, new, map, group)?.0, origin: *origin },
        PuppetOp::SetKeyTrigger { layer, key } => PuppetOp::SetKeyTrigger { layer: resolve(old, map, layer)?, key: key.clone() },
        PuppetOp::SetMidiTrigger { layer, note, channel } => PuppetOp::SetMidiTrigger { layer: resolve(old, map, layer)?, note: *note, channel: *channel },
        PuppetOp::AddSwapSet { name, members } => {
            let kept: Vec<String> = members.iter().filter_map(|m| resolve(old, map, m).ok()).collect();
            if kept.len() < 2 {
                return Err(format!("swap set {name:?} has fewer than two layers left"));
            }
            PuppetOp::AddSwapSet { name: name.clone(), members: kept }
        }
        PuppetOp::SetVisible { layer, visible } => PuppetOp::SetVisible { layer: resolve(old, map, layer)?, visible: *visible },
    })
}

/// Carries the edits of one puppet over to its new skeleton. Edits of other puppets are not in the result.
#[must_use]
pub fn carry(edits: &[RigEdit], puppet: &str, old: &Skeleton, new: &Skeleton) -> Carried {
    let map = layer_map(old, new);
    let mut out = Carried { matched: map.len(), ..Carried::default() };
    out.removed = old.layers.len().saturating_sub(map.len());
    let found: BTreeSet<&str> = map.values().map(String::as_str).collect();
    out.added = new.layers.iter().filter(|l| !found.contains(l.path.as_str())).count();
    for edit in edits.iter().filter(|e| e.puppet == puppet) {
        match remap_op(&edit.op, old, new, &map) {
            Ok(op) => out.edits.push(RigEdit { puppet: edit.puppet.clone(), op }),
            Err(why) => out.dropped.push(format!("{puppet}: {why}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use anim_artwork::{ArtNode, ArtworkTree};
    use anim_core::Size;
    use charcraft_puppet::{HandleKind, Puppet};

    use super::*;

    fn skeleton(children: Vec<ArtNode>) -> Skeleton {
        let tree = ArtworkTree { size: Size::new(100.0, 100.0), root: ArtNode::group("", children), warnings: Vec::new() };
        Skeleton::of(&Puppet::from_artwork(&tree).unwrap())
    }

    fn old_and_new() -> (Skeleton, Skeleton) {
        let old = skeleton(vec![
            ArtNode::group("Head", vec![ArtNode::group("Left Eye", vec![]), ArtNode::group("Hat", vec![])]),
            ArtNode::group("+Arm", vec![]),
            ArtNode::group("Prop", vec![]),
        ]);
        // The eye moved under a Face group, the hat was renamed, the arm kept, the prop deleted, a scarf added.
        let new = skeleton(vec![
            ArtNode::group("Head", vec![ArtNode::group("Face", vec![ArtNode::group("Left Eye", vec![])]), ArtNode::group("Cap", vec![])]),
            ArtNode::group("+Arm", vec![]),
            ArtNode::group("Scarf", vec![]),
        ]);
        (old, new)
    }

    #[test]
    fn layers_are_matched_by_path_then_by_name() {
        let (old, new) = old_and_new();
        let map = layer_map(&old, &new);
        assert_eq!(map.get("Head").map(String::as_str), Some("Head"));
        assert_eq!(map.get("Head/Left Eye").map(String::as_str), Some("Head/Face/Left Eye"), "moved into a group");
        assert_eq!(map.get("+Arm").map(String::as_str), Some("+Arm"));
        assert!(!map.contains_key("Head/Hat") && !map.contains_key("Prop"));
    }

    #[test]
    fn edits_follow_their_layers_and_dropped_ones_are_explained() {
        let (old, new) = old_and_new();
        let edits = vec![
            RigEdit { puppet: "p".into(), op: PuppetOp::AddTag { layer: "Head/Left Eye".into(), tag: "My.Tag".into() } },
            RigEdit { puppet: "p".into(), op: PuppetOp::AddTag { layer: "Prop".into(), tag: "My.Tag".into() } },
            RigEdit {
                puppet: "p".into(),
                op: PuppetOp::AddHandle { group: "Arm".into(), name: "Wrist".into(), kind: HandleKind::Draggable, position: [1.0, 2.0], end: None },
            },
            RigEdit { puppet: "p".into(), op: PuppetOp::RemoveHandle { group: "Arm".into(), key: "Arm/Wrist".into() } },
            RigEdit { puppet: "other".into(), op: PuppetOp::SetVisible { layer: "Prop".into(), visible: false } },
        ];
        let carried = carry(&edits, "p", &old, &new);
        assert_eq!(carried.edits.len(), 3);
        assert_eq!(carried.edits[0].op, PuppetOp::AddTag { layer: "Head/Face/Left Eye".into(), tag: "My.Tag".into() });
        assert!(matches!(&carried.edits[1].op, PuppetOp::AddHandle { group, .. } if group == "+Arm"));
        assert!(matches!(&carried.edits[2].op, PuppetOp::RemoveHandle { key, .. } if key == "Arm/Wrist"));
        assert_eq!(carried.dropped.len(), 1);
        assert!(carried.dropped[0].contains("Prop"), "{:?}", carried.dropped);
        assert_eq!((carried.removed, carried.added), (2, 3));
    }

    #[test]
    fn a_group_that_lost_its_plus_cannot_take_handles_any_more() {
        let old = skeleton(vec![ArtNode::group("+Tail", vec![])]);
        let new = skeleton(vec![ArtNode::group("Tail", vec![])]);
        let edits = vec![RigEdit { puppet: "p".into(), op: PuppetOp::SetOrigin { group: "Tail".into(), origin: [0.0, 0.0] } }];
        let carried = carry(&edits, "p", &old, &new);
        assert!(carried.edits.is_empty());
        assert!(carried.dropped[0].contains("independent"), "{:?}", carried.dropped);
    }

    #[test]
    fn swap_sets_keep_the_members_that_survive() {
        let (old, new) = old_and_new();
        let op = PuppetOp::AddSwapSet { name: "s".into(), members: vec!["Head/Left Eye".into(), "Head/Hat".into(), "+Arm".into()] };
        let kept = remap_op(&op, &old, &new, &layer_map(&old, &new)).unwrap();
        assert_eq!(kept, PuppetOp::AddSwapSet { name: "s".into(), members: vec!["Head/Face/Left Eye".into(), "+Arm".into()] });
        let two_gone = PuppetOp::AddSwapSet { name: "s".into(), members: vec!["Head/Hat".into(), "Prop".into(), "+Arm".into()] };
        assert!(remap_op(&two_gone, &old, &new, &layer_map(&old, &new)).is_err());
    }
}
