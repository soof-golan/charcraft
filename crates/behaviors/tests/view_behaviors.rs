//! Head Turner, Motion Trigger, Cycle Layers, walk styles, pinned feet, Physics bodies, pose-to-pose Face and the
//! Lip Sync audio source, driven by scripted inputs.
#![allow(
    clippy::float_cmp,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    clippy::missing_const_for_fn,
    clippy::default_trait_access,
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::similar_names,
    reason = "test code"
)]

use anim_core::{Point, Rect, Size};
use anim_lipsync::Viseme as Voice;
use charcraft_behaviors::behaviors;
use charcraft_behaviors::{BehaviorStack, ChainLink, FaceInput, Rig, RigHandle, Script, ScriptEvent, SimFrame, simulate};
use charcraft_puppet::tags::ids;
use charcraft_puppet::{HandleKind, Viseme};
use serde_json::{Value, json};

fn stack_of(rig: Rig, parts: &[(&str, Value)]) -> BehaviorStack {
    let mut stack = BehaviorStack::new(rig);
    for (id, params) in parts {
        stack.push(behaviors::create_with(id, params).unwrap());
    }
    stack.with_seed(1)
}

fn play(stack: &mut BehaviorStack, script: &Script) -> Vec<SimFrame> {
    simulate(stack, script, 1).unwrap()
}

fn face(frame: u32, f: FaceInput) -> ScriptEvent {
    ScriptEvent::Face { frame, face: f }
}

fn key(frame: u32, k: &str, down: bool) -> ScriptEvent {
    ScriptEvent::Key { frame, key: k.into(), down }
}

fn shown(f: &SimFrame, path: &str) -> Option<bool> {
    f.pose.visibility.get(path).copied()
}

// ---- Head Turner ------------------------------------------------------------------------------------------

fn head_rig() -> Rig {
    Rig::default()
        .with_tagged(ids::FRONT, "Head/Frontal")
        .with_tagged(ids::LEFT_QUARTER, "Head/Left Quarter")
        .with_tagged(ids::RIGHT_QUARTER, "Head/Right Quarter")
        .with_tagged(ids::LEFT_PROFILE, "Head/Left Profile")
        .with_tagged(ids::RIGHT_PROFILE, "Head/Right Profile")
        .with_tagged(ids::UPWARD, "Head/Upward")
        .with_tagged(ids::DOWNWARD, "Head/Downward")
}

const VIEWS: [&str; 7] = ["Head/Frontal", "Head/Left Quarter", "Head/Right Quarter", "Head/Left Profile", "Head/Right Profile", "Head/Upward", "Head/Downward"];

/// The one view shown at the end of a one-second run with the head held at this yaw and pitch.
fn view_at(params: Value, yaw: f64, pitch: f64) -> &'static str {
    let mut s = stack_of(head_rig(), &[("Adobe.HeadTurner.js", params)]);
    let f = play(&mut s, &Script::new(30.0, 30).with(face(0, FaceInput { head_yaw: yaw, head_pitch: pitch, ..FaceInput::default() })));
    let last = &f[29];
    let on: Vec<&str> = VIEWS.into_iter().filter(|v| shown(last, v) == Some(true)).collect();
    assert_eq!(on.len(), 1, "exactly one view shows: {on:?}");
    assert!(VIEWS.iter().all(|v| shown(last, v).is_some()), "every view is told to show or hide");
    on[0]
}

#[test]
fn the_view_follows_the_yaw_and_pitch() {
    let p = json!({"smoothingRate": 0});
    assert_eq!(view_at(p.clone(), 0.0, 0.0), "Head/Frontal");
    assert_eq!(view_at(p.clone(), 5.0, 0.0), "Head/Frontal");
    assert_eq!(view_at(p.clone(), 20.0, 0.0), "Head/Right Quarter");
    assert_eq!(view_at(p.clone(), -20.0, 0.0), "Head/Left Quarter");
    assert_eq!(view_at(p.clone(), 50.0, 0.0), "Head/Right Profile");
    assert_eq!(view_at(p.clone(), -50.0, 0.0), "Head/Left Profile");
    assert_eq!(view_at(p.clone(), 0.0, 30.0), "Head/Upward");
    assert_eq!(view_at(p.clone(), 0.0, -30.0), "Head/Downward");
    // Turned away, the nod does not matter.
    assert_eq!(view_at(p.clone(), 50.0, 30.0), "Head/Right Profile");
    assert_eq!(view_at(json!({"smoothingRate": 0, "useVerticalViews": false}), 0.0, 30.0), "Head/Frontal");
    assert_eq!(view_at(json!({"smoothingRate": 0, "mirror": true}), 50.0, 0.0), "Head/Left Profile");
    assert_eq!(view_at(json!({"smoothingRate": 0, "sensitivity": 50}), 20.0, 0.0), "Head/Frontal", "half the turn counts");
}

