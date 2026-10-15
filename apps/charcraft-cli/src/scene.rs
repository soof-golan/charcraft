//! Rendering a scene of a project: the puppets on its tracks, placed by their Transform behavior, over each other.
//!
//! Scene space is centred on the middle of the frame with y pointing down, like puppet space. A track item places
//! its puppet by the Transform behavior's `anchorPtX/Y` (a point of the puppet that lands on `positionX/Y`), a
//! scale in percent and a rotation. Tracks are drawn in index order, so a higher index is in front.

use anim_audio::PcmBuffer;
use anim_core::{Affine, Point, Rgba8, Size, Tick, TimeRange, Vec2};
use anim_display::Image;
use anim_render::{Background, RenderOptions, Renderer};
use charcraft_project::media::MediaKind;
use charcraft_project::{BindingKind, Model, Project, PuppetItem, Scene};
use charcraft_puppet::{Puppet, PuppetPose};
use color_eyre::eyre::{Result, WrapErr, bail, eyre};

use crate::perform::{Options, Performance};

/// Where a track item puts its puppet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Scene position of the anchor point.
    pub position: Vec2,
    /// The point of the puppet (relative to its origin) that sits on `position`.
    pub anchor: Vec2,
    /// Scale factors (1 = 100%).
    pub scale: Vec2,
    /// Rotation in degrees, clockwise on screen.
    pub rotation: f64,
    /// Opacity, 0 to 1.
    pub opacity: f64,
}

impl Default for Placement {
    fn default() -> Self {
        Self { position: Vec2::ZERO, anchor: Vec2::ZERO, scale: Vec2::new(1.0, 1.0), rotation: 0.0, opacity: 1.0 }
    }
}

impl Placement {
    /// Reads the parameters of a Transform behavior instance; absent ones keep their defaults.
    #[must_use]
    pub fn from_params(params: &[(String, charcraft_ltab::LValue)]) -> Self {
        let get = |name: &str| params.iter().find(|(k, _)| k == name).and_then(|(_, v)| v.as_f64()).filter(|v| v.is_finite());
        let uniform = get("uniformScale").unwrap_or(100.0) / 100.0;
        Self {
            position: Vec2::new(get("positionX").unwrap_or(0.0), get("positionY").unwrap_or(0.0)),
            anchor: Vec2::new(get("anchorPtX").unwrap_or(0.0), get("anchorPtY").unwrap_or(0.0)),
            scale: Vec2::new(uniform * get("scaleX").unwrap_or(100.0) / 100.0, uniform * get("scaleY").unwrap_or(100.0) / 100.0),
            rotation: get("rotation").unwrap_or(0.0),
            opacity: (get("opacity").unwrap_or(100.0) / 100.0).clamp(0.0, 1.0),
        }
    }

    /// Maps puppet pixels to scene pixels (before any output scaling). `centre` is the middle of the frame, `skin`
    /// the puppet-space position of pixel (0, 0) and `handle` the puppet's origin handle.
    #[must_use]
    pub fn matrix(&self, centre: Vec2, skin: Vec2, handle: Vec2) -> Affine {
        Affine::translate(centre + handle + self.position)
            * Affine::rotate(self.rotation.to_radians())
            * Affine::scale_non_uniform(self.scale.x, self.scale.y)
            * Affine::translate(skin - handle - self.anchor)
    }
}

/// One puppet on the stage.
pub struct StageLayer {
    pub name: String,
    pub puppet: Puppet,
    pub placement: Placement,
    /// Puppet-space position of the artwork's top-left pixel.
    pub skin: Vec2,
    /// The puppet's origin handle in puppet space.
    pub handle: Vec2,
    /// Has a mouth to move (otherwise it is drawn once).
    pub performed: bool,
}

/// Everything needed to render a scene.
pub struct Stage {
    pub name: String,
    pub size: Size,
    pub fps: f64,
    pub duration: Tick,
    /// Back to front.
    pub layers: Vec<StageLayer>,
    pub audio: Option<SceneAudio>,
    pub warnings: Vec<String>,
}

