//! The puppet model: an arena of layers with tags, handles, swap sets and the warps built from them.

use std::collections::BTreeSet;

use anim_core::{BlendMode, Point, Rect, Size};
use anim_display::{ImageStore, Node};
use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::tags::{self, Viseme, ids};
use crate::warp::Warp;

/// Index of a layer in [`Puppet::layers`]. Layer 0 is the artwork root, which is the puppet itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LayerId(pub u32);

impl LayerId {
    /// The artwork root.
    pub const ROOT: Self = Self(0);

    pub(crate) fn index(self) -> usize {
        anim_core::num::u32_to_usize(self.0)
    }
}

/// What a layer holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerKind {
    /// Child layers.
    Group,
    /// Pixels.
    Raster,
    /// Vector drawing.
    Vector,
}

/// What a handle does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandleKind {
    /// An attach point that is nothing special (where a child group is fixed to this one).
    Attach,
    /// Held in place while the rest of the group moves.
    Fixed,
    /// Follows the mouse or a dragger target.
    Draggable,
    /// Swings freely under physics.
    Dangle,
    /// A rigid link between two points (named `Stick`).
    Stick,
    /// A joint of the walk cycle.
    Walk,
}

impl HandleKind {
    /// True when the handle pins the group's mesh.
    #[must_use]
    pub const fn pins_mesh(self) -> bool {
        matches!(self, Self::Fixed | Self::Draggable | Self::Dangle)
    }
}

/// How an independent group is attached to the mesh of the group it sits in (Character Animator's attach styles).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachStyle {
    /// Welded into the parent's mesh: the mesh around the attach point follows the group, so the join stays
    /// seamless (the default).
    #[default]
    Weld,
    /// Pivots about its attach point, which rides on the parent's mesh without pulling it.
    Hinge,
    /// Not attached: the parent's mesh never moves it.
    Free,
}

impl AttachStyle {
    /// All styles, in menu order.
    pub const ALL: [Self; 3] = [Self::Weld, Self::Hinge, Self::Free];

    /// The name shown in menus.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Weld => "Weld",
            Self::Hinge => "Hinge",
            Self::Free => "Free",
        }
    }
}

/// How a layer cycle runs once it reaches its last child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CycleMode {
    /// Starts again at the first child.
    #[default]
    Loop,
    /// Runs back and forth.
    PingPong,
    /// Stops on the last child.
    Once,
}

impl CycleMode {
    /// All modes, in menu order.
    pub const ALL: [Self; 3] = [Self::Loop, Self::PingPong, Self::Once];

    /// The name shown in menus.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Loop => "Loop",
            Self::PingPong => "Ping-pong",
            Self::Once => "Once",
        }
    }
}

/// A layer cycle: the children of a group are shown one after the other, as an animated sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycleDef {
    /// How long each child stays, in milliseconds.
    pub step_millis: u32,
    /// What happens at the end.
    #[serde(default)]
    pub mode: CycleMode,
}

impl CycleDef {
    /// Shortest step accepted.
    pub const MIN_STEP: u32 = 10;
    /// Longest step accepted.
    pub const MAX_STEP: u32 = 60_000;

    /// A cycle that moves on every `step_millis` and loops.
    #[must_use]
    pub const fn looping(step_millis: u32) -> Self {
        Self { step_millis, mode: CycleMode::Loop }
    }

    /// The index of the child shown `time_millis` after the start, among `count` children.
    #[must_use]
    pub fn frame(&self, time_millis: u64, count: usize) -> usize {
        if count <= 1 {
            return 0;
        }
        let step = u64::from(self.step_millis.clamp(Self::MIN_STEP, Self::MAX_STEP));
        let n = u64::try_from(count).unwrap_or(u64::MAX);
        let tick = time_millis.checked_div(step).unwrap_or(0);
        let at = match self.mode {
            CycleMode::Loop => tick.checked_rem(n).unwrap_or(0),
            CycleMode::Once => tick.min(n.saturating_sub(1)),
            CycleMode::PingPong => {
                let period = n.saturating_mul(2).saturating_sub(2).max(1);
                let p = tick.checked_rem(period).unwrap_or(0);
                if p < n { p } else { period.saturating_sub(p) }
            }
        };
        usize::try_from(at).unwrap_or(0)
    }
}

/// The range of [`Layer::mesh_density`].
pub const MESH_DENSITY: (f64, f64) = (0.5, 4.0);