#[test]
fn a_head_held_near_a_threshold_does_not_flicker() {
    let mut s = stack_of(head_rig(), &[("Adobe.HeadTurner.js", json!({"smoothingRate": 0, "quarterAngle": 12, "hysteresis": 4}))]);
    // 13 degrees is past the 12 degree limit but inside the dead band; 15 clears it; 11 is still inside the band
    // on the way back and only 9 leaves it.
    let yaws = [13.0, 15.0, 13.0, 11.0, 9.0];
    let mut script = Script::new(10.0, 5);
    for (i, yaw) in yaws.iter().enumerate() {
        script = script.with(face(u32::try_from(i).unwrap(), FaceInput { head_yaw: *yaw, ..FaceInput::default() }));
    }
    let f = play(&mut s, &script);
    let quarter: Vec<bool> = f.iter().map(|f| shown(f, "Head/Right Quarter") == Some(true)).collect();
    assert_eq!(quarter, [false, true, true, true, false]);
}

#[test]
fn missing_views_fall_back_and_a_puppet_without_views_is_left_alone() {
    // No quarter or profile layers: a turn keeps the frontal view.
    let rig = Rig::default().with_tagged(ids::FRONT, "Head/Frontal");
    let mut s = stack_of(rig, &[("Adobe.HeadTurner.js", json!({"smoothingRate": 0}))]);
    let f = play(&mut s, &Script::new(10.0, 3).with(face(0, FaceInput { head_yaw: 60.0, ..FaceInput::default() })));
    assert_eq!(shown(&f[2], "Head/Frontal"), Some(true));
    // No quarter layers: a profile is the nearest for a quarter turn.
    let rig = Rig::default().with_tagged(ids::FRONT, "F").with_tagged(ids::RIGHT_PROFILE, "RP");
    let mut s = stack_of(rig, &[("Adobe.HeadTurner.js", json!({"smoothingRate": 0}))]);
    let f = play(&mut s, &Script::new(10.0, 3).with(face(0, FaceInput { head_yaw: 20.0, ..FaceInput::default() })));
    assert_eq!((shown(&f[2], "F"), shown(&f[2], "RP")), (Some(true), Some(false)), "profile only from the profile angle");
    // Nothing tagged: nothing is touched.
    let mut s = stack_of(Rig::default(), &[("Adobe.HeadTurner.js", json!({}))]);
    assert!(
        play(&mut s, &Script::new(10.0, 3).with(face(0, FaceInput { head_yaw: 60.0, ..FaceInput::default() }))).iter().all(|f| f.pose.visibility.is_empty())
    );
}

#[test]
fn smoothing_delays_the_swap() {
    let mut s = stack_of(head_rig(), &[("Adobe.HeadTurner.js", json!({"smoothingRate": 50}))]);
    let f = play(&mut s, &Script::new(30.0, 60).with(face(0, FaceInput::default())).with(face(5, FaceInput { head_yaw: 50.0, ..FaceInput::default() })));
    let first = f.iter().position(|f| shown(f, "Head/Right Profile") == Some(true)).unwrap();
    assert!(first > 8, "the profile shows after the smoothed angle gets there, not at once: frame {first}");
    assert!(first < 40);
}

// ---- Motion Trigger ---------------------------------------------------------------------------------------

fn motion_rig() -> Rig {
    Rig::new(Size::new(2000.0, 1000.0))
        .with_tagged(ids::AT_REST, "Rest")
        .with_tagged(ids::MOVING_LEFT, "Left")
        .with_tagged(ids::MOVING_RIGHT, "Right")
        .with_tagged(ids::IN_TRANSITION, "Turn")
}

