//! The layer tree of a puppet: its bindings resolved into nested layers and child puppets.

use serde::Serialize;

use super::Model;
use super::puppet::{Binding, BindingKind, PuppetItem, Tag};
use super::util::{last_segment, tree_path};

const MAX_DEPTH: usize = 64;

/// What a node of the layer tree is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LayerKind {
    /// A child puppet (it has its own file and may have children).
    Puppet,
    /// A layer of the artwork.
    Layer,
}

/// A layer or child puppet in a puppet's rig.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LayerNode {
    /// The name rigging tags are matched against.
    pub name: String,
    /// Puppet or artwork layer.
    pub kind: LayerKind,
    /// The store path of the puppet file, for [`LayerKind::Puppet`] nodes.
    pub puppet: Option<String>,
    /// The artwork layer path.
    pub ppath: Option<String>,
    /// The binding id in the parent.
    pub binding_id: Option<String>,
    /// Tags, from the parent's binding and the puppet's own.
    pub tags: Vec<Tag>,
    /// Shown by default.
    pub active: bool,
    /// Opacity 0 to 1.
    pub opacity: f64,
    /// Blend mode.
    pub blend_mode: Option<String>,
    /// Keyboard trigger key.
    pub trigger_key: Option<String>,
    /// A child puppet whose file is not in the project.
    pub missing: bool,
    /// Children in binding order.
    pub children: Vec<Self>,
}

impl LayerNode {
    /// The node and everything below it, pre-order.
    #[must_use]
    pub fn flatten(&self) -> Vec<&Self> {
        let mut out = Vec::new();
        let mut stack = vec![self];
        while let Some(node) = stack.pop() {
            out.push(node);
            stack.extend(node.children.iter().rev());
        }
        out
    }

    /// Nodes that carry a tag.
    #[must_use]
    pub fn with_tag(&self, tag: &str) -> Vec<&Self> {
        self.flatten().into_iter().filter(|n| n.tags.iter().any(|t| t.name == tag)).collect()
    }
}

fn merge_tags(first: &[Tag], second: &[Tag]) -> Vec<Tag> {
    let mut tags = first.to_vec();
    for tag in second {
        if !tags.iter().any(|t| t.name == tag.name) {
            tags.push(tag.clone());
        }
    }
    tags
}

/// The binding that stands for the puppet's own artwork layer.
fn self_binding(puppet: &PuppetItem) -> Option<&Binding> {
    let artwork = puppet.artwork.as_deref()?;
    puppet.bindings.iter().find(|b| b.kind == BindingKind::Skin && b.ppath.as_deref() == Some(artwork))
}

fn puppet_node(model: &Model, puppet: &PuppetItem, parent: Option<&Binding>, depth: usize, visiting: &mut Vec<String>) -> LayerNode {
    let own = self_binding(puppet);
    let own_tags = own.map(|b| b.tags.clone()).unwrap_or_default();
    let tags = match parent {
        Some(b) => merge_tags(&b.tags, &own_tags),
        None => own_tags,
    };
    let name = parent.and_then(|b| b.name.clone()).unwrap_or_else(|| puppet.display_name().to_owned());
    let source = parent.or(own);
    let mut children = Vec::new();
    if depth < MAX_DEPTH {
        visiting.push(puppet.path.clone());
        for binding in &puppet.bindings {
            if own.is_some_and(|o| o.id == binding.id) {
                continue;
            }
            children.push(child_node(model, binding, depth, visiting));
        }
        visiting.pop();
    }
    LayerNode {
        name,
        kind: LayerKind::Puppet,
        puppet: Some(puppet.path.clone()),
        ppath: puppet.artwork.clone(),
        binding_id: parent.map(|b| b.id.clone()),
        tags,
        active: source.is_none_or(|b| b.active),
        opacity: source.map_or(1.0, |b| b.opacity),
        blend_mode: source.and_then(|b| b.blend_mode.clone()),
        trigger_key: source.and_then(|b| b.key_trigger.key.clone()),
        missing: false,
        children,
    }
}

fn child_node(model: &Model, binding: &Binding, depth: usize, visiting: &mut Vec<String>) -> LayerNode {
    let target = binding.ppath.as_deref().map(tree_path);
    if binding.kind == BindingKind::Puppet {
        if let Some(path) = target
            && !visiting.iter().any(|v| v == path)
            && let Some(child) = model.puppet(path)
        {
            return puppet_node(model, child, Some(binding), depth.saturating_add(1), visiting);
        }
        return leaf(binding, true);
    }
    leaf(binding, false)
}

fn leaf(binding: &Binding, missing: bool) -> LayerNode {
    LayerNode {
        name: binding.name.clone().unwrap_or_else(|| binding.ppath.as_deref().map_or_else(String::new, |p| last_segment(p).to_owned())),
        kind: LayerKind::Layer,
        puppet: None,
        ppath: binding.ppath.clone(),
        binding_id: Some(binding.id.clone()),
        tags: binding.tags.clone(),
        active: binding.active,
        opacity: binding.opacity,
        blend_mode: binding.blend_mode.clone(),
        trigger_key: binding.key_trigger.key.clone(),
        missing,
        children: Vec::new(),
    }
}

/// Builds the tree for the puppet at `path`.
pub fn build(model: &Model, path: &str) -> Option<LayerNode> {
    let puppet = model.puppet(path)?;
    Some(puppet_node(model, puppet, None, 0, &mut Vec::new()))
}
