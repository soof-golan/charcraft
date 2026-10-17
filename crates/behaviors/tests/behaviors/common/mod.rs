//! Shared helpers of the visual tests: synthetic artwork, rendering, probes, crops and goldens.
#![allow(
    dead_code,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::print_stderr,
    clippy::many_single_char_names,
    clippy::chunks_exact_to_as_chunks,
    reason = "test code"
)]

use std::path::{Path, PathBuf};

use anim_artwork::{ArtNode, ArtworkTree};
use anim_core::{Point, Rgba8, Size};
use anim_display::Image;
use anim_render::{Background, RenderOptions, straight_rgba};
use charcraft_behaviors::{BehaviorStack, ChainLink, Rig, RigHandle, Script, SimFrame, behaviors, simulate};
use charcraft_puppet::{HandleKind, Puppet, PuppetPose, Viseme};
use serde_json::Value;

/// A behavior stack on `rig` made of the behaviors `parts` names (id and parameters), with a fixed seed.
pub fn stack_of(rig: Rig, parts: &[(&str, Value)]) -> BehaviorStack {
    let mut stack = BehaviorStack::new(rig);
    for (id, params) in parts {
        stack.push(behaviors::create_with(id, params).unwrap());
    }
    stack.with_seed(1)
}

/// Runs `script` through `stack` and returns every frame.
pub fn play(stack: &mut BehaviorStack, script: &Script) -> Vec<SimFrame> {
    simulate(stack, script, 1).unwrap()
}

/// A rig whose only handle is a hair lock dangling from the head.
pub fn hair_rig() -> Rig {
    let head = ChainLink {
        is_root: false,
        is_head: true,
        tags: std::collections::BTreeSet::default(),
        path: "Head".into(),
        label: "head".into(),
        origin: Point::new(100.0, 100.0),
    };
    let hair = RigHandle { chain: vec![head], reach: 120.0, ..RigHandle::new("head/Hair", HandleKind::Dangle, Point::new(200.0, 100.0)) };
    Rig::new(Size::new(1000.0, 600.0)).with_handle(hair)
}

pub const SKIN: [u8; 3] = [240, 190, 160];
pub const WHITE: [u8; 3] = [255, 255, 255];
pub const BLACK: [u8; 3] = [10, 10, 10];
pub const BLUE: [u8; 3] = [30, 60, 220];
pub const GRAY: [u8; 3] = [120, 120, 130];
pub const RED: [u8; 3] = [200, 30, 30];
pub const GREEN: [u8; 3] = [30, 170, 60];
pub const PURPLE: [u8; 3] = [130, 40, 160];

/// Where renders for inspection go: `$ANICRAFT_VISUAL_OUT/charcraft-puppet-behaviors`, or nowhere when it is unset (CI).
pub fn scratch() -> Option<PathBuf> {
    anim_testkit::out_dir("charcraft-puppet-behaviors")
}

/// Writes a PNG into [`scratch`] when there is one.
fn save_png(name: &str, w: u32, h: u32, rgba: &[u8]) {
    if let Some(dir) = scratch() {
        write_png(&dir.join(format!("{name}.png")), w, h, rgba);
    }
}

pub fn rect(name: &str, x: f64, y: f64, w: u32, h: u32, rgb: [u8; 3]) -> ArtNode {
    let n = usize::try_from(w * h).unwrap();
    let px: Vec<u8> = [rgb[0], rgb[1], rgb[2], 255].iter().copied().cycle().take(n * 4).collect();
    ArtNode::raster(name, Image::new(w, h, px).unwrap(), Point::new(x, y))
}

pub const fn hidden(mut node: ArtNode) -> ArtNode {
    node.visible = false;
    node
}

pub fn tree(w: u32, h: u32, children: Vec<ArtNode>) -> ArtworkTree {
    ArtworkTree { size: Size::new(f64::from(w), f64::from(h)), root: ArtNode::group("", children), warnings: Vec::new() }
}

pub fn viseme_color(i: usize) -> [u8; 3] {
    let i = u8::try_from(i).unwrap();
    [20 + i * 15, 220 - i * 12, 40 + i * 5]
}

