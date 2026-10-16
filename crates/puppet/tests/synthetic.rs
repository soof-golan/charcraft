//! Puppets built from artwork trees made of rectangles named like Character Animator puppets.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::assert_is_empty,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::many_single_char_names,
    reason = "test code"
)]

use anim_artwork::{ArtNode, ArtworkTree};
use anim_core::{Point, Rgba8, Size, Vec2};
use anim_display::{Image, ImageStore, Node};
use anim_render::{Background, RenderOptions};
use charcraft_puppet::{HandleKind, LayerId, Puppet, PuppetPose, Viseme, Xform, tags::ids};

const W: u32 = 200;

fn rect(name: &str, x: f64, y: f64, w: u32, h: u32, rgb: [u8; 3]) -> ArtNode {
    let n = usize::try_from(w * h).unwrap();
    let px: Vec<u8> = [rgb[0], rgb[1], rgb[2], 255].iter().copied().cycle().take(n * 4).collect();
    ArtNode::raster(name, Image::new(w, h, px).unwrap(), Point::new(x, y))
}

const fn hidden(mut node: ArtNode) -> ArtNode {
    node.visible = false;
    node
}

const SKIN: [u8; 3] = [240, 190, 160];
const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [10, 10, 10];
const BLUE: [u8; 3] = [30, 60, 220];
const BROW: [u8; 3] = [90, 40, 10];
const GRAY: [u8; 3] = [120, 120, 130];

fn viseme_color(i: usize) -> [u8; 3] {
    let i = u8::try_from(i).unwrap();
    [20 + i * 15, 220 - i * 12, 40 + i * 5]
}

fn mouth() -> ArtNode {
    let children = Viseme::ALL
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let node = rect(v.label(), 85.0, 90.0, 30, 15, viseme_color(i));
            if *v == Viseme::Neutral { node } else { hidden(node) }
        })
        .collect();
    ArtNode::group("mouth", children)
}

fn eye(side: &str, x: f64) -> ArtNode {
    ArtNode::group(
        format!("{side} Eye"),
        vec![
            rect("White", x, 50.0, 30, 20, WHITE),
            rect(&format!("+ {side} Pupil"), x + 10.0, 55.0, 10, 10, BLACK),
            hidden(rect(&format!("{side} Blink"), x, 58.0, 30, 4, BLUE)),
        ],
    )
}

fn puppet_tree() -> ArtworkTree {
    let head = ArtNode::group(
        "Head",
        vec![
            rect("Face", 50.0, 20.0, 100, 100, SKIN),
            eye("Left", 110.0),
            eye("Right", 60.0),
            rect("+Left Eyebrow", 110.0, 36.0, 30, 10, BROW),
            rect("+ right eyebrow", 60.0, 36.0, 30, 10, BROW),
            mouth(),
        ],
    );
    let body = ArtNode::group("Body", vec![rect("Torso", 60.0, 120.0, 80, 80, GRAY)]);
    ArtworkTree { size: Size::new(f64::from(W), f64::from(W)), root: ArtNode::group("", vec![body, head]), warnings: Vec::new() }
}

fn render(puppet: &Puppet, pose: &PuppetPose) -> Image {
    let list = puppet.evaluate(pose);
    anim_render::render(&list, puppet.images(), &RenderOptions::new(W, W).with_background(Background::Solid(Rgba8::WHITE))).unwrap()
}

fn px(img: &Image, x: u32, y: u32) -> [u8; 3] {
    let at = usize::try_from((y * img.width + x) * 4).unwrap();
    [img.premultiplied_rgba[at], img.premultiplied_rgba[at + 1], img.premultiplied_rgba[at + 2]]
}

fn id(p: &Puppet, path: &str) -> LayerId {
    p.find_path(path).unwrap_or_else(|| panic!("no layer {path}\n{}", p.outline()))
}