fn on(f: &SimFrame) -> Vec<&'static str> {
    ["Rest", "Left", "Right", "Turn"].into_iter().filter(|p| shown(f, p) == Some(true)).collect()
}

#[test]
fn motion_trigger_follows_the_walk() {
    let parts = [("Charcraft.Walk.js", json!({"easeRate": 0})), ("Adobe.MotionTrigger.js", json!({"transitionTime": 100, "restDelay": 100}))];
    let mut s = stack_of(motion_rig(), &parts);
    let script = Script::new(60.0, 360).with(key(30, "right", true)).with(key(120, "right", false)).with(key(180, "left", true)).with(key(270, "left", false));
    let f = play(&mut s, &script);
    assert_eq!(on(&f[10]), ["Rest"]);
    assert_eq!(on(&f[100]), ["Right"]);
    assert_eq!(on(&f[170]), ["Rest"], "stopped");
    assert_eq!(on(&f[260]), ["Left"]);
    assert_eq!(on(&f[350]), ["Rest"]);
}

#[test]
fn motion_trigger_shows_the_transition_between_directions() {
    let parts =
        [("Charcraft.Walk.js", json!({"easeRate": 0})), ("Adobe.MotionTrigger.js", json!({"transitionTime": 200, "restDelay": 5000, "smoothingRate": 0}))];
    let mut s = stack_of(motion_rig(), &parts);
    // Right, then straight to left with no rest in between.
    let script = Script::new(60.0, 240).with(key(10, "right", true)).with(key(100, "right", false)).with(key(100, "left", true));
    let f = play(&mut s, &script);
    assert_eq!(on(&f[90]), ["Right"]);
    assert!(f[100..115].iter().any(|f| on(f) == ["Turn"]), "the in-transition layer shows for a moment");
    assert_eq!(on(&f[200]), ["Left"]);
}

#[test]
fn motion_trigger_falls_back_to_rest_and_watches_the_chosen_source() {
    // Only an at-rest and a moving-right layer: moving left shows the rest layer.
    let rig = Rig::new(Size::new(2000.0, 1000.0)).with_tagged(ids::AT_REST, "Rest").with_tagged(ids::MOVING_RIGHT, "Right");
    let mut s = stack_of(rig.clone(), &[("Charcraft.Walk.js", json!({})), ("Adobe.MotionTrigger.js", json!({}))]);
    let f = play(&mut s, &Script::new(60.0, 120).with(key(0, "left", true)));
    assert_eq!((shown(&f[100], "Rest"), shown(&f[100], "Right")), (Some(true), Some(false)));
    // Walking moves the body, not the head: a head-only trigger stays at rest.
    let mut s = stack_of(motion_rig(), &[("Charcraft.Walk.js", json!({})), ("Adobe.MotionTrigger.js", json!({"source": "head"}))]);
    let f = play(&mut s, &Script::new(60.0, 120).with(key(0, "right", true)));
    assert_eq!(on(&f[100]), ["Rest"]);
    // A moving head does move it.
    let mut s = stack_of(
        motion_rig(),
        &[("Adobe.FaceTracker2.js", json!({"smoothingRate": 0})), ("Adobe.MotionTrigger.js", json!({"source": "head", "smoothingRate": 0}))],
    );
    let mut script = Script::new(60.0, 60);
    for i in 0..60_u32 {
        script = script.with(face(i, FaceInput { head_translation: anim_core::Vec2::new(f64::from(i) / 120.0, 0.0), ..FaceInput::default() }));
    }
    assert_eq!(on(&play(&mut s, &script)[50]), ["Right"]);
}

// ---- Cycle Layers -----------------------------------------------------------------------------------------

fn cycle_rig() -> Rig {
    Rig::default().with_tagged(ids::CYCLE, "Fire").with_cycle("Fire", &["Fire/1", "Fire/2", "Fire/3"])
}

fn frame_shown(f: &SimFrame) -> Vec<usize> {
    ["Fire/1", "Fire/2", "Fire/3"].iter().enumerate().filter(|(_, p)| shown(f, p) == Some(true)).map(|(i, _)| i).collect()
}

