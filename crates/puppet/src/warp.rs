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
    /// Independent groups welded to this mesh: their attach points are pinned after the handles.
    pub welded: Vec<Weld>,
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
    /// Deformed vertex positions for the given target of every bound handle (in `handle_indices` order), followed by
    /// the attach point of every welded group (in `welded` order). Missing targets stay at rest.
    /// With `welds_idle`, no welded group is moved and the handles alone decide (as without any weld).
    pub fn solve(&self, targets: &[Point], welds_idle: bool) -> Vec<Point> {
        let (set, arap) = match &self.plain {
            Some((h, s)) if welds_idle => (h, s),
            _ => (&self.handles, &self.solver),
        };
        let solved = arap.solve(&set.targets(targets));
        if solved.len() == self.mesh.vertices.len() && solved.iter().all(|p| p.x.is_finite() && p.y.is_finite()) { solved } else { self.mesh.vertices.clone() }
    }
}

/// How far deformed `positions` of `mesh` move a rest point.
pub fn displacement(mesh: &Mesh, positions: &[Point], rest: Point) -> Vec2 {
    Binding::bind(mesh, rest, BindingMode::Barycentric).and_then(|b| b.evaluate(mesh, positions)).map_or(Vec2::ZERO, |p| p - rest)
}

/// The mesh of an independent group that is welded to another group's mesh. Vertices near the group's origin follow
/// the other mesh; those far from it follow the group's own pose; the ring between is solved smoothly.
#[derive(Debug, Clone)]
pub struct Blend {
    pub group: LayerId,
    /// The group whose mesh the group is welded to.
    pub parent: LayerId,
    /// True when `parent` is a sibling (see [`Weld::sibling`]).
    pub sibling: bool,
    pub mesh: Mesh,
    solver: ArapSolver,
    /// For each pinned vertex (in solver order): follows the parent mesh (else its own pose).
    inner: Vec<bool>,
}

/// Distance from the origin, as a fraction of the group's extent, within which vertices follow the parent mesh.
const BLEND_INNER: f64 = 0.2;
/// Distance beyond which vertices follow the group's own pose.
const BLEND_OUTER: f64 = 0.5;
/// Fraction of the mesh extent, away from every attach point, beyond which an unpinned parent mesh is held in place.
const ANCHOR_REACH: f64 = 0.35;

impl Blend {
    /// Deformed vertices: pinned ones in the group's local space, near ones at `follow(rest)`.
    pub fn solve(&self, follow: impl Fn(Point) -> Point) -> Vec<Point> {
        let targets: Vec<Point> = self
            .solver
            .pinned()
            .iter()
            .zip(&self.inner)
            .map(|(&id, &near)| {
                let rest = self.mesh.vertex(id).unwrap_or(Point::ORIGIN);
                if near { follow(rest) } else { rest }
            })
            .collect();
        let solved = self.solver.solve(&targets);
        if solved.len() == self.mesh.vertices.len() && solved.iter().all(|p| p.x.is_finite() && p.y.is_finite()) { solved } else { self.mesh.vertices.clone() }
    }
}

/// Builds the blend meshes of every welded group that has no warp of its own.
pub fn build_blends(puppet: &Puppet) -> Vec<Blend> {
    plan_welds(puppet).into_iter().filter_map(|(w, parent)| build_blend(puppet, w, parent).ok()).collect()
}

fn build_blend(puppet: &Puppet, weld: Weld, parent: LayerId) -> Result<Blend, String> {
    let layer = puppet.layer(weld.child).ok_or("missing layer")?;
    let mesh = mesh_of(puppet, weld.child)?;
    let b = mesh.bounds().ok_or("empty mesh")?;
    let extent = b.width().max(b.height());
    let mut pinned = Vec::new();
    let mut inner = Vec::new();
    for (i, v) in mesh.vertices.iter().enumerate() {
        let d = v.distance(layer.origin) / extent.max(f64::MIN_POSITIVE);
        if d <= BLEND_INNER || d >= BLEND_OUTER {
            pinned.push(num::usize_to_u32_sat(i));
            inner.push(d <= BLEND_INNER);
        }
    }
    if !inner.iter().any(|&n| n) || inner.iter().all(|&n| n) {
        return Err("no ring to blend".into());
    }
    let solver = ArapSolver::new(&mesh, &pinned).map_err(|e| e.to_string())?;
    Ok(Blend { group: weld.child, parent, sibling: weld.sibling, mesh, solver, inner })
}

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
fn mesh_of(puppet: &Puppet, group: LayerId) -> Result<Mesh, String> {
    let nodes = own_nodes(puppet, group);
    if nodes.is_empty() {
        return Err("the group draws nothing".into());
    }
    let outline = silhouette_outline(puppet, &nodes).ok_or("empty silhouette")?;
    let bounds = outline.outer.iter().fold(None::<Rect>, |acc, p| Some(acc.map_or_else(|| Rect::from_points(*p, *p), |r| r.union_pt(*p))));
    let extent = bounds.map_or(1.0, |b| b.width().max(b.height()));
    let options = MeshOptions { max_edge: extent / 9.0 };
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

/// Builds the warp of one group: its pinning handles, then the attach points of the groups welded to it. The part of the mesh far from every attach point is held in place while a welded group moves.
pub fn build_one(puppet: &Puppet, group: LayerId, welded: Vec<Weld>) -> Result<Warp, String> {
    let layer = puppet.layer(group).ok_or("missing layer")?;
    let mesh = mesh_of(puppet, group)?;
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
    let attach: Vec<Point> = welded.iter().filter_map(|w| puppet.layer(w.child)).map(|l| l.origin).collect();
    rest.extend(attach.iter().copied());
    if !attach.is_empty() {
        let b = mesh.bounds().ok_or("empty mesh")?;
        let reach = ANCHOR_REACH * b.width().max(b.height());
        let mut far: Vec<(f64, Point)> = mesh.vertices.iter().map(|&v| (attach.iter().map(|a| a.distance(v)).fold(f64::MAX, f64::min), v)).collect();
        far.sort_by(|a, b| b.0.total_cmp(&a.0));
        let keep = far.iter().take_while(|(d, _)| *d > reach).count().max(2);
        rest.extend(far.iter().take(keep).map(|(_, v)| *v));
    }
    let handles = HandleSet::new(&mesh, &rest, BindingMode::Barycentric).ok_or("handles do not bind")?;
    let solver = ArapSolver::new(&mesh, handles.pinned()).map_err(|e| e.to_string())?;
    Ok(Warp { group, mesh, handles, solver, plain, handle_indices: indices, welded })
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