/// The soundtrack of a scene and where it sits.
pub struct SceneAudio {
    pub buffer: PcmBuffer,
    /// Where the clip starts on the scene timeline.
    pub offset: Tick,
    /// How far into the file the clip starts.
    pub trim_in: Tick,
}

fn strip(path: &str) -> &str {
    path.trim_start_matches('/')
}

/// A one-layer artwork tree from PNG bytes (backgrounds and props are plain pictures).
pub fn png_artwork(bytes: &[u8]) -> Result<anim_artwork::ArtworkTree> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().wrap_err("reading the PNG header")?;
    let (width, height) = reader.info().size();
    if width == 0 || height == 0 || width > 16_384 || height > 16_384 || u64::from(width).saturating_mul(u64::from(height)) > 64 * 1024 * 1024 {
        bail!("a {width}x{height} PNG is outside the supported size");
    }
    let mut buf = vec![0_u8; reader.output_buffer_size().ok_or_else(|| eyre!("the PNG is too large"))?];
    let frame = reader.next_frame(&mut buf).wrap_err("decoding the PNG")?;
    let data = buf.get(..frame.buffer_size()).ok_or_else(|| eyre!("short PNG data"))?;
    let rgba: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.as_chunks::<2>().0.iter().flat_map(|&[g, a]| [g, g, g, a]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => bail!("an indexed PNG was not expanded"),
    };
    let image = anim_render::image_from_straight_rgba(width, height, &rgba).map_err(|e| eyre!("{e}"))?;
    let root = anim_artwork::ArtNode::group("", vec![anim_artwork::ArtNode::raster("picture", image, Point::ORIGIN)]);
    Ok(anim_artwork::ArtworkTree { size: Size::new(f64::from(width), f64::from(height)), root, warnings: Vec::new() })
}

/// Builds the puppet of a project item. Pictures (PNG, JPEG) become one-layer puppets.
fn build_puppet(project: &Project, model: &Model, item: &PuppetItem) -> Result<Puppet> {
    let first = match Puppet::from_project(project, Some(&item.path)) {
        Ok(p) => return Ok(p),
        Err(e) => e,
    };
    let picture = item
        .artwork
        .iter()
        .chain(item.bindings.iter().filter(|b| b.kind == BindingKind::Skin).filter_map(|b| b.ppath.as_ref()))
        .flat_map(|p| p.split('/'))
        .find(|s| s.to_lowercase().ends_with(".png"));
    let Some(file_name) = picture else { return Err(first.into()) };
    let media = model
        .media
        .iter()
        .find(|m| m.kind == MediaKind::Artwork && m.file_name.as_deref() == Some(file_name))
        .and_then(|m| m.file.as_ref())
        .ok_or_else(|| eyre!("the picture {file_name} is not in the project"))?;
    let tree = png_artwork(&project.package().read(&media.path)?)?;
    let mut puppet = Puppet::from_artwork(&tree)?;
    puppet.apply_project(model, item);
    Ok(puppet)
}

fn origin_of(item: &PuppetItem) -> Vec2 {
    item.tagged_locations.iter().find(|(name, _)| name == "__user_origin").and_then(|(_, xy)| Some(Vec2::new(*xy.first()?, *xy.get(1)?))).unwrap_or(Vec2::ZERO)
}

fn skin_of(item: &PuppetItem) -> Vec2 {
    item.bindings.iter().find(|b| b.kind == BindingKind::Skin).map_or(Vec2::ZERO, |b| Vec2::new(b.transform.position[0], b.transform.position[1]))
}

fn audio_of(project: &Project, model: &Model, scene: &Scene) -> Result<Option<SceneAudio>> {
    for track in scene.tracks.iter().filter(|t| t.enabled && t.kind.as_deref() == Some("trackKind_audio")) {
        for clip in &track.items {
            let Some(item) = clip.source.as_deref().map(strip).and_then(|s| model.media.iter().find(|m| m.path == s)) else { continue };
            let Some(file) = item.file.as_ref() else { continue };
            let bytes = project.package().read(&file.path)?;
            let buffer = anim_audio::decode_bytes(&bytes, item.file_name.as_deref()).wrap_err_with(|| format!("decoding {}", file.path))?;
            let time = clip.time.as_ref();
            return Ok(Some(SceneAudio {
                buffer,
                offset: Tick::from_seconds_f64(time.map_or(0.0, |t| t.parent_offset)),
                trim_in: Tick::from_seconds_f64(time.map_or(0.0, |t| t.trim_in)),
            }));
        }
    }
    Ok(None)
}

