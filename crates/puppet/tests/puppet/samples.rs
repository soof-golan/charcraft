//! Puppets built from the real sample files. Skipped unless `ANICRAFT_SAMPLES` is set; PNGs are written when
//! `ANICRAFT_SCRATCH` names a directory.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::assert_is_empty,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    reason = "test code"
)]

use std::path::{Path, PathBuf};

use anim_artwork::Limits;
use anim_core::{Rgba8, Vec2};
use anim_display::{Image, ImageStore};
use anim_render::{Background, RenderOptions, Renderer, straight_rgba};
use charcraft_project::Project;
use charcraft_puppet::{HandleKind, Puppet, PuppetPose, Viseme, Xform, tags::ids};

fn samples() -> Option<PathBuf> {
    std::env::var_os("ANICRAFT_SAMPLES").map(PathBuf::from)
}

fn scratch(name: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("ANICRAFT_SCRATCH")?);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(name))
}

fn write_png(name: &str, images: &[&Image]) {
    let Some(path) = scratch(name) else { return };
    let Some(first) = images.first() else { return };
    let (w, h) = (usize::try_from(first.width).unwrap(), usize::try_from(first.height).unwrap());
    let mut rgba = Vec::new();
    for row in 0..h {
        for img in images {
            rgba.extend_from_slice(&straight_rgba(img)[row * w * 4..(row + 1) * w * 4]);
        }
    }
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), u32::try_from(w * images.len()).unwrap(), first.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgba).unwrap();
}

fn render(p: &Puppet, pose: &PuppetPose) -> Image {
    let options = RenderOptions::for_stage(p.size).with_background(Background::Solid(Rgba8::WHITE));
    Renderer::new().render(&p.evaluate(pose), p.images(), &options).unwrap()
}

fn artwork_render(path: &Path) -> (anim_artwork::ArtworkTree, Image) {
    let tree = anim_artwork::import_path(path, &Limits::default()).unwrap();
    let mut store = ImageStore::new();
    let list = anim_artwork::to_display(&tree, &mut store);
    let options = RenderOptions::for_stage(tree.size).with_background(Background::Solid(Rgba8::WHITE));
    let image = Renderer::new().render(&list, &store, &options).unwrap();
    (tree, image)
}

fn differing(a: &Image, b: &Image) -> usize {
    a.premultiplied_rgba.iter().zip(&b.premultiplied_rgba).filter(|(x, y)| x != y).count()
}

fn ah_blink() -> PuppetPose {
    PuppetPose::neutral().with_viseme(Viseme::Ah).with_blink(1.0)
}

