//! Meshes and ARAP warps for independent groups with pinning handles.

use anim_core::{Affine, Point, Rect, Size, Vec2, num};
use anim_display::{DisplayList, Node};
use anim_render::{Background, RenderOptions};
use anim_rig::{ArapSolver, Binding, BindingMode, HandleSet, Mesh, MeshOptions, Outline};

use crate::model::{AttachStyle, HandleDef, LayerId, Puppet};
use crate::silhouette;
use crate::tags::ids;

/// Cells along the longer side of the silhouette raster.
const GRID: f64 = 128.0;
/// Alpha above which a pixel counts as part of the silhouette.
const ALPHA_CUTOFF: u8 = 24;

/// A mesh with its handles bound, ready for per-frame ARAP solves.
#[derive(Debug, Clone)]
pub struct Warp {
    pub group: LayerId,
    pub mesh: Mesh,
    handles: HandleSet,
    solver: ArapSolver,
    /// The handles and solver without the welds, for groups that have handles of their own.
    plain: Option<(HandleSet, ArapSolver)>,
    /// For each bound pin, the index of its handle in the group's `handles` list.
    pub handle_indices: Vec<usize>,
    /// For each bound pin, where it sits relative to its handle's position (zero except along a stick).
    pub pin_offsets: Vec<Vec2>,
    /// Independent groups welded to this mesh.
    pub welded: Vec<Weld>,
    /// For each welded group, the vertices of this mesh under its artwork (the overlap), pinned to follow the
    /// group's pose rigidly. Pins after the handles', in this order; then `anchors`.
    overlaps: Vec<Vec<u32>>,
    /// Vertices far from every overlap, held in place while a welded group moves (the body does not follow the head).
    anchors: Vec<u32>,
}

/// An independent group attached to another group's mesh (Character Animator's "Weld" attach style, the default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Weld {
    /// The welded group.
    pub child: LayerId,
    /// True when the mesh belongs to a sibling of the child (so the mesh's own pose matrix sits between them); false
    /// when it belongs to the child's parent.
    pub sibling: bool,
}

/// The weld of every group that attaches to a mesh: the group, and the group whose mesh it is welded to.
pub fn plan_welds(puppet: &Puppet) -> Vec<(Weld, LayerId)> {
    let mut out = Vec::new();
    for child in puppet.layers().iter().filter(|l| l.independent && l.id != LayerId::ROOT) {
        if child.attach != AttachStyle::Weld
            || child.handles.iter().any(HandleDef::pins_mesh)
            || child.has_tag(ids::LEFT_EYEBROW)
            || child.has_tag(ids::RIGHT_EYEBROW)
            || child.has_tag(ids::LEFT_PUPIL)
            || child.has_tag(ids::RIGHT_PUPIL)
            || puppet.ancestor_with_tag(child.id, ids::HEAD).is_some()
        {
            continue;
        }
        let Some(parent) = child.parent.and_then(|p| puppet.layer(p)) else { continue };
        // A sibling group whose artwork surrounds the attach point (the smallest such one), else the parent itself.
        let sibling = parent
            .children
            .iter()
            .filter_map(|&c| puppet.layer(c))
            .filter(|l| l.independent && l.id != child.id && l.bounds.is_some_and(|b| b.inflate(1.0, 1.0).contains(puppet.pivot(child.id))))
            .filter(|l| !l.warp_independent && !own_nodes(puppet, l.id).is_empty())
            .min_by(|a, b| area(a.bounds).total_cmp(&area(b.bounds)));
        if let Some(s) = sibling {
            out.push((Weld { child: child.id, sibling: true }, s.id));
        } else if parent.independent && !parent.warp_independent && !own_nodes(puppet, parent.id).is_empty() {
            out.push((Weld { child: child.id, sibling: false }, parent.id));
        }
    }
    out
}

fn area(bounds: Option<Rect>) -> f64 {
    bounds.map_or(f64::MAX, |b| b.width() * b.height())
}

