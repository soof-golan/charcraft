//! Meshes and ARAP warps for independent groups with pinning handles.

use anim_core::{Affine, Point, Rect, Size, Vec2, num};
use anim_display::{DisplayList, Node};
use anim_render::{Background, RenderOptions};
use anim_rig::{ArapSolver, Binding, BindingMode, HandleSet, Mesh, MeshOptions, Outline};

use crate::model::{LayerId, Puppet};
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
    /// For each bound handle, its index in the group's `handles` list.
    pub handle_indices: Vec<usize>,
    /// Independent groups welded to this mesh.
    pub welded: Vec<Weld>,
    /// For each welded group, the vertices of this mesh under its artwork (the overlap), pinned to follow the
    /// group's pose rigidly. Pins after the handles', in this order; then `anchors`.
    overlaps: Vec<Vec<u32>>,
    /// Vertices far from every overlap, held in place while a welded group moves.
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
        if child.handles.iter().any(|h| h.kind.pins_mesh())
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
            .filter(|l| l.independent && l.id != child.id && l.bounds.is_some_and(|b| b.inflate(1.0, 1.0).contains(child.origin)))
            .filter(|l| !own_nodes(puppet, l.id).is_empty())
            .min_by(|a, b| area(a.bounds).total_cmp(&area(b.bounds)));
        if let Some(s) = sibling {
            out.push((Weld { child: child.id, sibling: true }, s.id));
        } else if parent.independent && !own_nodes(puppet, parent.id).is_empty() {
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

/// Distance from the overlap, as a fraction of the mesh extent, beyond which a parent mesh is held in place while a
/// welded group moves.
const ANCHOR_REACH: f64 = 0.04;
/// Edge length of a mesh that has welded groups, as a fraction of its extent (finer than the handle meshes, so the
/// overlap is a few vertices wide).
const WELD_EDGE: f64 = 1.0 / 40.0;

/// Builds the warp of every independent group that has mesh-pinning handles. Returns the warps and warnings for
/// groups that could not be meshed.
pub fn build_all(puppet: &Puppet) -> (Vec<Warp>, Vec<String>) {
    let mut warps = Vec::new();
    let mut warnings = Vec::new();
    let plan = plan_welds(puppet);
    for layer in puppet.layers() {
        let welded = welds_in(&plan, layer.id);
        let pinned = layer.handles.iter().any(|h| h.kind.pins_mesh());
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

/// Builds the warp of one group: its pinning handles, the vertices under the artwork of each group welded to it
/// (which follow that group) and the vertices far from those (which stay).
pub fn build_one(puppet: &Puppet, group: LayerId, welded: Vec<Weld>) -> Result<Warp, String> {
    let layer = puppet.layer(group).ok_or("missing layer")?;
    let mesh = mesh_of(puppet, group, if welded.is_empty() { 1.0 / 9.0 } else { WELD_EDGE })?;
    let mut rest = Vec::new();
    let mut indices = Vec::new();
    for (i, h) in layer.handles.iter().enumerate() {
        if h.kind.pins_mesh() {
            rest.push(h.position);
            indices.push(i);
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
        let child = puppet.layer(w.child).ok_or("missing layer")?;
        let ring = silhouette_outline(puppet, &own_nodes(puppet, w.child)).map(|o| o.outer);
        let mut under: Vec<u32> = mesh
            .vertices
            .iter()
            .enumerate()
            .filter(|(_, v)| ring.as_deref().is_some_and(|r| inside(r, **v)))
            .map(|(i, _)| num::usize_to_u32_sat(i))
            .filter(|i| !taken.contains(i))
            .collect();
        if under.is_empty() {
            // The artworks do not overlap at a vertex: hold the vertex nearest the attach point.
            let nearest = mesh
                .vertices
                .iter()
                .enumerate()
                .filter(|(i, _)| !taken.contains(&num::usize_to_u32_sat(*i)))
                .min_by(|a, b| a.1.distance(child.origin).total_cmp(&b.1.distance(child.origin)));
            under.extend(nearest.map(|(i, _)| num::usize_to_u32_sat(i)));
        }
        taken.extend(under.iter().copied());
        overlaps.push(under);
    }
    let anchors: Vec<u32> = if welded.is_empty() {
        Vec::new()
    } else {
        let b = mesh.bounds().ok_or("empty mesh")?;
        let reach = ANCHOR_REACH * b.width().max(b.height());
        let held: Vec<Point> = overlaps.iter().flatten().filter_map(|&i| mesh.vertex(i)).collect();
        mesh.vertices
            .iter()
            .enumerate()
            .map(|(i, v)| (num::usize_to_u32_sat(i), *v))
            .filter(|(i, v)| !taken.contains(i) && held.iter().all(|h| h.distance(*v) > reach))
            .map(|(i, _)| i)
            .collect()
    };
    let mut all: Vec<u32> = handles.pinned().to_vec();
    all.extend(overlaps.iter().flatten().copied());
    all.extend(anchors.iter().copied());
    let solver = ArapSolver::new(&mesh, &all).map_err(|e| e.to_string())?;
    Ok(Warp { group, mesh, handles, solver, plain, handle_indices: indices, welded, overlaps, anchors })
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