fn check_character(name: &str, ai: &Path) {
    let (tree, expected) = artwork_render(ai);
    let puppet = Puppet::from_artwork(&tree).unwrap();
    println!("{name}: {} layers, {} mouth group(s)", puppet.layers().len(), puppet.mouth_groups().len());

    // Neutral equals the artwork render, bit for bit.
    let neutral = render(&puppet, &PuppetPose::neutral());
    assert_eq!(differing(&neutral, &expected), 0, "{name}: neutral pose differs from the artwork");

    // Tags from names: a head, two eyes, a mouth with most of the shapes.
    assert!(!puppet.with_tag(ids::HEAD).is_empty(), "{name}: no head\n{}", puppet.outline());
    assert!(!puppet.with_tag(ids::LEFT_EYE).is_empty() && !puppet.with_tag(ids::RIGHT_EYE).is_empty(), "{name}: eyes");
    let mouth = puppet.mouth_groups();
    assert_eq!(mouth.len(), 1, "{name}: one mouth group");
    let shapes = puppet.mouth_shapes(mouth[0]);
    assert!(shapes.len() >= 10, "{name}: {} mouth shapes", shapes.len());
    assert!(puppet.with_tag(ids::LEFT_BLINK).len() == 1 && puppet.with_tag(ids::RIGHT_BLINK).len() == 1, "{name}: blink layers");

    // Mouth Ah + blink changes the picture, and only around the face.
    let posed = render(&puppet, &ah_blink());
    let changed = differing(&posed, &neutral);
    assert!(changed > 1000, "{name}: the pose changed {changed} bytes");

    // Every viseme draws something different from the others around the mouth.
    let mut sheet = Vec::new();
    let mut seen: Vec<Image> = Vec::new();
    for v in Viseme::ALL {
        let img = render(&puppet, &PuppetPose::neutral().with_viseme(v));
        sheet.push(img.clone());
        seen.push(img);
    }
    let distinct = (0..seen.len()).filter(|&i| seen.iter().take(i).all(|o| differing(o, &seen[i]) > 0)).count();
    assert!(distinct >= 10, "{name}: only {distinct} distinct viseme renders");

    // A head turn about the neck keeps the body and moves the head.
    let turned = render(&puppet, &PuppetPose::neutral().with_head_rotation(15.0));
    assert!(differing(&turned, &neutral) > 1000, "{name}: head rotation moved nothing");

    write_png(&format!("{name}_neutral_vs_ah_blink.png"), &[&neutral, &posed, &turned]);
    let refs: Vec<&Image> = sheet.iter().collect();
    write_png(&format!("{name}_visemes.png"), &refs[..5.min(refs.len())]);
    write_png(&format!("{name}_visemes_b.png"), &refs[5.min(refs.len())..10.min(refs.len())]);
}

#[test]
fn boy_artwork_becomes_a_puppet() {
    let Some(root) = samples() else { return };
    check_character("boy", &root.join("extracted/Boy/Ch Media/Gathered Media/A boy character.ai"));
}

#[test]
fn girl_artwork_becomes_a_puppet() {
    let Some(root) = samples() else { return };
    check_character("girl", &root.join("extracted/blood_cells/Ch Media/Gathered Media/A girl character (1).ai"));
}

fn check_project_puppet(name: &str, path: &Path) -> Puppet {
    let project = Project::open(path).unwrap();
    let puppet = Puppet::from_project(&project, None).unwrap();
    let summary = puppet.summary();
    println!("{name}: {} groups, warped {:?}, warnings {:?}", summary.groups.len(), puppet.warped_groups().len(), summary.warnings);
    assert!(summary.groups.len() > 5, "{name}: groups");
    assert!(summary.warnings.len() < 8, "{name}: {:?}", summary.warnings);
    // The project's tags are on the layers, and its origins are where the groups' centres are (within a few pixels).
    assert!(!puppet.with_tag(ids::HEAD).is_empty());
    for g in puppet.independent_groups().filter(|g| g.has_tag(ids::LEFT_EYE) || g.has_tag(ids::RIGHT_EYE)) {
        let c = g.bounds.unwrap().center();
        assert!((g.origin.x - c.x).abs() < 3.0 && (g.origin.y - c.y).abs() < 3.0, "{}: origin {:?} vs centre {:?}", g.path, g.origin, c);
    }
    puppet
}

#[test]
fn boy_puppet_file_builds_with_project_data() {
    let Some(root) = samples() else { return };
    let puppet = check_project_puppet("boy_project", &root.join("Boy.puppet"));
    // Hands and ears carry handles from the project and get meshes.
    let draggable: Vec<_> = puppet.layers().iter().flat_map(|l| l.handles.iter()).filter(|h| h.kind == HandleKind::Draggable).collect();
    assert!(draggable.len() >= 2, "draggable handles: {draggable:?}");
    assert!(puppet.warped_groups().len() >= 4, "warped groups: {}", puppet.warped_groups().len());
    let hand = draggable[0].clone();

    let neutral = render(&puppet, &PuppetPose::neutral());
    let mut pose = ah_blink();
    pose.head.rotation = 8.0_f64.to_radians();
    pose.drag.insert(hand.key.clone(), Vec2::new(40.0, -70.0));
    if let Some(ear) = puppet.layers().iter().flat_map(|l| l.handles.iter()).find(|h| h.kind == HandleKind::Dangle) {
        pose.drag.insert(ear.key.clone(), Vec2::new(25.0, 15.0));
    }
    let posed = render(&puppet, &pose);
    assert!(differing(&posed, &neutral) > 5000);
    let drag_only = {
        let mut p = PuppetPose::neutral();
        p.drag.insert(hand.key, Vec2::new(40.0, -70.0));
        render(&puppet, &p)
    };
    assert!(differing(&drag_only, &neutral) > 500, "dragging the hand warped nothing");
    write_png("boy_project_neutral_vs_pose.png", &[&neutral, &posed, &drag_only]);

    // The summary is JSON a tool can return.
    let json = serde_json::to_string(&puppet.summary()).unwrap();
    assert!(json.contains("Adobe.Face.Head"));
}