impl Warp {
    /// Deformed vertex positions for the given target of every bound handle (in `handle_indices` order) and the
    /// pose matrix of every welded group (in `welded` order; `None` for a group that is not moved). The vertices
    /// under a welded group's artwork follow its matrix, the far ones stay put, the rest solves ARAP between them
    /// and the handles. With no welded group moved, the handles alone decide (as without any weld).
    pub fn solve(&self, targets: &[Point], matrices: &[Option<Affine>]) -> Vec<Point> {
        let idle = matrices.iter().all(Option::is_none);
        let mut pins = match &self.plain {
            Some((h, s)) if idle => return self.checked(s.solve(&h.targets(targets))),
            _ => self.handles.targets(targets),
        };
        let at = |id: u32| self.mesh.vertex(id).unwrap_or(Point::ORIGIN);
        for (ids, matrix) in self.overlaps.iter().zip(matrices.iter().map(|m| m.unwrap_or(Affine::IDENTITY)).chain(std::iter::repeat(Affine::IDENTITY))) {
            pins.extend(ids.iter().map(|&id| matrix * at(id)));
        }
        pins.extend(self.anchors.iter().map(|&id| at(id)));
        self.checked(self.solver.solve(&pins))
    }

    fn checked(&self, solved: Vec<Point>) -> Vec<Point> {
        if solved.len() == self.mesh.vertices.len() && solved.iter().all(|p| p.x.is_finite() && p.y.is_finite()) { solved } else { self.mesh.vertices.clone() }
    }
}

/// How far deformed `positions` of `mesh` move a rest point.
pub fn displacement(mesh: &Mesh, positions: &[Point], rest: Point) -> Vec2 {
    Binding::bind(mesh, rest, BindingMode::Barycentric).and_then(|b| b.evaluate(mesh, positions)).map_or(Vec2::ZERO, |p| p - rest)
}

/// A welded group overlapping fewer mesh vertices than this counts as touching only thinly (12 vertices of a mesh
/// whose edge is 1/40 of its extent; the weld mesh is finer, so the count scales with the square of the ratio).
const THIN_OVERLAP: usize = 75;
/// How far from the attach point, as a fraction of the mesh extent, vertices under a thinly overlapping group's
/// whole subtree follow it.
const NECK_REACH: f64 = 0.1;
/// The body does not follow a welded group: the vertices outside a band around its overlap are held in place. The
/// band reaches this many overlap widths to each side of the overlap's centre ...
const BAND_HALF: f64 = 0.85;
/// ... and this many overlap widths away from the overlap.
const BAND_DEPTH: f64 = 1.2;
/// The overlap reaches this far (fraction of the mesh extent) beyond the welded group's outline, so the neck stays
/// rigid with the head for a short way below the chin and the bend starts lower.
const OVERLAP_MARGIN: f64 = 0.03;
/// Only vertices this near the attach point (fraction of the mesh extent) follow the welded group: a head sitting
/// straight on a wide torso pins the part of the torso at the neck, not its whole top edge.
const OVERLAP_RADIUS: f64 = 0.12;
/// Smallest overlap width the band is sized from, as a fraction of the mesh extent.
const ANCHOR_FLOOR: f64 = 0.03;
/// Edge length of a mesh that has welded groups, as a fraction of its extent (finer than the handle meshes, so the
/// overlap is a few vertices wide).
const WELD_EDGE: f64 = 1.0 / 100.0;

/// Builds the warp of every independent group that has mesh-pinning handles. Returns the warps and warnings for
/// groups that could not be meshed.
pub fn build_all(puppet: &Puppet) -> (Vec<Warp>, Vec<String>) {
    let mut warps = Vec::new();
    let mut warnings = Vec::new();
    let plan = plan_welds(puppet);
    for layer in puppet.layers() {
        let welded = welds_in(&plan, layer.id);
        let pinned = layer.handles.iter().any(HandleDef::pins_mesh);
        if !layer.independent || (!pinned && welded.is_empty()) {
            continue;
        }
        match build_one(puppet, layer.id, welded) {
            Ok(w) => warps.push(w),
            Err(why) => warnings.push(format!("no mesh for {:?}: {why}", layer.label)),
        }
    }
    (warps, warnings)
}

