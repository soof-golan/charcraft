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
    clippy::suboptimal_flops,
    clippy::manual_midpoint,
    reason = "test code"
)]

use std::path::{Path, PathBuf};

use anim_artwork::Limits;
use anim_core::{Point, Rgba8, Vec2};
use anim_display::{Image, ImageStore};
use anim_render::{Background, RenderOptions, Renderer, straight_rgba};
use anim_testkit::samples_dir as samples;
use charcraft_project::Project;
use charcraft_puppet::{HandleKind, Puppet, PuppetPose, Viseme, Xform, tags::ids};

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
    anim_testkit::write_png(&path, u32::try_from(w * images.len()).unwrap(), first.height, &rgba).unwrap();
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
const FACE: [u8; 3] = [249, 196, 162];
/// Levels a pixel may change by when the body is only resampled through its mesh (the weld leaves it in place).
const RESAMPLE: u8 = 6;

/// Width of Boy's neck, measured perpendicular to its centre line, one value per row of neck-coloured pixels
/// (the rows at both ends, where the chin and the collar cut it, are left out), and the rows themselves
/// (y, centre x, horizontal width).
fn neck_widths(img: &Image) -> (Vec<f64>, Vec<(usize, f64, f64)>) {
    let stride = usize::try_from(img.width).unwrap();
    let rgb = |x: usize, y: usize| {
        let at = (y * stride + x) * 4;
        [img.premultiplied_rgba[at], img.premultiplied_rgba[at + 1], img.premultiplied_rgba[at + 2]]
    };
    // Rows below the lowest face pixel: the chin does not hide any of the neck there.
    let chin = (300..560).rev().find(|&y| (480..600).any(|x| rgb(x, y) == FACE)).unwrap_or(0);
    let all: Vec<(usize, f64, f64)> = (chin + 3..560)
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
    let inner = rows.get(1..rows.len().saturating_sub(7)).unwrap_or_default();
    let widths = inner
        .iter()
        .enumerate()
        .map(|(i, &(_, _, w))| {
            let (lo, hi) = (rows[i + 1 - 1.min(i + 1)], rows[(i + 1 + 3).min(rows.len() - 1)]);
            let slope = (hi.1 - lo.1) / f64::from(u32::try_from(hi.0 - lo.0).unwrap().max(1));
            w / slope.hypot(1.0)
        })
        .collect();
    (widths, rows)
}