/// Reads a scene: its stage size, puppets and sound.
pub fn load_stage(project: &Project, scene_name: Option<&str>) -> Result<Stage> {
    let model = project.model()?;
    let scene = match scene_name {
        Some(name) => model.scene(name).ok_or_else(|| {
            eyre!("no scene named {name:?}; the project has: {}", model.scenes.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "))
        })?,
        None => model.scenes.first().ok_or_else(|| eyre!("the project has no scenes"))?,
    };
    let mut warnings = Vec::new();
    let mut layers = Vec::new();
    // Tracks are sorted by index and a higher index is drawn later, in front of the lower ones (the desk track sits
    // below the anchors' tracks in the news anchors sample, and the hands rest on the desk).
    for track in scene.tracks.iter().filter(|t| t.enabled && t.kind.as_deref() == Some("trackKind_puppet")) {
        for clip in &track.items {
            let Some(item) = clip.source.as_deref().map(strip).and_then(|s| model.puppet(s)).filter(|p| !p.is_part) else { continue };
            let puppet = match build_puppet(project, &model, item) {
                Ok(p) => p,
                Err(e) => {
                    warnings.push(format!("{}: {e:#}", item.path));
                    continue;
                }
            };
            let transform = clip.behaviors.iter().find(|b| b.behavior == "Adobe.Transform.js" && b.enabled);
            let performed = !puppet.mouth_groups().is_empty() || !puppet.with_tag(charcraft_puppet::tags::ids::LEFT_EYE).is_empty();
            layers.push(StageLayer {
                name: item.display_name().to_owned(),
                puppet,
                placement: transform.map(|t| Placement::from_params(&t.params)).unwrap_or_default(),
                skin: skin_of(item),
                handle: origin_of(item),
                performed,
            });
        }
    }
    let audio = audio_of(project, &model, scene)?;
    Ok(Stage {
        name: scene.name.clone(),
        size: Size::new(f64::from(u32::try_from(scene.width.max(1)).unwrap_or(1)), f64::from(u32::try_from(scene.height.max(1)).unwrap_or(1))),
        fps: scene.fps,
        duration: Tick::from_seconds_f64(scene.duration),
        layers,
        audio,
        warnings,
    })
}

impl SceneAudio {
    /// The audio heard in `[start, start + length)` of scene time, as a buffer starting at `start`.
    #[must_use]
    pub fn window(&self, start: Tick, length: Tick) -> PcmBuffer {
        let rate = self.buffer.sample_rate;
        let channels = self.buffer.channels;
        // Position in the file of scene time `start`.
        let file_start = Tick(start.0.saturating_sub(self.offset.0).saturating_add(self.trim_in.0));
        let lead = Tick(file_start.0.min(0).saturating_neg());
        let lead_frames = anim_audio::tick_to_frame(lead.min(length), rate);
        let mut out = PcmBuffer::silence(rate, channels, lead_frames);
        if length.0 > lead.0 {
            let part = self.buffer.slice_ticks(TimeRange::new(Tick(file_start.0.max(0)), Tick(length.0.saturating_sub(lead.0))));
            out.append(&part);
        }
        out
    }
}

/// Renders frames of a stage.
pub struct StageRenderer<'a> {
    stage: &'a Stage,
    renderer: Renderer,
    performances: Vec<Performance>,
    cache: Vec<Option<Image>>,
    out_scale: f64,
    width: u32,
    height: u32,
    background: Option<Rgba8>,
}

