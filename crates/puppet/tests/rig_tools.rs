//! The rig tools' edits (independence, handles, sticks, attach styles, mesh density, cycles, triggers, swap sets)
//! on synthetic puppets: each edit changes what the evaluator draws.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::assert_is_empty,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code"
)]

use anim_artwork::{ArtNode, ArtworkTree};
use anim_core::{Point, Rgba8, Size, Vec2};
use anim_display::Image;
use anim_render::{Background, RenderOptions};
use charcraft_puppet::{AttachStyle, CycleDef, CycleMode, HandleKind, LayerId, Puppet, PuppetOp, PuppetPose, Xform};

const W: u32 = 200;
const SKIN: [u8; 3] = [240, 190, 160];
const GRAY: [u8; 3] = [120, 120, 130];
const BLUE: [u8; 3] = [30, 60, 220];
const RED: [u8; 3] = [220, 30, 30];
const GREEN: [u8; 3] = [30, 200, 60];

fn rect(name: &str, left: f64, top: f64, width: u32, height: u32, rgb: [u8; 3]) -> ArtNode {
    let count = usize::try_from(width * height).unwrap();
    let px: Vec<u8> = [rgb[0], rgb[1], rgb[2], 255].iter().copied().cycle().take(count * 4).collect();
    ArtNode::raster(name, Image::new(width, height, px).unwrap(), Point::new(left, top))
}

fn tree(children: Vec<ArtNode>) -> ArtworkTree {
    ArtworkTree { size: Size::new(f64::from(W), f64::from(W)), root: ArtNode::group("", children), warnings: Vec::new() }
}

/// A head resting on a body through a thin neck that belongs to the body's artwork.
fn neck() -> Puppet {
    let head = ArtNode::group("Head", vec![rect("Face", 50.0, 20.0, 100, 100, SKIN)]);
    let body = ArtNode::group("Body", vec![rect("Torso", 60.0, 140.0, 80, 60, GRAY), rect("Neck", 90.0, 110.0, 20, 40, BLUE)]);
    Puppet::from_artwork(&tree(vec![body, head])).unwrap()
}

fn id(p: &Puppet, path: &str) -> LayerId {
    p.find_path(path).unwrap_or_else(|| panic!("no layer {path}\n{}", p.outline()))
}

fn render(p: &Puppet, pose: &PuppetPose) -> Image {
    let list = p.evaluate(pose);
    anim_render::render(&list, p.images(), &RenderOptions::new(W, W).with_background(Background::Solid(Rgba8::WHITE))).unwrap()
}

fn px(img: &Image, x: u32, y: u32) -> [u8; 3] {
    let at = usize::try_from((y * img.width + x) * 4).unwrap();
    [img.premultiplied_rgba[at], img.premultiplied_rgba[at + 1], img.premultiplied_rgba[at + 2]]
}

fn head_up() -> PuppetPose {
    let mut pose = PuppetPose::neutral();
    pose.head = Xform { translation: Vec2::new(0.0, -30.0), rotation: 15.0_f64.to_radians(), ..Xform::IDENTITY };
    pose
}

fn apply(p: &mut Puppet, json: serde_json::Value) -> charcraft_puppet::OpOutcome {
    let op: PuppetOp = serde_json::from_value(json).unwrap();
    p.apply(&op).unwrap()
}

#[test]
fn an_ordinary_layer_becomes_an_independent_group_and_back() {
    let mut p = neck();
    let torso = id(&p, "Body");
    assert!(p.layer(id(&p, "Body/Torso")).is_some_and(|l| !l.independent));
    let out = apply(&mut p, serde_json::json!({ "op": "set_independent", "layer": "Body/Torso", "independent": true }));
    assert!(out.changed);
    assert!(p.layer(id(&p, "Body/Torso")).unwrap().independent);
    // It now takes handles.
    let out = apply(&mut p, serde_json::json!({ "op": "add_handle", "group": "Body/Torso", "name": "Hip", "kind": "Fixed", "position": [100.0, 190.0] }));
    assert_eq!(out.key.as_deref(), Some("Torso/Hip"));
    let out = apply(&mut p, serde_json::json!({ "op": "set_independent", "layer": "Body/Torso", "independent": false }));
    assert!(out.changed);
    assert!(p.layer(id(&p, "Body/Torso")).unwrap().handles.is_empty(), "handles go with the independence");
    let _ = torso;
    assert!(p.apply(&serde_json::from_value(serde_json::json!({ "op": "set_independent", "layer": "", "independent": false })).unwrap()).is_err());
}

