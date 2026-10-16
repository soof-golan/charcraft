//! Evaluating a pose into a display list.

use anim_core::{Affine, ColorTransform, Point, Rect, Vec2};
use anim_display::{DisplayList, Group, Mesh, Node};

use crate::model::{AttachStyle, HandleKind, Layer, LayerId, LayerKind, Puppet};
use crate::pose::{PuppetPose, Sprite, Xform};
use crate::tags::{self, ids};
use crate::warp::{self, Warp, Weld};

/// The tag of the handle a head attaches to the body at.
const NECK_TAG: &str = "Adobe.Walk.Neck";
/// A child group of a warped group follows the warp only by what its displacement exceeds this many pixels.
const CHILD_DEAD_ZONE: f64 = 1.5;
/// Deepest layer nesting evaluated.
const MAX_DEPTH: usize = 128;
/// Eyebrows move by this fraction of their own height at full raise.
const EYEBROW_REACH: f64 = 0.9;
/// Breathing grows layers by this fraction at full breath.
const BREATHE_GROWTH: f64 = 0.03;
/// Blink layers show from this closed amount.
const BLINK_THRESHOLD: f64 = 0.5;

/// A layer's emitted node, with the group it stands for when it is an independent child.
struct Emitted {
    node: Node,
    child: Option<LayerId>,
}

struct Frame<'a> {
    puppet: &'a Puppet,
    pose: &'a PuppetPose,
    visible: Vec<bool>,
    /// For each of the puppet's warps: its deformed vertices, when the pose moves it.
    solved: Vec<Option<Vec<Point>>>,
}

impl Puppet {
    /// Turns a pose into a display list in artwork pixels: independent groups transformed about their origins,
    /// mesh-warped groups as [`Mesh`] nodes, mouth shapes and blinks swapped by visibility. The neutral pose draws
    /// the artwork unchanged. Draw it with [`Puppet::images`].
    #[must_use]
    pub fn evaluate(&self, pose: &PuppetPose) -> DisplayList {
        let mut frame = Frame { puppet: self, pose, visible: self.visibility(pose), solved: Vec::new() };
        let solved = self.warps.iter().map(|w| frame.solve_warp(w)).collect();
        frame.solved = solved;
        let mut list = DisplayList::new(self.size, None);
        if let Some(node) = frame.emit(LayerId::ROOT, false, 0) {
            list.nodes.push(node);
        }
        list.nodes.extend(pose.sprites.iter().filter_map(|sprite| self.sprite_node(sprite)));
        list
    }

    /// The node of one extra copy of a layer: the layer at rest, moved by the sprite's own transform.
    fn sprite_node(&self, sprite: &Sprite) -> Option<Node> {
        if !sprite.opacity.is_finite() || sprite.opacity <= 0.0 {
            return None;
        }
        let key = tags::normalize(&sprite.layer);
        let layer = self.layers.iter().find(|l| l.path == sprite.layer || (!l.label.is_empty() && tags::normalize(&l.label) == key))?;
        let rest = PuppetPose::neutral();
        let mut visible: Vec<bool> = self.layers.iter().map(|l| l.visible).collect();
        if let Some(v) = visible.get_mut(layer.id.index()) {
            *v = true;
        }
        let frame = Frame { puppet: self, pose: &rest, visible, solved: vec![None; self.warps.len()] };
        let inner = frame.emit(layer.id, false, 0)?;
        let shade = sprite.shade.clamp(0.0, 1.0);
        let alpha = anim_core::num::f64_to_f32(sprite.opacity.clamp(0.0, 1.0));
        let color = if shade > 0.0 {
            let keep = anim_core::num::f64_to_f32(1.0 - shade);
            ColorTransform { mul: [keep, keep, keep, alpha], add: [0.0; 4] }
        } else {
            ColorTransform::alpha(alpha)
        };
        Some(Node::Group(Group { transform: sprite.xform.about(layer.origin), color, children: vec![inner], ..Group::default() }))
    }

