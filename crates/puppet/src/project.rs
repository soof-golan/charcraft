//! Project data: what a Character Animator project says about a puppet beyond its artwork.

use anim_artwork::Limits;
use anim_core::Point;
use charcraft_project::{Binding, BindingKind, Handle, MediaKind, Model, Project, PuppetItem, ReplacementKind};

use crate::error::{Error, Result};
use crate::model::{HandleDef, HandleKind, LayerId, Puppet, SwapSet};
use crate::tags::{self, ids};

/// Deepest chain of child puppets followed.
const MAX_PUPPET_DEPTH: usize = 32;
/// File extensions that end the artwork file part of a layer path.
const ARTWORK_EXTENSIONS: [&str; 6] = [".ai", ".psd", ".psb", ".svg", ".pdf", ".png"];

impl Puppet {
    /// Opens a puppet of a project: loads its artwork from the project's media, builds the layer tree and applies
    /// the project data (origins, handles, tags, active layers, key triggers, swap sets).
    ///
    /// `puppet_path` is a store path (`ProjectItems/Hero`); `None` takes the first top-level puppet. A puppet that
    /// mixes several artwork files only gets the first one.
    ///
    /// # Errors
    ///
    /// [`Error::NoPuppet`], [`Error::MissingArtwork`], or an artwork or project error.
    pub fn from_project(project: &Project, puppet_path: Option<&str>) -> Result<Self> {
        let model = project.model()?;
        let item = match puppet_path {
            Some(path) => model.puppet(path).ok_or_else(|| Error::NoPuppet(path.to_owned()))?,
            None => model.root_puppets().next().ok_or_else(|| Error::NoPuppet("(none)".to_owned()))?,
        };
        let file_name = artwork_file_name(&model, item).ok_or_else(|| Error::MissingArtwork(item.path.clone()))?;
        let media = model
            .media
            .iter()
            .find(|m| m.kind == MediaKind::Artwork && m.file_name.as_deref() == Some(file_name.as_str()) && m.file.is_some())
            .and_then(|m| m.file.as_ref())
            .ok_or_else(|| Error::MissingArtwork(file_name.clone()))?;
        let bytes = project.package().read(&media.path)?;
        let tree = anim_artwork::import(&bytes, &Limits::default())?;
        let mut puppet = Self::from_artwork(&tree)?;
        puppet.apply_project(&model, item);
        Ok(puppet)
    }

    /// Applies a project's puppet data to a puppet built from the matching artwork. Layers the project names but
    /// the artwork lacks are reported in [`Puppet::warnings`].
    pub fn apply_project(&mut self, model: &Model, item: &PuppetItem) {
        let mut missing = Vec::new();
        self.apply_item(model, item, 0, &mut missing);
        for path in missing {
            self.warnings.push(format!("the project names a layer the artwork does not have: {path}"));
        }
        self.rebuild_warps();
    }

    /// The layer at an artwork path from the project (`/ProjectItems/.../Hero.ai/Ch/Head`).
    fn locate(&self, ppath: &str) -> Option<LayerId> {
        let rest = layer_path(ppath)?;
        if rest.is_empty() {
            return Some(LayerId::ROOT);
        }
        self.find_path(&rest)
    }

    fn apply_item(&mut self, model: &Model, item: &PuppetItem, depth: usize, missing: &mut Vec<String>) {
        if depth > MAX_PUPPET_DEPTH {
            return;
        }
        let own = item.artwork.as_deref();
        let group = own.and_then(|a| self.locate(a));
        if let (Some(a), None) = (own, group) {
            missing.push(a.to_owned());
        }
        if let Some(g) = group {
            self.apply_group(g, item);
        }

        for binding in &item.bindings {
            let Some(ppath) = binding.ppath.as_deref() else { continue };
            match binding.kind {
                BindingKind::Skin if Some(ppath) != own => match self.locate(ppath) {
                    Some(layer) => self.apply_binding(layer, binding),
                    None => missing.push(ppath.to_owned()),
                },
                BindingKind::Puppet => {
                    let Some(child) = model.puppet(ppath.trim_start_matches('/')) else { continue };
                    if let Some(layer) = child.artwork.as_deref().and_then(|a| self.locate(a)) {
                        self.apply_binding(layer, binding);
                    }
                    self.apply_item(model, child, depth.saturating_add(1), missing);
                }
                BindingKind::Skin | BindingKind::Other => {}
            }
        }

        for set in item.replacement_sets() {
            if set.kind != ReplacementKind::Trigger {
                continue;
            }
            let members: Vec<LayerId> = set
                .members
                .iter()
                .filter_map(|id| item.binding(id))
                .filter_map(|b| b.ppath.as_deref())
                .filter_map(|p| {
                    self.locate(p).or_else(|| model.puppet(p.trim_start_matches('/')).and_then(|c| c.artwork.as_deref()).and_then(|a| self.locate(a)))
                })
                .collect();
            if members.len() > 1 {
                let name = group.and_then(|g| self.layer(g)).map_or_else(String::new, |l| l.label.clone());
                self.swap_sets.push(SwapSet { name: format!("{name} swap {}", self.swap_sets.len().saturating_add(1)).trim().to_owned(), members });
            }
        }
    }