#[test]
fn the_layers_of_a_cycle_show_in_turn() {
    let mut s = stack_of(cycle_rig(), &[("Adobe.CycleLayers.js", json!({"framesPerSecond": 10}))]);
    let f = play(&mut s, &Script::new(10.0, 8));
    let seq: Vec<usize> = f.iter().map(|f| frame_shown(f)[0]).collect();
    assert_eq!(seq, [0, 1, 2, 0, 1, 2, 0, 1]);
    for f in &f {
        assert_eq!(frame_shown(f).len(), 1, "one layer at a time");
    }
    let mut s = stack_of(cycle_rig(), &[("Adobe.CycleLayers.js", json!({"framesPerSecond": 10, "playMode": "ping_pong"}))]);
    let seq: Vec<usize> = play(&mut s, &Script::new(10.0, 6)).iter().map(|f| frame_shown(f)[0]).collect();
    assert_eq!(seq, [0, 1, 2, 1, 0, 1]);
    let mut s = stack_of(cycle_rig(), &[("Adobe.CycleLayers.js", json!({"framesPerSecond": 10, "playMode": "once"}))]);
    let seq: Vec<usize> = play(&mut s, &Script::new(10.0, 6)).iter().map(|f| frame_shown(f)[0]).collect();
    assert_eq!(seq, [0, 1, 2, 2, 2, 2]);
}

#[test]
fn a_cycle_can_wait_for_a_key() {
    let parts = [("Adobe.CycleLayers.js", json!({"framesPerSecond": 10, "start": "while_key_held", "key": "f", "rest": "last"}))];
    let mut s = stack_of(cycle_rig(), &parts);
    let f = play(&mut s, &Script::new(10.0, 12).with(key(3, "f", true)).with(key(5, "f", false)));
    assert_eq!(frame_shown(&f[1]), [2], "rests on the last layer");
    assert_eq!(frame_shown(&f[3]), [0]);
    assert_eq!(frame_shown(&f[4]), [1]);
    assert_eq!(frame_shown(&f[8]), [2], "back to rest when released");

    let parts = [("Adobe.CycleLayers.js", json!({"framesPerSecond": 10, "start": "after_key_press", "key": "f"}))];
    let mut s = stack_of(cycle_rig(), &parts);
    let f = play(&mut s, &Script::new(10.0, 14).with(key(2, "f", true)).with(key(3, "f", false)));
    assert_eq!(frame_shown(&f[0]), [0]);
    let run: Vec<usize> = f[2..7].iter().map(|f| frame_shown(f)[0]).collect();
    assert_eq!(run, [0, 1, 2, 0, 0], "plays through once from the press, then rests on the first layer");
}

#[test]
fn a_puppet_without_cycles_is_left_alone() {
    let mut s = stack_of(Rig::default(), &[("Adobe.CycleLayers.js", json!({}))]);
    assert!(play(&mut s, &Script::new(10.0, 5)).iter().all(|f| f.pose.is_neutral()));
}

// ---- Walk styles and pinned feet --------------------------------------------------------------------------

fn walked(style: &str, seconds: u32) -> SimFrame {
    let mut s = stack_of(Rig::new(Size::new(4000.0, 1000.0)), &[("Charcraft.Walk.js", json!({"style": style}))]);
    let f = play(&mut s, &Script::new(60.0, 60 * seconds).with(key(0, "right", true)));
    f.last().unwrap().clone()
}