    /// Which layers the pose shows (before their ancestors are considered): the artwork's own visibility with the
    /// mouth, blink, swap, trigger and override switches applied.
    #[must_use]
    pub fn visibility(&self, pose: &PuppetPose) -> Vec<bool> {
        let mut visible: Vec<bool> = self.layers.iter().map(|l| l.visible).collect();
        let mut set = |id: LayerId, on: bool| {
            if let Some(v) = visible.get_mut(id.index()) {
                *v = on;
            }
        };

        if let Some(wanted) = pose.viseme {
            for group in self.mouth_groups() {
                let shapes = self.mouth_shapes(group);
                let chosen =
                    shapes.iter().find(|(v, _)| *v == wanted).or_else(|| shapes.iter().find(|(v, _)| *v == crate::tags::Viseme::Neutral)).map(|(_, id)| *id);
                // Neither the shape nor a neutral one exists: keep what the artwork shows rather than an empty mouth.
                if chosen.is_none() {
                    continue;
                }
                for (_, id) in shapes {
                    set(id, Some(id) == chosen);
                }
            }
        }

        for layer in &self.layers {
            let amount = if layer.has_tag(ids::LEFT_BLINK) {
                pose.blink_left
            } else if layer.has_tag(ids::RIGHT_BLINK) {
                pose.blink_right
            } else {
                continue;
            };
            if amount.is_finite() && amount >= BLINK_THRESHOLD {
                // The blink layer replaces what else the eye shows.
                if let Some(parent) = layer.parent.and_then(|p| self.layer(p)) {
                    for &sibling in &parent.children {
                        set(sibling, sibling == layer.id);
                    }
                }
            }
        }

        for swap in &self.swap_sets {
            if let Some(choice) = pose.swaps.get(&swap.name) {
                let key = tags::normalize(choice);
                for &member in &swap.members {
                    let on = self.layer(member).is_some_and(|l| tags::normalize(&l.label) == key || l.path == *choice);
                    set(member, on);
                }
            }
        }

        for layer in &self.layers {
            let Some(cycle) = layer.cycle.filter(|_| layer.children.len() > 1) else { continue };
            let shown = cycle.frame(pose.time_millis, layer.children.len());
            for (i, &child) in layer.children.iter().enumerate() {
                set(child, i == shown);
            }
        }

        for layer in &self.layers {
            if let Some(key) = &layer.key_trigger {
                set(layer.id, pose.triggers.contains(key));
            }
        }

        // A layer that hides the others hides its siblings for as long as its key is down.
        for layer in &self.layers {
            let on = layer.hide_others && layer.key_trigger.as_ref().is_some_and(|k| pose.triggers.contains(k));
            if let (true, Some(parent)) = (on, layer.parent.and_then(|p| self.layer(p))) {
                for &sibling in parent.children.iter().filter(|&&s| s != layer.id) {
                    set(sibling, false);
                }
            }
        }

        for (name, on) in &pose.visibility {
            let key = tags::normalize(name);
            for layer in &self.layers {
                if layer.path == *name || (!layer.label.is_empty() && tags::normalize(&layer.label) == key) {
                    set(layer.id, *on);
                }
            }
        }
        visible
    }