impl<'a> StageRenderer<'a> {
    /// `keys` are the mouth shapes of the scene audio over scene time (empty without sound).
    #[must_use]
    pub fn new(stage: &'a Stage, scale: f64, even: bool, keys: &[anim_lipsync::VisemeKey], length: Tick, seed: u64, background: Option<Rgba8>) -> Self {
        let round = |extent: f64| {
            let n = anim_core::num::round_f64_to_u32((extent * scale).max(1.0));
            if even { n.saturating_add(n % 2) } else { n }
        };
        let performances = (0_u64..)
            .zip(&stage.layers)
            .map(|(i, _)| Performance::new(keys.to_vec(), length, Options { seed: seed.wrapping_add(i), blink: true, breathe: true }))
            .collect();
        Self {
            stage,
            renderer: Renderer::new(),
            performances,
            cache: stage.layers.iter().map(|_| None).collect(),
            out_scale: scale,
            width: round(stage.size.width),
            height: round(stage.size.height),
            background,
        }
    }

    /// Output size in pixels.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn layer_image(&mut self, index: usize, t: Tick) -> Result<Option<Image>> {
        let Some(layer) = self.stage.layers.get(index) else { return Ok(None) };
        if !layer.performed
            && let Some(Some(image)) = self.cache.get(index)
        {
            return Ok(Some(image.clone()));
        }
        let pose = if layer.performed { self.performances.get(index).map_or_else(PuppetPose::neutral, |p| p.pose_at(t)) } else { PuppetPose::neutral() };
        let centre = Vec2::new(self.stage.size.width / 2.0, self.stage.size.height / 2.0);
        let view = Affine::scale(self.out_scale) * layer.placement.matrix(centre, layer.skin, layer.handle);
        let options = RenderOptions::new(self.width, self.height).with_view(view).with_background(Background::Transparent);
        let list = layer.puppet.evaluate(&pose);
        let image = self.renderer.render(&list, layer.puppet.images(), &options).wrap_err_with(|| format!("rendering {}", layer.name))?;
        if !layer.performed
            && let Some(slot) = self.cache.get_mut(index)
        {
            *slot = Some(image.clone());
        }
        Ok(Some(image))
    }

    /// The finished frame at scene time `t`.
    pub fn frame(&mut self, t: Tick) -> Result<Image> {
        let pixels = anim_display::Image::byte_len(self.width, self.height).ok_or_else(|| eyre!("the frame is too large"))?;
        let mut out = vec![0_u8; pixels];
        if let Some(bg) = self.background {
            for px in out.as_chunks_mut::<4>().0 {
                *px = [bg.r, bg.g, bg.b, bg.a];
            }
        }
        for index in 0..self.stage.layers.len() {
            let opacity = self.stage.layers.get(index).map_or(1.0, |l| l.placement.opacity);
            if let Some(image) = self.layer_image(index, t)? {
                over(&mut out, &image.premultiplied_rgba, opacity);
            }
        }
        Image::new(self.width, self.height, out).map_err(|e| eyre!("{e}"))
    }
}

