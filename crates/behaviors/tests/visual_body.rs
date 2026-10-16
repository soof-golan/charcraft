//! Predict, render, inspect: draggers, dangles, overlapping independent groups and nested warps. Predictions are
//! in `tests/fixtures/visual/*.expect.md`.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::many_single_char_names,
    clippy::suboptimal_flops,
    clippy::explicit_iter_loop,
    reason = "test code"
)]

mod common;

use anim_artwork::ArtNode;
use anim_core::{Point, Tick, Vec2};
use charcraft_behaviors::behaviors;
use charcraft_behaviors::{BehaviorStack, FaceInput, Inputs, Rig};
use charcraft_puppet::{HandleKind, Puppet, PuppetPose, Xform};
use common::*;
use serde_json::json;

// ---- dragger-extremes --------------------------------------------------------------------------------------

fn torso_puppet() -> Puppet {
    let body = ArtNode::group("Body", vec![rect("Torso", 80.0, 100.0, 80, 100, GRAY), rect("Belt", 80.0, 150.0, 80, 10, RED)]);
    let mut p = Puppet::from_artwork(&tree(400, 300, vec![body])).unwrap();
    let id = p.find_path("Body").unwrap();
    p.add_handle(id, "Fixed", HandleKind::Fixed, Point::new(120.0, 190.0)).unwrap();
    p.add_handle(id, "Draggable", HandleKind::Draggable, Point::new(120.0, 110.0)).unwrap();
    assert_eq!(p.warped_groups(), [id], "{:?}", p.warnings);
    p
}

fn dragger_stack(p: &Puppet, params: &serde_json::Value) -> BehaviorStack {
    let mut s = BehaviorStack::new(Rig::from_puppet(p));
    s.push(behaviors::create_with("Adobe.Dragger3.js", params).unwrap());
    s.with_seed(1)
}

fn frame(stack: &mut BehaviorStack, frame: u32, pointers: &[(u32, f64, f64, bool)]) -> PuppetPose {
    let mut i = Inputs::at(Tick::from_seconds_f64(f64::from(frame) / 30.0));
    for &(id, x, y, down) in pointers {
        i = i.with_pointer(id, x, y, down);
    }
    stack.step(&i)
}

const KEY: &str = "Body/Draggable";

#[test]
fn dragger_extremes() {
    let p = torso_puppet();
    let neutral = render(&p, &PuppetPose::neutral());
    let mut stack = dragger_stack(&p, &json!({}));
    let _ = frame(&mut stack, 0, &[]);

    // 1. Press exactly on the handle: nothing moves.
    let pose = frame(&mut stack, 1, &[(0, 120.0, 110.0, true)]);
    assert_eq!(render(&p, &pose), neutral, "a press without motion is the neutral picture");

    // 2. Far pulls while held.
    let targets = [(150.0, 110.0), (270.0, 110.0), (120.0, -300.0), (-5000.0, 110.0)];
    let mut imgs = Vec::new();
    for (i, (tx, ty)) in targets.iter().enumerate() {
        let pose = frame(&mut stack, 2 + u32::try_from(i).unwrap(), &[(0, *tx, *ty, true)]);
        let d = pose.drag.get(KEY).copied().unwrap();
        assert!((d.x - (tx - 120.0)).abs() < 1e-9 && (d.y - (ty - 110.0)).abs() < 1e-9, "drag {d:?} for pointer ({tx},{ty})");
        let img = render(&p, &pose);
        assert!(ink(&img) > 500, "pull {i}: the canvas is blank");
        assert_eq!(components(&img), 1, "pull {i} ({tx},{ty}): the drawing tore into pieces");
        assert_px(&img, 120, 191, GRAY, &format!("pull {i}: the fixed base holds"));
        if i == 1 {
            assert_px(&img, 262, 112, GRAY, "the top follows the handle");
        }
        imgs.push(img);
    }
    let refs: Vec<&anim_display::Image> = imgs.iter().collect();
    save_sheet("dragger-extremes", &refs);
    save_zoom("dragger-extremes-pull270-base", &imgs[1], 90, 150, 70, 60, 8);
    golden("dragger-extremes", &refs);

    // 5. Release: half way through the glide, then home.
    let release = 6_u32;
    let pose = frame(&mut stack, release, &[]);
    let full = pose.drag.get(KEY).copied().unwrap_or(Vec2::ZERO);
    assert!(full.x < -4000.0, "the release starts from where the pointer left it: {full:?}");
    let pose = frame(&mut stack, release + 3, &[]);
    let half = pose.drag.get(KEY).copied().unwrap();
    assert!((half.x - full.x / 2.0).abs() < 1.0, "half way: {half:?} vs {full:?}");
    let pose = frame(&mut stack, release + 7, &[]);
    assert!(pose.drag.is_empty(), "home after the release: {:?}", pose.drag);
    assert_eq!(render(&p, &pose), neutral, "released handle is back to the neutral picture");

    // 4. Too far from the handle: nothing is grabbed.
    let mut stack = dragger_stack(&p, &json!({}));
    for f in 0..5 {
        let pose = frame(&mut stack, f, &[(0, 120.0 + 20.0 * f64::from(f), 40.0, true)]);
        assert!(pose.drag.is_empty(), "frame {f}: grabbed from 70 px away");
    }

    // 6. A second pointer cannot steal a held handle.
    let mut stack = dragger_stack(&p, &json!({}));
    let _ = frame(&mut stack, 0, &[(0, 120.0, 110.0, true)]);
    let _ = frame(&mut stack, 1, &[(0, 150.0, 110.0, true)]);
    let _ = frame(&mut stack, 2, &[(0, 150.0, 110.0, true), (1, 150.0, 110.0, true)]);
    let pose = frame(&mut stack, 3, &[(0, 150.0, 110.0, true), (1, 300.0, 250.0, true)]);
    let d = pose.drag.get(KEY).copied().unwrap();
    assert!((d.x - 30.0).abs() < 1e-9 && d.y.abs() < 1e-9, "pointer 1 moved a handle that pointer 0 holds: {d:?}");
}