#[test]
fn names_map_to_tags_and_independence() {
    let p = Puppet::from_artwork(&puppet_tree()).unwrap();
    let tags = |path: &str| p.layer(id(&p, path)).unwrap().tags.iter().cloned().collect::<Vec<_>>();
    assert_eq!(tags("Head"), [ids::HEAD]);
    assert_eq!(tags("Body"), [ids::BODY]);
    assert!(tags("Head/Left Eye").contains(&ids::LEFT_EYE.to_owned()));
    assert!(tags("Head/Right Eye").contains(&ids::RIGHT_EYE.to_owned()));
    assert!(tags("Head/Left Eye/+ Left Pupil").contains(&ids::LEFT_PUPIL.to_owned()));
    assert!(tags("Head/Right Eye/+ Right Pupil").contains(&ids::RIGHT_PUPIL.to_owned()));
    assert_eq!(tags("Head/Left Eye/Left Blink"), [ids::LEFT_BLINK]);
    assert_eq!(tags("Head/Right Eye/Right Blink"), [ids::RIGHT_BLINK]);
    assert_eq!(tags("Head/+Left Eyebrow"), [ids::LEFT_EYEBROW]);
    assert_eq!(tags("Head/+ right eyebrow"), [ids::RIGHT_EYEBROW]);
    let mouth_tags = tags("Head/mouth");
    assert!(mouth_tags.contains(&ids::MOUTH.to_owned()) && mouth_tags.contains(&ids::MOUTHS_PARENT.to_owned()));

    for (i, v) in Viseme::ALL.iter().enumerate() {
        let layer = p.layer(id(&p, &format!("Head/mouth/{}", v.label()))).unwrap();
        assert_eq!(layer.viseme(), Some(*v), "viseme {i}");
        assert!(layer.has_tag(v.tag_id()));
    }
    for path in ["Head", "Body", "Head/Left Eye", "Head/mouth", "Head/+Left Eyebrow", "Head/Left Eye/+ Left Pupil"] {
        assert!(p.layer(id(&p, path)).unwrap().independent, "{path}");
    }
    for path in ["Head/Face", "Head/Left Eye/White", "Head/mouth/Ah", "Body/Torso"] {
        assert!(!p.layer(id(&p, path)).unwrap().independent, "{path}");
    }
    assert_eq!(p.mouth_groups(), [id(&p, "Head/mouth")]);
    assert_eq!(p.mouth_shapes(id(&p, "Head/mouth")).len(), 14);
}

#[test]
fn single_letters_outside_a_mouth_are_not_visemes() {
    let tree = ArtworkTree {
        size: Size::new(10.0, 10.0),
        root: ArtNode::group("", vec![ArtNode::group("Hair", vec![rect("S", 0.0, 0.0, 2, 2, BLACK), rect("D", 2.0, 0.0, 2, 2, BLACK)])]),
        warnings: Vec::new(),
    };
    let p = Puppet::from_artwork(&tree).unwrap();
    assert!(p.mouth_groups().is_empty(), "two lone letters do not make a mouth");
    let tree = ArtworkTree {
        size: Size::new(10.0, 10.0),
        root: ArtNode::group("", vec![ArtNode::group("Hair", vec![rect("S", 0.0, 0.0, 2, 2, BLACK)])]),
        warnings: Vec::new(),
    };
    let p = Puppet::from_artwork(&tree).unwrap();
    assert!(p.mouth_groups().is_empty());
    assert!(p.layers().iter().all(|l| l.viseme().is_none()));
}

#[test]
fn neutral_pose_equals_the_artwork_render() {
    let tree = puppet_tree();
    let puppet = Puppet::from_artwork(&tree).unwrap();
    let mut store = ImageStore::new();
    let list = anim_artwork::to_display(&tree, &mut store);
    let options = RenderOptions::new(W, W).with_background(Background::Solid(Rgba8::WHITE));
    let expected = anim_render::render(&list, &store, &options).unwrap();
    assert_eq!(render(&puppet, &PuppetPose::neutral()), expected);
}

