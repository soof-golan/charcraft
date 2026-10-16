//! Magnets (this program's own `Charcraft.Magnets.js`): groups named `Magnet` pull a dragged handle in.
//!
//! A handle that is pulled within a magnet's range is drawn towards it and, once close enough, sticks to it until the
//! pointer pulls it well away again.

use std::collections::BTreeMap;

use anim_core::{Point, Vec2};
use charcraft_puppet::tags::ids;
use charcraft_puppet::{HandleKind, PuppetPose};
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::{percent, smoothstep};

/// Parameters of Magnets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MagnetsParams {
    /// How far a magnet reaches, pixels.
    pub range: f64,
    /// How hard it pulls, percent: 100 draws a handle in completely before it touches.
    pub strength: f64,
    /// Within this share of the range a handle sticks, percent.
    #[serde(rename = "stickRange")]
    pub stick_range: f64,
    /// How far, as a multiple of the range, the pointer has to pull a stuck handle to free it, percent.
    #[serde(rename = "breakAway")]
    pub break_away: f64,
    /// Only handles of groups with these names or tags are pulled (comma separated); empty pulls every draggable
    /// handle.
    pub only: String,
}

impl Default for MagnetsParams {
    fn default() -> Self {
        Self { range: 80.0, strength: 100.0, stick_range: 25.0, break_away: 150.0, only: String::new() }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "range", "px", "How far a magnet reaches.";
    "strength", "%", "How hard it pulls.";
    "stickRange", "%", "Share of the range within which a handle sticks.";
    "breakAway", "%", "How far, in ranges, the pointer must pull a stuck handle to free it.";
    "only", "", "Handles of these groups only (comma separated); empty pulls every draggable handle.";
};

/// Pulls dragged handles towards `Magnet` groups.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Magnets {
    /// The parameters.
    pub params: MagnetsParams,
    stuck: BTreeMap<String, usize>,
}

impl Magnets {
    /// The handles stuck to a magnet, with the index of the magnet.
    #[must_use]
    pub fn stuck(&self) -> Vec<(&str, usize)> {
        self.stuck.iter().map(|(k, v)| (k.as_str(), *v)).collect()
    }
}

impl Behavior for Magnets {
    behavior_common!("Charcraft.Magnets.js", "Magnets", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = self.params.clone();
        let magnets: Vec<Point> = cx.rig.anchors_with(ids::MAGNET).map(|a| a.driven_position(pose)).collect();
        if magnets.is_empty() {
            self.stuck.clear();
            return;
        }
        let range = if p.range.is_finite() { p.range.max(1.0) } else { 80.0 };
        let stick = range * percent(p.stick_range).clamp(0.0, 1.0);
        let free_at = range * percent(p.break_away).max(1.0);
        let pull = percent(p.strength).clamp(0.0, 1.0);
        let only: Vec<String> = p.only.split(',').map(|s| charcraft_puppet::tags::normalize(s.trim())).filter(|s| !s.is_empty()).collect();
        for h in cx.rig.handles_of(HandleKind::Draggable) {
            if !only.is_empty() && !h.key.split('/').next().is_some_and(|g| only.contains(&charcraft_puppet::tags::normalize(g))) {
                continue;
            }
            let rest = h.driven_position(pose);
            let local = pose.drag_of(&h.key);
            // Where the pointer has the handle, before any magnet.
            let natural = h.matrix(pose) * (h.position + local);
            if (natural - rest).hypot() < 1.0 && !self.stuck.contains_key(&h.key) {
                continue;
            }
            let nearest = magnets.iter().enumerate().map(|(i, m)| (i, *m, (natural - *m).hypot())).min_by(|a, b| a.2.total_cmp(&b.2));
            let Some((index, magnet, distance)) = nearest else { continue };
            let target = if let Some(&held) = self.stuck.get(&h.key) {
                match magnets.get(held) {
                    Some(m) if (natural - *m).hypot() <= free_at => Some(*m),
                    _ => {
                        self.stuck.remove(&h.key);
                        None
                    }
                }
            } else if distance <= stick {
                self.stuck.insert(h.key.clone(), index);
                Some(magnet)
            } else if distance <= range {
                let k = pull * smoothstep(1.0 - (distance - stick) / (range - stick).max(1e-6));
                Some(natural + (magnet - natural) * k)
            } else {
                None
            };
            if let Some(t) = target {
                let wanted: Vec2 = h.to_group_space(pose, t - rest);
                if wanted.x.is_finite() && wanted.y.is_finite() {
                    pose.drag.insert(h.key.clone(), wanted);
                }
            }
        }
        let alive: Vec<String> = cx.rig.handles.iter().map(|h| h.key.clone()).collect();
        self.stuck.retain(|k, _| alive.contains(k));
    }