#[test]
fn a_limited_range_caps_the_pull() {
    let p = torso_puppet();
    for (target, want) in [((270.0, 110.0), (20.0, 0.0)), ((120.0, -300.0), (0.0, -20.0)), ((-5000.0, 110.0), (-20.0, 0.0))] {
        let mut stack = dragger_stack(&p, &json!({"limit_range_cb": true, "range": 20}));
        let _ = frame(&mut stack, 0, &[(0, 120.0, 110.0, true)]);
        let pose = frame(&mut stack, 1, &[(0, target.0, target.1, true)]);
        let d = pose.drag.get(KEY).copied().unwrap();
        assert!((d.x - want.0).abs() < 1e-9 && (d.y - want.1).abs() < 1e-9, "pointer {target:?}: {d:?}");
        let img = render(&p, &pose);
        assert_eq!(components(&img), 1);
        assert_px(&img, 120, 191, GRAY, "base holds");
    }
}

// ---- dangle-large-steps ------------------------------------------------------------------------------------

fn hair_puppet() -> Puppet {
    let hair = ArtNode::group("+ Hair", vec![rect("Strand", 112.0, 100.0, 16, 70, PURPLE)]);
    let head = ArtNode::group("Head", vec![rect("Face", 60.0, 20.0, 120, 100, SKIN), hair]);
    let mut p = Puppet::from_artwork(&tree(240, 300, vec![head])).unwrap();
    let id = p.find_path("Head/+ Hair").unwrap();
    p.add_handle(id, "Fixed", HandleKind::Fixed, Point::new(120.0, 105.0)).unwrap();
    p.add_handle(id, "Dangle", HandleKind::Dangle, Point::new(120.0, 165.0)).unwrap();
    assert_eq!(p.warped_groups(), [id], "{:?}", p.warnings);
    p
}

fn hair_stack(p: &Puppet) -> BehaviorStack {
    let mut s = BehaviorStack::new(Rig::from_puppet(p));
    s.push(behaviors::create_with("Adobe.FaceTracker2.js", &json!({"smoothingRate": 0, "headRotFactor": 100})).unwrap());
    s.push(behaviors::create_with("Adobe.RigDynamics.js", &json!({})).unwrap());
    s.with_seed(3)
}

/// Runs `(time, roll)` ticks and returns every pose with its time.
fn swing(p: &Puppet, ticks: &[(f64, f64)]) -> (Rig, Vec<(f64, PuppetPose)>) {
    let mut stack = hair_stack(p);
    let rig = stack.rig().clone();
    let mut out = Vec::new();
    for &(t, roll) in ticks {
        let face = FaceInput { head_roll: roll, ..FaceInput::default() };
        out.push((t, stack.step(&Inputs::at(Tick::from_seconds_f64(t)).with_face(face))));
    }
    (rig, out)
}

