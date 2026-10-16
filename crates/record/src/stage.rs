//! Putting puppets on the stage: the cast, a scene's frames as a [`FrameSource`], and a frame renderer.
//!
//! The scene names its puppets; the [`Cast`] holds them. [`SceneFrames`] evaluates the scene once
//! ([`Evaluator`]), keeps the poses, and builds each frame's display list on demand, so it is a pure function of
//! the frame index (the [`FrameSource`] contract) and exporters may ask for frames in any order.

use std::collections::BTreeMap;

use anim_core::{ColorTransform, Size, Vec2};
use anim_display::{DisplayList, Group, ImageId, ImageStore, Node};
use anim_export::FrameSource;
use charcraft_behaviors::{BehaviorStack, Rig};
use charcraft_project::{BindingKind, Model, Project, PuppetItem};
use charcraft_puppet::{Puppet, PuppetPose};

use crate::error::{Error, Result};
use crate::playback::{Evaluator, Templates};
use crate::scene::Scene;

/// A puppet that can be placed on tracks.
#[derive(Debug, Clone)]
pub struct CastMember {
    /// The puppet.
    pub puppet: Puppet,
    /// Its behaviors with their stored parameters.
    pub template: BehaviorStack,
    /// Puppet-space position of the artwork's top-left pixel.
    pub skin: Vec2,
    /// The puppet's origin handle in puppet space.
    pub handle: Vec2,
}

impl CastMember {
    /// A member with the standard behaviors and the artwork at the origin.
    #[must_use]
    pub fn new(puppet: Puppet) -> Self {
        let template = BehaviorStack::standard(Rig::from_puppet(&puppet));
        Self { puppet, template, skin: Vec2::ZERO, handle: Vec2::ZERO }
    }
}

/// The puppets of a scene by key (a project path for imported scenes).
#[derive(Debug, Clone, Default)]
pub struct Cast {
    members: BTreeMap<String, CastMember>,
}

impl Cast {
    /// An empty cast.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a member.
    pub fn insert(&mut self, key: &str, member: CastMember) {
        self.members.insert(key.to_owned(), member);
    }

    /// The member for a key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&CastMember> {
        self.members.get(key)
    }

    /// The keys.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.members.keys().map(String::as_str)
    }

    /// Number of members.
    #[must_use]
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Whether the cast is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The behavior stacks to hand to the [`Evaluator`] (and to a [`RecordDoc`](crate::RecordDoc)).
    #[must_use]
    pub fn templates(&self) -> Templates {
        self.members.iter().map(|(k, m)| (k.clone(), m.template.clone())).collect()
    }

    /// Builds the members for the given puppet paths of a project. Puppets that cannot be built are skipped and
    /// named in the returned warnings.
    #[must_use]
    pub fn from_project<'a>(project: &Project, model: &Model, paths: impl IntoIterator<Item = &'a str>) -> (Self, Vec<String>) {
        let defaults = project.manifest().map(charcraft_project::ArchiveInfo::behavior_defaults).unwrap_or_default();
        let mut cast = Self::new();
        let mut warnings = Vec::new();
        for path in paths {
            let key = path.trim_start_matches('/');
            if cast.members.contains_key(key) {
                continue;
            }
            let Some(item) = model.puppet(key) else {
                warnings.push(format!("{key}: not a puppet of the project"));
                continue;
            };
            match Puppet::from_project(project, Some(key)) {
                Ok(puppet) => {
                    let (template, skipped) = BehaviorStack::from_bound(Rig::from_puppet(&puppet), &item.behaviors, &defaults);
                    if !skipped.is_empty() {
                        warnings.push(format!("{key}: behaviors not implemented: {}", skipped.join(", ")));
                    }
                    cast.members.insert(key.to_owned(), CastMember { puppet, template, skin: skin_of(item), handle: origin_of(item) });
                }
                Err(e) => warnings.push(format!("{key}: {e}")),
            }
        }
        (cast, warnings)
    }
}

fn origin_of(item: &PuppetItem) -> Vec2 {
    item.tagged_locations.iter().find(|(name, _)| name == "__user_origin").and_then(|(_, xy)| Some(Vec2::new(*xy.first()?, *xy.get(1)?))).unwrap_or(Vec2::ZERO)
}

fn skin_of(item: &PuppetItem) -> Vec2 {
    item.bindings.iter().find(|b| b.kind == BindingKind::Skin).map_or(Vec2::ZERO, |b| Vec2::new(b.transform.position[0], b.transform.position[1]))
}

/// The ids a store holds, found by probing from 0 (a store has no iterator; puppets insert densely).
fn store_ids(store: &ImageStore) -> Vec<ImageId> {
    let want = store.len();
    let mut ids = Vec::with_capacity(want);
    let mut id = 0_u64;
    while ids.len() < want && id < u64::try_from(want).unwrap_or(0).saturating_add(65_536) {
        if store.get(ImageId(id)).is_some() {
            ids.push(ImageId(id));
        }
        id = id.saturating_add(1);
    }
    ids
}

