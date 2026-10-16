//! Motion Lines (this program's own `Charcraft.MotionLines.js`): groups named `Trails` leave fading copies of
//! themselves behind while they move fast, like the streaks behind a swinging fist.

use std::collections::VecDeque;

use anim_core::{Point, Vec2};
use charcraft_puppet::{PuppetPose, Sprite, Xform};
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::percent;

/// The tag of the layers that leave trails.
pub const TRAILS_TAG: &str = "Adobe.MotionLines.Trails";

/// Parameters of Motion Lines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MotionLinesParams {
    /// How many copies trail behind.
    pub count: f64,
    /// Seconds between copies.
    pub interval: f64,
    /// Opacity of the newest copy, percent.
    pub opacity: f64,
    /// Size of the oldest copy relative to the newest, percent.
    #[serde(rename = "tailScale")]
    pub tail_scale: f64,
    /// The speed below which no trail shows, pixels per second.
    #[serde(rename = "minSpeed")]
    pub min_speed: f64,
}

impl Default for MotionLinesParams {
    fn default() -> Self {
        Self { count: 5.0, interval: 0.04, opacity: 60.0, tail_scale: 70.0, min_speed: 150.0 }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "count", "", "How many copies trail behind.";
    "interval", "s", "Seconds between copies.";
    "opacity", "%", "Opacity of the newest copy.";
    "tailScale", "%", "Size of the oldest copy.";
    "minSpeed", "px/s", "The speed below which no trail shows.";
};

#[derive(Debug, Clone, PartialEq, Default)]
struct Trail {
    /// Past places, newest first, with the time they were taken.
    places: VecDeque<(f64, Point)>,
    speed: f64,
}

/// Leaves fading copies of fast-moving `Trails` layers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MotionLines {
    /// The parameters.
    pub params: MotionLinesParams,
    trails: Vec<Trail>,
}

impl Behavior for MotionLines {
    behavior_common!("Charcraft.MotionLines.js", "Motion Lines", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = self.params.clone();
        let count = anim_core::num::round_f64_to_u32(p.count.clamp(0.0, 30.0)).min(30);
        let interval = p.interval.max(1e-3);
        let anchors: Vec<_> = cx.rig.anchors_with(TRAILS_TAG).collect();
        self.trails.resize_with(anchors.len(), Trail::default);
        for (anchor, trail) in anchors.iter().zip(&mut self.trails) {
            let now = anchor.driven_position(pose);
            if trail.places.front().is_some_and(|(t, _)| cx.time < *t) {
                trail.places.clear();
            }
            let pushed = trail.places.front().is_none_or(|(t, _)| cx.time - t >= interval);
            if pushed {
                trail.places.push_front((cx.time, now));
                trail.places.truncate(usize::try_from(count).unwrap_or(0).saturating_add(1));
            }
            let speed = trail.places.iter().find(|(t, _)| cx.time - t > 1e-9).map_or(0.0, |(t, q)| (now - *q).hypot() / (cx.time - t).max(1e-3));
            trail.speed = trail.speed.mul_add(0.5, speed * 0.5);
            if trail.speed < p.min_speed {
                continue;
            }
            // A place stored on this very tick is where the layer is now, not a copy. The rest run back in time.
            for (i, (_, place)) in trail.places.iter().skip(usize::from(pushed)).take(usize::try_from(count).unwrap_or(0)).enumerate() {
                let age = f64::from(u32::try_from(i).unwrap_or(0)) / f64::from(count.max(1));
                let size = 1.0 - (1.0 - percent(p.tail_scale)) * age;
                pose.sprites.push(Sprite {
                    layer: anchor.layer.clone(),
                    xform: Xform { translation: *place - now, rotation: 0.0, scale: Vec2::new(size.max(0.05), size.max(0.05)) },
                    opacity: (percent(p.opacity) * (1.0 - age)).clamp(0.0, 1.0),
                    shade: 0.0,
                });
            }
        }
    }

    fn reset(&mut self, _seed: u64) {
        self.trails.clear();
    }
}

#[cfg(test)]
mod tests {
    use anim_core::Tick;

    use super::*;
    use crate::input::Inputs;
    use crate::rig::{ChainLink, Rig, RigAnchor};

    /// A trailing layer carried by a group that the pose moves along x at `speed` px/s.
    fn rig() -> Rig {
        let mut anchor = RigAnchor::new(TRAILS_TAG, "Fist", Point::new(100.0, 100.0));
        anchor.chain = vec![ChainLink::named("arm", Point::new(100.0, 100.0))];
        Rig::default().with_anchor(anchor)
    }

    fn run(m: &mut MotionLines, speed: f64, seconds: f64) -> PuppetPose {
        let rig = rig();
        let mut pose = PuppetPose::neutral();
        for f in 0..(seconds * 60.0) as u32 {
            let t = f64::from(f) / 60.0;
            pose = PuppetPose::neutral();
            pose.groups.insert("arm".into(), Xform::moved(speed * t, 0.0));
            m.step(&Cx { inputs: &Inputs::at(Tick::from_seconds_f64(t)), rig: &rig, time: t, dt: 1.0 / 60.0 }, &mut pose);
        }
        pose
    }

    #[test]
    fn a_fast_layer_leaves_older_copies_behind_it_that_fade() {
        let mut m = MotionLines::default();
        let pose = run(&mut m, 600.0, 1.0);
        assert_eq!(pose.sprites.len(), 5, "{:?}", pose.sprites.len());
        assert!(pose.sprites.iter().all(|s| s.xform.translation.x < 0.0), "copies are behind the layer");
        let xs: Vec<f64> = pose.sprites.iter().map(|s| s.xform.translation.x).collect();
        assert!(xs.windows(2).all(|w| w[1] < w[0]), "older ones are further back: {xs:?}");
        let alphas: Vec<f64> = pose.sprites.iter().map(|s| s.opacity).collect();
        assert!(alphas.windows(2).all(|w| w[1] < w[0]), "and fainter: {alphas:?}");
    }

    #[test]
    fn a_slow_or_still_layer_shows_no_trail() {
        let mut m = MotionLines::default();
        assert!(run(&mut m, 20.0, 1.0).sprites.is_empty());
        let mut still = MotionLines::default();
        assert!(run(&mut still, 0.0, 1.0).sprites.is_empty());
    }
}
