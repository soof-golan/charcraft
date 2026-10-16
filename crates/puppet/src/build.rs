//! Building a puppet from an artwork tree.

use std::collections::BTreeSet;

use anim_artwork::{ArtKind, ArtMask, ArtNode, ArtworkTree};
use anim_core::{Affine, Point, Rect};
use anim_display::{Bitmap, ImageStore, Node};

use crate::error::{Error, Result};
use crate::model::{Content, Layer, LayerId, LayerKind, Puppet};
use crate::tags::{self, TagKind, Viseme, ids};

/// Layers accepted per puppet.
pub const MAX_LAYERS: usize = 250_000;
/// Deepest nesting followed.
const MAX_DEPTH: usize = 128;

impl Puppet {
    /// Builds a puppet from imported artwork (PSD, AI or SVG): one layer per artwork node, independence from the
    /// `+` prefix, tags from the names (see [`crate::tags`]).
    ///
    /// Without project data an independent group's origin is the centre of its artwork (the bottom centre for a
    /// head); the project's own origins replace it in [`Puppet::from_project`].
    ///
    /// # Errors
    ///
    /// [`Error::TooManyLayers`] for artwork beyond [`MAX_LAYERS`].
    pub fn from_artwork(tree: &ArtworkTree) -> Result<Self> {
        let mut flat: Vec<(&ArtNode, Option<usize>)> = Vec::new();
        let mut stack = vec![(&tree.root, None::<usize>, 0_usize)];
        while let Some((node, parent, depth)) = stack.pop() {
            if flat.len() >= MAX_LAYERS {
                return Err(Error::TooManyLayers(MAX_LAYERS));
            }
            let index = flat.len();
            flat.push((node, parent));
            if depth < MAX_DEPTH {
                for child in node.children().iter().rev() {
                    stack.push((child, Some(index), depth.saturating_add(1)));
                }
            }
        }

        let mut images = ImageStore::new();
        let mut contents = Vec::with_capacity(flat.len());
        let mut layers: Vec<Layer> = Vec::with_capacity(flat.len());
        for (index, &(node, parent)) in flat.iter().enumerate() {
            let id = LayerId(u32::try_from(index).map_err(|_| Error::TooManyLayers(MAX_LAYERS))?);
            let parent_id = parent.map(|p| LayerId(u32::try_from(p).unwrap_or(0)));
            let path = match parent.and_then(|p| layers.get(p)) {
                Some(p) if !p.path.is_empty() => format!("{}/{}", p.path, node.name),
                _ => {
                    if parent.is_some() {
                        node.name.clone()
                    } else {
                        String::new()
                    }
                }
            };
            let independent = parent.is_none()
                || tags::is_independent(&node.name)
                || (node.is_group() && tags::auto_tags(&node.name).any(|t| tags::AUTO_INDEPENDENT.contains(&t.id)));
            contents.push(convert_content(node, &mut images));
            layers.push(Layer {
                id,
                parent: parent_id,
                children: Vec::new(),
                name: node.name.clone(),
                label: tags::label(&node.name).to_owned(),
                path,
                kind: match node.kind {
                    ArtKind::Group(_) => LayerKind::Group,
                    ArtKind::Raster { .. } => LayerKind::Raster,
                    ArtKind::Vector { .. } => LayerKind::Vector,
                },
                independent,
                visible: node.visible,
                opacity: node.opacity,
                blend: node.blend,
                clipping: node.clipping,
                tags: BTreeSet::new(),
                bounds: if node.is_group() { None } else { node.bounds() },
                origin: Point::ORIGIN,
                handles: Vec::new(),
                key_trigger: None,
            });
        }

        // Children lists and subtree bounds (parents precede children, so one reverse pass aggregates).
        for index in 1..layers.len() {
            let Some(parent) = layers.get(index).and_then(|l| l.parent) else { continue };
            if let Some(p) = layers.get_mut(parent.index()) {
                p.children.push(LayerId(u32::try_from(index).unwrap_or(0)));
            }
        }
        for index in (1..layers.len()).rev() {
            let (bounds, parent) = match layers.get(index) {
                Some(l) => (l.bounds, l.parent),
                None => continue,
            };
            if let (Some(b), Some(p)) = (bounds, parent.and_then(|p| layers.get_mut(p.index()))) {
                p.bounds = Some(p.bounds.map_or(b, |o| o.union(b)));
            }
        }

        for layer in &mut layers {
            for tag in tags::auto_tags(&layer.name) {
                if tag.kind == TagKind::Layer || layer.independent {
                    layer.tags.insert(tag.id.to_owned());
                }
            }
            layer.origin = default_origin(layer.bounds, layer.has_tag(ids::HEAD));
        }
        prune_viseme_tags(&mut layers);
        infer_blink_sides(&mut layers);

        let mut puppet =
            Self { size: tree.size, layers, contents, images, warps: Vec::new(), blends: Vec::new(), swap_sets: Vec::new(), warnings: tree.warnings.clone() };
        puppet.rebuild_warps();
        Ok(puppet)
    }