/// A 14-shape mouth group at 100..140 x 100..120; the shapes in `keep` exist, the first of `visible` is shown.
pub fn mouth_with(keep: &[Viseme], visible: Viseme) -> ArtNode {
    let children = Viseme::ALL
        .iter()
        .enumerate()
        .filter(|(_, v)| keep.contains(v))
        .map(|(i, v)| {
            let node = rect(v.label(), 100.0, 100.0, 40, 20, viseme_color(i));
            if *v == visible { node } else { hidden(node) }
        })
        .collect();
    ArtNode::group("mouth", children)
}

pub fn eye(side: &str, x: f64, wide_lid: bool) -> ArtNode {
    let (lid_x, lid_y, lid_w, lid_h) = if wide_lid { (x - 8.0, 70.0, 56, 4) } else { (x, 69.0, 40, 6) };
    ArtNode::group(
        format!("{side} Eye"),
        vec![
            rect("White", x, 60.0, 40, 24, WHITE),
            rect(&format!("+ {side} Pupil"), x + 14.0, 66.0, 12, 12, BLACK),
            hidden(rect(&format!("{side} Blink"), lid_x, lid_y, lid_w, lid_h, BLUE)),
        ],
    )
}

/// The 240x240 face puppet: skin face 40..200 x 20..140, eyes at 130 (left) and 50 (right), a mouth, a body below.
pub fn face_tree(wide_lids: bool, with_body: bool) -> ArtworkTree {
    let head = ArtNode::group(
        "Head",
        vec![rect("Face", 40.0, 20.0, 160, 120, SKIN), eye("Left", 130.0, wide_lids), eye("Right", 50.0, wide_lids), mouth_with(&Viseme::ALL, Viseme::Neutral)],
    );
    let mut kids = Vec::new();
    if with_body {
        kids.push(ArtNode::group("Body", vec![rect("Torso", 60.0, 140.0, 120, 100, GRAY)]));
    }
    kids.push(head);
    tree(240, 240, kids)
}

pub fn render(puppet: &Puppet, pose: &PuppetPose) -> Image {
    let w = puppet.size.width as u32;
    let h = puppet.size.height as u32;
    let list = puppet.evaluate(pose);
    anim_render::render(&list, puppet.images(), &RenderOptions::new(w, h).with_background(Background::Solid(Rgba8::WHITE))).unwrap()
}

pub fn px(img: &Image, x: i32, y: i32) -> [u8; 3] {
    if x < 0 || y < 0 || x >= img.width as i32 || y >= img.height as i32 {
        return [1, 2, 3];
    }
    let at = usize::try_from((y as u32 * img.width + x as u32) * 4).unwrap();
    [img.premultiplied_rgba[at], img.premultiplied_rgba[at + 1], img.premultiplied_rgba[at + 2]]
}

pub fn near(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
    a.iter().zip(b).all(|(p, q)| p.abs_diff(q) <= tol)
}

#[track_caller]
pub fn assert_px(img: &Image, x: i32, y: i32, want: [u8; 3], what: &str) {
    let got = px(img, x, y);
    assert!(near(got, want, 6), "{what}: pixel ({x},{y}) is {got:?}, expected {want:?}");
}

pub fn is_dark(c: [u8; 3]) -> bool {
    c.iter().all(|&v| v < 70)
}

/// 4-connected regions of pixels that are not the white background.
pub fn components(img: &Image) -> usize {
    let (w, h) = (img.width as usize, img.height as usize);
    let solid = |x: usize, y: usize| {
        let at = (y * w + x) * 4;
        img.premultiplied_rgba[at + 3] > 0 && !near([img.premultiplied_rgba[at], img.premultiplied_rgba[at + 1], img.premultiplied_rgba[at + 2]], WHITE, 3)
    };
    let mut seen = vec![false; w * h];
    let mut count = 0;
    for y0 in 0..h {
        for x0 in 0..w {
            if seen[y0 * w + x0] || !solid(x0, y0) {
                continue;
            }
            count += 1;
            let mut stack = vec![(x0, y0)];
            seen[y0 * w + x0] = true;
            while let Some((x, y)) = stack.pop() {
                let mut visit = |nx: usize, ny: usize| {
                    if !seen[ny * w + nx] && solid(nx, ny) {
                        seen[ny * w + nx] = true;
                        stack.push((nx, ny));
                    }
                };
                if x > 0 {
                    visit(x - 1, y);
                }
                if y > 0 {
                    visit(x, y - 1);
                }
                if x + 1 < w {
                    visit(x + 1, y);
                }
                if y + 1 < h {
                    visit(x, y + 1);
                }
            }
        }
    }
    count
}