#[test]
fn moving_a_handle_moves_what_it_pins() {
    let mut p = neck();
    apply(&mut p, serde_json::json!({ "op": "add_handle", "group": "Body", "name": "Grip", "kind": "Draggable", "position": [100.0, 190.0] }));
    apply(&mut p, serde_json::json!({ "op": "add_handle", "group": "Body", "name": "Anchor", "kind": "Fixed", "position": [100.0, 115.0] }));
    let body = id(&p, "Body");
    let pose = |p: &Puppet| {
        let mut pose = PuppetPose::neutral();
        pose.drag.insert("Body/Grip".into(), Vec2::new(30.0, 0.0));
        p.warp_solution(body, &pose).unwrap()
    };
    let (_, before) = pose(&p);
    let out = apply(&mut p, serde_json::json!({ "op": "move_handle", "group": "Body", "key": "Body/Grip", "position": [70.0, 150.0] }));
    assert!(out.changed);
    assert_eq!(p.layer(body).unwrap().handles.iter().find(|h| h.name == "Grip").unwrap().position, Point::new(70.0, 150.0));
    let (_, after) = pose(&p);
    assert_ne!(before, after, "the new pin place gives another deformation");
    let missing = apply(&mut p, serde_json::json!({ "op": "move_handle", "group": "Body", "key": "Body/Nope", "position": [1.0, 1.0] }));
    assert!(!missing.changed);
}

#[test]
fn a_stick_keeps_its_segment_straight() {
    let mut p = neck();
    let body = id(&p, "Body");
    // A stick along the torso top edge and one handle on the right end: dragging the stick moves all its pins together.
    apply(
        &mut p,
        serde_json::json!({ "op": "add_handle", "group": "Body", "name": "Stick", "kind": "Stick", "position": [60.0, 150.0], "end": [140.0, 150.0] }),
    );
    let h = &p.layer(body).unwrap().handles[0];
    assert_eq!(h.kind, HandleKind::Stick);
    assert!(h.pins_mesh() && h.end.is_some());
    let mut pose = PuppetPose::neutral();
    pose.drag.insert("Body/Stick".into(), Vec2::new(0.0, 12.0));
    let (mesh, now) = p.warp_solution(body, &pose).unwrap();
    // Vertices on the stick line moved down together by the drag.
    let on_line: Vec<_> = mesh.iter().zip(&now).filter(|(r, _)| (r.y - 150.0).abs() < 1.0 && r.x > 62.0 && r.x < 138.0).collect();
    assert!(!on_line.is_empty());
    for (rest, moved) in on_line {
        assert!((moved.y - rest.y - 12.0).abs() < 1.5, "{rest:?} -> {moved:?}");
    }
    // Moving the stick moves both ends.
    apply(&mut p, serde_json::json!({ "op": "move_handle", "group": "Body", "key": "Body/Stick", "position": [60.0, 160.0] }));
    let h = &p.layer(body).unwrap().handles[0];
    assert_eq!(h.end, Some(Point::new(140.0, 160.0)));
}

#[test]
fn a_stick_without_an_end_pins_nothing() {
    let mut p = neck();
    apply(&mut p, serde_json::json!({ "op": "add_handle", "group": "Body", "name": "Stick", "kind": "Stick", "position": [60.0, 150.0] }));
    assert!(p.warped_groups().is_empty());
}

#[test]
fn attach_styles_decide_whether_the_parent_mesh_follows() {
    let body_follows = |style: AttachStyle| {
        let mut p = neck();
        apply(&mut p, serde_json::json!({ "op": "set_attach_style", "group": "Head", "style": style }));
        let body = id(&p, "Body");
        p.warp_solution(body, &head_up()).is_some()
    };
    assert!(body_follows(AttachStyle::Weld), "weld pulls the neck along");
    assert!(!body_follows(AttachStyle::Hinge), "a hinge pivots without pulling the mesh");
    assert!(!body_follows(AttachStyle::Free));
}

#[test]
fn warp_independence_keeps_attached_groups_out_of_the_mesh() {
    let mut p = neck();
    let body = id(&p, "Body");
    assert!(p.warp_solution(body, &head_up()).is_some());
    let out = apply(&mut p, serde_json::json!({ "op": "set_warp_independent", "group": "Body", "on": true }));
    assert!(out.changed);
    assert!(p.warp_solution(body, &head_up()).is_none());
    apply(&mut p, serde_json::json!({ "op": "set_warp_independent", "group": "Body", "on": false }));
    assert!(p.warp_solution(body, &head_up()).is_some());
}