    /// Origin, tagged locations and handles of an independent group.
    fn apply_group(&mut self, id: LayerId, item: &PuppetItem) {
        // The puppet's own artwork binding places the artwork relative to the origin: origin = -position.
        let origin_art =
            item.bindings.iter().find(|b| b.ppath.is_some() && b.ppath == item.artwork).map(|b| Point::new(-b.transform.position[0], -b.transform.position[1]));
        let location = |name: &str| item.tagged_locations.iter().find(|(n, _)| n == name).and_then(|(_, v)| Some((*v.first()?, *v.get(1)?)));
        let user_origin = location("__user_origin");

        let Some(layer) = self.layer_mut(id) else { return };
        layer.independent = true;
        if let Some(base) = origin_art {
            layer.origin = user_origin.map_or(base, |(x, y)| Point::new(base.x + x, base.y + y));
        }
        if let Some(handle) = &item.origin {
            layer.tags.extend(handle.tags.iter().map(|t| t.name.clone()));
        }
        let (Some(base), Some(handle)) = (origin_art, &item.origin) else { return };
        let label = layer.label.clone();
        layer.handles.clear();
        collect_handles(handle, &label, base, &|n| location(n), &mut layer.handles, 0);
    }

    /// Tags, activity and key trigger a binding gives the layer it points to.
    fn apply_binding(&mut self, id: LayerId, binding: &Binding) {
        let Some(layer) = self.layer_mut(id) else { return };
        layer.tags.extend(binding.tags.iter().map(|t| t.name.clone()));
        layer.visible = binding.active;
        if binding.key_trigger.key.is_some() {
            layer.key_trigger.clone_from(&binding.key_trigger.key);
        }
    }
}

fn collect_handles(handle: &Handle, label: &str, origin: Point, location: &dyn Fn(&str) -> Option<(f64, f64)>, out: &mut Vec<HandleDef>, depth: usize) {
    if depth > MAX_PUPPET_DEPTH {
        return;
    }
    for child in &handle.children {
        let Some(loc) = child.location.as_ref() else { continue };
        let Some((x, y)) = location(&loc.name) else { continue };
        let name = if child.name.is_empty() { loc.name.clone() } else { child.name.clone() };
        let tag_names: Vec<String> = child.tags.iter().map(|t| t.name.clone()).collect();
        let kind = if tag_names.iter().any(|t| t == ids::FIXED) {
            HandleKind::Fixed
        } else if tag_names.iter().any(|t| t == ids::DRAGGABLE) {
            HandleKind::Draggable
        } else if tag_names.iter().any(|t| t == ids::DANGLE) {
            HandleKind::Dangle
        } else if tag_names.iter().any(|t| t.starts_with(ids::WALK_PREFIX)) {
            HandleKind::Walk
        } else if tags::normalize(&name).starts_with("stick") {
            HandleKind::Stick
        } else {
            HandleKind::Attach
        };
        out.push(HandleDef { key: format!("{label}/{name}"), name, kind, position: Point::new(origin.x + x, origin.y + y), tags: tag_names });
        collect_handles(child, label, origin, location, out, depth.saturating_add(1));
    }
}

/// The layer path inside the artwork file of a project path: the part after the segment that names the file.
fn layer_path(ppath: &str) -> Option<String> {
    let segments: Vec<&str> = ppath.split('/').collect();
    let at = segments.iter().position(|s| {
        let lower = s.to_lowercase();
        ARTWORK_EXTENSIONS.iter().any(|e| lower.ends_with(e))
    })?;
    Some(segments.get(at.saturating_add(1)..)?.join("/"))
}

/// The name of the artwork file a puppet (or one of its parts) is made from.
fn artwork_file_name(model: &Model, item: &PuppetItem) -> Option<String> {
    let mut stack = vec![(item, 0_usize)];
    while let Some((current, depth)) = stack.pop() {
        if let Some(a) = current.artwork.as_deref() {
            let segments: Vec<&str> = a.split('/').collect();
            if let Some(name) = segments.iter().find(|s| {
                let lower = s.to_lowercase();
                ARTWORK_EXTENSIONS.iter().any(|e| lower.ends_with(e))
            }) {
                return Some((*name).to_owned());
            }
        }
        if depth < MAX_PUPPET_DEPTH {
            for b in &current.bindings {
                if let Some(child) = b.ppath.as_deref().filter(|_| b.kind == BindingKind::Puppet).and_then(|p| model.puppet(p.trim_start_matches('/'))) {
                    stack.push((child, depth.saturating_add(1)));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_paths_follow_the_file_name() {
        assert_eq!(layer_path("/ProjectItems/.Parts/X/Boy.ai/Ch/Head/+ Left eyebrow").as_deref(), Some("Ch/Head/+ Left eyebrow"));
        assert_eq!(layer_path("/ProjectItems/.Parts/X/anchor.PSD").as_deref(), Some(""));
        assert_eq!(layer_path("/ProjectItems/Hero"), None);
    }
}