/// A handle of an independent group: a named point in the artwork.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandleDef {
    /// The name (`Fixed 2`, `location`).
    pub name: String,
    /// `<group label>/<name>`, the key used by [`crate::PuppetPose::drag`].
    pub key: String,
    /// What it does.
    pub kind: HandleKind,
    /// Its rest position in artwork pixels.
    pub position: Point,
    /// Its tags.
    pub tags: Vec<String>,
    /// The far end of a stick (`position` is the near end); `None` for every other handle.
    #[serde(default)]
    pub end: Option<Point>,
}

/// How many pins a stick puts along its length.
pub const STICK_PINS: usize = 6;

impl HandleDef {
    /// True when the handle pins its group's mesh (a stick only does once it has two ends).
    #[must_use]
    pub const fn pins_mesh(&self) -> bool {
        self.kind.pins_mesh() || (matches!(self.kind, HandleKind::Stick) && self.end.is_some())
    }

    /// The rest positions the handle pins, each with its offset from `position` (a stick pins points along its
    /// length, every other handle just its position).
    #[must_use]
    pub fn pins(&self) -> Vec<(Point, anim_core::Vec2)> {
        match (self.kind, self.end) {
            (HandleKind::Stick, Some(end)) => (0..STICK_PINS)
                .map(|i| {
                    let t = f64::from(u32::try_from(i).unwrap_or(0)) / f64::from(u32::try_from(STICK_PINS.saturating_sub(1)).unwrap_or(1).max(1));
                    let at = self.position.lerp(end, t);
                    (at, at - self.position)
                })
                .collect(),
            _ => vec![(self.position, anim_core::Vec2::ZERO)],
        }
    }
}

/// One layer of the artwork with the roles its name and the project data give it.
#[expect(clippy::struct_excessive_bools, reason = "independent role flags")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// Index in the arena.
    pub id: LayerId,
    /// Enclosing layer (`None` for the root).
    pub parent: Option<LayerId>,
    /// Children, bottom-most first.
    pub children: Vec<LayerId>,
    /// The name exactly as the artwork has it (`+ Left eyebrow`).
    pub name: String,
    /// The name without the `+` marker and blanks.
    pub label: String,
    /// Names from the root down, joined with `/`; the root's own name is not part of it.
    pub path: String,
    /// What it holds.
    pub kind: LayerKind,
    /// An independent group: it has its own origin and moves on its own. The root always is.
    pub independent: bool,
    /// Whether the artwork shows it by default.
    pub visible: bool,
    /// 0 to 1.
    pub opacity: f32,
    /// How it is blended.
    pub blend: BlendMode,
    /// Clipped to the layer below (PSD).
    pub clipping: bool,
    /// Tag ids (see [`crate::tags::ids`]): from the name and from the project data.
    pub tags: BTreeSet<String>,
    /// Bounds of everything below it, in artwork pixels.
    pub bounds: Option<Rect>,
    /// The fixed point of an independent group's rotation and scale, in artwork pixels.
    pub origin: Point,
    /// Handles (independent groups only).
    pub handles: Vec<HandleDef>,
    /// The key that shows the layer (project data).
    pub key_trigger: Option<String>,
    /// While this layer is shown by its key trigger, its siblings are hidden.
    #[serde(default)]
    pub hide_others: bool,
    /// How this independent group attaches to the mesh of the group around it.
    #[serde(default)]
    pub attach: AttachStyle,
    /// The group's own mesh ignores the groups attached to it (they hinge instead of welding).
    #[serde(default)]
    pub warp_independent: bool,
    /// How finely the group's mesh is cut: 1 is the default, 2 doubles the vertices along each side.
    #[serde(default = "default_density")]
    pub mesh_density: f64,
    /// Shows the children one after the other.
    #[serde(default)]
    pub cycle: Option<CycleDef>,
    /// The head this layer moves with, when it sits next to the head instead of inside it (an ear drawn behind
    /// the face on its own layer). Derived from the artwork by `Puppet::refresh_followers`, never stored.
    #[serde(skip)]
    pub follows: Option<LayerId>,
}

/// The default [`Layer::mesh_density`].
const fn default_density() -> f64 {
    1.0
}