#[test]
fn the_styles_walk_differently() {
    let walk = walked("walk", 3);
    let run = walked("run", 3);
    let sneak = walked("sneak", 3);
    assert!(run.pose.root.translation.x > walk.pose.root.translation.x * 2.0, "running covers more ground");
    assert!(sneak.pose.root.translation.x < walk.pose.root.translation.x * 0.6, "sneaking covers less");
    let lean = |f: &SimFrame| f.pose.groups.get(ids::BODY).map_or(0.0, |x| x.rotation);
    // Over a whole number of cycles the sway cancels and the lean is what is left of the body angle: sample the mean.
    let mean_lean = |style: &str| {
        let mut s = stack_of(Rig::new(Size::new(4000.0, 1000.0)), &[("Charcraft.Walk.js", json!({"style": style}))]);
        let f = play(&mut s, &Script::new(60.0, 240).with(key(0, "right", true)));
        f[60..].iter().map(lean).sum::<f64>() / 180.0
    };
    assert!(mean_lean("run") > 0.1, "a runner leans forward");
    assert!(mean_lean("strut") < -0.02, "a strutter leans back");
    assert!(mean_lean("walk").abs() < 0.02);
    // A sneak crouches: the root sits lower (larger y) than a walk's at its lowest.
    let lowest = |style: &str| {
        let mut s = stack_of(Rig::new(Size::new(4000.0, 1000.0)), &[("Charcraft.Walk.js", json!({"style": style}))]);
        play(&mut s, &Script::new(60.0, 240).with(key(0, "right", true))).iter().map(|f| f.pose.root.translation.y).fold(f64::MIN, f64::max)
    };
    assert!(lowest("sneak") > lowest("walk") + 10.0);
    // A prance bounces higher.
    let highest = |style: &str| {
        let mut s = stack_of(Rig::new(Size::new(4000.0, 1000.0)), &[("Charcraft.Walk.js", json!({"style": style}))]);
        play(&mut s, &Script::new(60.0, 240).with(key(0, "right", true))).iter().map(|f| f.pose.root.translation.y).fold(f64::MAX, f64::min)
    };
    assert!(highest("prance") < highest("walk") - 5.0);
    // Unknown styles are plain walks.
    assert_eq!(walked("moonwalk", 2).pose.root, walked("walk", 2).pose.root);
}

fn feet_rig() -> Rig {
    // The ankle lives in a group the Transform behavior moves.
    let root = ChainLink { is_root: true, is_head: false, tags: Default::default(), path: String::new(), label: String::new(), origin: Point::ORIGIN };
    let body = ChainLink::named(ids::TRANSFORM, Point::new(100.0, 100.0));
    let ankle = RigHandle { chain: vec![root.clone(), body.clone()], ..RigHandle::new("legs/Left Ankle", HandleKind::Walk, Point::new(300.0, 600.0)) };
    let wrist = RigHandle { chain: vec![root, body], ..RigHandle::new("legs/Wrist", HandleKind::Walk, Point::new(300.0, 300.0)) };
    Rig::new(Size::new(1000.0, 1000.0)).with_handle(ankle).with_handle(wrist)
}

#[test]
fn standing_feet_stay_planted_when_the_body_moves() {
    let transform = ("Adobe.Transform.js", json!({"positionX": 20}));
    let mut s = stack_of(feet_rig(), &[transform.clone(), ("Charcraft.Walk.js", json!({}))]);
    let f = play(&mut s, &Script::new(30.0, 3));
    let d = f[2].pose.drag_of("legs/Left Ankle");
    assert!((d.x + 20.0).abs() < 1e-6 && d.y.abs() < 1e-6, "the foot is pulled back by the body's move: {d:?}");
    assert_eq!(f[2].pose.drag_of("legs/Wrist"), Default::default(), "only feet are pinned");
    let mut s = stack_of(feet_rig(), &[transform, ("Charcraft.Walk.js", json!({"pinFeet": false}))]);
    assert!(play(&mut s, &Script::new(30.0, 3))[2].pose.drag.is_empty());
}

#[test]
fn feet_are_free_while_walking_and_pinned_to_the_ground_where_it_stopped() {
    let mut s = stack_of(feet_rig(), &[("Charcraft.Walk.js", json!({"easeRate": 0}))]);
    let script = Script::new(60.0, 240).with(key(0, "right", true)).with(key(60, "right", false));
    let f = play(&mut s, &script);
    assert!(f[30].pose.drag.is_empty());
    // Standing again after the walk: the puppet travelled but the feet are where the puppet put them.
    assert!(f[200].pose.root.translation.x > 50.0);
    assert!(f[200].pose.drag.is_empty(), "{:?}", f[200].pose.drag);
}

// ---- Physics: gravity, collide, squash --------------------------------------------------------------------