    fn reset(&mut self, _seed: u64) {
        self.stuck.clear();
    }
}

#[cfg(test)]
mod tests {
    use anim_core::Tick;

    use super::*;
    use crate::input::Inputs;
    use crate::rig::{Rig, RigAnchor, RigHandle};

    fn rig() -> Rig {
        Rig::default().with_handle(RigHandle::new("hand/Draggable", HandleKind::Draggable, Point::new(100.0, 100.0))).with_anchor(RigAnchor::new(
            ids::MAGNET,
            "Magnet",
            Point::new(300.0, 100.0),
        ))
    }

    /// One tick with the pointer dragging the handle by `by` (what a Dragger before this behavior would write).
    fn tick(m: &mut Magnets, rig: &Rig, by: Vec2) -> Vec2 {
        let mut pose = PuppetPose::neutral();
        pose.drag.insert("hand/Draggable".into(), by);
        m.step(&Cx { inputs: &Inputs::at(Tick::ZERO), rig, time: 0.0, dt: 1.0 / 60.0 }, &mut pose);
        pose.drag_of("hand/Draggable")
    }

    #[test]
    fn a_handle_far_from_every_magnet_is_left_alone() {
        let mut m = Magnets::default();
        assert_eq!(tick(&mut m, &rig(), Vec2::new(30.0, 0.0)), Vec2::new(30.0, 0.0));
    }

    #[test]
    fn an_undisturbed_handle_is_not_snapped() {
        let r = Rig::default().with_handle(RigHandle::new("hand/Draggable", HandleKind::Draggable, Point::new(100.0, 100.0))).with_anchor(RigAnchor::new(
            ids::MAGNET,
            "Magnet",
            Point::new(110.0, 100.0),
        ));
        let mut m = Magnets::default();
        assert_eq!(tick(&mut m, &r, Vec2::ZERO), Vec2::ZERO);
    }

    #[test]
    fn a_handle_pulled_into_range_is_drawn_towards_the_magnet() {
        let mut m = Magnets::default();
        // The handle rests at x 100 and the magnet is at 300. Pulled 240 px it is at 340: 40 px past the magnet,
        // inside the range of 80 but outside the sticking distance of 20.
        let d = tick(&mut m, &rig(), Vec2::new(240.0, 0.0));
        assert!(d.x < 240.0 && d.x > 200.0, "drawn closer to the magnet than the pointer put it: {d:?}");
        assert!(m.stuck().is_empty());
    }

    #[test]
    fn close_enough_it_sticks_and_holds_until_pulled_far_away() {
        let mut m = Magnets::default();
        // 190 px right: 10 px from the magnet.
        let d = tick(&mut m, &rig(), Vec2::new(190.0, 0.0));
        assert!((d.x - 200.0).abs() < 1e-6, "snapped onto the magnet: {d:?}");
        assert_eq!(m.stuck().len(), 1);
        // Pulled 100 px away (within break-away 120): still stuck on the magnet.
        let held = tick(&mut m, &rig(), Vec2::new(100.0, 0.0));
        assert!((held.x - 200.0).abs() < 1e-6, "{held:?}");
        // Pulled back to the start (200 px away): freed.
        let freed = tick(&mut m, &rig(), Vec2::new(5.0, 0.0));
        assert!((freed.x - 5.0).abs() < 1e-6, "{freed:?}");
        assert!(m.stuck().is_empty());
    }

    #[test]
    fn no_magnets_no_effect_and_restriction_by_group() {
        let mut m = Magnets::default();
        let bare = Rig::default().with_handle(RigHandle::new("hand/Draggable", HandleKind::Draggable, Point::new(100.0, 100.0)));
        assert_eq!(tick(&mut m, &bare, Vec2::new(190.0, 0.0)), Vec2::new(190.0, 0.0));
        let mut only_feet = Magnets { params: MagnetsParams { only: "foot".into(), ..MagnetsParams::default() }, ..Magnets::default() };
        assert_eq!(tick(&mut only_feet, &rig(), Vec2::new(190.0, 0.0)), Vec2::new(190.0, 0.0));
    }
}