impl Layer {
    /// True when the layer has this tag.
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.contains(tag)
    }

    /// The mouth shape the layer shows, when it is one.
    #[must_use]
    pub fn viseme(&self) -> Option<Viseme> {
        Viseme::ALL.into_iter().find(|v| self.has_tag(v.tag_id()))
    }

    /// True for an independent group with a `Head` handle.
    #[must_use]
    pub fn is_head(&self) -> bool {
        self.independent && self.has_tag(ids::HEAD)
    }
}

/// Layers of which exactly one is shown (a hat set, a hand pose set).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapSet {
    /// The name used in [`crate::PuppetPose::swaps`].
    pub name: String,
    /// The alternatives.
    pub members: Vec<LayerId>,
}

/// What a layer draws on its own: converted once, cloned into each frame.
#[derive(Debug, Clone, Default)]
pub struct Content {
    /// The nodes of a raster or vector leaf (bottom-most first).
    pub nodes: Vec<Node>,
    /// The layer mask, in artwork pixels.
    pub mask: Option<Vec<Node>>,
}

/// A puppet: the artwork's layer tree, what its names mean and the meshes behind its handles.
#[derive(Debug, Clone)]
pub struct Puppet {
    /// The artwork canvas.
    pub size: Size,
    pub(crate) layers: Vec<Layer>,
    pub(crate) contents: Vec<Content>,
    pub(crate) images: ImageStore,
    pub(crate) warps: Vec<Warp>,
    /// Swap sets from the project data.
    pub swap_sets: Vec<SwapSet>,
    /// Things that could not be carried over (project layers missing from the artwork, meshes that failed).
    pub warnings: Vec<String>,
}