fn remap(nodes: &mut [Node], map: &BTreeMap<ImageId, ImageId>) {
    for node in nodes {
        match node {
            Node::Bitmap(b) => {
                if let Some(new) = map.get(&b.image) {
                    b.image = *new;
                }
            }
            Node::Group(g) => {
                if let Some(mask) = g.mask.as_mut() {
                    remap(mask, map);
                }
                remap(&mut g.children, map);
            }
            Node::Mesh(m) => remap(&mut m.content, map),
            Node::Fill(_) | Node::Stroke(_) => {}
        }
    }
}

/// The frames of a scene, ready for the exporters.
#[derive(Debug)]
pub struct SceneFrames<'a> {
    scene: Scene,
    cast: &'a Cast,
    poses: Vec<Vec<PuppetPose>>,
    store: ImageStore,
    remaps: BTreeMap<String, BTreeMap<ImageId, ImageId>>,
}

impl<'a> SceneFrames<'a> {
    /// Evaluates the whole scene (every frame, in order) and merges the cast's pictures into one store.
    ///
    /// # Errors
    /// Whatever [`Evaluator::new`] reports.
    pub fn new(scene: &Scene, cast: &'a Cast) -> Result<Self> {
        let mut evaluator = Evaluator::new(scene, &cast.templates())?;
        let frames = i64::from(evaluator.scene().frame_count());
        let poses = evaluator.bake(frames);
        let scene = evaluator.scene().clone();
        let mut store = ImageStore::new();
        let mut remaps = BTreeMap::new();
        for track in scene.tracks.iter().filter(|t| t.enabled) {
            if remaps.contains_key(&track.puppet) {
                continue;
            }
            let Some(member) = cast.get(&track.puppet) else { continue };
            let mut map = BTreeMap::new();
            for id in store_ids(member.puppet.images()) {
                if let Some(image) = member.puppet.images().get(id) {
                    map.insert(id, store.insert(image.clone()));
                }
            }
            remaps.insert(track.puppet.clone(), map);
        }
        Ok(Self { scene, cast, poses, store, remaps })
    }

    /// The scene being rendered (sanitised).
    #[must_use]
    pub const fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The pose of track `track` at `frame`.
    #[must_use]
    pub fn pose(&self, track: usize, frame: u32) -> Option<&PuppetPose> {
        self.poses.get(usize::try_from(frame).ok()?)?.get(track)
    }

    /// Tracks that have no puppet in the cast (and so draw nothing).
    #[must_use]
    pub fn missing(&self) -> Vec<&str> {
        self.scene.tracks.iter().filter(|t| t.enabled && self.cast.get(&t.puppet).is_none()).map(|t| t.puppet.as_str()).collect()
    }
}

impl FrameSource for SceneFrames<'_> {
    fn frame_count(&self) -> u32 {
        u32::try_from(self.poses.len()).unwrap_or(u32::MAX)
    }

    fn size(&self) -> Size {
        self.scene.size
    }

    fn fps(&self) -> f64 {
        self.scene.fps_f64()
    }

    fn display_list(&self, index: u32) -> DisplayList {
        let mut list = DisplayList::new(self.scene.size, self.scene.background);
        let centre = Vec2::new(self.scene.size.width / 2.0, self.scene.size.height / 2.0);
        for (i, track) in self.scene.tracks.iter().enumerate() {
            if !track.enabled {
                continue;
            }
            let (Some(member), Some(pose), Some(map)) = (self.cast.get(&track.puppet), self.pose(i, index), self.remaps.get(&track.puppet)) else { continue };
            let mut nodes = member.puppet.evaluate(pose).nodes;
            remap(&mut nodes, map);
            let opacity =
                f32::from(u8::try_from(anim_core::num::round_f64_to_u32(track.placement.opacity.clamp(0.0, 1.0) * 255.0).min(255)).unwrap_or(255)) / 255.0;
            list.nodes.push(Node::Group(Group {
                transform: track.placement.matrix(centre, member.skin, member.handle),
                color: ColorTransform::alpha(opacity),
                children: nodes,
                key: u64::try_from(i).ok(),
                ..Group::default()
            }));
        }
        list
    }

    fn images(&self) -> &ImageStore {
        &self.store
    }
}

/// Builds the frames of a scene and renders frame `frame` to an image. Convenience for previews and tests.
///
/// # Errors
/// [`Error::Invalid`] for a frame past the end, or whatever building the frames reports.
pub fn render_frame(scene: &Scene, cast: &Cast, frame: u32) -> Result<anim_export::RgbaFrame> {
    let frames = SceneFrames::new(scene, cast)?;
    if frame >= frames.frame_count() {
        return Err(Error::invalid(format!("frame {frame} is past the end ({} frames)", frames.frame_count())));
    }
    anim_export::render_frame(&frames, frame, &anim_export::RasterOptions::default()).map_err(|e| Error::invalid(e.to_string()))
}