    /// The change an independent group gets from the pose (before it is turned into a matrix).
    #[must_use]
    pub fn group_xform(&self, id: LayerId, pose: &PuppetPose) -> Xform {
        let Some(g) = self.layer(id) else { return Xform::IDENTITY };
        if !g.independent {
            return Xform::IDENTITY;
        }
        let mut x = Xform::IDENTITY;
        if id == LayerId::ROOT {
            x = x.then(&pose.root);
        }
        if g.has_tag(ids::HEAD) {
            x = x.then(&pose.head);
        }
        for (key, xf) in &pose.groups {
            if group_matches(g, key) {
                x = x.then(xf);
            }
        }
        let unit = |v: f64| if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 };
        let height = g.bounds.map_or(0.0, |b| b.height());
        if g.has_tag(ids::LEFT_EYEBROW) {
            x = x.then(&Xform::moved(0.0, -unit(pose.eyebrow_left) * EYEBROW_REACH * height));
        }
        if g.has_tag(ids::RIGHT_EYEBROW) {
            x = x.then(&Xform::moved(0.0, -unit(pose.eyebrow_right) * EYEBROW_REACH * height));
        }
        if g.has_tag(ids::LEFT_PUPIL) {
            x = x.then(&Xform::moved_by(self.pupil_offset(g, ids::LEFT_EYE, pose.pupil_left.unwrap_or(pose.gaze))));
        }
        if g.has_tag(ids::RIGHT_PUPIL) {
            x = x.then(&Xform::moved_by(self.pupil_offset(g, ids::RIGHT_EYE, pose.pupil_right.unwrap_or(pose.gaze))));
        }
        if g.has_tag(ids::BREATHE) && pose.breathe.is_finite() && pose.breathe != 0.0 {
            let s = 1.0 + BREATHE_GROWTH * pose.breathe.clamp(0.0, 1.0);
            x = x.then(&Xform { scale: Vec2::new(s, s), ..Xform::IDENTITY });
        }
        x
    }

    /// How far a pupil moves for a gaze: within what is left of the eye around the pupil.
    fn pupil_offset(&self, pupil: &Layer, eye_tag: &str, gaze: Vec2) -> Vec2 {
        let eye_bounds = self.ancestor_with_tag(pupil.id, eye_tag).or(pupil.parent).and_then(|id| self.shown_bounds(id));
        let (Some(eye), Some(own)) = (eye_bounds, self.shown_bounds(pupil.id).or(pupil.bounds)) else { return Vec2::ZERO };
        let range = Vec2::new(((eye.width() - own.width()) / 2.0).max(0.0), ((eye.height() - own.height()) / 2.0).max(0.0));
        let unit = |v: f64| if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 };
        Vec2::new(unit(gaze.x) * range.x, unit(gaze.y) * range.y)
    }

    /// Bounds of what a layer draws at rest: layers the artwork hides (closed lids, alternative mouths) and blink
    /// layers do not count, so a lid that is wider than the eye does not widen the eye.
    fn shown_bounds(&self, id: LayerId) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        let mut stack = vec![id];
        while let Some(at) = stack.pop() {
            let Some(l) = self.layer(at) else { continue };
            if !l.visible || l.has_tag(ids::LEFT_BLINK) || l.has_tag(ids::RIGHT_BLINK) {
                continue;
            }
            if l.kind == LayerKind::Group {
                stack.extend(l.children.iter().copied());
            } else if let Some(b) = l.bounds {
                out = Some(out.map_or(b, |o| o.union(b)));
            }
        }
        out
    }

    /// The closest ancestor (not the layer itself) carrying a tag.
    #[must_use]
    pub fn ancestor_with_tag(&self, id: LayerId, tag: &str) -> Option<LayerId> {
        let mut at = self.layer(id)?.parent;
        while let Some(current) = at {
            let l = self.layer(current)?;
            if l.has_tag(tag) {
                return Some(current);
            }
            at = l.parent;
        }
        None
    }

    /// The matrix applied to an independent group in the pose.
    #[must_use]
    pub fn group_matrix(&self, id: LayerId, pose: &PuppetPose) -> Affine {
        // A layer next to the head moves with it, about the head's pivot, on top of any move of its own.
        let own = self.own_matrix(id, pose);
        self.layer(id).and_then(|l| l.follows).map_or(own, |head| self.own_matrix(head, pose) * own)
    }

    fn own_matrix(&self, id: LayerId, pose: &PuppetPose) -> Affine {
        let x = self.group_xform(id, pose);
        if x.is_identity() { Affine::IDENTITY } else { x.about(self.pivot(id)) }
    }

    /// The point an independent group rotates and scales about, and attaches to the body at: its handle tagged
    /// `Adobe.Walk.Neck` (where Character Animator attaches a head), else its origin.
    #[must_use]
    pub fn pivot(&self, id: LayerId) -> Point {
        let Some(layer) = self.layer(id) else { return Point::ORIGIN };
        layer.handles.iter().find(|h| h.tags.iter().any(|t| t == NECK_TAG)).map_or(layer.origin, |h| h.position)
    }

    /// The mesh of a group's warp in the pose: rest vertices and deformed vertices, when the pose moves the warp
    /// (pulls its handles or moves a group welded to it). For tools and tests.
    #[must_use]
    pub fn warp_solution(&self, group: LayerId, pose: &PuppetPose) -> Option<(Vec<Point>, Vec<Point>)> {
        let frame = Frame { puppet: self, pose, visible: self.visibility(pose), solved: Vec::new() };
        let warp = self.warps.iter().find(|w| w.group == group)?;
        Some((warp.mesh.vertices.clone(), frame.solve_warp(warp)?))
    }

    /// Bounds of the pose's drawing, for fitting a view.
    #[must_use]
    pub fn rest_bounds(&self) -> Option<Rect> {
        self.layers.first().and_then(|l| l.bounds)
    }
}