impl Puppet {
    /// All layers; the root is first and every parent comes before its children.
    #[must_use]
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// One layer.
    #[must_use]
    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.get(id.index())
    }

    pub(crate) fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.get_mut(id.index())
    }

    /// The root layer (the puppet).
    ///
    /// # Errors
    ///
    /// [`Error::Empty`] when the artwork had no layers (cannot happen for a built puppet).
    pub fn root(&self) -> Result<&Layer, Error> {
        self.layers.first().ok_or(Error::Empty)
    }

    /// The pixels the layers draw with; give it to the renderer together with the display list.
    #[must_use]
    pub const fn images(&self) -> &ImageStore {
        &self.images
    }

    /// The layer at a path of names (`Head/mouth/Oh`), starting below the root.
    #[must_use]
    pub fn find_path(&self, path: &str) -> Option<LayerId> {
        self.layers.iter().find(|l| l.path == path).map(|l| l.id)
    }

    /// The first layer whose label matches (case and blanks ignored).
    #[must_use]
    pub fn find_label(&self, label: &str) -> Option<LayerId> {
        let key = tags::normalize(label);
        self.layers.iter().find(|l| tags::normalize(&l.label) == key).map(|l| l.id)
    }

    /// Every layer with a tag, in tree order.
    #[must_use]
    pub fn with_tag(&self, tag: &str) -> Vec<LayerId> {
        self.layers.iter().filter(|l| l.has_tag(tag)).map(|l| l.id).collect()
    }

    /// The independent groups (the root first).
    pub fn independent_groups(&self) -> impl Iterator<Item = &Layer> {
        self.layers.iter().filter(|l| l.independent)
    }

    /// The nearest independent group at or above a layer.
    #[must_use]
    pub fn owner_group(&self, id: LayerId) -> Option<LayerId> {
        let mut at = Some(id);
        while let Some(current) = at {
            let layer = self.layer(current)?;
            if layer.independent {
                return Some(current);
            }
            at = layer.parent;
        }
        None
    }

    /// The mouth shapes a mouth group offers (its children that carry a viseme tag).
    #[must_use]
    pub fn mouth_shapes(&self, group: LayerId) -> Vec<(Viseme, LayerId)> {
        let Some(g) = self.layer(group) else {
            return Vec::new();
        };
        g.children.iter().filter_map(|&c| Some((self.layer(c)?.viseme()?, c))).collect()
    }

    /// Mouth groups: layers with at least one viseme child.
    #[must_use]
    pub fn mouth_groups(&self) -> Vec<LayerId> {
        self.layers.iter().filter(|l| !self.mouth_shapes(l.id).is_empty()).map(|l| l.id).collect()
    }

    /// The warps built for groups with mesh-pinning handles.
    #[must_use]
    pub fn warped_groups(&self) -> Vec<LayerId> {
        self.warps.iter().filter(|w| !w.handle_indices.is_empty()).map(|w| w.group).collect()
    }

    /// Adds a handle to an independent group, rebuilding the group's warp when the handle pins its mesh. The key is
    /// `<group label>/<name>`. Returns the key, or `None` when `group` is not an independent group.
    pub fn add_handle(&mut self, group: LayerId, name: &str, kind: HandleKind, position: Point) -> Option<String> {
        let layer = self.layer_mut(group).filter(|l| l.independent)?;
        let key = format!("{}/{name}", layer.label);
        layer.handles.retain(|h| h.key != key);
        let tags = match kind {
            HandleKind::Fixed => vec![ids::FIXED.to_owned()],
            HandleKind::Draggable => vec![ids::DRAGGABLE.to_owned()],
            HandleKind::Dangle => vec![ids::DANGLE.to_owned()],
            HandleKind::Attach | HandleKind::Stick | HandleKind::Walk => Vec::new(),
        };
        layer.handles.push(HandleDef { name: name.to_owned(), key: key.clone(), kind, position, tags, end: None });
        self.rebuild_warps();
        Some(key)
    }

    /// Adds a stick between two points of an independent group: a rigid segment of the mesh. Returns the handle key.
    pub fn add_stick(&mut self, group: LayerId, name: &str, from: Point, to: Point) -> Option<String> {
        let key = self.add_handle(group, name, HandleKind::Stick, from)?;
        if let Some(h) = self.layer_mut(group).and_then(|l| l.handles.iter_mut().find(|h| h.key == key)) {
            h.end = Some(to);
        }
        self.rebuild_warps();
        Some(key)
    }

    /// Moves a handle (both ends of a stick, unless `end` says where the far end goes). False when it does not exist.
    pub fn move_handle(&mut self, group: LayerId, key: &str, position: Point, end: Option<Point>) -> bool {
        let Some(h) = self.layer_mut(group).and_then(|l| l.handles.iter_mut().find(|h| h.key == key)) else { return false };
        let delta = position - h.position;
        h.position = position;
        if let Some(old) = h.end {
            h.end = Some(end.unwrap_or(old + delta));
        }
        self.rebuild_warps();
        true
    }

    /// Makes a layer an independent group, or an ordinary layer again (which drops its handles). The root always
    /// is one.
    pub fn set_independent(&mut self, layer: LayerId, independent: bool) -> bool {
        if layer == LayerId::ROOT {
            return false;
        }
        let Some(l) = self.layer_mut(layer) else { return false };
        if l.independent == independent {
            return false;
        }
        l.independent = independent;
        if !independent {
            l.handles.clear();
        }
        self.rebuild_warps();
        true
    }

    /// Chooses how an independent group attaches to its parent's mesh.
    pub fn set_attach(&mut self, group: LayerId, style: AttachStyle) -> bool {
        match self.layer_mut(group).filter(|l| l.independent) {
            Some(l) if l.attach != style => {
                l.attach = style;
                self.rebuild_warps();
                true
            }
            _ => false,
        }
    }

    /// Switches whether a group's mesh ignores the groups attached to it.
    pub fn set_warp_independent(&mut self, group: LayerId, on: bool) -> bool {
        match self.layer_mut(group).filter(|l| l.independent) {
            Some(l) if l.warp_independent != on => {
                l.warp_independent = on;
                self.rebuild_warps();
                true
            }
            _ => false,
        }
    }

    /// Sets how finely a group's mesh is cut (clamped to [`MESH_DENSITY`]).
    pub fn set_mesh_density(&mut self, group: LayerId, density: f64) -> bool {
        let density = density.clamp(MESH_DENSITY.0, MESH_DENSITY.1);
        match self.layer_mut(group).filter(|l| l.independent) {
            Some(l) if (l.mesh_density - density).abs() > f64::EPSILON => {
                l.mesh_density = density;
                self.rebuild_warps();
                true
            }
            _ => false,
        }
    }

    /// Hides the siblings of a layer while its key trigger shows it.
    pub fn set_hide_others(&mut self, layer: LayerId, on: bool) -> bool {
        match self.layer_mut(layer) {
            Some(l) if l.hide_others != on => {
                l.hide_others = on;
                true
            }
            _ => false,
        }
    }

    /// Makes a group show its children one after the other, or stops it (`None`).
    pub fn set_cycle(&mut self, layer: LayerId, cycle: Option<CycleDef>) -> bool {
        match self.layer_mut(layer).filter(|l| l.kind == LayerKind::Group) {
            Some(l) if l.cycle != cycle => {
                l.cycle = cycle.map(|c| CycleDef { step_millis: c.step_millis.clamp(CycleDef::MIN_STEP, CycleDef::MAX_STEP), ..c });
                true
            }
            _ => false,
        }
    }

    /// Deletes a swap set; true when it existed.
    pub fn remove_swap_set(&mut self, name: &str) -> bool {
        let before = self.swap_sets.len();
        self.swap_sets.retain(|s| s.name != name);
        self.swap_sets.len() != before
    }

    /// Renames a swap set. False when it does not exist or the new name is taken.
    pub fn rename_swap_set(&mut self, name: &str, new_name: &str) -> bool {
        if name == new_name || self.swap_sets.iter().any(|s| s.name == new_name) {
            return false;
        }
        match self.swap_sets.iter_mut().find(|s| s.name == name) {
            Some(s) => {
                new_name.clone_into(&mut s.name);
                true
            }
            None => false,
        }
    }

    /// Removes a handle by key; true when it existed.
    pub fn remove_handle(&mut self, group: LayerId, key: &str) -> bool {
        let Some(layer) = self.layer_mut(group) else { return false };
        let before = layer.handles.len();
        layer.handles.retain(|h| h.key != key);
        let removed = layer.handles.len() != before;
        if removed {
            self.rebuild_warps();
        }
        removed
    }

    /// Moves the point an independent group rotates and scales about.
    pub fn set_origin(&mut self, group: LayerId, origin: Point) -> bool {
        match self.layer_mut(group).filter(|l| l.independent) {
            Some(l) => {
                l.origin = origin;
                self.rebuild_warps();
                true
            }
            None => false,
        }
    }

    /// Gives a layer a tag.
    pub fn add_tag(&mut self, layer: LayerId, tag: &str) -> bool {
        self.layer_mut(layer).is_some_and(|l| l.tags.insert(tag.to_owned()))
    }

    /// Takes a tag off a layer.
    pub fn remove_tag(&mut self, layer: LayerId, tag: &str) -> bool {
        self.layer_mut(layer).is_some_and(|l| l.tags.remove(tag))
    }

    /// Makes a layer show only while a trigger key is on (`None` removes the trigger).
    pub fn set_key_trigger(&mut self, layer: LayerId, key: Option<&str>) -> bool {
        match self.layer_mut(layer) {
            Some(l) => {
                l.key_trigger = key.map(str::to_owned);
                true
            }
            None => false,
        }
    }

    /// Defines a swap set: of its members only the one the pose picks is shown. Sets with fewer than two members
    /// are ignored (false); a set with an existing name replaces it.
    pub fn add_swap_set(&mut self, name: &str, members: Vec<LayerId>) -> bool {
        if members.len() < 2 || members.iter().any(|m| self.layer(*m).is_none()) {
            return false;
        }
        self.swap_sets.retain(|s| s.name != name);
        self.swap_sets.push(SwapSet { name: name.to_owned(), members });
        true
    }

    /// An indented outline: one line per layer with its tags, for logs and tools.
    #[must_use]
    pub fn outline(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let mut stack = vec![(LayerId::ROOT, 0_usize)];
        while let Some((id, depth)) = stack.pop() {
            let Some(l) = self.layer(id) else { continue };
            // Unnamed drawing objects (every path of an Illustrator layer) would drown the outline.
            if l.name.is_empty() && l.kind != LayerKind::Group {
                continue;
            }
            for _ in 0..depth {
                out.push_str("  ");
            }
            let label = match (l.name.is_empty(), id == LayerId::ROOT) {
                (false, _) => l.name.as_str(),
                (true, true) => "(puppet)",
                (true, false) => "(unnamed)",
            };
            let _ = write!(out, "{label}");
            if l.independent {
                out.push_str(" [independent]");
            }
            if !l.visible {
                out.push_str(" [hidden]");
            }
            if !l.tags.is_empty() {
                let list: Vec<&str> = l.tags.iter().map(String::as_str).collect();
                let _ = write!(out, " tags={}", list.join(","));
            }
            for h in &l.handles {
                let _ = write!(out, " <{:?} {}>", h.kind, h.name);
            }
            out.push('\n');
            for &c in l.children.iter().rev() {
                stack.push((c, depth.saturating_add(1)));
            }
        }
        out
    }
}
