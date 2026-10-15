//! Meshes and ARAP warps for independent groups with pinning handles.

use anim_core::{Affine, Point, Rect, Size, Vec2, num};
use anim_display::{DisplayList, Node};
use anim_render::{Background, RenderOptions};
use anim_rig::{ArapSolver, Binding, BindingMode, HandleSet, Mesh, MeshOptions, Outline};

use crate::model::{LayerId, Puppet};
use crate::silhouette;

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
    /// For each bound handle, its index in the group's `handles` list.
    pub handle_indices: Vec<usize>,
}

impl Warp {
    /// Deformed vertex positions for the given target of every bound handle (in `handle_indices` order).
    pub fn solve(&self, targets: &[Point]) -> Vec<Point> {
        let pin_targets = self.handles.targets(targets);
        let solved = self.solver.solve(&pin_targets);
        if solved.len() == self.mesh.vertices.len() && solved.iter().all(|p| p.x.is_finite() && p.y.is_finite()) { solved } else { self.mesh.vertices.clone() }
    }

    /// How far the warp moves a rest point of the group.
    pub fn displacement(&self, positions: &[Point], rest: Point) -> Vec2 {
        Binding::bind(&self.mesh, rest, BindingMode::Barycentric).and_then(|b| b.evaluate(&self.mesh, positions)).map_or(Vec2::ZERO, |p| p - rest)
    }
}

/// Builds the warp of every independent group that has mesh-pinning handles. Returns the warps and warnings for
/// groups that could not be meshed.
pub fn build_all(puppet: &Puppet) -> (Vec<Warp>, Vec<String>) {
    let mut warps = Vec::new();
    let mut warnings = Vec::new();
    for layer in puppet.layers() {
        if !layer.independent || !layer.handles.iter().any(|h| h.kind.pins_mesh()) {
            continue;
        }
        match build_one(puppet, layer.id) {
            Ok(w) => warps.push(w),
            Err(why) => warnings.push(format!("no mesh for {:?}: {why}", layer.label)),
        }
    }
    (warps, warnings)
}

pub fn build_one(puppet: &Puppet, group: LayerId) -> Result<Warp, String> {
    let layer = puppet.layer(group).ok_or("missing layer")?;
    let nodes = own_nodes(puppet, group);
    if nodes.is_empty() {
        return Err("the group draws nothing".into());
    }
    let outline = silhouette_outline(puppet, &nodes).ok_or("empty silhouette")?;
    let bounds = outline.outer.iter().fold(None::<Rect>, |acc, p| Some(acc.map_or_else(|| Rect::from_points(*p, *p), |r| r.union_pt(*p))));
    let extent = bounds.map_or(1.0, |b| b.width().max(b.height()));
    let options = MeshOptions { max_edge: extent / 9.0 };
    let mesh = Mesh::from_outline(&outline, &options)
        .or_else(|_| {
            // A pinched or self-touching outline: fall back to its bounding box.
            let b = bounds.ok_or(anim_rig::MeshError::DegenerateOuter)?;
            let rect =
                Outline { outer: vec![Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)], holes: Vec::new() };
            Mesh::from_outline(&rect, &options)
        })
        .map_err(|e| e.to_string())?;
    let mut rest = Vec::new();
    let mut indices = Vec::new();
    for (i, h) in layer.handles.iter().enumerate() {
        if h.kind.pins_mesh() {
            rest.push(h.position);
            indices.push(i);
        }
    }
    let handles = HandleSet::new(&mesh, &rest, BindingMode::Barycentric).ok_or("handles do not bind")?;
    let solver = ArapSolver::new(&mesh, handles.pinned()).map_err(|e| e.to_string())?;
    Ok(Warp { group, mesh, handles, solver, handle_indices: indices })
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