/// How far the dangling tip is from where its group carries it, on stage.
fn tip_offset(rig: &Rig, pose: &PuppetPose) -> f64 {
    let h = rig.handles.iter().find(|h| h.key == "Hair/Dangle").unwrap();
    let m = h.matrix(pose);
    let d = pose.drag_of(&h.key);
    let (a, b) = (m * h.position, m * (h.position + d));
    (b - a).hypot()
}

#[test]
fn dangle_large_steps() {
    let p = hair_puppet();
    let dt = 1.0 / 30.0;
    let mut s_ticks: Vec<(f64, f64)> = (0..60).map(|i| (f64::from(i) * dt, 0.0)).collect();
    s_ticks.extend((60..300).map(|i| (f64::from(i) * dt, 90.0)));
    // One big step: tick at 0 s, then 1 s later the swing, then 30 fps.
    let mut l_ticks = vec![(0.0, 0.0)];
    l_ticks.extend((0..270).map(|i| (1.0 + f64::from(i) * dt, 90.0)));

    let (rig, s) = swing(&p, &s_ticks);
    let (_, l) = swing(&p, &l_ticks);
    let reach = rig.handles.iter().find(|h| h.key == "Hair/Dangle").unwrap().reach;
    assert!((reach - 30.0).abs() < 1e-9, "reach {reach}");
    for (name, run) in [("S", &s), ("L", &l)] {
        for (t, pose) in run.iter() {
            let d = pose.drag_of("Hair/Dangle");
            assert!(d.x.is_finite() && d.y.is_finite(), "{name} t={t}: {d:?}");
            let off = tip_offset(&rig, pose);
            assert!(off <= reach + 0.5, "{name} t={t}: the tip is {off} px from its anchor, reach {reach}");
        }
    }

    // Pick the frames to draw: just after the swing, a bit later, settled.
    let pick = |run: &[(f64, PuppetPose)], from: f64, after: f64| run.iter().find(|(t, _)| *t >= from + after - 1e-9).map(|(_, pose)| pose.clone()).unwrap();
    let s_frames: Vec<PuppetPose> = [0.0, 0.1, 0.4, 8.0 - 0.05].iter().map(|a| pick(&s, 2.0, *a)).collect();
    let l_frames: Vec<PuppetPose> = [0.0, 0.1, 0.4, 8.0 - 0.05].iter().map(|a| pick(&l, 1.0, *a)).collect();
    let s_imgs: Vec<_> = s_frames.iter().map(|f| render(&p, f)).collect();
    let l_imgs: Vec<_> = l_frames.iter().map(|f| render(&p, f)).collect();
    let all: Vec<&anim_display::Image> = s_imgs.iter().chain(&l_imgs).collect();
    save_sheet("dangle-large-steps", &all[..4]);
    save_sheet("dangle-large-steps-L", &all[4..]);
    for (i, img) in all.iter().enumerate() {
        assert_eq!(components(img), 1, "frame {i}: head and hair are one piece");
    }
    let settled_s = &s_frames[3];
    let settled_l = &l_frames[3];
    assert!(tip_offset(&rig, settled_s) < 0.5 && tip_offset(&rig, settled_l) < 0.5, "settled: {} {}", tip_offset(&rig, settled_s), tip_offset(&rig, settled_l));
    let (a, b) = (&s_imgs[3], &l_imgs[3]);
    let bad = a.premultiplied_rgba.iter().zip(&b.premultiplied_rgba).filter(|(x, y)| x.abs_diff(**y) > 2).count();
    assert!(bad * 200 < a.premultiplied_rgba.len(), "settled S and L differ in {bad} values");

    // Head turned 90 degrees clockwise about its origin (120,170) (bottom centre of the head including the hair):
    // (x,y) goes to (290-y, x+50). The strand lies on x 120..190, y 162..178.
    for img in [a, b] {
        assert_px(img, 180, 170, PURPLE, "hair over the face");
        assert_px(img, 150, 170, PURPLE, "hair outside the face");
        assert_px(img, 124, 170, PURPLE, "hair near the tip");
        assert_px(img, 116, 170, WHITE, "nothing beyond the tip");
        assert_px(img, 150, 160, WHITE, "strand is thin");
        assert_px(img, 150, 180, WHITE, "strand is thin");
    }
    golden("dangle-large-steps", &all[..4]);
    golden("dangle-large-steps-L", &all[4..]);
}