#[test]
fn every_viseme_shows_its_own_shape() {
    let p = Puppet::from_artwork(&puppet_tree()).unwrap();
    for (i, v) in Viseme::ALL.iter().enumerate() {
        let img = render(&p, &PuppetPose::neutral().with_viseme(*v));
        assert_eq!(px(&img, 100, 97), viseme_color(i), "{}", v.label());
    }
}

#[test]
fn a_missing_viseme_falls_back_to_neutral() {
    let mut tree = puppet_tree();
    let mouth = tree.root.find_path_mut(&["Head", "mouth"]).unwrap();
    if let anim_artwork::ArtKind::Group(children) = &mut mouth.kind {
        children.retain(|c| c.name != "Ee");
    }
    let p = Puppet::from_artwork(&tree).unwrap();
    let img = render(&p, &PuppetPose::neutral().with_viseme(Viseme::Ee));
    assert_eq!(px(&img, 100, 97), viseme_color(0));
}

#[test]
fn blinking_swaps_the_eye_for_its_blink_layer() {
    let p = Puppet::from_artwork(&puppet_tree()).unwrap();
    let open = render(&p, &PuppetPose::neutral());
    assert_eq!(px(&open, 125, 52), WHITE);
    assert_eq!(px(&open, 125, 60), BLACK);

    let mut pose = PuppetPose::neutral();
    pose.blink_left = 1.0;
    let left = render(&p, &pose);
    assert_eq!(px(&left, 125, 60), BLUE, "left blink layer shows");
    assert_eq!(px(&left, 125, 52), SKIN, "the rest of the left eye is hidden");
    assert_eq!(px(&left, 75, 60), BLACK, "the right eye is unaffected");

    pose.blink_left = 0.49;
    assert_eq!(render(&p, &pose), open, "below the threshold nothing changes");
}

#[test]
fn head_rotation_turns_everything_in_it_about_the_neck() {
    let p = Puppet::from_artwork(&puppet_tree()).unwrap();
    let head = p.layer(id(&p, "Head")).unwrap();
    assert_eq!(head.origin, Point::new(100.0, 120.0), "bottom centre of the head");
    let mut pose = PuppetPose::neutral();
    pose.head = Xform::rotated_degrees(90.0);
    let img = render(&p, &pose);
    // The top-left of the face (60,30) lands at (190,80) when turned 90 degrees clockwise about (100,120).
    assert_eq!(px(&img, 188, 80), SKIN);
    assert_eq!(px(&img, 60, 30), WHITE);
    // The body does not move.
    assert_eq!(px(&img, 70, 160), GRAY);
}

#[test]
fn eyebrows_raise_and_gaze_moves_pupils_within_the_eye() {
    let p = Puppet::from_artwork(&puppet_tree()).unwrap();
    assert_eq!(px(&render(&p, &PuppetPose::neutral()), 125, 30), SKIN);
    let mut up = PuppetPose::neutral();
    up.eyebrow_left = 1.0;
    let img = render(&p, &up);
    assert_eq!(px(&img, 125, 30), BROW, "left eyebrow moved up");
    assert_eq!(px(&img, 75, 40), BROW, "right eyebrow stayed");

    let mut look = PuppetPose::neutral();
    look.gaze = Vec2::new(1.0, 0.0);
    let img = render(&p, &look);
    assert_eq!(px(&img, 135, 60), BLACK, "pupil moved right");
    assert_eq!(px(&img, 115, 60), WHITE);
    look.gaze = Vec2::new(5.0, 0.0);
    assert_eq!(render(&p, &look), img, "gaze is clamped to the eye");
    look.gaze = Vec2::new(f64::NAN, f64::INFINITY);
    let _ = render(&p, &look);
}

