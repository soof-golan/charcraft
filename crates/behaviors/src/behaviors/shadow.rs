//! Shadow (this program's own `Charcraft.Shadow.js`): the layer named `Shadow` stays on the floor under the
//! puppet, follows it sideways, and shrinks and fades as the puppet leaves the ground.

use anim_core::Vec2;
use charcraft_puppet::tags::ids;
use charcraft_puppet::{PuppetPose, Sprite, Xform};
use serde::{Deserialize, Serialize};

use crate::behavior::{Behavior, Cx, ParamDoc, behavior_common, docs};
use crate::util::percent;

/// Parameters of Shadow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShadowParams {
    /// How much of the puppet's sideways movement the shadow follows, percent.
    pub follow: f64,
    /// The height, pixels, at which the shadow is gone.
    #[serde(rename = "fadeHeight")]
    pub fade_height: f64,
    /// How much the shadow shrinks at that height, percent.
    pub shrink: f64,
    /// The shadow's opacity on the ground, percent.
    pub opacity: f64,
    /// Paint the shadow black whatever colour the artwork is, percent.
    pub darken: f64,
}

impl Default for ShadowParams {
    fn default() -> Self {
        Self { follow: 100.0, fade_height: 300.0, shrink: 40.0, opacity: 100.0, darken: 0.0 }
    }
}

const DOCS: &[ParamDoc] = docs! {
    "follow", "%", "How much of the puppet's sideways movement the shadow follows.";
    "fadeHeight", "px", "The height at which the shadow is gone.";
    "shrink", "%", "How much the shadow shrinks at that height.";
    "opacity", "%", "Opacity on the ground.";
    "darken", "%", "Paint the shadow black, whatever the artwork's colour.";
};

/// Keeps a shadow under the puppet.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shadow {
    /// The parameters.
    pub params: ShadowParams,
}

impl Behavior for Shadow {
    behavior_common!("Charcraft.Shadow.js", "Shadow", DOCS);

    fn step(&mut self, cx: &Cx<'_>, pose: &mut PuppetPose) {
        let p = self.params.clone();
        let root = pose.root.sanitized();
        // Up is negative y: only a puppet above the ground casts a smaller shadow.
        let lift = (-root.translation.y).max(0.0);
        let k = if p.fade_height > 1e-6 { (lift / p.fade_height).clamp(0.0, 1.0) } else { 0.0 };
        for anchor in cx.rig.anchors_with(ids::SHADOW) {
            let size = 1.0 - percent(p.shrink).clamp(0.0, 1.0) * k;
            pose.visibility.insert(anchor.layer.clone(), false);
            pose.sprites.push(Sprite {
                layer: anchor.layer.clone(),
                xform: Xform { translation: Vec2::new(root.translation.x * percent(p.follow), 0.0), rotation: 0.0, scale: Vec2::new(size, size) },
                opacity: (percent(p.opacity) * (1.0 - k)).clamp(0.0, 1.0),
                shade: percent(p.darken).clamp(0.0, 1.0),
            });
        }
    }

    fn reset(&mut self, _seed: u64) {}
}

#[cfg(test)]
mod tests {
    use anim_core::{Point, Tick};

    use super::*;
    use crate::input::Inputs;
    use crate::rig::{Rig, RigAnchor};

    fn pose_with_root(x: f64, y: f64) -> PuppetPose {
        let rig = Rig::default().with_anchor(RigAnchor::new(ids::SHADOW, "Blob", Point::new(500.0, 900.0)));
        let mut pose = PuppetPose::neutral();
        pose.root = Xform::moved(x, y);
        Shadow::default().step(&Cx { inputs: &Inputs::at(Tick::ZERO), rig: &rig, time: 0.0, dt: 0.0 }, &mut pose);
        pose
    }

    #[test]
    fn the_shadow_follows_sideways_and_stays_on_the_ground() {
        let pose = pose_with_root(120.0, 0.0);
        let s = &pose.sprites[0];
        assert_eq!(s.xform.translation, Vec2::new(120.0, 0.0));
        assert_eq!((s.opacity, s.xform.scale.x), (1.0, 1.0));
        assert_eq!(pose.visibility.get("Blob"), Some(&false));
    }

    #[test]
    fn a_jump_shrinks_and_fades_it() {
        let low = &pose_with_root(0.0, -100.0).sprites[0].clone();
        let high = &pose_with_root(0.0, -250.0).sprites[0].clone();
        assert!(low.xform.scale.x < 1.0 && high.xform.scale.x < low.xform.scale.x);
        assert!(high.opacity < low.opacity && low.opacity < 1.0);
        assert_eq!(pose_with_root(0.0, -900.0).sprites.len(), 1);
        assert_eq!(pose_with_root(0.0, -900.0).sprites[0].opacity, 0.0);
    }

    #[test]
    fn no_shadow_layer_means_no_shadow() {
        let mut pose = PuppetPose::neutral();
        Shadow::default().step(&Cx { inputs: &Inputs::at(Tick::ZERO), rig: &Rig::default(), time: 0.0, dt: 0.0 }, &mut pose);
        assert!(pose.is_neutral());
    }
}
