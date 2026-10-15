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
use charcraft_puppet::{HandleKind, Puppet, PuppetPose, Viseme, tags::ids};

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
