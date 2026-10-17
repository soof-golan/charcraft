//! Putting posed puppets on the stage: one picture store for the whole cast and the display list of a frame.
//!
//! Every puppet brings its own pictures with ids that start at zero, so the stage copies them into one store and
//! renumbers the nodes of each puppet. The result is the same display list `charcraft-record`'s frame source builds,
//! but for any set of poses, so the window can mix played-back tracks with the one being performed live.

use std::collections::BTreeMap;

use anim_core::{ColorTransform, Rgba8, Size, Vec2};
use anim_display::{DisplayList, Group, Image, ImageId, ImageStore, Node};
use anim_render::{Background, RenderError, RenderOptions, Renderer, straight_rgba};
use charcraft_puppet::PuppetPose;
use charcraft_record::{Cast, Scene};

/// The cast with all its pictures in one store.
#[derive(Debug)]
pub struct Stage {
    cast: Cast,
    store: ImageStore,
    remaps: BTreeMap<String, BTreeMap<ImageId, ImageId>>,
}

/// The pixels a render produced, straight alpha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, straight alpha.
    pub rgba: Vec<u8>,
}

impl Picture {
    fn of(image: &Image) -> Self {
        Self { width: image.width, height: image.height, rgba: straight_rgba(image) }
    }
}

impl Stage {
    /// Merges the pictures of every puppet of `cast`.
    #[must_use]
    pub fn new(cast: Cast) -> Self {
        let mut store = ImageStore::new();
        let mut remaps = BTreeMap::new();
        for key in cast.keys() {
            let Some(member) = cast.get(key) else { continue };
            remaps.insert(key.to_owned(), store.merge(member.puppet.images()));
        }
        Self { cast, store, remaps }
    }

    /// The cast.
    #[must_use]
    pub const fn cast(&self) -> &Cast {
        &self.cast
    }

    /// The merged pictures.
    #[must_use]
    pub const fn images(&self) -> &ImageStore {
        &self.store
    }

    /// The display list of a scene with one pose per track (a missing pose draws the track neutral).
    #[must_use]
    pub fn display_list(&self, scene: &Scene, poses: &[PuppetPose]) -> DisplayList {
        let mut list = DisplayList::new(scene.size, scene.background);
        let centre = Vec2::new(scene.size.width / 2.0, scene.size.height / 2.0);
        let neutral = PuppetPose::neutral();
        for (i, track) in scene.tracks.iter().enumerate() {
            if !track.enabled {
                continue;
            }
            let (Some(member), Some(map)) = (self.cast.get(&track.puppet), self.remaps.get(&track.puppet)) else { continue };
            let mut nodes = member.puppet.evaluate(poses.get(i).unwrap_or(&neutral)).nodes;
            Node::remap_images(&mut nodes, map);
            let opacity = anim_core::num::f64_to_f32(track.placement.opacity.clamp(0.0, 1.0));
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

    /// Renders a scene frame at `scale` (1 = stage size).
    ///
    /// # Errors
    /// [`RenderError`] for sizes the renderer refuses.
    pub fn render_scene(&self, renderer: &mut Renderer, scene: &Scene, poses: &[PuppetPose], scale: f64) -> Result<Picture, RenderError> {
        self.render_scene_image(renderer, scene, poses, scale, None, false).map(|image| Picture::of(&image))
    }

    /// Renders a scene frame to the renderer's premultiplied image (what the video encoder takes). `pixels`
    /// overrides the output size (video needs even sizes); with `transparent` the stage background is left out.
    ///
    /// # Errors
    /// [`RenderError`] for sizes the renderer refuses.
    pub fn render_scene_image(
        &self,
        renderer: &mut Renderer,
        scene: &Scene,
        poses: &[PuppetPose],
        scale: f64,
        pixels: Option<(u32, u32)>,
        transparent: bool,
    ) -> Result<Image, RenderError> {
        let mut options = RenderOptions::scaled_stage(scene.size, scale);
        if let Some((width, height)) = pixels {
            options.width = width;
            options.height = height;
        }
        if transparent {
            options = options.with_background(Background::Transparent);
        }
        let list = self.display_list(scene, poses);
        renderer.render(&list, &self.store, &options)
    }

    /// Renders one puppet alone at `scale`, on `background` (transparent when `None`).
    ///
    /// # Errors
    /// [`RenderError`] for sizes the renderer refuses; `Ok(None)` when the cast has no such puppet.
    pub fn render_puppet(
        &self,
        renderer: &mut Renderer,
        key: &str,
        pose: &PuppetPose,
        scale: f64,
        background: Option<Rgba8>,
    ) -> Result<Option<Picture>, RenderError> {
        let Some(member) = self.cast.get(key) else { return Ok(None) };
        let size: Size = member.puppet.size;
        let backdrop = background.map_or(Background::Transparent, Background::Solid);
        let options = RenderOptions::scaled_stage(size, scale).with_background(backdrop);
        let list = member.puppet.evaluate(pose);
        Ok(Some(renderer.render(&list, member.puppet.images(), &options).map(|image| Picture::of(&image))?))
    }
}

#[cfg(test)]
mod tests {
    use anim_core::{FrameRate, Tick};
    use charcraft_puppet::Viseme;
    use charcraft_record::{CastMember, PuppetTrack};

    use super::*;

    fn stage() -> (Stage, Scene) {
        let mut cast = Cast::new();
        cast.insert("a", CastMember::new(crate::demo::puppet().unwrap()));
        cast.insert("b", CastMember::new(crate::demo::puppet().unwrap()));
        let mut scene = Scene::new("s", Size::new(800.0, 450.0), FrameRate::FPS_30, Tick::from_seconds_f64(1.0));
        let mut left = PuppetTrack::new("A", "a");
        left.placement.position = Vec2::new(-200.0, 0.0);
        left.placement.anchor = Vec2::new(320.0, 360.0);
        left.placement.scale = Vec2::new(0.5, 0.5);
        let mut right = left.clone();
        right.name = "B".into();
        right.puppet = "b".into();
        right.placement.position = Vec2::new(200.0, 0.0);
        scene.add_track(left).unwrap();
        scene.add_track(right).unwrap();
        (Stage::new(cast), scene)
    }

    #[test]
    fn two_puppets_draw_side_by_side_with_their_own_pictures() {
        let (stage, scene) = stage();
        let mut renderer = Renderer::new();
        let poses = vec![PuppetPose::neutral().with_viseme(Viseme::Ah), PuppetPose::neutral()];
        let pic = stage.render_scene(&mut renderer, &scene, &poses, 1.0).unwrap();
        assert_eq!((pic.width, pic.height), (800, 450));
        let px = |x: usize, y: usize| pic.rgba.get((y * 800 + x) * 4..(y * 800 + x) * 4 + 3).unwrap().to_vec();
        assert_ne!(px(200, 150), px(10, 10), "the left puppet covers its half");
        assert_ne!(px(600, 150), px(10, 10), "the right puppet covers its half");
    }

    #[test]
    fn a_single_puppet_renders_alone_and_unknown_keys_are_none() {
        let (stage, _) = stage();
        let mut renderer = Renderer::new();
        let pic = stage.render_puppet(&mut renderer, "a", &PuppetPose::neutral(), 0.5, None).unwrap().unwrap();
        assert_eq!((pic.width, pic.height), (320, 360));
        assert!(stage.render_puppet(&mut renderer, "nobody", &PuppetPose::neutral(), 1.0, None).unwrap().is_none());
    }
}