#[test]
fn pose_overrides_for_any_group_by_label() {
    let p = Puppet::from_artwork(&puppet_tree()).unwrap();
    let mut pose = PuppetPose::neutral();
    pose.groups.insert("left eyebrow".into(), Xform::moved(0.0, -9.0));
    assert_eq!(px(&render(&p, &pose), 125, 30), BROW);
    let mut pose = PuppetPose::neutral();
    pose.root = Xform::moved(30.0, 0.0);
    let img = render(&p, &pose);
    assert_eq!(px(&img, 160, 160), GRAY, "the whole puppet moved right");
    assert_eq!(px(&img, 70, 160), WHITE);
}

#[test]
fn swap_sets_triggers_and_overrides_switch_layers() {
    let tree = ArtworkTree {
        size: Size::new(f64::from(W), f64::from(W)),
        root: ArtNode::group(
            "",
            vec![
                rect("Cap", 0.0, 0.0, 20, 20, [200, 0, 0]),
                hidden(rect("Crown", 0.0, 0.0, 20, 20, [0, 200, 0])),
                hidden(rect("Star", 50.0, 0.0, 20, 20, [0, 0, 200])),
            ],
        ),
        warnings: Vec::new(),
    };
    let mut p = Puppet::from_artwork(&tree).unwrap();
    assert!(!p.add_swap_set("hats", vec![id(&p, "Cap")]), "needs two members");
    assert!(p.add_swap_set("hats", vec![id(&p, "Cap"), id(&p, "Crown")]));
    assert_eq!(px(&render(&p, &PuppetPose::neutral()), 5, 5), [200, 0, 0]);
    let mut pose = PuppetPose::neutral();
    pose.swaps.insert("hats".into(), "Crown".into());
    assert_eq!(px(&render(&p, &pose), 5, 5), [0, 200, 0]);

    assert!(p.set_key_trigger(id(&p, "Star"), Some("s")));
    assert_eq!(px(&render(&p, &PuppetPose::neutral()), 55, 5), WHITE);
    let mut pose = PuppetPose::neutral();
    pose.triggers.insert("s".into());
    assert_eq!(px(&render(&p, &pose), 55, 5), [0, 0, 200]);

    let mut pose = PuppetPose::neutral();
    pose.visibility.insert("Cap".into(), false);
    assert_eq!(px(&render(&p, &pose), 5, 5), WHITE);
}

#[test]
fn dragging_a_handle_warps_the_group_with_a_mesh() {
    let mut p = Puppet::from_artwork(&puppet_tree()).unwrap();
    let body = id(&p, "Body");
    assert!(p.warped_groups().is_empty());
    let fixed = p.add_handle(body, "Fixed", HandleKind::Fixed, Point::new(100.0, 195.0)).unwrap();
    let drag = p.add_handle(body, "Draggable", HandleKind::Draggable, Point::new(100.0, 125.0)).unwrap();
    assert_eq!(fixed, "Body/Fixed");
    assert_eq!(p.warped_groups(), [body], "{:?}", p.warnings);

    let has_mesh = |list: &anim_display::DisplayList| {
        let mut found = false;
        list.visit(&mut |n| found |= matches!(n, Node::Mesh(_)));
        found
    };
    assert!(!has_mesh(&p.evaluate(&PuppetPose::neutral())), "nothing pulled, no mesh node");
    assert_eq!(render(&p, &PuppetPose::neutral()), render(&Puppet::from_artwork(&puppet_tree()).unwrap(), &PuppetPose::neutral()));

    let mut pose = PuppetPose::neutral();
    pose.drag.insert(drag.clone(), Vec2::new(30.0, 0.0));
    let list = p.evaluate(&pose);
    assert!(has_mesh(&list));
    let img = render(&p, &pose);
    // Pulling the top 30 px right about the fixed bottom tilts the torso about 23 degrees clockwise.
    assert_eq!(px(&img, 155, 150), GRAY, "the torso swung out to the right");
    assert_eq!(px(&img, 65, 140), WHITE, "the left side moved away");
    assert_eq!(px(&img, 120, 195), GRAY, "the fixed bottom stayed");

    assert!(p.remove_handle(body, &drag));
    assert!(!p.remove_handle(body, &drag));
}