/// Number of non-background pixels.
pub fn ink(img: &Image) -> usize {
    img.premultiplied_rgba.chunks_exact(4).filter(|p| !near([p[0], p[1], p[2]], WHITE, 3) && p[3] > 0).count()
}

/// Bounding box `[x0, y0, x1, y1)` of pixels near a colour.
pub fn bbox_of(img: &Image, rgb: [u8; 3], tol: u8) -> Option<[i32; 4]> {
    let mut b: Option<[i32; 4]> = None;
    for y in 0..img.height as i32 {
        for x in 0..img.width as i32 {
            if near(px(img, x, y), rgb, tol) {
                b = Some(match b {
                    None => [x, y, x + 1, y + 1],
                    Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1)],
                });
            }
        }
    }
    b
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) {
    let _ = anim_testkit::write_png(path, w, h, rgba);
}

/// Tiles images side by side (4 px gaps) into one PNG at `path`, returning the sheet.
pub fn sheet(imgs: &[&Image]) -> (u32, u32, Vec<u8>) {
    let gap = 4_u32;
    let h = imgs.iter().map(|i| i.height).max().unwrap_or(1);
    let w: u32 = imgs.iter().map(|i| i.width + gap).sum::<u32>().saturating_sub(gap).max(1);
    let mut out = vec![200_u8; (w * h * 4) as usize];
    for p in out.chunks_exact_mut(4) {
        p[3] = 255;
    }
    let mut x0 = 0_u32;
    for img in imgs {
        let rgba = straight_rgba(img);
        for y in 0..img.height {
            let src = (y * img.width * 4) as usize;
            let dst = ((y * w + x0) * 4) as usize;
            out[dst..dst + (img.width * 4) as usize].copy_from_slice(&rgba[src..src + (img.width * 4) as usize]);
        }
        x0 += img.width + gap;
    }
    (w, h, out)
}

/// Saves a contact sheet of renders for inspection under [`scratch`].
pub fn save_sheet(name: &str, imgs: &[&Image]) {
    let (w, h, rgba) = sheet(imgs);
    save_png(name, w, h, &rgba);
}

/// Saves a crop scaled up with nearest-neighbour sampling.
pub fn save_zoom(name: &str, img: &Image, x: u32, y: u32, w: u32, h: u32, k: u32) {
    let crop = anim_testkit::crop_zoom(&straight_rgba(img), img.width, x, y, w, h, k);
    save_png(name, crop.width, crop.height, &crop.rgba);
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/visual")
}

/// Compares a sheet with its golden PNG (a few levels of tolerance per channel). A missing golden is written and
/// fails the test, so a golden always gets looked at once before it counts.
#[track_caller]
pub fn golden(name: &str, imgs: &[&Image]) {
    let (w, h, rgba) = sheet(imgs);
    let path = golden_dir().join(format!("{name}.png"));
    if !path.exists() {
        write_png(&path, w, h, &rgba);
        panic!("golden {} did not exist; wrote it, inspect it and run again", path.display());
    }
    let golden = anim_testkit::read_png(&path).unwrap();
    assert_eq!((golden.width, golden.height), (w, h), "golden {name} has another size");
    let bad = golden.rgba.iter().zip(&rgba).filter(|(a, b)| a.abs_diff(**b) > 4).count();
    if bad > (rgba.len() / 2000).max(16) {
        save_png(&format!("{name}.new"), w, h, &rgba);
        panic!("golden {name}: {bad} channel values differ (new render saved under $ANICRAFT_VISUAL_OUT when it is set)");
    }
}