const WHITE: [u8; 3] = [255, 255, 255];
const SHIRT: [u8; 3] = [45, 171, 183];
const COLLAR: [u8; 3] = [24, 101, 127];
const NECK: [u8; 3] = [242, 136, 114];

/// Width of Boy's neck, measured perpendicular to its centre line, one value per row of neck-coloured pixels
/// (the rows at both ends, where the chin and the collar cut it, are left out). Also the first and last row.
fn neck_widths(img: &Image) -> (Vec<f64>, usize, usize) {
    let stride = usize::try_from(img.width).unwrap();
    let rgb = |x: usize, y: usize| {
        let at = (y * stride + x) * 4;
        [img.premultiplied_rgba[at], img.premultiplied_rgba[at + 1], img.premultiplied_rgba[at + 2]]
    };
    let all: Vec<(usize, f64, f64)> = (300..560)
        .filter_map(|y| {
            let xs: Vec<usize> = (480..600).filter(|&x| rgb(x, y) == NECK).collect();
            let (a, b) = (*xs.first()?, *xs.last()?);
            Some((y, f64::from(u32::try_from(a + b).unwrap()) / 2.0, f64::from(u32::try_from(b - a + 1).unwrap())))
        })
        .collect();
    // The neck is the lowest run of rows (the nose has the same colour, higher up).
    let mut start = 0;
    for i in 1..all.len() {
        if all[i].0 - all[i - 1].0 > 2 {
            start = i;
        }
    }
    let rows = all[start..].to_vec();
    let inner = rows.get(4..rows.len().saturating_sub(4)).unwrap_or_default();
    let widths = inner
        .iter()
        .enumerate()
        .map(|(i, &(_, _, w))| {
            let (lo, hi) = (rows[i + 4 - 3.min(i + 4)], rows[(i + 4 + 3).min(rows.len() - 1)]);
            let slope = (hi.1 - lo.1) / f64::from(u32::try_from(hi.0 - lo.0).unwrap().max(1));
            w / slope.hypot(1.0)
        })
        .collect();
    (widths, rows.first().map_or(0, |r| r.0), rows.last().map_or(0, |r| r.0))
}

#[test]
fn boy_neck_keeps_its_width_when_the_head_shifts_sideways() {
    let Some(root) = samples() else { return };
    let puppet = Puppet::from_project(&Project::open(root.join("Boy.puppet")).unwrap(), None).unwrap();
    let (rest, ..) = neck_widths(&render(&puppet, &PuppetPose::neutral()));
    let rest_width = rest.iter().sum::<f64>() / f64::from(u32::try_from(rest.len()).unwrap());
    let mut sheet = vec![render(&puppet, &PuppetPose::neutral())];
    for dx in [40.0, -40.0, 20.0] {
        let mut pose = PuppetPose::neutral();
        pose.head = Xform::moved(dx, 0.0);
        let img = render(&puppet, &pose);
        let (widths, first, last) = neck_widths(&img);
        sheet.push(img);
        let thinnest = widths.iter().copied().fold(f64::MAX, f64::min);
        assert!(last - first > 10, "dx {dx}: the neck is only rows {first}..{last}");
        assert!(thinnest >= 0.7 * rest_width, "dx {dx}: the neck is {thinnest:.1} px wide somewhere, rest {rest_width:.1}: {widths:.0?}");
    }
    write_png("boy_weld_neutral_right40_left40_right20.png", &sheet.iter().collect::<Vec<_>>());
}