    /// Recomputes the meshes after handles changed.
    pub(crate) fn rebuild_warps(&mut self) {
        let (warps, warnings) = crate::warp::build_all(self);
        self.warps = warps;
        self.blends = crate::warp::build_blends(self);
        for w in warnings {
            if !self.warnings.contains(&w) {
                self.warnings.push(w);
            }
        }
    }
}

/// The origin an independent group gets without project data: the centre of its artwork, as Character Animator
/// places it (checked against the origins stored in sample projects), except for a head, which turns about its
/// neck: the bottom centre.
fn default_origin(bounds: Option<Rect>, is_head: bool) -> Point {
    bounds.map_or(Point::ORIGIN, |b| if is_head { Point::new(b.center().x, b.y1) } else { b.center() })
}

/// Viseme tags apply to the children of a mouth group only: a layer named `S` or `D` elsewhere is not a mouth shape.
fn prune_viseme_tags(layers: &mut [Layer]) {
    let mut drop: Vec<usize> = Vec::new();
    for parent in layers.iter() {
        let shaped: Vec<usize> = parent.children.iter().map(|c| c.index()).filter(|&c| layers.get(c).is_some_and(|l| l.viseme().is_some())).collect();
        if shaped.is_empty() {
            continue;
        }
        let is_mouth = parent.has_tag(ids::MOUTHS_PARENT) || shaped.len() >= 3;
        if !is_mouth {
            drop.extend(shaped);
        }
    }
    for index in drop {
        if let Some(l) = layers.get_mut(index) {
            for v in Viseme::ALL {
                l.tags.remove(v.tag_id());
            }
        }
    }
}

/// A layer just called `Blink` inside an eye group is that eye's blink layer.
fn infer_blink_sides(layers: &mut [Layer]) {
    let mut found: Vec<(usize, &'static str)> = Vec::new();
    for (index, layer) in layers.iter().enumerate() {
        if tags::normalize(&layer.name) != "blink" || layer.has_tag(ids::LEFT_BLINK) || layer.has_tag(ids::RIGHT_BLINK) {
            continue;
        }
        let mut at = layer.parent;
        while let Some(parent) = at.and_then(|p| layers.get(p.index())) {
            if parent.has_tag(ids::LEFT_EYE) {
                found.push((index, ids::LEFT_BLINK));
                break;
            }
            if parent.has_tag(ids::RIGHT_EYE) {
                found.push((index, ids::RIGHT_BLINK));
                break;
            }
            at = parent.parent;
        }
    }
    for (index, tag) in found {
        if let Some(l) = layers.get_mut(index) {
            l.tags.insert(tag.to_owned());
        }
    }
}

fn convert_content(node: &ArtNode, store: &mut ImageStore) -> Content {
    let nodes = match &node.kind {
        ArtKind::Group(_) => Vec::new(),
        ArtKind::Raster { image, offset } => {
            let id = store.insert(image.clone());
            vec![Node::Bitmap(Bitmap { transform: Affine::translate(offset.to_vec2()), image: id, smoothing: true })]
        }
        ArtKind::Vector { nodes, .. } => nodes.clone(),
    };
    let mask = node.mask.as_ref().map(|mask| match mask {
        ArtMask::Raster { image, offset } => {
            let id = store.insert(image.clone());
            vec![Node::Bitmap(Bitmap { transform: Affine::translate(offset.to_vec2()), image: id, smoothing: true })]
        }
        ArtMask::Vector { nodes } => nodes.clone(),
    });
    Content { nodes, mask }
}
