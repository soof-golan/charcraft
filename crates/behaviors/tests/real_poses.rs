//! Predict, render, inspect on the real sample puppets: head turn, tilt and move, draggers, eye gaze and blink.
//! Predictions are in `tests/fixtures/visual/real-puppets.expect.md`. Skipped unless `ANICRAFT_SAMPLES` points at the
//! sample folder; renders for inspection go to `$ANICRAFT_VISUAL_OUT/charcraft-puppet-behaviors/real-*.png` and are
//! never committed (they are derived from Adobe-authored files).
#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::print_stdout,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::many_single_char_names,
    clippy::missing_const_for_fn,
    clippy::float_cmp,
    reason = "test code"
)]

mod common;

use std::path::PathBuf;

use anim_artwork::Limits;
use anim_core::{Point, Rgba8, Vec2};
use anim_display::Image;
use anim_render::{Background, RenderOptions};
use charcraft_behaviors::{BehaviorStack, FaceInput, Rig, Script, ScriptEvent, simulate};
use charcraft_project::Project;
use charcraft_puppet::{HandleKind, Puppet, PuppetPose};

fn samples() -> Option<PathBuf> {
    std::env::var_os("ANICRAFT_SAMPLES").map(PathBuf::from)
}

struct Subject {
    name: &'static str,
    puppet: Puppet,
    stack: BehaviorStack,
    rig: Rig,
}

fn from_puppet_file(root: &std::path::Path, file: &'static str, name: &'static str) -> Subject {
    let project = Project::open(root.join(file)).unwrap();
    let puppet = Puppet::from_project(&project, None).unwrap();
    let model = project.model().unwrap();
    let item = model.root_puppets().next().unwrap();
    let rig = Rig::from_puppet(&puppet);
    let defaults = project.manifest().unwrap().behavior_defaults();
    let (stack, _) = BehaviorStack::from_bound(rig.clone(), &item.behaviors, &defaults);
    Subject { name, puppet, stack, rig }
}

fn from_psd(root: &std::path::Path, file: &'static str, name: &'static str) -> Subject {
    let path = root.join("chproj_full/news anchors/Ch Media/Gathered Media").join(file);
    let tree = anim_artwork::import_path(&path, &Limits::default()).unwrap();
    let puppet = Puppet::from_artwork(&tree).unwrap();
    let rig = Rig::from_puppet(&puppet);
    let stack = BehaviorStack::standard(rig.clone());
    Subject { name, puppet, stack, rig }
}

fn subjects() -> Option<Vec<Subject>> {
    let root = samples()?;
    Some(vec![
        from_puppet_file(&root, "Boy.puppet", "boy"),
        from_puppet_file(&root, "blood_cells.puppet", "girl"),
        from_psd(&root, "male_anchor.psd", "male"),
        from_psd(&root, "female_anchor.psd", "female"),
    ])
}

/// Frames a pose is settled for (the smoothing lags about 0.15 s).
const SETTLE: u32 = 60;

fn face_pose(s: &mut Subject, face: FaceInput) -> PuppetPose {
    let script = Script::new(30.0, SETTLE).with(ScriptEvent::Face { frame: 0, face });
    simulate(&mut s.stack, &script, 1).unwrap().pop().unwrap().pose
}

fn neutral_pose(s: &mut Subject) -> PuppetPose {
    simulate(&mut s.stack, &Script::new(30.0, SETTLE), 1).unwrap().pop().unwrap().pose
}

fn render_on(puppet: &Puppet, pose: &PuppetPose, background: Background) -> Image {
    let list = puppet.evaluate(pose);
    let (w, h) = (puppet.size.width as u32, puppet.size.height as u32);
    anim_render::render(&list, puppet.images(), &RenderOptions::new(w, h).with_background(background)).unwrap()
}

fn render(puppet: &Puppet, pose: &PuppetPose) -> Image {
    render_on(puppet, pose, Background::Solid(Rgba8::new(255, 255, 255, 255)))
}

/// Pixel counts of the 4-connected regions of pixels with some opacity, largest first.
fn regions(img: &Image) -> Vec<usize> {
    let (w, h) = (img.width as usize, img.height as usize);
    let solid = |x: usize, y: usize| img.premultiplied_rgba[(y * w + x) * 4 + 3] > 24;
    let mut seen = vec![false; w * h];
    let mut sizes = Vec::new();
    for y0 in 0..h {
        for x0 in 0..w {
            if seen[y0 * w + x0] || !solid(x0, y0) {
                continue;
            }
            let mut size = 0;
            let mut stack = vec![(x0, y0)];
            seen[y0 * w + x0] = true;
            while let Some((x, y)) = stack.pop() {
                size += 1;
                for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
                    if nx < w && ny < h && !seen[ny * w + nx] && solid(nx, ny) {
                        seen[ny * w + nx] = true;
                        stack.push((nx, ny));
                    }
                }
            }
            sizes.push(size);
        }
    }
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    sizes
}

fn alpha_img(s: &Subject, pose: &PuppetPose) -> Image {
    render_on(&s.puppet, pose, Background::Transparent)
}

fn differing(a: &Image, b: &Image) -> usize {
    let (pa, _) = a.premultiplied_rgba.as_chunks::<4>();
    let (pb, _) = b.premultiplied_rgba.as_chunks::<4>();
    pa.iter().zip(pb).filter(|(p, q)| p.iter().zip(*q).any(|(x, y)| x.abs_diff(*y) > 6)).count()
}

fn save(name: &str, subject: &str, imgs: &[&Image]) {
    common::save_sheet(&format!("real-{subject}-{name}"), imgs);
}