#[test]
fn handles_only_attach_to_independent_groups() {
    let mut p = Puppet::from_artwork(&puppet_tree()).unwrap();
    let face = id(&p, "Head/Face");
    assert!(p.add_handle(face, "Fixed", HandleKind::Fixed, Point::new(1.0, 1.0)).is_none());
    assert!(!p.set_origin(face, Point::ORIGIN));
    let head = id(&p, "Head");
    assert!(p.set_origin(head, Point::new(100.0, 70.0)));
}

#[test]
fn right_to_left_names_are_kept_verbatim() {
    let tree = ArtworkTree {
        size: Size::new(40.0, 40.0),
        root: ArtNode::group(
            "",
            vec![ArtNode::group("+ ראש", vec![rect("עין", 0.0, 0.0, 10, 10, BLACK)]), ArtNode::group("+مرحبا", vec![rect("فم", 20.0, 0.0, 10, 10, BLACK)])],
        ),
        warnings: Vec::new(),
    };
    let p = Puppet::from_artwork(&tree).unwrap();
    let hebrew = p.layer(id(&p, "+ ראש")).unwrap();
    assert!(hebrew.independent);
    assert_eq!(hebrew.label, "ראש");
    assert_eq!(p.layer(id(&p, "+ ראש/עין")).unwrap().name, "עין");
    let arabic = p.layer(id(&p, "+مرحبا")).unwrap();
    assert!(arabic.independent && arabic.label == "مرحبا");
    let mut pose = PuppetPose::neutral();
    pose.groups.insert("ראש".into(), Xform::moved(5.0, 0.0));
    let list = p.evaluate(&pose);
    assert!(!list.nodes.is_empty());
}

#[test]
fn summary_describes_the_puppet() {
    let p = Puppet::from_artwork(&puppet_tree()).unwrap();
    let s = p.summary();
    assert_eq!(s.mouth_groups, ["Head/mouth"]);
    assert_eq!(s.visemes.len(), 14);
    assert!(s.groups.iter().any(|g| g.label == "Head" && g.tags.contains(&ids::HEAD.to_owned())));
    assert_eq!(s.tag_counts.get(ids::LEFT_BLINK), Some(&1));
    let json = serde_json::to_string(&s).unwrap();
    let back: charcraft_puppet::PuppetSummary = serde_json::from_str(&json).unwrap();
    assert_eq!(back, s);
    assert!(p.outline().contains("Head [independent] tags=Adobe.Face.Head"));
}

#[test]
fn tool_ops_edit_a_puppet_by_path() {
    use charcraft_puppet::{Error, PuppetOp};
    let mut p = Puppet::from_artwork(&puppet_tree()).unwrap();
    let out = p.apply(&PuppetOp::AddHandle { group: "Body".into(), name: "Fixed".into(), kind: HandleKind::Fixed, position: [100.0, 195.0] }).unwrap();
    assert_eq!(out.key.as_deref(), Some("Body/Fixed"));
    assert!(p.apply(&PuppetOp::AddTag { layer: "Head/Face".into(), tag: "X.Tag".into() }).unwrap().changed);
    assert!(!p.apply(&PuppetOp::AddTag { layer: "Head/Face".into(), tag: "X.Tag".into() }).unwrap().changed);
    assert!(matches!(p.apply(&PuppetOp::AddTag { layer: "Nope".into(), tag: "t".into() }), Err(Error::NoLayer(_))));
    assert!(matches!(p.apply(&PuppetOp::SetOrigin { group: "Head/Face".into(), origin: [0.0, 0.0] }), Err(Error::NotIndependent(_))));
    assert!(matches!(p.apply(&PuppetOp::SetOrigin { group: "Head".into(), origin: [f64::NAN, 0.0] }), Err(Error::Rejected(_))));
    assert!(p.apply(&PuppetOp::SetVisible { layer: "Head/Face".into(), visible: false }).unwrap().changed);
    let info = p.layer_info("mouth").unwrap();
    assert_eq!(info.path, "Head/mouth");
    assert_eq!(info.children.len(), 14);
    assert!(info.independent && info.origin.is_some());
    assert!(p.layer_info("Nope").is_none());
}

