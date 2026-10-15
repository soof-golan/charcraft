//! Dragger (`Adobe.Dragger3.js`): lets the mouse or a finger pull draggable handles.

use std::collections::{BTreeMap, BTreeSet};

use anim_core::Vec2;
use charcraft_puppet::{HandleKind, PuppetPose};
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::{percent, smoothstep};

/// What a handle does when the button comes up.
pub const ON_MOUSE_UP_STAY: f64 = 0.0;
/// What a handle does when the button comes up: spring back to its rest position.
pub const ON_MOUSE_UP_RETURN: f64 = 1.0;

/// Parameters of Dragger, named as Character Animator stores them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DraggerParams {
    /// Seconds a released handle takes to return to rest.
    #[serde(rename = "handleReleaseDuration")]
    pub handle_release_duration: f64,
    /// Limits how far a handle can be pulled to `range`.
    pub limit_range_cb: bool,
    /// 0 keeps the handle where it was dropped, 1 returns it to rest.
    #[serde(rename = "onMouseUp")]
    pub on_mouse_up: f64,
    /// The limit, percent of the handle's reach (the size of its group).
    pub range: f64,
    /// Draw the drag targets (UI only).
    #[serde(rename = "showTargets")]
    pub show_targets: bool,
    /// Handle paths are relative to the behavior (stored, no effect).
    #[serde(rename = "useBehaviorRelativeHandlePathsV2")]
    pub use_behavior_relative_handle_paths_v2: bool,
}

impl Default for DraggerParams {
    fn default() -> Self {
        Self {
            handle_release_duration: 0.2,
            limit_range_cb: false,
            on_mouse_up: ON_MOUSE_UP_RETURN,
            range: 20.0,
            show_targets: false,
            use_behavior_relative_handle_paths_v2: true,
        }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "handleReleaseDuration", "s", "Time a released handle takes to glide back to rest.";
    "limit_range_cb", "", "Switch on the range limit.";
    "onMouseUp", "", "0 stays where it was dropped, 1 returns to rest.";
    "range", "%", "Largest pull as a percentage of the handle's reach, when limited.";
};

#[derive(Debug, Clone, PartialEq)]
struct Drag {
    pointer: Option<u32>,
    /// Pointer position minus the handle's position at the moment of the grab.
    grab_offset: Vec2,
    /// Displacement on stage.
    delta: Vec2,
    released_at: f64,
    release_from: Vec2,
}

/// Pulls `Draggable` handles with pointers. A pointer that goes down within the handle's grab radius takes the
/// nearest free handle and moves it until it comes up.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dragger {
    /// The parameters.
    pub params: DraggerParams,
    drags: BTreeMap<String, Drag>,
    down: BTreeSet<u32>,
}

impl Dragger {
    /// Keys of the handles currently held by a pointer.
    #[must_use]
    pub fn held(&self) -> Vec<&str> {
        self.drags.iter().filter(|(_, d)| d.pointer.is_some()).map(|(k, _)| k.as_str()).collect()
    }
}

impl Behavior for Dragger {
    behavior_common!("Adobe.Dragger3.js", "Dragger", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let t = cx.time;
        let p = self.params.clone();
        let duration = p.handle_release_duration.max(0.0);

        // Pointers that went away or came up release what they held.
        let now_down: BTreeSet<u32> = cx.inputs.pointers.iter().filter(|q| q.down).map(|q| q.id).collect();
        for d in self.drags.values_mut() {
            if let Some(id) = d.pointer
                && !now_down.contains(&id)
            {
                d.pointer = None;
                if p.on_mouse_up >= 0.5 {
                    d.released_at = t;
                    d.release_from = d.delta;
                }
            }
        }

        // New presses take the nearest free draggable handle in reach.
        for ptr in cx.inputs.pointers.iter().filter(|q| q.down && !self.down.contains(&q.id)) {
            let free = |key: &str| self.drags.get(key).is_none_or(|d| d.pointer.is_none());
            let best = cx
                .rig
                .handles_of(HandleKind::Draggable)
                .filter(|h| free(&h.key))
                .map(|h| (h, (ptr.position - h.driven_position(pose)).hypot()))
                .filter(|(h, dist)| *dist <= h.grab_radius)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((h, _)) = best {
                let grab_offset = ptr.position - h.driven_position(pose);
                // Grabbing a handle that is still gliding home picks it up where it is.
                let delta = self.drags.get(&h.key).map_or(Vec2::ZERO, |d| d.delta);
                self.drags.insert(h.key.clone(), Drag { pointer: Some(ptr.id), grab_offset: grab_offset - delta, delta, released_at: t, release_from: delta });
            }
        }
        self.down = now_down;

        let mut finished = Vec::new();
        for (key, d) in &mut self.drags {
            let Some(h) = cx.rig.handles.iter().find(|h| h.key == *key) else {
                finished.push(key.clone());
                continue;
            };
            if let Some(id) = d.pointer {
                if let Some(ptr) = cx.inputs.pointers.iter().find(|q| q.id == id) {
                    let mut delta = ptr.position - d.grab_offset - h.driven_position(pose);
                    if p.limit_range_cb {
                        let max = (h.reach * percent(p.range)).max(0.0);
                        if delta.hypot() > max {
                            delta = delta.normalize() * max;
                        }
                    }
                    if delta.x.is_finite() && delta.y.is_finite() {
                        d.delta = delta;
                    }
                }
            } else if p.on_mouse_up >= 0.5 {
                let k = if duration > 1e-9 { smoothstep((t - d.released_at) / duration) } else { 1.0 };
                d.delta = d.release_from * (1.0 - k);
                if k >= 1.0 {
                    finished.push(key.clone());
                    continue;
                }
            }
            let local = h.to_group_space(pose, d.delta);
            if local != Vec2::ZERO {
                pose.drag.insert(key.clone(), pose.drag_of(key) + local);
            }
        }
        for key in finished {
            self.drags.remove(&key);
        }
    }

    fn reset(&mut self, _seed: u64) {
        self.drags.clear();
        self.down.clear();
    }
}