fn welds_in(plan: &[(Weld, LayerId)], group: LayerId) -> Vec<Weld> {
    plan.iter().filter(|(_, p)| *p == group).map(|(w, _)| *w).collect()
}

/// The mesh of an independent group's own artwork.
fn mesh_of(puppet: &Puppet, group: LayerId, edge: f64) -> Result<Mesh, String> {
    let edge = edge / puppet.layer(group).map_or(1.0, |l| l.mesh_density).clamp(crate::model::MESH_DENSITY.0, crate::model::MESH_DENSITY.1);
    let nodes = own_nodes(puppet, group);
    if nodes.is_empty() {
        return Err("the group draws nothing".into());
    }
    let outline = silhouette_outline(puppet, &nodes).ok_or("empty silhouette")?;
    let bounds = outline.outer.iter().fold(None::<Rect>, |acc, p| Some(acc.map_or_else(|| Rect::from_points(*p, *p), |r| r.union_pt(*p))));
    let extent = bounds.map_or(1.0, |b| b.width().max(b.height()));
    let options = MeshOptions { max_edge: extent * edge };
    Mesh::from_outline(&outline, &options)
        .or_else(|_| {
            // A pinched or self-touching outline: fall back to its bounding box.
            let b = bounds.ok_or(anim_rig::MeshError::DegenerateOuter)?;
            let rect =
                Outline { outer: vec![Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)], holes: Vec::new() };
            Mesh::from_outline(&rect, &options)
        })
        .map_err(|e| e.to_string())
}

impl Puppet {
    /// The edges of the mesh an independent group is (or would be) cut into, in artwork pixels, each once. The
    /// mesh tool draws them; they follow the group's mesh density. Empty when the group draws nothing of its own.
    #[must_use]
    pub fn mesh_wire(&self, group: LayerId) -> Vec<[Point; 2]> {
        let Some(layer) = self.layer(group).filter(|l| l.independent) else { return Vec::new() };
        let welded = plan_welds(self).into_iter().filter(|(_, owner)| *owner == group).count();
        let edge = if welded == 0 { 1.0 / 9.0 } else { WELD_EDGE };
        let Ok(mesh) = mesh_of(self, layer.id, edge) else { return Vec::new() };
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for &[t0, t1, t2] in &mesh.triangles {
            for (a, b) in [(t0, t1), (t1, t2), (t2, t0)] {
                let key = (a.min(b), a.max(b));
                if seen.insert(key)
                    && let (Some(p), Some(q)) = (mesh.vertex(a), mesh.vertex(b))
                {
                    out.push([p, q]);
                }
            }
        }
        out
    }
}