fn hair_rig() -> Rig {
    let head =
        ChainLink { is_root: false, is_head: true, tags: Default::default(), path: "Head".into(), label: "head".into(), origin: Point::new(100.0, 100.0) };
    let hair = RigHandle { chain: vec![head], reach: 120.0, ..RigHandle::new("head/Hair", HandleKind::Dangle, Point::new(200.0, 100.0)) };
    Rig::new(Size::new(1000.0, 600.0)).with_handle(hair)
}

fn settled(rig: Rig, params: Value) -> (f64, f64) {
    let mut s = stack_of(rig, &[("Adobe.RigDynamics.js", params)]);
    let d = play(&mut s, &Script::new(60.0, 600)).last().unwrap().pose.drag_of("head/Hair");
    (d.x, d.y)
}

#[test]
fn gravity_pulls_hair_the_way_it_points() {
    let down = settled(hair_rig(), json!({}));
    assert!(down.0.abs() < 1e-6 && down.1.abs() < 1e-6, "the artwork already hangs: {down:?}");
    let up = settled(hair_rig(), json!({"gravityDirection": 0}));
    assert!(up.1 < -2.0 && up.0.abs() < 1e-3, "upside down gravity lifts the lock: {up:?}");
    let right = settled(hair_rig(), json!({"gravityDirection": 90}));
    assert!(right.0 > 1.0 && right.1 < 0.0, "sideways gravity swings it out and up from where it hung: {right:?}");
    let strong = settled(hair_rig(), json!({"gravityDirection": 0, "gravityStrength": 20}));
    assert!(strong.1 < up.1 * 0.9, "{strong:?} vs {up:?}");
}

#[test]
fn hair_bounces_off_a_collide_layer() {
    let wind = json!({"windStrength": 20, "windDirection": 90});
    let free = settled(hair_rig(), wind.clone());
    assert!(free.0 > 4.0, "the wind carries the lock a few pixels: {free:?}");
    let wall = Rect::new(203.0, 50.0, 400.0, 150.0);
    let blocked = settled(hair_rig().with_collider(wall), wind);
    assert!(blocked.0 <= 3.0 + 1e-6, "the lock stops at the wall: {blocked:?}");
    assert!(blocked.0 > 2.0, "and rests against it: {blocked:?}");
}

#[test]
fn squashiness_stretches_a_swinging_group() {
    let swing = |squash: f64| {
        let mut s = stack_of(
            hair_rig(),
            &[("Adobe.FaceTracker2.js", json!({"smoothingRate": 0, "headRotFactor": 100})), ("Adobe.RigDynamics.js", json!({"squashiness": squash}))],
        );
        let script = Script::new(60.0, 120).with(face(0, FaceInput::default())).with(face(10, FaceInput { head_roll: 30.0, ..FaceInput::default() }));
        play(&mut s, &script)
    };
    let plain = swing(0.0);
    assert!(plain.iter().all(|f| !f.pose.groups.contains_key("head")));
    let squashed = swing(100.0);
    let widest = squashed.iter().filter_map(|f| f.pose.groups.get("head")).map(|x| x.scale.x.max(x.scale.y)).fold(1.0, f64::max);
    assert!(widest > 1.02 && widest <= 1.5 + 1e-9, "{widest}");
    let thinnest = squashed.iter().filter_map(|f| f.pose.groups.get("head")).map(|x| x.scale.x.min(x.scale.y)).fold(1.0, f64::min);
    assert!(thinnest < 0.98, "volume is kept: what stretches one way thins the other ({thinnest})");
    let at_rest = squashed.last().unwrap().pose.groups.get("head").map_or(1.0, |x| x.scale.x);
    assert!((at_rest - 1.0).abs() < 0.01, "a settled lock is not squashed");
}

// ---- Face pose-to-pose ------------------------------------------------------------------------------------

fn head_rotations(params: Value) -> Vec<f64> {
    let mut s = stack_of(Rig::default(), &[("Adobe.FaceTracker2.js", params)]);
    // The head wobbles by a degree or two around zero for two seconds, then turns 25 degrees and wobbles there.
    let mut script = Script::new(30.0, 120);
    for i in 0..120_u32 {
        let wobble = if i % 2 == 0 { 1.5 } else { -1.5 };
        let base = if i < 60 { 0.0 } else { 25.0 };
        script = script.with(face(i, FaceInput { head_roll: base + wobble, ..FaceInput::default() }));
    }
    play(&mut s, &script).iter().map(|f| f.pose.head.rotation.to_degrees()).collect()
}