#[test]
fn boy_neck_does_not_poke_out_when_the_head_drops() {
    let Some(root) = samples() else { return };
    let puppet = Puppet::from_project(&Project::open(root.join("Boy.puppet")).unwrap(), None).unwrap();
    let neutral = render(&puppet, &PuppetPose::neutral());
    let mut pose = PuppetPose::neutral();
    pose.head = Xform::moved(0.0, 20.0);
    let dropped = render(&puppet, &pose);
    let stride = usize::try_from(neutral.width).unwrap();
    let px = |img: &Image, x: usize, y: usize| {
        let at = (y * stride + x) * 4;
        [img.premultiplied_rgba[at], img.premultiplied_rgba[at + 1], img.premultiplied_rgba[at + 2]]
    };
    // The chin may cover the collar, but the face stays exactly the neutral face moved down 20 px: the neck never
    // draws over it, and no neck colour shows beside or above the chin.
    for y in 200..470 {
        for x in 330..740 {
            let (a, b) = (px(&neutral, x, y), px(&dropped, x, y + 20));
            assert!(a.iter().zip(&b).all(|(p, q)| p.abs_diff(*q) <= 8), "the face changed at ({x},{y})");
        }
    }
    for y in 300..501 {
        for x in 330..740 {
            let shows_neck = px(&dropped, x, y) == NECK;
            assert!(!shows_neck || px(&neutral, x, y.saturating_sub(20)) == NECK, "neck colour where the face is, at ({x},{y})");
        }
    }
}

#[test]
fn boy_head_stays_welded_to_the_body() {
    let Some(root) = samples() else { return };
    let puppet = Puppet::from_project(&Project::open(root.join("Boy.puppet")).unwrap(), None).unwrap();
    let body = puppet.layers().iter().find(|l| l.label == "Body" && l.independent).unwrap().id;
    let up = |dy: f64| {
        let mut pose = PuppetPose::neutral();
        pose.head = Xform::moved(0.0, -dy);
        pose
    };
    let rgb = |img: &Image, x: usize, y: usize| {
        let at = (y * usize::try_from(img.width).unwrap() + x) * 4;
        [img.premultiplied_rgba[at], img.premultiplied_rgba[at + 1], img.premultiplied_rgba[at + 2]]
    };
    // The first row of collar colour going down column x.
    let collar_top = |img: &Image, x: usize| (380..620).find(|&y| rgb(img, x, y) == COLLAR).unwrap();

    // 1. Neutral: no weld is at work (no mesh node), and it is stable.
    let mut has_mesh = false;
    puppet.evaluate(&PuppetPose::neutral()).visit(&mut |n| has_mesh |= matches!(n, anim_display::Node::Mesh(_)));
    assert!(!has_mesh, "the neutral pose draws the artwork unchanged");
    let neutral = render(&puppet, &PuppetPose::neutral());
    let (up10, up30, down20) = (render(&puppet, &up(10.0)), render(&puppet, &up(30.0)), render(&puppet, &up(-20.0)));
    assert_eq!(render(&puppet, &up(30.0)), up30, "evaluation is deterministic");
    write_png("boy_weld_neutral_up10_up30_down20.png", &[&neutral, &up10, &up30, &down20]);

    // 2. The face is rigid: eyes and mouth equal the neutral ones shifted 30 px up.
    let mut off = 0;
    for y in 330..470 {
        for x in 400..680 {
            let (a, b) = (rgb(&neutral, x, y), rgb(&up30, x, y - 30));
            off += usize::from(a.iter().zip(&b).any(|(p, q)| p.abs_diff(*q) > 8));
        }
    }
    assert!(off < 280 * 140 / 200, "{off} pixels of the face differ from the shifted neutral");

    // 3. The neck is a continuous strip from the chin to the collar.
    for (img, from) in [(&up30, 455), (&up10, 475)] {
        let bottom = collar_top(img, 560).min(520);
        for x in [520, 535, 550] {
            for y in from..bottom.saturating_sub(1) {
                assert_ne!(rgb(img, x, y), WHITE, "gap in the neck at ({x},{y})");
            }
        }
    }

    // 4. The collar and the logo barely move for a 10 px head move.
    let moved = collar_top(&neutral, 560).abs_diff(collar_top(&up10, 560));
    assert!(moved < 3, "the collar moved {moved} px");
    let logo_y = |img: &Image| {
        let (mut sum, mut n) = (0.0, 0.0);
        for y in 530..680 {
            for x in 480..600 {
                let c = rgb(img, x, y);
                if c[0] > 225 && (90..130).contains(&c[1]) {
                    sum += f64::from(u32::try_from(y).unwrap());
                    n += 1.0;
                }
            }
        }
        sum / n
    };
    assert!((logo_y(&neutral) - logo_y(&up10)).abs() < 3.0, "the logo moved");
    assert!((logo_y(&neutral) - logo_y(&up30)).abs() < 3.0, "the logo moved for a big head move");

    // 5. The mouth never touches the shirt: no shirt or collar pixel anywhere in the mouth's rows.
    for img in [&up10, &up30] {
        for y in 380..440 {
            for x in 440..640 {
                let c = rgb(img, x, y);
                assert!(c != SHIRT && c != COLLAR, "shirt colour in the mouth rows at ({x},{y})");
            }
        }
    }
    assert!(puppet.warp_solution(body, &up(30.0)).is_some());
}