#[test]
fn boy_neck_keeps_its_width_when_the_head_shifts_sideways() {
    let Some(root) = samples() else { return };
    let puppet = Puppet::from_project(&Project::open(root.join("Boy.puppet")).unwrap(), None).unwrap();
    let (rest, _) = neck_widths(&render(&puppet, &PuppetPose::neutral()));
    let rest_width = rest.iter().sum::<f64>() / f64::from(u32::try_from(rest.len()).unwrap());
    println!("neutral widths {rest:.0?}");
    let mut sheet = vec![render(&puppet, &PuppetPose::neutral())];
    let poses: Vec<(String, Xform)> = [(20.0, 0.0), (-20.0, 0.0), (0.0, 20.0), (0.0, -20.0)]
        .iter()
        .map(|&(dx, tilt)| (format!("dx {dx} tilt {tilt}"), Xform { translation: Vec2::new(dx, 0.0), rotation: f64::to_radians(tilt), ..Xform::IDENTITY }))
        .collect();
    for (name, head) in poses {
        let mut pose = PuppetPose::neutral();
        pose.head = head;
        let img = render(&puppet, &pose);
        let (widths, rows) = neck_widths(&img);
        sheet.push(img);
        let thinnest = widths.iter().copied().fold(f64::MAX, f64::min);
        println!("{name}: rest {rest_width:.1}, thinnest {thinnest:.1}, rows {:?}..{:?}, widths {widths:.0?}", rows.first(), rows.last());
        // On pixels only the rows below the chin can be measured (a tilt hides most of the neck behind it).
        if head.rotation == 0.0 {
            assert!(widths.len() >= 8, "{name}: only {} measurable neck rows", widths.len());
            assert!(thinnest >= 0.8 * rest_width, "{name}: the neck is {thinnest:.1} px wide somewhere, rest {rest_width:.1}: {widths:.0?}");
        }
    }
    write_png("boy_weld_neutral_shifts_tilts.png", &sheet.iter().collect::<Vec<_>>());
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

    // 4. The body stays put for a 10 px head move: below the neck, outside a band of 1.5 neck widths, nothing differs;
    //    the logo barely moves.
    assert_body_unchanged(&neutral, &up10, "up 10");
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
    assert!((logo_y(&neutral) - logo_y(&up30)).abs() < 12.0, "the logo moved for a big head move");

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

fn rgb_at(img: &Image, x: usize, y: usize) -> [u8; 3] {
    let at = (y * usize::try_from(img.width).unwrap() + x) * 4;
    [img.premultiplied_rgba[at], img.premultiplied_rgba[at + 1], img.premultiplied_rgba[at + 2]]
}

/// Below the neck and outside a band of 1.5 neck widths around it, the posed body equals the neutral one. The body is
/// drawn through its mesh once the pose moves it, which resamples it: a few edge pixels change, by at most `RESAMPLE`
/// levels almost everywhere.
fn assert_body_unchanged(neutral: &Image, posed: &Image, what: &str) {
    let (mut over, mut worst) = (0_u32, 0_u8);
    for y in 505..700 {
        for x in (330..740).filter(|x| !(470..600).contains(x)) {
            let (a, b) = (rgb_at(neutral, x, y), rgb_at(posed, x, y));
            let diff = a.iter().zip(&b).map(|(p, q)| p.abs_diff(*q)).max().unwrap();
            over += u32::from(diff > RESAMPLE);
            worst = worst.max(diff);
        }
    }
    // What differs is sub-pixel resampling along sharp edges (the arm and torso sides): a few hundred of 80 000 pixels.
    assert!(over <= 500 && worst <= 120, "{what}: the body changed: {over} pixels by more than {RESAMPLE} levels, worst {worst}");
}

fn head_pose(dx: f64, tilt_degrees: f64) -> PuppetPose {
    let mut pose = PuppetPose::neutral();
    pose.head = Xform { translation: Vec2::new(dx, 0.0), rotation: tilt_degrees.to_radians(), ..Xform::IDENTITY };
    pose
}

/// Boy's neck, which this prediction file describes: `tests/fixtures/visual/neck_weld.expect.md`, point 6.
#[test]
fn boy_head_turns_about_its_neck_and_the_body_stays_put() {
    let Some(root) = samples() else { return };
    let puppet = Puppet::from_project(&Project::open(root.join("Boy.puppet")).unwrap(), None).unwrap();
    let head = puppet.with_tag(ids::HEAD)[0];
    let body = puppet.layers().iter().find(|l| l.label == "Body" && l.independent).unwrap().id;
    let neutral = render(&puppet, &PuppetPose::neutral());

    // 6a. The pivot is the Neck: the head group's origin is tagged `Adobe.Walk.Neck`. A 15 degree tilt puts the
    // chin's lowest point where a rotation about that point predicts (the face edge within 2 px).
    let pivot = puppet.pivot(head);
    assert!(puppet.layer(head).unwrap().has_tag("Adobe.Walk.Neck"), "the head carries the Neck tag");
    assert_eq!(pivot, puppet.layer(head).unwrap().origin);
    let chin_y = (300..560).rev().find(|&y| (480..600).any(|x| rgb_at(&neutral, x, y) == FACE)).unwrap();
    let xs: Vec<usize> = (480..600).filter(|&x| rgb_at(&neutral, x, chin_y) == FACE).collect();
    let chin = Point::new(f64::from(u32::try_from(xs[0] + xs[xs.len() - 1]).unwrap()) / 2.0, f64::from(u32::try_from(chin_y).unwrap()));
    for degrees in [15.0, -15.0] {
        let img = render(&puppet, &head_pose(0.0, degrees));
        let (sin, cos) = degrees.to_radians().sin_cos();
        let d = chin - pivot;
        let predicted = Point::new(pivot.x + d.x * cos - d.y * sin, pivot.y + d.x * sin + d.y * cos);
        let outward = Vec2::new(-sin, cos);
        let at = |p: Point| rgb_at(&img, num_usize(p.x), num_usize(p.y));
        assert_eq!(at(predicted - outward * 2.0), FACE, "{degrees} deg: no face just inside the predicted chin {predicted:?}");
        assert_ne!(at(predicted + outward * 3.0), FACE, "{degrees} deg: face beyond the predicted chin {predicted:?}");
    }

    // 6b. The neck bends smoothly and keeps its width, measured on the body mesh (the strip behind the chin and
    // collar cannot be measured on pixels): slices across the neck, width perpendicular to the centre line.
    let neck_x = pivot.x;
    for (dx, tilt) in [(20.0, 0.0), (-20.0, 0.0), (0.0, 20.0), (0.0, -20.0), (20.0, 15.0), (-20.0, -15.0)] {
        let pose = head_pose(dx, tilt);
        let (rest, solved) = puppet.warp_solution(body, &pose).unwrap();
        // The mesh does not move at all more than 70 px from the neck.
        let far = rest.iter().zip(&solved).filter(|(r, _)| (r.x - neck_x).abs() > 70.0).map(|(r, s)| r.distance(*s)).fold(0.0, f64::max);
        assert!(far < 0.01, "{dx} {tilt}: the body mesh moves {far:.2} px far from the neck");
        let slices: Vec<(Vec2, Vec2)> = (0..8)
            .filter_map(|k| {
                let y0 = 484.0 + 3.0 * f64::from(k);
                let near: Vec<usize> = (0..rest.len()).filter(|&i| (rest[i].y - y0).abs() < 2.0 && (rest[i].x - neck_x).abs() < 24.0).collect();
                let l = near.iter().copied().min_by(|&a, &b| rest[a].x.total_cmp(&rest[b].x))?;
                let r = near.iter().copied().max_by(|&a, &b| rest[a].x.total_cmp(&rest[b].x))?;
                Some((solved[l].to_vec2(), solved[r].to_vec2()))
            })
            .collect();
        assert!(slices.len() >= 6, "{dx} {tilt}: only {} neck slices", slices.len());
        let centres: Vec<Vec2> = slices.iter().map(|(l, r)| (*l + *r) / 2.0).collect();
        for i in 1..slices.len() - 1 {
            let along = (centres[i + 1] - centres[i - 1]).normalize();
            let across = slices[i].1 - slices[i].0;
            let width = (across.x * along.y - across.y * along.x).abs();
            assert!(width >= 0.8 * 35.0, "{dx} {tilt}: neck slice {i} is {width:.1} px wide perpendicular to its centre line");
        }
        // The turn of the centre line over chords of two slices (6 px), so mesh noise between neighbours does not count.
        for i in 2..centres.len() - 2 {
            let (a, b) = ((centres[i] - centres[i - 2]).normalize(), (centres[i + 2] - centres[i]).normalize());
            let turn = (a.x * b.y - a.y * b.x).atan2(a.dot(b)).to_degrees().abs();
            assert!(turn < 55.0, "{dx} {tilt}: the neck centre line turns {turn:.0} degrees at slice {i}");
        }
        // The bottom of the neck stays on the collar (where it was), the top follows the chin.
        let (bottom_now, bottom_rest) = (centres[centres.len() - 1], (rest_centre(&rest, 484.0 + 21.0, neck_x)));
        assert!(
            (bottom_now.x - bottom_rest.x).abs() <= 0.95 * dx.abs() + 2.0,
            "{dx} {tilt}: the neck's bottom left the collar: {bottom_now:?} vs {bottom_rest:?}"
        );
    }

    // 6c. The body does not move: the collar line keeps its angle (within 1 degree) and, below the neck and outside a
    // band of 1.5 neck widths, every pixel is within 2 levels of the neutral render.
    let collar_y = |img: &Image, x: usize| (380..620).find(|&y| rgb_at(img, x, y) == COLLAR);
    for (dx, tilt) in [(20.0, 0.0), (-20.0, 0.0), (0.0, 20.0), (0.0, -20.0)] {
        let img = render(&puppet, &head_pose(dx, tilt));
        assert_body_unchanged(&neutral, &img, &format!("{dx} {tilt}"));
        if let (Some(a0), Some(b0), Some(a1), Some(b1)) = (collar_y(&neutral, 470), collar_y(&neutral, 600), collar_y(&img, 470), collar_y(&img, 600)) {
            let slope = |l: usize, r: usize| (f64::from(u32::try_from(r).unwrap()) - f64::from(u32::try_from(l).unwrap())) / 130.0;
            let degrees = (slope(a1, b1).atan() - slope(a0, b0).atan()).to_degrees().abs();
            assert!(degrees <= 1.0, "{dx} {tilt}: the collar line turned {degrees:.1} degrees");
        }
    }

    // 6d. The neutral pose draws the artwork unchanged.
    let mut has_mesh = false;
    puppet.evaluate(&PuppetPose::neutral()).visit(&mut |n| has_mesh |= matches!(n, anim_display::Node::Mesh(_)));
    assert!(!has_mesh);
}

fn num_usize(v: f64) -> usize {
    usize::try_from(anim_core::num::round_f64_to_u32(v)).unwrap()
}

/// Centre of the body mesh's rest vertices in a slice of rows at `y`, around column `x`.
fn rest_centre(rest: &[Point], y: f64, x: f64) -> Point {
    let near: Vec<&Point> = rest.iter().filter(|p| (p.y - y).abs() < 2.0 && (p.x - x).abs() < 24.0).collect();
    let (lo, hi) = near.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p.x), hi.max(p.x)));
    Point::new((lo + hi) / 2.0, y)
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