#[test]
fn pose_to_pose_holds_limited_poses() {
    let base = json!({"smoothingRate": 0, "headRotFactor": 100});
    let continuous = head_rotations(base.clone());
    let mut posed = base;
    posed["poseFilteringLevel"] = json!(50);
    posed["minPoseDuration"] = json!(0.5);
    let held = head_rotations(posed);
    let changes = |v: &[f64]| v.windows(2).filter(|w| (w[0] - w[1]).abs() > 1e-9).count();
    assert!(changes(&continuous) > 100, "the wobble moves the head every frame");
    assert!(changes(&held) <= 6, "a held pose changes only a few times: {}", changes(&held));
    assert!(held[30].abs() < 3.0, "still near zero before the turn: {}", held[30]);
    assert!((held[119] - 25.0).abs() < 4.0, "and near the turned head after it: {}", held[119]);
    // The pose does not change faster than the shortest gap allows.
    let stamps: Vec<usize> = held.windows(2).enumerate().filter(|(_, w)| (w[0] - w[1]).abs() > 1e-9).map(|(i, _)| i).collect();
    assert!(stamps.windows(2).all(|w| w[1] - w[0] >= 12), "poses are held at least half a second: {stamps:?}");
}

#[test]
fn level_zero_is_the_unfiltered_face() {
    let a = head_rotations(json!({"smoothingRate": 0, "headRotFactor": 100}));
    let b = head_rotations(json!({"smoothingRate": 0, "headRotFactor": 100, "poseFilteringLevel": 0, "minPoseDuration": 5}));
    assert_eq!(a, b);
}

// ---- Lip Sync audio source --------------------------------------------------------------------------------

fn mouth(source: &str, mic: Option<Voice>, scene: Option<Voice>) -> Option<Viseme> {
    let mut s = stack_of(Rig::default(), &[("Adobe.LipSync2.js", json!({"audioSource": source}))]);
    let mut script = Script::new(30.0, 3);
    if mic.is_some() {
        script = script.with(ScriptEvent::Voice { frame: 0, viseme: mic });
    }
    if scene.is_some() {
        script = script.with(ScriptEvent::SceneVoice { frame: 0, viseme: scene });
    }
    play(&mut s, &script)[2].pose.viseme
}

#[test]
fn lip_sync_listens_to_the_chosen_audio() {
    let (mic, scene) = (Some(Voice::Ah), Some(Voice::Oh));
    assert_eq!(mouth("microphone", mic, scene), Some(Viseme::Ah));
    assert_eq!(mouth("scene", mic, scene), Some(Viseme::Oh));
    assert_eq!(mouth("both", mic, scene), Some(Viseme::Ah), "the microphone wins while it is on");
    assert_eq!(mouth("both", None, scene), Some(Viseme::Oh), "otherwise the scene's sound is used");
    assert_eq!(mouth("microphone", None, scene), None);
    assert_eq!(mouth("scene", mic, None), None);
}

#[test]
fn the_new_behaviors_are_in_the_catalog_with_their_choices() {
    let catalog = charcraft_behaviors::api::catalog();
    for id in ["Adobe.HeadTurner.js", "Adobe.MotionTrigger.js", "Adobe.CycleLayers.js"] {
        assert!(catalog.iter().any(|c| c.id == id), "{id}");
    }
    let walk = catalog.iter().find(|c| c.id == "Charcraft.Walk.js").unwrap();
    let style = walk.docs.iter().find(|d| d.name == "style").unwrap();
    assert!(style.choices.contains(&"run") && style.choices.contains(&"sneak"));
    for entry in &catalog {
        for d in entry.docs.iter().filter(|d| !d.choices.is_empty()) {
            let default = entry.params[d.name].as_str().unwrap();
            assert!(d.choices.contains(&default), "{}: default {default:?} of {} is not a choice", entry.id, d.name);
        }
    }
}