#[test]
fn a_free_group_is_not_carried_by_the_parent_mesh() {
    // A hand with a handle of its own (so it is not welded) sits on an arm whose mesh bends when its handle is pulled.
    let arm = ArtNode::group("+Arm", vec![rect("Sleeve", 20.0, 100.0, 120, 20, BLUE), ArtNode::group("+Hand", vec![rect("Palm", 128.0, 98.0, 24, 24, SKIN)])]);
    let mut p = Puppet::from_artwork(&tree(vec![arm])).unwrap();
    let arm_id = id(&p, "+Arm");
    let hand = id(&p, "+Arm/+Hand");
    p.set_origin(hand, Point::new(130.0, 110.0));
    p.add_handle(hand, "Pin", HandleKind::Fixed, Point::new(140.0, 110.0)).unwrap();
    p.add_handle(arm_id, "Tip", HandleKind::Draggable, Point::new(138.0, 110.0)).unwrap();
    p.add_handle(arm_id, "Root", HandleKind::Fixed, Point::new(20.0, 110.0)).unwrap();
    let mut pose = PuppetPose::neutral();
    pose.drag.insert("Arm/Tip".into(), Vec2::new(0.0, 40.0));
    let centre = |p: &Puppet| {
        let img = render(p, &pose);
        (0..W)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .filter(|&(x, y)| px(&img, x, y) == SKIN)
            .map(|(x, y)| (f64::from(x), f64::from(y)))
            .fold((0.0, 0.0, 0.0), |(sx, sy, n), (x, y)| (sx + x, sy + y, n + 1.0))
    };
    let (sx, sy, n) = centre(&p);
    let carried_y = sy / n;
    assert!(n > 0.0 && carried_y > 110.0 + 10.0, "the hand rides on the bent arm: {carried_y}");
    apply(&mut p, serde_json::json!({ "op": "set_attach_style", "group": "+Arm/+Hand", "style": "free" }));
    let (_, sy, n) = centre(&p);
    assert!((sy / n - 110.0).abs() < 2.0, "a free hand stays where it was: {}", sy / n);
    let _ = sx;
}

#[test]
fn mesh_density_cuts_the_mesh_finer() {
    let mut p = neck();
    let body = id(&p, "Body");
    apply(&mut p, serde_json::json!({ "op": "add_handle", "group": "Body", "name": "Grip", "kind": "Draggable", "position": [100.0, 190.0] }));
    let vertices = |p: &Puppet| {
        let mut pose = PuppetPose::neutral();
        pose.drag.insert("Body/Grip".into(), Vec2::new(10.0, 0.0));
        p.warp_solution(body, &pose).unwrap().0.len()
    };
    let normal = vertices(&p);
    let out = apply(&mut p, serde_json::json!({ "op": "set_mesh_density", "group": "Body", "density": 3.0 }));
    assert!(out.changed);
    assert!(vertices(&p) > normal * 2, "{} vs {normal}", vertices(&p));
    apply(&mut p, serde_json::json!({ "op": "set_mesh_density", "group": "Body", "density": 100.0 }));
    assert!((p.layer(body).unwrap().mesh_density - charcraft_puppet::MESH_DENSITY.1).abs() < f64::EPSILON, "clamped");
    assert!(p.apply(&PuppetOp::SetMeshDensity { group: "Body".into(), density: f64::NAN }).is_err());
}

fn swatches() -> Puppet {
    let sw = ArtNode::group(
        "Swatches",
        vec![rect("Red", 10.0, 10.0, 40, 40, RED), rect("Green", 10.0, 10.0, 40, 40, GREEN), rect("Blue", 10.0, 10.0, 40, 40, BLUE)],
    );
    Puppet::from_artwork(&tree(vec![sw])).unwrap()
}

#[test]
fn a_layer_cycle_shows_the_children_in_turn() {
    let mut p = swatches();
    let colors = |p: &Puppet, ms: u64| {
        let mut pose = PuppetPose::neutral();
        pose.time_millis = ms;
        px(&render(p, &pose), 30, 30)
    };
    // Without a cycle all three are drawn, the topmost on top.
    assert_eq!(colors(&p, 0), BLUE);
    let out = apply(&mut p, serde_json::json!({ "op": "set_cycle", "layer": "Swatches", "cycle": { "step_millis": 100 } }));
    assert!(out.changed);
    assert_eq!(colors(&p, 0), RED);
    assert_eq!(colors(&p, 99), RED);
    assert_eq!(colors(&p, 100), GREEN);
    assert_eq!(colors(&p, 250), BLUE);
    assert_eq!(colors(&p, 300), RED, "loops");
    apply(&mut p, serde_json::json!({ "op": "set_cycle", "layer": "Swatches", "cycle": { "step_millis": 100, "mode": "once" } }));
    assert_eq!(colors(&p, 5000), BLUE, "stops on the last");
    apply(&mut p, serde_json::json!({ "op": "set_cycle", "layer": "Swatches", "cycle": null }));
    assert_eq!(colors(&p, 0), BLUE);
    assert!(p.apply(&PuppetOp::SetCycle { layer: "Swatches/Red".into(), cycle: Some(CycleDef::looping(100)) }).is_err(), "a leaf has no children to cycle");
}