#[test]
fn an_empty_artwork_is_a_puppet_with_nothing_to_draw() {
    let tree = ArtworkTree { size: Size::new(4.0, 4.0), root: ArtNode::group("", Vec::new()), warnings: Vec::new() };
    let p = Puppet::from_artwork(&tree).unwrap();
    assert_eq!(p.layers().len(), 1);
    let _ = p.evaluate(&PuppetPose::neutral().with_viseme(Viseme::Ah));
}

/// A head resting on a body through a thin neck that belongs to the body's artwork.
fn neck_tree() -> ArtworkTree {
    let head = ArtNode::group("Head", vec![rect("Face", 50.0, 20.0, 100, 100, SKIN)]);
    let body = ArtNode::group("Body", vec![rect("Torso", 60.0, 140.0, 80, 60, GRAY), rect("Neck", 90.0, 110.0, 20, 40, BLUE)]);
    ArtworkTree { size: Size::new(f64::from(W), f64::from(W)), root: ArtNode::group("", vec![body, head]), warnings: Vec::new() }
}

fn is_blank(c: [u8; 3]) -> bool {
    c == [255, 255, 255]
}

fn head_moved_up_and_turned() -> PuppetPose {
    let mut pose = PuppetPose::neutral();
    pose.head = Xform { translation: Vec2::new(0.0, -30.0), rotation: 15.0_f64.to_radians(), ..Xform::IDENTITY };
    pose
}

#[test]
fn a_moved_head_stays_connected_to_the_body() {
    let p = Puppet::from_artwork(&neck_tree()).unwrap();
    let neutral = render(&p, &PuppetPose::neutral());
    let mut store = ImageStore::new();
    let list = anim_artwork::to_display(&neck_tree(), &mut store);
    let expected = anim_render::render(&list, &store, &RenderOptions::new(W, W).with_background(Background::Solid(Rgba8::WHITE))).unwrap();
    assert_eq!(neutral, expected, "the neutral pose is unchanged");

    let posed = render(&p, &head_moved_up_and_turned());
    // The head's chin stays where the attach point went (y 90); the body's neck follows it: no white gap between
    // the head and the torso along the neck line.
    for x in [96, 100, 104] {
        for y in 92..140 {
            assert!(!is_blank(px(&posed, x, y)), "gap at ({x},{y})");
        }
    }
    assert_eq!(render(&p, &head_moved_up_and_turned()), posed, "evaluation is deterministic");
}

#[test]
fn the_body_mesh_follows_the_moved_attach_point() {
    let p = Puppet::from_artwork(&neck_tree()).unwrap();
    let body = id(&p, "Body");
    let head = id(&p, "Head");
    assert!(p.warped_groups().is_empty(), "a weld alone does not count as a handle warp");
    let pose = head_moved_up_and_turned();
    let matrix = p.group_matrix(head, &pose);
    let (mesh, positions) = p.warp_solution(body, &pose).expect("the body is welded to the head");
    // Every body vertex under the head's artwork (the face covers y 20 to 120, x 50 to 150) follows the head rigidly.
    let under: Vec<_> = mesh.iter().zip(&positions).filter(|(r, _)| r.y < 119.0 && r.x > 51.0 && r.x < 149.0).collect();
    assert!(!under.is_empty(), "the neck overlaps the head");
    for (rest, now) in under {
        assert!(now.distance(matrix * *rest) < 1.0, "{rest:?} went to {now:?}, the head's matrix gives {:?}", matrix * *rest);
    }
    // The torso, far from the head, stays where it was.
    for (rest, now) in mesh.iter().zip(&positions).filter(|(r, _)| r.y > 185.0) {
        assert!(now.distance(*rest) < 1.0, "{rest:?} moved to {now:?}");
    }
}