/// Source-over for premultiplied RGBA8, with the source scaled by `opacity`.
pub fn over(dst: &mut [u8], src: &[u8], opacity: f64) {
    let factor = anim_core::num::round_f64_to_u32(opacity.clamp(0.0, 1.0) * 256.0).min(256);
    for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
        let alpha = u32::from(s[3]).saturating_mul(factor) / 256;
        if *s == [0; 4] {
            continue;
        }
        let keep = 255_u32.saturating_sub(alpha);
        for (dc, sc) in d.iter_mut().zip(s) {
            let source = u32::from(*sc).saturating_mul(factor) / 256;
            let mixed = source.saturating_add(u32::from(*dc).saturating_mul(keep).saturating_add(127) / 255);
            *dc = u8::try_from(mixed.min(255)).unwrap_or(255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn over_composites_premultiplied_pixels() {
        let mut dst = vec![255, 255, 255, 255, 0, 0, 0, 0];
        let src = vec![128, 0, 0, 128, 10, 20, 30, 255];
        over(&mut dst, &src, 1.0);
        assert_eq!(&dst[4..], &[10, 20, 30, 255]);
        // Half red over white: red channel stays high, green and blue are halved.
        assert!(dst[0] >= 254 && (126..=129).contains(&dst[1]) && (126..=129).contains(&dst[2]) && dst[3] == 255, "{dst:?}");
    }

    fn encode(color: png::ColorType, width: u32, height: u32, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(data).unwrap();
        out
    }

    fn first_pixels(bytes: &[u8]) -> (Size, Vec<u8>) {
        let tree = png_artwork(bytes).unwrap();
        Puppet::from_artwork(&tree).unwrap();
        let anim_artwork::ArtKind::Group(children) = &tree.root.kind else { panic!("root is a group") };
        let anim_artwork::ArtKind::Raster { image, .. } = &children[0].kind else { panic!("one raster") };
        (tree.size, image.premultiplied_rgba.clone())
    }

    #[test]
    fn pictures_become_one_layer_artwork_in_every_color_type() {
        let (size, px) = first_pixels(&encode(png::ColorType::Rgba, 2, 1, &[255, 0, 0, 255, 0, 255, 0, 128]));
        assert_eq!((size.width, size.height), (2.0, 1.0));
        assert_eq!(&px[..4], &[255, 0, 0, 255]);
        assert_eq!(px[7], 128);
        let (_, px) = first_pixels(&encode(png::ColorType::Rgb, 1, 1, &[1, 2, 3]));
        assert_eq!(px, [1, 2, 3, 255]);
        let (_, px) = first_pixels(&encode(png::ColorType::Grayscale, 1, 1, &[77]));
        assert_eq!(px, [77, 77, 77, 255]);
        let (_, px) = first_pixels(&encode(png::ColorType::GrayscaleAlpha, 1, 1, &[200, 255]));
        assert_eq!(px, [200, 200, 200, 255]);
    }

    #[test]
    fn hostile_pictures_are_errors() {
        assert!(png_artwork(b"").is_err());
        assert!(png_artwork(b"\x89PNG\r\n\x1a\n garbage").is_err());
        let mut truncated = encode(png::ColorType::Rgba, 4, 4, &[9; 64]);
        truncated.truncate(truncated.len() / 2);
        assert!(png_artwork(&truncated).is_err());
    }

    #[test]
    fn placement_reads_transform_parameters() {
        use charcraft_ltab::LValue;
        let params = vec![("anchorPtX".to_owned(), LValue::Int(-516)), ("uniformScale".to_owned(), LValue::Int(50)), ("scaleX".to_owned(), LValue::Int(200))];
        let p = Placement::from_params(&params);
        assert_eq!(p.anchor, Vec2::new(-516.0, 0.0));
        assert_eq!(p.scale, Vec2::new(1.0, 0.5));
        assert_eq!(Placement::from_params(&[]), Placement::default());
    }

    #[test]
    fn opacity_scales_the_source() {
        let mut dst = vec![0, 0, 0, 0];
        over(&mut dst, &[200, 200, 200, 200], 0.5);
        assert!((98..=102).contains(&dst[3]), "{dst:?}");
    }

    #[test]
    fn identity_placement_is_a_translation_by_the_centre() {
        let m = Placement::default().matrix(Vec2::new(960.0, 540.0), Vec2::new(-100.0, -50.0), Vec2::ZERO);
        let p = m * Point::new(0.0, 0.0);
        assert!((p.x - 860.0).abs() < 1e-9 && (p.y - 490.0).abs() < 1e-9);
    }

    #[test]
    fn anchor_and_scale_follow_the_transform_behavior() {
        let placement = Placement { anchor: Vec2::new(-516.0, -122.0), scale: Vec2::new(0.5, 0.5), ..Placement::default() };
        // The puppet's origin lands anchor-offset from the centre, scaled.
        let p = placement.matrix(Vec2::new(960.0, 540.0), Vec2::ZERO, Vec2::ZERO) * Point::new(0.0, 0.0);
        assert!((p.x - (960.0 + 258.0)).abs() < 1e-9 && (p.y - (540.0 + 61.0)).abs() < 1e-9, "{p:?}");
    }
}