#[test]
fn cycle_frames_follow_the_mode() {
    let c = |mode| CycleDef { step_millis: 100, mode };
    let seq = |mode, n| (0..8_u64).map(|i| c(mode).frame(i * 100, n)).collect::<Vec<_>>();
    assert_eq!(seq(CycleMode::Loop, 3), [0, 1, 2, 0, 1, 2, 0, 1]);
    assert_eq!(seq(CycleMode::PingPong, 3), [0, 1, 2, 1, 0, 1, 2, 1]);
    assert_eq!(seq(CycleMode::Once, 3), [0, 1, 2, 2, 2, 2, 2, 2]);
    assert_eq!(c(CycleMode::Loop).frame(12345, 1), 0);
    assert!(c(CycleMode::PingPong).frame(u64::MAX, 2) < 2);
}

#[test]
fn hide_others_hides_the_siblings_while_the_key_is_down() {
    let mut p = swatches();
    apply(&mut p, serde_json::json!({ "op": "set_key_trigger", "layer": "Swatches/Red", "key": "r" }));
    apply(&mut p, serde_json::json!({ "op": "set_hide_others", "layer": "Swatches/Red", "on": true }));
    let at = |p: &Puppet, keys: &[&str]| {
        let mut pose = PuppetPose::neutral();
        pose.triggers = keys.iter().map(|k| (*k).to_owned()).collect();
        px(&render(p, &pose), 30, 30)
    };
    // Key up: Red is hidden by its trigger, the rest draw (Blue on top).
    assert_eq!(at(&p, &[]), BLUE);
    // Key down: Red shows and Green and Blue are hidden.
    assert_eq!(at(&p, &["r"]), RED);
    apply(&mut p, serde_json::json!({ "op": "set_hide_others", "layer": "Swatches/Red", "on": false }));
    assert_eq!(at(&p, &["r"]), BLUE, "without it the siblings stay");
}

#[test]
fn swap_sets_can_be_defined_renamed_and_deleted() {
    let mut p = swatches();
    apply(&mut p, serde_json::json!({ "op": "add_swap_set", "name": "colors", "members": ["Swatches/Red", "Swatches/Green"] }));
    assert_eq!(p.swap_sets.len(), 1);
    // Redefining under the same name edits the members.
    apply(&mut p, serde_json::json!({ "op": "add_swap_set", "name": "colors", "members": ["Swatches/Green", "Swatches/Blue", "Swatches/Red"] }));
    assert_eq!(p.swap_sets.len(), 1);
    assert_eq!(p.swap_sets[0].members.len(), 3);
    assert!(apply(&mut p, serde_json::json!({ "op": "rename_swap_set", "name": "colors", "new_name": "palette" })).changed);
    assert_eq!(p.swap_sets[0].name, "palette");
    let mut pose = PuppetPose::neutral();
    pose.swaps.insert("palette".into(), "Green".into());
    assert_eq!(px(&render(&p, &pose), 30, 30), GREEN);
    assert!(!apply(&mut p, serde_json::json!({ "op": "remove_swap_set", "name": "nope" })).changed);
    assert!(apply(&mut p, serde_json::json!({ "op": "remove_swap_set", "name": "palette" })).changed);
    assert!(p.swap_sets.is_empty());
}

#[test]
fn layer_info_reports_the_new_settings() {
    let mut p = neck();
    apply(&mut p, serde_json::json!({ "op": "set_attach_style", "group": "Head", "style": "hinge" }));
    apply(&mut p, serde_json::json!({ "op": "set_mesh_density", "group": "Head", "density": 2.0 }));
    let info = p.layer_info("Head").unwrap();
    assert_eq!(info.attach, Some(AttachStyle::Hinge));
    assert_eq!(info.mesh_density, Some(2.0));
    assert_eq!(info.warp_independent, Some(false));
    assert_eq!(p.layer_info("Body/Torso").unwrap().attach, None, "only groups attach");
}