impl Xform {
    pub(crate) const fn moved_by(v: Vec2) -> Self {
        Self::moved(v.x, v.y)
    }
}

fn group_matches(g: &Layer, key: &str) -> bool {
    g.has_tag(key) || g.path == key || (!g.label.is_empty() && tags::normalize(&g.label) == tags::normalize(key))
}

impl Frame<'_> {
    fn is_visible(&self, id: LayerId) -> bool {
        self.visible.get(id.index()).copied().unwrap_or(false)
    }

    /// The product of the pose's opacity entries that name the layer (by path or label).
    fn pose_opacity(&self, layer: &Layer) -> f32 {
        if self.pose.opacity.is_empty() {
            return 1.0;
        }
        let product = self
            .pose
            .opacity
            .iter()
            .filter(|(name, _)| layer.path == **name || (!layer.label.is_empty() && tags::normalize(&layer.label) == tags::normalize(name)))
            .map(|(_, v)| if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 })
            .product::<f64>();
        anim_core::num::f64_to_f32(product)
    }

    fn emit(&self, id: LayerId, shape_only: bool, depth: usize) -> Option<Node> {
        if depth > MAX_DEPTH || !self.is_visible(id) {
            return None;
        }
        let puppet = self.puppet;
        let layer = puppet.layer(id)?;
        let content = puppet.contents.get(id.index())?;
        let mut group = Group { key: Some(u64::from(id.0)), ..Group::default() };
        if !shape_only {
            group.color = ColorTransform::alpha((layer.opacity * self.pose_opacity(layer)).clamp(0.0, 1.0));
            group.blend = layer.blend;
        }
        group.mask.clone_from(&content.mask);
        group.children = if layer.kind == LayerKind::Group { self.emit_children(layer, depth.saturating_add(1)) } else { content.nodes.clone() };
        if layer.independent || layer.follows.is_some() {
            group.transform = puppet.group_matrix(id, self.pose);
        }
        Some(Node::Group(group))
    }

    /// The children of a layer as nodes, mesh-warped when the layer has a warp the pose moves.
    fn emit_children(&self, layer: &Layer, depth: usize) -> Vec<Node> {
        let mut entries: Vec<Emitted> = Vec::with_capacity(layer.children.len());
        let mut iter = layer.children.iter().copied().peekable();
        while let Some(base_id) = iter.next() {
            let Some(base) = self.puppet.layer(base_id) else { continue };
            let mut clipped = Vec::new();
            if !base.clipping {
                while let Some(next) = iter.next_if(|c| self.puppet.layer(*c).is_some_and(|l| l.clipping)) {
                    clipped.push(next);
                }
            }
            // A layer that follows the head is not part of the mesh it sits in: it moves as a whole.
            let child = (base.independent || base.follows.is_some()).then_some(base_id);
            if clipped.is_empty() {
                if let Some(node) = self.emit(base_id, false, depth) {
                    entries.push(Emitted { node, child });
                }
                continue;
            }
            let Some(base_node) = self.emit(base_id, false, depth) else { continue };
            let shape: Vec<Node> = self.emit(base_id, true, depth).into_iter().collect();
            let content: Vec<Node> = clipped.into_iter().filter_map(|c| self.emit(c, false, depth)).collect();
            let node = Node::Group(Group {
                children: vec![base_node, Node::Group(Group { mask: Some(shape), children: content, ..Group::default() })],
                ..Group::default()
            });
            entries.push(Emitted { node, child });
        }

        if let Some((warp, positions)) = self.solved_warp(layer) {
            return self.apply_warp(&warp.mesh, positions, &warp.welded, entries);
        }
        entries.into_iter().map(|e| e.node).collect()
    }

    /// The warp of a group and its deformed vertices, when the pose moves it.
    fn solved_warp(&self, layer: &Layer) -> Option<(&Warp, &[Point])> {
        let i = self.puppet.warps.iter().position(|w| w.group == layer.id)?;
        Some((self.puppet.warps.get(i)?, self.solved.get(i)?.as_deref()?))
    }

    /// Deformed vertices of a warp when the pose pulls any of its handles or moves a group welded to it.
    fn solve_warp(&self, warp: &Warp) -> Option<Vec<Point>> {
        let layer = self.puppet.layer(warp.group)?;
        let mut moved = false;
        let targets: Vec<Point> = warp
            .handle_indices
            .iter()
            .zip(&warp.pin_offsets)
            .filter_map(|(&i, &offset)| layer.handles.get(i).map(|h| (h, offset)))
            .map(|(h, offset)| {
                let d = if h.kind == HandleKind::Fixed { Vec2::ZERO } else { self.pose.drag_of(&h.key) };
                moved |= d != Vec2::ZERO;
                h.position + offset + d
            })
            .collect();
        let matrices: Vec<Option<Affine>> = warp.welded.iter().map(|&weld| self.weld_matrix(warp, weld)).collect();
        moved |= matrices.iter().any(Option::is_some);
        moved.then(|| warp.solve(&targets, &matrices))
    }

    /// How the pose moves a welded group, in the space of the mesh it is welded to (`None` when it stays).
    fn weld_matrix(&self, warp: &Warp, weld: Weld) -> Option<Affine> {
        if !self.is_visible(weld.child) || self.puppet.group_xform(weld.child, self.pose).is_identity() {
            return None;
        }
        let posed = self.puppet.group_matrix(weld.child, self.pose);
        Some(if weld.sibling { self.puppet.group_matrix(warp.group, self.pose).inverse() * posed } else { posed })
    }

    fn apply_warp(&self, warp_mesh: &anim_rig::Mesh, positions: &[Point], welded: &[Weld], entries: Vec<Emitted>) -> Vec<Node> {
        let mut out = Vec::new();
        let mut run: Vec<Node> = Vec::new();
        let flush = |run: &mut Vec<Node>, out: &mut Vec<Node>| {
            if run.is_empty() {
                return;
            }
            out.push(Node::Mesh(Mesh {
                transform: Affine::IDENTITY,
                content: std::mem::take(run),
                source: warp_mesh.vertices.clone(),
                target: positions.to_vec(),
                triangles: warp_mesh.triangles.clone(),
            }));
        };
        for entry in entries {
            match entry.child.and_then(|c| self.puppet.layer(c)) {
                Some(child) => {
                    flush(&mut run, &mut out);
                    let held = welded.iter().any(|w| w.child == child.id) || child.attach == AttachStyle::Free || child.follows.is_some();
                    let d = if held { Vec2::ZERO } else { warp::displacement(warp_mesh, positions, child.origin) };
                    // A displacement of a fraction of a pixel is the mesh's long triangles, not a move: resampling the
                    // child for it would blur it. The dead zone is soft, so a child never jumps.
                    let moved = d.hypot();
                    let d = if moved > CHILD_DEAD_ZONE { d * ((moved - CHILD_DEAD_ZONE) / moved) } else { Vec2::ZERO };
                    out.push(if d == Vec2::ZERO {
                        entry.node
                    } else {
                        Node::Group(Group { transform: Affine::translate(d), children: vec![entry.node], ..Group::default() })
                    });
                }
                None => run.push(entry.node),
            }
        }
        flush(&mut run, &mut out);
        out
    }
}