#[test]
fn head_poses_on_real_puppets() {
    let Some(mut all) = subjects() else { return };
    for s in &mut all {
        let neutral = neutral_pose(s);
        let base = render(&s.puppet, &neutral);
        let base_regions = regions(&alpha_img(s, &neutral));
        println!("== {}: {}x{}, regions at neutral {:?}", s.name, base.width, base.height, base_regions);
        let cases: Vec<(&str, FaceInput)> = vec![
            ("turn-right", FaceInput { head_yaw: 30.0, head_pitch: 20.0, ..FaceInput::default() }),
            ("turn-left", FaceInput { head_yaw: -30.0, ..FaceInput::default() }),
            ("tilt-right", FaceInput { head_roll: 20.0, ..FaceInput::default() }),
            ("tilt-left", FaceInput { head_roll: -20.0, ..FaceInput::default() }),
            ("move-right", FaceInput { head_translation: Vec2::new(0.6, 0.0), ..FaceInput::default() }),
            ("move-up", FaceInput { head_translation: Vec2::new(0.0, 0.6), ..FaceInput::default() }),
            ("move-down", FaceInput { head_translation: Vec2::new(0.0, -0.6), ..FaceInput::default() }),
            ("move-tilt", FaceInput { head_translation: Vec2::new(0.4, 0.3), head_roll: 20.0, ..FaceInput::default() }),
            ("move-tilt-left", FaceInput { head_translation: Vec2::new(-0.4, 0.0), head_roll: -25.0, ..FaceInput::default() }),
        ];
        let mut renders = vec![base.clone()];
        for (name, face) in cases {
            let pose = face_pose(s, face);
            let img = render(&s.puppet, &pose);
            let n = regions(&alpha_img(s, &pose));
            println!(
                "{}: {name}: head {:?} rot {:.1} deg, regions {:?}, {} px differ from neutral",
                s.name,
                pose.head.translation,
                pose.head.rotation.to_degrees(),
                n,
                differing(&base, &img)
            );
            renders.push(img);
        }
        let refs: Vec<&Image> = renders.iter().collect();
        save("head", s.name, &refs);
    }
}

#[test]
fn draggers_on_real_puppets() {
    let Some(mut all) = subjects() else { return };
    for s in &mut all {
        let neutral = neutral_pose(s);
        let base = render(&s.puppet, &neutral);
        let handles: Vec<_> = s.rig.handles_of(HandleKind::Draggable).cloned().collect();
        println!("== {}: {} draggable handles", s.name, handles.len());
        let mut renders = vec![base.clone()];
        for (i, h) in handles.iter().enumerate() {
            let at: Point = h.driven_position(&PuppetPose::neutral());
            let target = Point::new(at.x + 120.0, at.y - 200.0);
            let mut script = Script::new(30.0, SETTLE).with(ScriptEvent::Pointer { frame: 2, id: 0, x: at.x, y: at.y, down: true });
            for step in 1..=20_u32 {
                let t = f64::from(step) / 20.0;
                script = script.with(ScriptEvent::Pointer {
                    frame: 2 + step,
                    id: 0,
                    x: (target.x - at.x).mul_add(t, at.x),
                    y: (target.y - at.y).mul_add(t, at.y),
                    down: true,
                });
            }
            let pose = simulate(&mut s.stack, &script, 1).unwrap().pop().unwrap().pose;
            let driven = h.driven_position(&pose);
            println!(
                "{}: drag {} from ({:.0},{:.0}) toward ({:.0},{:.0}) ends at ({:.0},{:.0})",
                s.name, h.key, at.x, at.y, target.x, target.y, driven.x, driven.y
            );
            let img = render(&s.puppet, &pose);
            println!("   regions {:?}, {} px differ", regions(&alpha_img(s, &pose)), differing(&base, &img));
            renders.push(img);
            if i >= 4 {
                break;
            }
        }
        let refs: Vec<&Image> = renders.iter().collect();
        save("drag", s.name, &refs);
    }
}

#[test]
fn eyes_on_real_puppets() {
    let Some(mut all) = subjects() else { return };
    for s in &mut all {
        let neutral = neutral_pose(s);
        let base = render(&s.puppet, &neutral);
        let cases: Vec<(&str, FaceInput)> = vec![
            ("gaze-right", FaceInput { gaze: Vec2::new(1.0, 0.0), ..FaceInput::default() }),
            ("gaze-left", FaceInput { gaze: Vec2::new(-1.0, 0.0), ..FaceInput::default() }),
            ("gaze-down", FaceInput { gaze: Vec2::new(0.0, 1.0), ..FaceInput::default() }),
            ("gaze-up", FaceInput { gaze: Vec2::new(0.0, -1.0), ..FaceInput::default() }),
            ("gaze-corner-tilt", FaceInput { gaze: Vec2::new(1.0, 1.0), head_roll: 20.0, ..FaceInput::default() }),
            ("blink-both", FaceInput { eye_open_left: 0.0, eye_open_right: 0.0, ..FaceInput::default() }),
            ("blink-left", FaceInput { eye_open_left: 0.0, ..FaceInput::default() }),
            ("blink-right-tilt", FaceInput { eye_open_right: 0.0, head_roll: -15.0, ..FaceInput::default() }),
        ];
        let mut renders = vec![base.clone()];
        for (name, face) in cases {
            let pose = face_pose(s, face);
            let img = render(&s.puppet, &pose);
            println!(
                "{}: {name}: gaze {:?}, blink {:.2}/{:.2}, {} px differ from neutral",
                s.name,
                pose.gaze,
                pose.blink_left,
                pose.blink_right,
                differing(&base, &img)
            );
            renders.push(img);
        }
        let refs: Vec<&Image> = renders.iter().collect();
        save("eyes", s.name, &refs);
    }
}