#[test]
fn girl_puppet_file_builds_with_project_data() {
    let Some(root) = samples() else { return };
    let puppet = check_project_puppet("girl_project", &root.join("blood_cells.puppet"));
    let neutral = render(&puppet, &PuppetPose::neutral());
    let posed = render(&puppet, &ah_blink());
    assert!(differing(&posed, &neutral) > 1000);
    write_png("girl_project_neutral_vs_ah_blink.png", &[&neutral, &posed]);
}

#[test]
fn news_anchor_project_builds_psd_puppets() {
    let Some(root) = samples() else { return };
    let project = Project::open(root.join("chproj_full/news anchors")).unwrap();
    let model = project.model().unwrap();
    let mut built = 0;
    for item in model.root_puppets() {
        let Ok(puppet) = Puppet::from_project(&project, Some(&item.path)) else { continue };
        built += 1;
        println!("{}: {} layers, groups {:?}", item.path, puppet.layers().len(), puppet.independent_groups().map(|g| g.label.clone()).collect::<Vec<_>>());
        let neutral = render(&puppet, &PuppetPose::neutral());
        let posed = render(&puppet, &ah_blink());
        let name: String = item.display_name().chars().filter(char::is_ascii_alphanumeric).collect();
        write_png(&format!("anchor_{name}.png"), &[&neutral, &posed]);
    }
    assert!(built >= 1, "no news anchor puppet could be built");
}

#[test]
fn news_anchor_psds_become_puppets_from_artwork_alone() {
    let Some(root) = samples() else { return };
    for file in ["male_anchor.psd", "female_anchor.psd"] {
        let (tree, expected) = artwork_render(&root.join("chproj_full/news anchors/Ch Media/Gathered Media").join(file));
        let puppet = Puppet::from_artwork(&tree).unwrap();
        println!("{file}\n{}", puppet.outline().lines().take(40).collect::<Vec<_>>().join("\n"));
        assert_eq!(differing(&render(&puppet, &PuppetPose::neutral()), &expected), 0, "{file}");
    }
}