/// Builds the warp of one group: its pinning handles, the vertices under the artwork of each group welded to it
/// (which follow that group) and the vertices far from those (which stay).
pub fn build_one(puppet: &Puppet, group: LayerId, welded: Vec<Weld>) -> Result<Warp, String> {
    let layer = puppet.layer(group).ok_or("missing layer")?;
    let mesh = mesh_of(puppet, group, if welded.is_empty() { 1.0 / 9.0 } else { WELD_EDGE })?;
    let mut rest = Vec::new();
    let mut indices = Vec::new();
    let mut offsets = Vec::new();
    for (i, h) in layer.handles.iter().enumerate().filter(|(_, h)| h.pins_mesh()) {
        for (at, offset) in h.pins() {
            rest.push(at);
            indices.push(i);
            offsets.push(offset);
        }
    }
    let plain = if rest.is_empty() || welded.is_empty() {
        None
    } else {
        let h = HandleSet::new(&mesh, &rest, BindingMode::Barycentric).ok_or("handles do not bind")?;
        let s = ArapSolver::new(&mesh, h.pinned()).map_err(|e| e.to_string())?;
        Some((h, s))
    };
    let handles = HandleSet::new(&mesh, &rest, BindingMode::Barycentric).ok_or("handles do not bind")?;
    let mut taken: Vec<u32> = handles.pinned().to_vec();
    let mut overlaps: Vec<Vec<u32>> = Vec::new();
    for w in &welded {
        let pivot = puppet.pivot(w.child);
        let extent = mesh.bounds().map_or(0.0, |b| b.width().max(b.height()));
        let ring = silhouette_outline(puppet, &own_nodes(puppet, w.child)).map(|o| o.outer);
        let margin = OVERLAP_MARGIN * extent;
        let mut under: Vec<u32> = mesh
            .vertices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.distance(pivot) <= OVERLAP_RADIUS * extent && ring.as_deref().is_some_and(|r| inside(r, **v) || ring_distance(r, **v) <= margin))
            .map(|(i, _)| num::usize_to_u32_sat(i))
            .filter(|i| !taken.contains(i))
            .collect();
        if under.len() < THIN_OVERLAP {
            // A thin overlap (the head's own artwork only just touches a narrow neck, or a head built from
            // independent parts draws almost nothing itself) would pin one or two vertices, and the neck then
            // collapses to a spike under the chin when the head moves. The whole welded subtree moves with the
            // group, so the vertices under all of it and near the attach point follow it as well; the neck keeps
            // its width right up to the chin while the shoulders under hanging hair stay on the body.
            let reach = extent * NECK_REACH;
            if let Some(full) = silhouette_outline(puppet, &subtree_nodes(puppet, w.child)).map(|o| o.outer) {
                let more: Vec<u32> = mesh
                    .vertices
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (num::usize_to_u32_sat(i), *v))
                    .filter(|(i, v)| !taken.contains(i) && !under.contains(i) && v.distance(pivot) <= reach && inside(&full, *v))
                    .map(|(i, _)| i)
                    .collect();
                under.extend(more);
            }
        }
        if under.is_empty() {
            // The artworks do not overlap at a vertex: hold the vertex nearest the attach point.
            let nearest = mesh
                .vertices
                .iter()
                .enumerate()
                .filter(|(i, _)| !taken.contains(&num::usize_to_u32_sat(*i)))
                .min_by(|a, b| a.1.distance(pivot).total_cmp(&b.1.distance(pivot)));
            under.extend(nearest.map(|(i, _)| num::usize_to_u32_sat(i)));
        }
        taken.extend(under.iter().copied());
        overlaps.push(under);
    }
    let anchors: Vec<u32> = if welded.is_empty() {
        Vec::new()
    } else {
        let b = mesh.bounds().ok_or("empty mesh")?;
        let floor = ANCHOR_FLOOR * b.width().max(b.height());
        // Each weld frees a band around its overlap: as wide as the overlap, a little deeper than it is wide.
        let bands: Vec<(f64, f64, Vec<Point>)> = overlaps
            .iter()
            .map(|ids| {
                let pts: Vec<Point> = ids.iter().filter_map(|&i| mesh.vertex(i)).collect();
                let (lo, hi) = pts.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.x), hi.max(p.x)));
                (f64::midpoint(lo, hi), (hi - lo).max(floor), pts)
            })
            .collect();
        mesh.vertices
            .iter()
            .enumerate()
            .map(|(i, v)| (num::usize_to_u32_sat(i), *v))
            .filter(|(i, v)| {
                !taken.contains(i)
                    && bands.iter().all(|(cx, width, pts)| (v.x - cx).abs() > BAND_HALF * width || pts.iter().all(|p| p.distance(*v) > BAND_DEPTH * width))
            })
            .map(|(i, _)| i)
            .collect()
    };
    let mut all: Vec<u32> = handles.pinned().to_vec();
    all.extend(overlaps.iter().flatten().copied());
    all.extend(anchors.iter().copied());
    let solver = ArapSolver::new(&mesh, &all).map_err(|e| e.to_string())?;
    Ok(Warp { group, mesh, handles, solver, plain, handle_indices: indices, pin_offsets: offsets, welded, overlaps, anchors })
}