// ---- overlap-and-nested ------------------------------------------------------------------------------------

fn overlap_puppet() -> Puppet {
    // Each group's origin (its centre) lies outside the other squares, so nothing is welded to anything.
    let g = |name: &str, x: f64, rgb| ArtNode::group(name, vec![rect("sq", x, x, 60, 60, rgb)]);
    Puppet::from_artwork(&tree(240, 240, vec![g("+ Red", 30.0, RED), g("+ Blue", 70.0, BLUE), g("+ Green", 110.0, GREEN)])).unwrap()
}

#[test]
fn overlapping_groups_keep_their_draw_order() {
    let p = overlap_puppet();
    let neutral = render(&p, &PuppetPose::neutral());
    assert_px(&neutral, 40, 40, RED, "n red");
    assert_px(&neutral, 80, 80, BLUE, "n blue above red");
    assert_px(&neutral, 100, 100, BLUE, "n blue");
    assert_px(&neutral, 120, 120, GREEN, "n green above blue");
    assert_px(&neutral, 60, 85, RED, "n red");
    assert_px(&neutral, 160, 160, GREEN, "n green");

    let mut pa = PuppetPose::neutral();
    pa.groups.insert("+ Red".into(), Xform::moved(50.0, 50.0));
    let a = render(&p, &pa);
    assert_px(&a, 85, 85, BLUE, "A blue above moved red");
    assert_px(&a, 135, 135, GREEN, "A green above moved red");
    assert_px(&a, 135, 95, RED, "A red alone");
    assert_px(&a, 125, 125, GREEN, "A green above");
    assert_px(&a, 40, 40, WHITE, "A red left");

    let mut pb = PuppetPose::neutral();
    pb.groups.insert("Green".into(), Xform::rotated_degrees(45.0));
    let b = render(&p, &pb);
    assert_px(&b, 140, 175, GREEN, "B diamond bottom");
    assert_px(&b, 128, 118, GREEN, "B diamond above blue");
    assert_px(&b, 111, 111, BLUE, "B old corner now shows blue");
    assert_px(&b, 165, 165, WHITE, "B old corner gone");

    let mut pc = PuppetPose::neutral();
    pc.groups.insert("+ Blue".into(), Xform { scale: Vec2::new(0.0, 0.0), ..Xform::IDENTITY });
    let c = render(&p, &pc);
    assert_px(&c, 80, 80, RED, "C blue collapsed, red shows");
    assert_px(&c, 120, 120, GREEN, "C green");
    assert_px(&c, 100, 80, WHITE, "C blue area empty");
    assert_px(&c, 95, 95, WHITE, "C blue area empty");
    assert_eq!(bbox_of(&c, BLUE, 6), None, "C no blue speck survives a zero scale");

    let mut pd = PuppetPose::neutral();
    pd.root = Xform::moved(10.0, 0.0);
    pd.visibility.insert("+ Red".into(), false);
    let d = render(&p, &pd);
    assert_px(&d, 105, 95, BLUE, "D shifted blue");
    assert_px(&d, 40, 40, WHITE, "D red hidden");
    assert_px(&d, 85, 100, BLUE, "D blue");
    assert_px(&d, 75, 100, WHITE, "D blue starts at 80, red is hidden");
    save_sheet("overlap-and-nested-overlap", &[&neutral, &a, &b, &c, &d]);
    golden("overlap-and-nested-overlap", &[&neutral, &a, &b, &c, &d]);
}

/// The origin of `+ Blue` lies on the border of `+ Red`, so Weld attaches it to red's mesh: red moving drags blue's
/// attach corner along (design of the weld, see `plan_welds`), but a zero scale still has to collapse the group.
#[test]
#[ignore = "known bug: the weld ring near the attach point follows the parent mesh and ignores the group's zero scale (eval.rs/warp.rs weld logic, owned by another agent)"]
fn a_welded_group_collapses_with_zero_scale() {
    let g = |name: &str, x: f64, rgb| ArtNode::group(name, vec![rect("sq", x, x, 60, 60, rgb)]);
    let p = Puppet::from_artwork(&tree(240, 240, vec![g("+ Red", 40.0, RED), g("+ Blue", 70.0, BLUE)])).unwrap();
    let mut pose = PuppetPose::neutral();
    pose.groups.insert("+ Blue".into(), Xform { scale: Vec2::new(0.0, 0.0), ..Xform::IDENTITY });
    let img = render(&p, &pose);
    assert_eq!(bbox_of(&img, BLUE, 6), None, "a speck of the collapsed blue group is left");
}