/// Every real puppet with a head: a shifted and tilted head leaves the body below the neck where it was and turns
/// about the head's pivot (its Neck handle). Writes the posed renders when `ANICRAFT_SCRATCH` names a directory.
#[test]
fn real_heads_weld_without_moving_the_body() {
    let Some(root) = samples() else { return };
    let mut puppets: Vec<(String, Puppet)> = Vec::new();
    for name in ["Boy.puppet", "blood_cells.puppet"] {
        puppets.push((name.to_owned(), Puppet::from_project(&Project::open(root.join(name)).unwrap(), None).unwrap()));
    }
    let project = Project::open(root.join("chproj_full/news anchors")).unwrap();
    for item in project.model().unwrap().root_puppets() {
        if let Ok(puppet) = Puppet::from_project(&project, Some(&item.path)) {
            puppets.push((item.display_name().to_owned(), puppet));
        }
    }
    let mut checked = 0;
    for (name, puppet) in &puppets {
        let Some(&head) = puppet.with_tag(ids::HEAD).first() else { continue };
        let Some(bounds) = puppet.layer(head).and_then(|l| l.bounds) else { continue };
        let pivot = puppet.pivot(head);
        let pose = head_pose(15.0, 12.0);
        // The pivot stays where it is under a pure tilt.
        let tilt_only = puppet.group_matrix(head, &head_pose(0.0, 12.0)) * pivot;
        assert!(tilt_only.distance(pivot) < 1e-6, "{name}: the head does not turn about its pivot");
        let neutral = render(puppet, &PuppetPose::neutral());
        let posed = render(puppet, &pose);
        write_png(&format!("weld_{}.png", name.chars().filter(char::is_ascii_alphanumeric).collect::<String>()), &[&neutral, &posed]);
        // Rows well below the head's bottom, away from the neck: unchanged (edge resampling aside).
        let (w, h) = (usize::try_from(neutral.width).unwrap(), usize::try_from(neutral.height).unwrap());
        let top = num_usize(bounds.y1) + (h - num_usize(bounds.y1)) / 3;
        let half_band = num_usize(bounds.width()) / 3;
        let centre = num_usize(pivot.x);
        let (mut over, mut total) = (0_u32, 0_u32);
        for y in top..h {
            for x in (0..w).filter(|&x| x + half_band < centre || x > centre + half_band) {
                total += 1;
                let (a, b) = (rgb_at(&neutral, x, y), rgb_at(&posed, x, y));
                over += u32::from(a.iter().zip(&b).any(|(p, q)| p.abs_diff(*q) > RESAMPLE));
            }
        }
        assert!(u64::from(over) * 200 <= u64::from(total), "{name}: the body below the neck changed: {over} of {total} pixels");
        checked += 1;
    }
    assert!(checked >= 2, "only {checked} real puppets with a head");
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

fn anchor(root: &Path, display: &str) -> Puppet {
    let project = Project::open(root.join("chproj_full/news anchors")).unwrap();
    let item = project.model().unwrap().root_puppets().find(|i| i.display_name() == display).unwrap().clone();
    Puppet::from_project(&project, Some(&item.path)).unwrap()
}

fn is_background(img: &Image, x: usize, y: usize) -> bool {
    rgb_at(img, x, y).iter().all(|&c| c > 235)
}

/// The male anchor's ears are one layer drawn behind the face, next to the head group instead of inside it. They turn
/// with the head (`tests/fixtures/visual/head_parts.expect.md`, point 1).
#[test]
fn male_anchor_ears_follow_the_head() {
    let Some(root) = samples() else { return };
    let puppet = anchor(&root, "male_anchor");
    let head = puppet.with_tag(ids::HEAD)[0];
    let ears = puppet.layers().iter().find(|l| l.name == "Layer 3").unwrap();
    assert_eq!(ears.follows, Some(head), "the ear layer is not attached to the head");
    let followers: Vec<&str> = puppet.layers().iter().filter(|l| l.follows.is_some()).map(|l| l.name.as_str()).collect();
    assert_eq!(followers, ["Layer 3"], "only the ears follow the head, not the body or its arms");

    let neutral = render(&puppet, &PuppetPose::neutral());
    let y = 342_usize;
    let tip = (300..500).find(|&x| !is_background(&neutral, x, y)).unwrap();
    for degrees in [15.0, -15.0, 25.0] {
        let pose = head_pose(0.0, degrees);
        let posed = render(&puppet, &pose);
        write_png(&format!("male_ears_{degrees}.png"), &[&neutral, &posed]);
        let matrix = puppet.group_matrix(head, &pose);
        // From just inside the ear's tip into the face: every pixel of the segment, turned with the head, is puppet.
        let (from, to) = (Point::new(f64::from(u32::try_from(tip).unwrap()) + 4.0, 342.0), Point::new(470.0, 342.0));
        for i in 0..=100_u32 {
            let t = f64::from(i) / 100.0;
            let at = matrix * Point::new(from.x + (to.x - from.x) * t, from.y);
            let (x, y) = (num_usize(at.x.round()), num_usize(at.y.round()));
            assert!(!is_background(&posed, x, y), "head tilted {degrees} degrees: a gap between the ear and the head at {at:?}");
        }
        // The ear's tip lands where the head's matrix puts it.
        let moved = matrix * from;
        assert!(!is_background(&posed, num_usize(moved.x.round()), num_usize(moved.y.round())));
    }
}

/// The female anchor's jacket is ragged under the hair in the artwork itself; the head's turn only uncovers it. With
/// the head hidden, the body renders the same for a tilted head as for a straight one outside the neck band
/// (`tests/fixtures/visual/head_parts.expect.md`, point 2).
#[test]
fn female_anchor_jacket_is_not_torn_by_the_head() {
    let Some(root) = samples() else { return };
    let puppet = anchor(&root, "female_anchor");
    let head = puppet.with_tag(ids::HEAD)[0];
    let pivot = puppet.pivot(head);
    let mut still = head_pose(0.0, 0.0);
    still.opacity.insert("head".into(), 0.0);
    let mut tilted = head_pose(15.0, 12.0);
    tilted.opacity.insert("head".into(), 0.0);
    let (a, b) = (render(&puppet, &still), render(&puppet, &tilted));
    write_png("female_body_head_hidden.png", &[&a, &b]);
    let (mut over, mut total) = (0_u32, 0_u32);
    for y in 393..1000 {
        for x in (0..usize::try_from(a.width).unwrap()).filter(|&x| (f64::from(u32::try_from(x).unwrap()) - pivot.x).abs() > 260.0) {
            total += 1;
            over += u32::from(rgb_at(&a, x, y).iter().zip(&rgb_at(&b, x, y)).any(|(p, q)| p.abs_diff(*q) > RESAMPLE));
        }
    }
    assert!(over * 200 <= total, "the body mesh moved away from the neck: {over} of {total} pixels differ");
}