/// Distance from a point to a closed polygon's outline.
fn ring_distance(ring: &[Point], p: Point) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(a, b)| {
            let ab = *b - *a;
            let len2 = ab.hypot2();
            let t = if len2 > 0.0 { ((p - *a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
            p.distance(*a + ab * t)
        })
        .fold(f64::MAX, f64::min)
}

/// True when a point is inside a closed polygon (even-odd rule).
fn inside(ring: &[Point], p: Point) -> bool {
    let mut odd = false;
    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            odd = !odd;
        }
    }
    odd
}

/// What the group draws itself (independent children excluded), at default visibility, in artwork pixels.
pub fn own_nodes(puppet: &Puppet, group: LayerId) -> Vec<Node> {
    let mut out = Vec::new();
    let mut stack: Vec<LayerId> = puppet.layer(group).map(|l| l.children.iter().rev().copied().collect()).unwrap_or_default();
    while let Some(id) = stack.pop() {
        let Some(layer) = puppet.layer(id) else { continue };
        if !layer.visible || layer.independent {
            continue;
        }
        if let Some(content) = puppet.contents.get(id.index()) {
            out.extend(content.nodes.iter().cloned());
        }
        stack.extend(layer.children.iter().rev().copied());
    }
    out
}

/// Everything a group and its independent descendants draw, at default visibility, in artwork pixels.
fn subtree_nodes(puppet: &Puppet, group: LayerId) -> Vec<Node> {
    let mut out = own_nodes(puppet, group);
    let mut stack: Vec<LayerId> = vec![group];
    let mut seen = 0_usize;
    while let Some(id) = stack.pop() {
        seen = seen.saturating_add(1);
        if seen > puppet.layers().len() {
            break;
        }
        let Some(layer) = puppet.layer(id) else { continue };
        for &child in &layer.children {
            let Some(c) = puppet.layer(child) else { continue };
            if c.independent && c.visible {
                out.extend(own_nodes(puppet, child));
            }
            stack.push(child);
        }
    }
    out
}

fn silhouette_outline(puppet: &Puppet, nodes: &[Node]) -> Option<Outline> {
    let mut list = DisplayList::new(Size::new(1.0, 1.0), None);
    list.nodes = nodes.to_vec();
    let bounds = anim_render::content_bounds(&list, puppet.images(), Affine::IDENTITY)?;
    if !(bounds.width() > 0.0 && bounds.height() > 0.0) {
        return None;
    }
    let scale = (GRID / bounds.width().max(bounds.height())).min(1.0);
    let pad = 2.0;
    let w = num::round_f64_to_u32(bounds.width().mul_add(scale, 2.0 * pad)).max(1);
    let h = num::round_f64_to_u32(bounds.height().mul_add(scale, 2.0 * pad)).max(1);
    let view = Affine::translate((pad, pad)) * Affine::scale(scale) * Affine::translate((-bounds.x0, -bounds.y0));
    let options = RenderOptions::new(w, h).with_view(view).with_background(Background::Transparent);
    list.size = Size::new(f64::from(w), f64::from(h));
    let image = anim_render::render(&list, puppet.images(), &options).ok()?;
    let (wu, hu) = (num::u32_to_usize(image.width), num::u32_to_usize(image.height));
    let mask: Vec<bool> = image.premultiplied_rgba.as_chunks::<4>().0.iter().map(|px| px[3] > ALPHA_CUTOFF).collect();
    let ring = silhouette::largest_contour(&mask, wu, hu)?;
    let ring = silhouette::simplify(&ring, 0.7);
    if ring.len() < 3 {
        return None;
    }
    let to_art = |c: (i32, i32)| Point::new(bounds.x0 + (f64::from(c.0) - pad) / scale, bounds.y0 + (f64::from(c.1) - pad) / scale);
    let mut outer: Vec<Point> = ring.into_iter().map(to_art).collect();
    // Counter-clockwise on screen (negative shoelace area with y down) is what the mesher takes.
    if shoelace(&outer) > 0.0 {
        outer.reverse();
    }
    Some(Outline { outer, holes: Vec::new() })
}

fn shoelace(points: &[Point]) -> f64 {
    let sum: f64 = points.iter().zip(points.iter().cycle().skip(1)).map(|(a, b)| a.x * b.y - b.x * a.y).sum();
    sum / 2.0
}