fn arm_puppet() -> Puppet {
    let arm = ArtNode::group("+ Arm", vec![rect("Sleeve", 150.0, 100.0, 70, 20, BLUE)]);
    let body = ArtNode::group("Body", vec![rect("Torso", 80.0, 60.0, 80, 120, GRAY), arm]);
    let mut p = Puppet::from_artwork(&tree(300, 240, vec![body])).unwrap();
    let b = p.find_path("Body").unwrap();
    let a = p.find_path("Body/+ Arm").unwrap();
    p.add_handle(b, "Fixed", HandleKind::Fixed, Point::new(120.0, 170.0)).unwrap();
    p.add_handle(b, "Draggable", HandleKind::Draggable, Point::new(120.0, 70.0)).unwrap();
    p.add_handle(a, "Fixed", HandleKind::Fixed, Point::new(155.0, 110.0)).unwrap();
    p.add_handle(a, "Draggable", HandleKind::Draggable, Point::new(215.0, 110.0)).unwrap();
    assert_eq!(p.warped_groups().len(), 2, "{:?}", p.warnings);
    p
}

#[test]
fn a_child_group_follows_its_bending_parent() {
    let p = arm_puppet();
    let neutral = render(&p, &PuppetPose::neutral());
    assert_eq!(components(&neutral), 1);
    let pose = |body: (f64, f64), arm: (f64, f64)| {
        let mut pose = PuppetPose::neutral();
        if body != (0.0, 0.0) {
            pose.drag.insert("Body/Draggable".into(), Vec2::new(body.0, body.1));
        }
        if arm != (0.0, 0.0) {
            pose.drag.insert("Arm/Draggable".into(), Vec2::new(arm.0, arm.1));
        }
        pose
    };
    let n1 = render(&p, &pose((-40.0, 0.0), (0.0, 0.0)));
    let n2 = render(&p, &pose((0.0, 0.0), (0.0, -50.0)));
    let n3 = render(&p, &pose((-40.0, 0.0), (0.0, -50.0)));
    let n4 = render(&p, &pose((40.0, 0.0), (30.0, 40.0)));
    save_sheet("overlap-and-nested-arm", &[&neutral, &n1, &n2, &n3, &n4]);
    save_zoom("overlap-and-nested-arm-n1-shoulder", &n1, 100, 80, 90, 60, 8);

    assert_eq!(components(&n1), 1, "N1: the arm was left behind when the body bent away");
    assert_eq!(components(&n2), 1, "N2");
    assert_px(&n2, 215, 64, BLUE, "N2 raised tip");
    assert_eq!(components(&n3), 1, "N3");
    assert_eq!(components(&n4), 1, "N4");
    // Only the pictures that do not depend on the parent carrying the child are kept as goldens (N1, N3 and N4 wait
    // for the weld fix, see below).
    golden("overlap-and-nested-arm", &[&neutral, &n2]);
}

/// The body bends away by 40 px at the top: at the shoulder row its surface moves left by about 22 px, and the arm,
/// attached there, must move with it. The arm is carried by the displacement of the parent's mesh sampled at the arm
/// group's own origin, which lies outside that mesh; the extrapolated value overshoots (the arm slid 50 px, its
/// shoulder end ending 38 px inside the torso). Part of the weld work another agent owns (`eval.rs`, `warp.rs`).
#[test]
#[ignore = "known bug: warp::displacement of an origin outside the parent mesh adds the distance to the clamped point (fix: evaluate the binding at rest too); eval.rs apply_warp, owned by the weld work"]
fn the_arm_moves_with_the_shoulder_surface() {
    let p = arm_puppet();
    let mut pose = PuppetPose::neutral();
    pose.drag.insert("Body/Draggable".into(), Vec2::new(-40.0, 0.0));
    let img = render(&p, &pose);
    assert_eq!(components(&img), 1);
    let blue = bbox_of(&img, BLUE, 6).unwrap();
    assert!((120..=140).contains(&blue[0]), "arm starts at x={} instead of following the shoulder to about 128", blue[0]);
}
