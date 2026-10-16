//! Each behavior driven by scripted inputs.
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
    clippy::redundant_clone,
    clippy::assert_is_empty,
    clippy::unnecessary_get_then_check,
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::similar_names,
    reason = "test code"
)]

use anim_core::{Point, Size, Tick, Vec2};
use anim_lipsync::Viseme as Voice;
use charcraft_behaviors::behaviors::{self, TriggerMode};
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

fn face(f: FaceInput) -> ScriptEvent {
    ScriptEvent::Face { frame: 0, face: f }
}

fn rises(frames: &[SimFrame]) -> Vec<f64> {
    let mut out = Vec::new();
    let mut was = false;
    for f in frames {
        let on = f.pose.blink_left > 0.5;
        if on && !was {
            out.push(f.time.seconds());
        }
        was = on;
    }
    out
}

// ---- Auto Blink -------------------------------------------------------------------------------------------

#[test]
fn a_metronome_blinks_at_an_even_pace() {
    let mut s = stack_of(Rig::default(), &[("Adobe.Blinker.js", json!({"rate": 60, "randomness": 0, "length": 100}))]);
    let frames = play(&mut s, &Script::new(120.0, 1200));
    let t = rises(&frames);
    assert!(t.len() >= 8, "{t:?}");
    for pair in t.windows(2) {
        assert!((pair[1] - pair[0] - 1.1).abs() < 0.03, "{t:?}");
    }
    assert!(frames.iter().all(|f| (0.0..=1.0).contains(&f.pose.blink_left) && f.pose.blink_left == f.pose.blink_right));
}

#[test]
fn blinks_are_seeded_and_do_not_depend_on_the_frame_rate() {
    let make = |seed| stack_of(Rig::default(), &[("Adobe.Blinker.js", json!({"rate": 30, "randomness": 100}))]).with_seed(seed);
    let slow = rises(&play(&mut make(3), &Script::new(30.0, 900)));
    let fast = rises(&play(&mut make(3), &Script::new(120.0, 3600)));
    assert!(slow.len() > 8, "{slow:?}");
    assert_eq!(slow.len(), fast.len());
    for (a, b) in slow.iter().zip(&fast) {
        assert!((a - b).abs() < 0.06, "{a} vs {b}");
    }
    assert_eq!(slow, rises(&play(&mut make(3), &Script::new(30.0, 900))), "same seed, same blinks");
    assert_ne!(slow, rises(&play(&mut make(4), &Script::new(30.0, 900))), "another seed blinks elsewhere");
}

#[test]
fn a_rate_of_zero_never_blinks_and_randomness_spreads_the_gaps() {
    let mut never = stack_of(Rig::default(), &[("Adobe.Blinker.js", json!({"rate": 0}))]);
    assert!(rises(&play(&mut never, &Script::new(30.0, 3000))).is_empty());

    let mut random = stack_of(Rig::default(), &[("Adobe.Blinker.js", json!({"rate": 60, "randomness": 100, "length": 80}))]);
    let t = rises(&play(&mut random, &Script::new(60.0, 6000)));
    let gaps: Vec<f64> = t.windows(2).map(|p| p[1] - p[0]).collect();
    let (min, max) = gaps.iter().fold((f64::MAX, 0.0_f64), |(lo, hi), g| (lo.min(*g), hi.max(*g)));
    assert!(max - min > 0.8, "gaps barely vary: {min}..{max}");
    let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
    assert!((mean - 1.08).abs() < 0.3, "mean gap {mean}");
}

// ---- Face -------------------------------------------------------------------------------------------------

fn crisp_face() -> Value {
    json!({"smoothingRate": 0, "headRotFactor": 100, "eyebrowFactor": 100, "moveEyebrowsTogether": false, "blinkOnly": false, "mouthFactor": 100})
}

#[test]
fn the_face_moves_the_head() {
    let mut s = stack_of(Rig::new(Size::new(1000.0, 600.0)), &[("Adobe.FaceTracker2.js", crisp_face())]);
    let f = FaceInput { head_roll: 20.0, head_translation: Vec2::new(1.0, -0.5), head_scale: 1.4, ..FaceInput::default() };
    let pose = play(&mut s, &Script::new(30.0, 2).with(face(f)))[1].pose.clone();
    assert!((pose.head.rotation - 20.0_f64.to_radians()).abs() < 1e-9);
    assert!((pose.head.translation.x - 1000.0 * 0.5 * 0.3).abs() < 1e-9, "{:?}", pose.head);
    assert!((pose.head.translation.y + 600.0 * 0.5 * 0.3 * 0.5).abs() < 1e-9);
    assert!((pose.head.scale.x - 1.1).abs() < 1e-9, "scale strength 25%: {:?}", pose.head.scale);
}

#[test]
fn a_head_turn_shifts_the_head_by_the_parallax() {
    let mut s = stack_of(Rig::new(Size::new(1000.0, 600.0)), &[("Adobe.FaceTracker2.js", crisp_face())]);
    let f = FaceInput { head_yaw: 45.0, ..FaceInput::default() };
    let pose = play(&mut s, &Script::new(30.0, 1).with(face(f)))[0].pose.clone();
    assert!((pose.head.translation.x - 1000.0 * 0.02).abs() < 1e-9);
}

#[test]
fn eyebrows_tilt_and_mirror() {
    let mut s = stack_of(Rig::default(), &[("Adobe.FaceTracker2.js", crisp_face())]);
    let f = FaceInput { eyebrow_left: 1.0, eyebrow_right: -1.0, ..FaceInput::default() };
    let pose = play(&mut s, &Script::new(30.0, 1).with(face(f)))[0].pose.clone();
    assert_eq!((pose.eyebrow_left, pose.eyebrow_right), (1.0, -1.0));
    assert!((pose.groups[ids::LEFT_EYEBROW].rotation - 20.0_f64.to_radians()).abs() < 1e-9);
    // The right brow is lowered (down tilt -20) and mirrored.
    assert!((pose.groups[ids::RIGHT_EYEBROW].rotation - 20.0_f64.to_radians()).abs() < 1e-9);

    let mut together = stack_of(Rig::default(), &[("Adobe.FaceTracker2.js", json!({"smoothingRate": 0, "eyebrowFactor": 100, "moveEyebrowsTogether": true}))]);
    let pose = play(&mut together, &Script::new(30.0, 1).with(face(f)))[0].pose.clone();
    assert_eq!((pose.eyebrow_left, pose.eyebrow_right), (0.0, 0.0), "the mean of up and down is rest");
}

#[test]
fn eyes_follow_the_lids_or_only_the_blink() {
    let half_shut = FaceInput { eye_open_left: 0.2, eye_open_right: 0.8, ..FaceInput::default() };
    let mut lids = stack_of(Rig::default(), &[("Adobe.FaceTracker2.js", crisp_face())]);
    let p = play(&mut lids, &Script::new(30.0, 1).with(face(half_shut)))[0].pose.clone();
    assert!((p.blink_left - 0.8).abs() < 1e-9 && (p.blink_right - 0.2).abs() < 1e-9);

    let mut blink_only = stack_of(Rig::default(), &[("Adobe.FaceTracker2.js", json!({"smoothingRate": 0, "blinkOnly": true}))]);
    let p = play(&mut blink_only, &Script::new(30.0, 1).with(face(half_shut)))[0].pose.clone();
    assert_eq!((p.blink_left, p.blink_right), (1.0, 0.0));
}

#[test]
fn the_tracked_mouth_picks_a_shape_only_with_a_mouth_strength() {
    let shape = |params: Value, f: FaceInput| {
        let mut s = stack_of(Rig::default(), &[("Adobe.FaceTracker2.js", params)]);
        play(&mut s, &Script::new(30.0, 1).with(face(f)))[0].pose.viseme
    };
    let open = FaceInput { mouth_open: 0.5, ..FaceInput::default() };
    assert_eq!(shape(json!({"smoothingRate": 0}), open), None, "mouthFactor defaults to 0");
    assert_eq!(shape(crisp_face(), open), Some(Viseme::Ah));
    assert_eq!(shape(crisp_face(), FaceInput { mouth_open: 0.5, mouth_wide: 0.1, ..FaceInput::default() }), Some(Viseme::Oh));
    assert_eq!(shape(crisp_face(), FaceInput { mouth_open: 0.5, mouth_wide: 0.9, ..FaceInput::default() }), Some(Viseme::Ee));
    assert_eq!(shape(crisp_face(), FaceInput { mouth_open: 0.95, mouth_wide: 0.3, ..FaceInput::default() }), Some(Viseme::Surprised));
    assert_eq!(shape(crisp_face(), FaceInput { smile: 1.0, ..FaceInput::default() }), Some(Viseme::Smile));
    assert_eq!(shape(crisp_face(), FaceInput::default()), None);
}

#[test]
fn smoothing_lags_then_catches_up_and_a_lost_face_relaxes() {
    let mut s = stack_of(Rig::default(), &[("Adobe.FaceTracker2.js", json!({"smoothingRate": 15, "headRotFactor": 100}))]);
    let f = FaceInput { head_roll: 30.0, ..FaceInput::default() };
    let script = Script::new(30.0, 240)
        .with(ScriptEvent::Face { frame: 0, face: FaceInput::default() })
        .with(ScriptEvent::Face { frame: 1, face: f })
        .with(ScriptEvent::NoFace { frame: 120 });
    let frames = play(&mut s, &script);
    let deg = |i: usize| frames[i].pose.head.rotation.to_degrees();
    assert!(deg(0).abs() < 1e-9);
    assert!(deg(2) > 1.0 && deg(2) < 15.0, "{}", deg(2));
    assert!((1..100).all(|i| deg(i + 1) >= deg(i) - 1e-9), "monotone rise");
    assert!((deg(110) - 30.0).abs() < 0.5);
    assert!(deg(239).abs() < 0.5, "relaxed to {}", deg(239));
}

#[test]
fn without_a_face_the_pose_stays_neutral() {
    let mut s = stack_of(Rig::default(), &[("Adobe.FaceTracker2.js", json!({}))]);
    assert!(play(&mut s, &Script::new(30.0, 30)).iter().all(|f| f.pose.is_neutral()));
}

// ---- Lip Sync ---------------------------------------------------------------------------------------------

fn voice(frame: u32, v: Voice) -> ScriptEvent {
    ScriptEvent::Voice { frame, viseme: Some(v) }
}

#[test]
fn the_voice_picks_the_mouth() {
    let mut s = stack_of(Rig::default(), &[("Adobe.LipSync2.js", json!({}))]);
    let script = Script::new(10.0, 6).with(voice(0, Voice::Ah)).with(voice(2, Voice::WOo)).with(voice(4, Voice::M));
    let shown: Vec<_> = play(&mut s, &script).iter().map(|f| f.pose.viseme).collect();
    assert_eq!(shown, [Viseme::Ah, Viseme::Ah, Viseme::WOo, Viseme::WOo, Viseme::M, Viseme::M].map(Some));
    let mut quiet = stack_of(Rig::default(), &[("Adobe.LipSync2.js", json!({}))]);
    assert!(play(&mut quiet, &Script::new(10.0, 3)).iter().all(|f| f.pose.viseme.is_none()), "no microphone, no opinion");
}

#[test]
fn trims_hold_a_shape_a_little_longer() {
    let script = Script::new(100.0, 60).with(voice(0, Voice::Ah)).with(voice(10, Voice::M));
    let mut s = stack_of(Rig::default(), &[("Adobe.LipSync2.js", json!({"Trim_Ah": 200}))]);
    let frames = play(&mut s, &script);
    assert_eq!(frames[20].pose.viseme, Some(Viseme::Ah));
    assert_eq!(frames[28].pose.viseme, Some(Viseme::Ah));
    assert_eq!(frames[35].pose.viseme, Some(Viseme::M));
}

#[test]
fn long_silence_hands_the_mouth_back_to_the_face() {
    let mut s = stack_of(Rig::default(), &[("Adobe.FaceTracker2.js", crisp_face()), ("Adobe.LipSync2.js", json!({}))]);
    let script = Script::new(10.0, 20).with(face(FaceInput { mouth_open: 0.5, ..FaceInput::default() })).with(voice(0, Voice::Neutral));
    let frames = play(&mut s, &script);
    assert_eq!(frames[2].pose.viseme, Some(Viseme::Neutral), "silence is not yet long");
    assert_eq!(frames[8].pose.viseme, Some(Viseme::Ah), "the camera's mouth shows after 500 ms");
    // Speaking again takes the mouth back.
    let script = script.with(voice(12, Voice::Oh));
    let frames = play(&mut s, &script);
    assert_eq!(frames[14].pose.viseme, Some(Viseme::Oh));
}

#[test]
fn the_jaw_drops_with_an_open_mouth() {
    let mut rig = Rig::new(Size::new(1000.0, 500.0));
    rig.tags.insert(ids::JAW.to_owned());
    let mut s = stack_of(rig, &[("Adobe.LipSync2.js", json!({}))]);
    let frames = play(&mut s, &Script::new(30.0, 60).with(voice(0, Voice::Ah)));
    let y = frames[59].pose.groups[ids::JAW].translation.y;
    assert!((y - 0.02 * 500.0).abs() < 0.1, "{y}");
    let mut stiff = stack_of(Rig::new(Size::new(1000.0, 500.0)), &[("Adobe.LipSync2.js", json!({}))]);
    assert!(play(&mut stiff, &Script::new(30.0, 60).with(voice(0, Voice::Ah)))[59].pose.groups.is_empty(), "no jaw layer, no jaw");
}

// ---- Eye Gaze ---------------------------------------------------------------------------------------------

fn gaze_rig() -> Rig {
    let mut rig = Rig::new(Size::new(1000.0, 600.0));
    rig.eyes_center = Point::new(500.0, 300.0);
    rig
}

fn pointer(frame: u32, x: f64, y: f64) -> ScriptEvent {
    ScriptEvent::Pointer { frame, id: 0, x, y, down: false }
}

#[test]
fn the_eyes_follow_the_mouse() {
    let crisp = json!({"smoothingRate": 0, "snapEyeGaze": false});
    let mut s = stack_of(gaze_rig(), &[("Adobe.EyeGaze.js", crisp.clone())]);
    let right = play(&mut s, &Script::new(30.0, 2).with(pointer(0, 590.0, 300.0)))[1].pose.gaze;
    // The cursor is half a full look away; the sideways range is scaled by 0.7.
    assert!((right.x - 0.35).abs() < 1e-9 && right.y.abs() < 1e-9, "{right:?}");
    let far = play(&mut s, &Script::new(30.0, 2).with(pointer(0, 5000.0, 300.0)))[1].pose.gaze;
    assert!((far.hypot() - 1.0).abs() < 1e-9 || (far.x - 0.7).abs() < 1e-9, "{far:?}");
    let up = play(&mut s, &Script::new(30.0, 2).with(pointer(0, 500.0, 300.0 - 90.0)))[1].pose.gaze;
    assert!((up.y + 0.75).abs() < 1e-9, "up/down range is scaled by 1.5: {up:?}");
    let none = play(&mut s, &Script::new(30.0, 2))[1].pose.gaze;
    assert_eq!(none, Vec2::ZERO);
}

#[test]
fn arrow_keys_beat_the_mouse() {
    let mut s = stack_of(gaze_rig(), &[("Adobe.EyeGaze.js", json!({"smoothingRate": 0, "snapEyeGaze": false}))]);
    let script = Script::new(30.0, 2).with(pointer(0, 590.0, 300.0)).with(ScriptEvent::Key { frame: 0, key: "Left".into(), down: true });
    let g = play(&mut s, &script)[1].pose.gaze;
    assert!((g.x + 0.7).abs() < 1e-9, "{g:?}");
}

#[test]
fn snapping_darts_between_directions_and_holds_them() {
    let mut s = stack_of(gaze_rig(), &[("Adobe.EyeGaze.js", json!({"smoothingRate": 0, "minGazeDuration": 1.0}))]);
    // Twenty degrees below the horizontal snaps to the horizontal (eight directions, 45 degrees apart).
    let near = play(&mut s, &Script::new(10.0, 2).with(pointer(0, 500.0 + 170.0, 300.0 + 62.0)))[1].pose.gaze;
    assert!(near.y.abs() < 1e-9 && near.x > 0.5, "{near:?}");
    // A look smaller than the minimum snap distance stays centred.
    let tiny = play(&mut s, &Script::new(10.0, 2).with(pointer(0, 510.0, 300.0)))[1].pose.gaze;
    assert_eq!(tiny, Vec2::ZERO);
    // After looking right, a move to the left waits for the minimum gaze duration.
    let script = Script::new(10.0, 30).with(pointer(0, 700.0, 300.0)).with(pointer(5, 300.0, 300.0));
    let frames = play(&mut s, &script);
    assert!(frames[8].pose.gaze.x > 0.5, "still looking right at 0.8 s: {:?}", frames[8].pose.gaze);
    assert!(frames[15].pose.gaze.x < -0.5, "looking left at 1.5 s: {:?}", frames[15].pose.gaze);
}

#[test]
fn the_tracked_gaze_passes_through_unsnapped() {
    let mut s = stack_of(
        gaze_rig(),
        &[
            ("Adobe.FaceTracker2.js", crisp_face()),
            ("Adobe.EyeGaze.js", json!({"smoothingRate": 0, "leftRightScaleFactor": 1.0, "upDownScaleFactor": 1.0, "offAxisScaleFactor": 1.0})),
        ],
    );
    let script = Script::new(30.0, 2).with(face(FaceInput { gaze: Vec2::new(0.3, -0.2), ..FaceInput::default() }));
    let g = play(&mut s, &script)[1].pose.gaze;
    assert!((g.x - 0.3).abs() < 1e-9 && (g.y + 0.2).abs() < 1e-9, "{g:?}");
}

// ---- Dragger ----------------------------------------------------------------------------------------------

fn hand_rig() -> Rig {
    Rig::new(Size::new(1000.0, 600.0)).with_handle(RigHandle::new("hand/Drag", HandleKind::Draggable, Point::new(200.0, 300.0)))
}

fn mouse(frame: u32, x: f64, y: f64, down: bool) -> ScriptEvent {
    ScriptEvent::Pointer { frame, id: 0, x, y, down }
}

fn drag_script() -> Script {
    Script::new(100.0, 60).with(mouse(5, 205.0, 302.0, true)).with(mouse(10, 255.0, 322.0, true)).with(mouse(20, 255.0, 322.0, false))
}

#[test]
fn a_pointer_pulls_a_draggable_handle_and_it_springs_back() {
    let mut s = stack_of(hand_rig(), &[("Adobe.Dragger3.js", json!({}))]);
    let frames = play(&mut s, &drag_script());
    assert!(frames[4].pose.drag.is_empty());
    let d = frames[12].pose.drag_of("hand/Drag");
    assert!((d.x - 50.0).abs() < 1e-9 && (d.y - 20.0).abs() < 1e-9, "{d:?}");
    // 0.2 s glide home: halfway at 0.1 s after the release at frame 20.
    let mid = frames[30].pose.drag_of("hand/Drag");
    assert!((mid.x - 25.0).abs() < 1.0 && (mid.y - 10.0).abs() < 1.0, "{mid:?}");
    assert!(frames[45].pose.drag.is_empty(), "back at rest");
}

#[test]
fn a_handle_can_stay_where_it_was_dropped() {
    let mut s = stack_of(hand_rig(), &[("Adobe.Dragger3.js", json!({"onMouseUp": 0}))]);
    let frames = play(&mut s, &drag_script());
    assert_eq!(frames[59].pose.drag_of("hand/Drag"), Vec2::new(50.0, 20.0));
}

#[test]
fn the_range_limit_stops_a_long_pull() {
    let mut s = stack_of(hand_rig(), &[("Adobe.Dragger3.js", json!({"limit_range_cb": true, "range": 50}))]);
    let frames = play(&mut s, &drag_script());
    // The reach of a hand-made handle is 100 px; 50 percent of it is 50 px, and the pull is 53.9 px.
    assert!((frames[12].pose.drag_of("hand/Drag").hypot() - 50.0).abs() < 1e-9);
}

#[test]
fn only_a_press_near_a_handle_grabs_it() {
    let mut s = stack_of(hand_rig(), &[("Adobe.Dragger3.js", json!({}))]);
    let far = Script::new(100.0, 30).with(mouse(5, 400.0, 300.0, true)).with(mouse(10, 450.0, 300.0, true));
    assert!(play(&mut s, &far).iter().all(|f| f.pose.drag.is_empty()));
    // A pointer that is already down when it arrives over the handle did not press it.
    let slid = Script::new(100.0, 30).with(mouse(0, 400.0, 300.0, true)).with(mouse(5, 200.0, 300.0, true)).with(mouse(10, 260.0, 300.0, true));
    assert!(play(&mut s, &slid).iter().all(|f| f.pose.drag.is_empty()));
}

#[test]
fn a_recorded_drag_holds_the_named_handle_at_its_target_and_lets_go() {
    let rig = hand_rig().with_handle(RigHandle::new("other/Drag", HandleKind::Draggable, Point::new(700.0, 300.0)));
    let mut s = stack_of(rig, &[("Adobe.Dragger3.js", json!({}))]);
    let at = |i: i64, targets: Vec<charcraft_behaviors::HandleTarget>| {
        let mut inputs = charcraft_behaviors::Inputs::at(Tick::from_seconds_f64(i as f64 * 0.01));
        inputs.handle_targets = targets;
        inputs
    };
    let held = |x: f64, y: f64| vec![charcraft_behaviors::HandleTarget { handle: "hand/Drag".into(), position: Point::new(x, y), held: true }];
    // Far from the handle, and no press: the target alone takes the handle (a pointer would have missed it).
    assert!(s.step(&at(0, vec![])).drag.is_empty());
    let pose = s.step(&at(1, held(500.0, 340.0)));
    assert_eq!(pose.drag_of("hand/Drag"), Vec2::new(300.0, 40.0));
    assert!(pose.drag_of("other/Drag") == Vec2::ZERO, "only the named handle moves");
    let pose = s.step(&at(2, held(260.0, 300.0)));
    assert_eq!(pose.drag_of("hand/Drag"), Vec2::new(60.0, 0.0));
    // Not held any more: glides home in 0.2 s.
    assert_eq!(s.step(&at(12, vec![])).drag_of("hand/Drag"), Vec2::new(60.0, 0.0), "the glide starts where it was held");
    let mid = s.step(&at(22, vec![])).drag_of("hand/Drag");
    assert!(mid.x > 5.0 && mid.x < 55.0, "{mid:?}");
    assert!(s.step(&at(40, vec![])).drag.is_empty());
    // A target naming no handle is ignored.
    let ghost = vec![charcraft_behaviors::HandleTarget { handle: "nobody/Drag".into(), position: Point::new(1.0, 1.0), held: true }];
    assert!(s.step(&at(50, ghost)).drag.is_empty());
}

#[test]
fn two_fingers_pull_two_handles() {
    let rig = hand_rig().with_handle(RigHandle::new("other/Drag", HandleKind::Draggable, Point::new(700.0, 300.0)));
    let mut s = stack_of(rig, &[("Adobe.Dragger3.js", json!({}))]);
    let finger = |frame, id, x, y| ScriptEvent::Pointer { frame, id, x, y, down: true };
    let script = Script::new(100.0, 20)
        .with(finger(2, 1, 200.0, 300.0))
        .with(finger(2, 2, 700.0, 300.0))
        .with(finger(6, 1, 230.0, 300.0))
        .with(finger(6, 2, 700.0, 340.0));
    let pose = &play(&mut s, &script)[10].pose;
    assert_eq!(pose.drag_of("hand/Drag"), Vec2::new(30.0, 0.0));
    assert_eq!(pose.drag_of("other/Drag"), Vec2::new(0.0, 40.0));
}

#[test]
fn a_drag_is_turned_into_the_groups_own_space() {
    let head =
        ChainLink { is_root: false, is_head: true, tags: Default::default(), path: "Head".into(), label: "head".into(), origin: Point::new(200.0, 300.0) };
    let handle = RigHandle { chain: vec![head], ..RigHandle::new("hair/Drag", HandleKind::Draggable, Point::new(300.0, 300.0)) };
    let rig = Rig::new(Size::new(1000.0, 600.0)).with_handle(handle);
    let mut s = stack_of(rig, &[("Adobe.FaceTracker2.js", json!({"smoothingRate": 0, "headRotFactor": 100})), ("Adobe.Dragger3.js", json!({}))]);
    // The head is rolled a quarter turn clockwise, so the handle now sits below the origin at (200, 400).
    let script = Script::new(100.0, 20)
        .with(face(FaceInput { head_roll: 90.0, ..FaceInput::default() }))
        .with(mouse(2, 200.0, 400.0, true))
        .with(mouse(6, 200.0, 440.0, true));
    let d = play(&mut s, &script)[10].pose.drag_of("hair/Drag");
    // Forty pixels down on stage is forty pixels to the right in the turned group.
    assert!((d.x - 40.0).abs() < 1e-6 && d.y.abs() < 1e-6, "{d:?}");
}

// ---- Physics ----------------------------------------------------------------------------------------------

fn hair_rig() -> Rig {
    let head =
        ChainLink { is_root: false, is_head: true, tags: Default::default(), path: "Head".into(), label: "head".into(), origin: Point::new(100.0, 100.0) };
    let hair = RigHandle { chain: vec![head], reach: 120.0, ..RigHandle::new("head/Hair", HandleKind::Dangle, Point::new(200.0, 100.0)) };
    Rig::new(Size::new(1000.0, 600.0)).with_handle(hair)
}

#[test]
fn hair_swings_when_the_head_turns_and_settles() {
    let mut s = stack_of(hair_rig(), &[("Adobe.FaceTracker2.js", json!({"smoothingRate": 0, "headRotFactor": 100})), ("Adobe.RigDynamics.js", json!({}))]);
    let script = Script::new(60.0, 480)
        .with(face(FaceInput::default()))
        .with(ScriptEvent::Face { frame: 30, face: FaceInput { head_roll: 30.0, ..FaceInput::default() } });
    let frames = play(&mut s, &script);
    assert!(frames[..30].iter().all(|f| f.pose.drag.is_empty()), "nothing swings while nothing moves");
    let swing: Vec<f64> = frames.iter().map(|f| f.pose.drag_of("head/Hair").hypot()).collect();
    let peak = swing.iter().copied().fold(0.0, f64::max);
    assert!(peak > 2.0 && peak <= 240.0, "peak {peak}");
    assert!(swing[470] < 0.5, "settled: {}", swing[470]);
    // It overshoots: the displacement changes direction at least once before it settles.
    let signs = frames.iter().map(|f| f.pose.drag_of("head/Hair").y.signum()).collect::<Vec<_>>();
    assert!(signs.windows(2).any(|w| w[0] * w[1] < 0.0), "an underdamped spring crosses rest");
}

#[test]
fn the_same_inputs_swing_the_same_way() {
    let script = Script::new(60.0, 120).with(ScriptEvent::Face { frame: 10, face: FaceInput { head_roll: 25.0, ..FaceInput::default() } });
    let run = || {
        let mut s = stack_of(
            hair_rig(),
            &[("Adobe.FaceTracker2.js", json!({"smoothingRate": 0})), ("Adobe.RigDynamics.js", json!({"windStrength": 3, "windVariation": 50}))],
        );
        play(&mut s, &script)
    };
    assert_eq!(run(), run());
}

#[test]
fn wind_leans_the_hair_and_stiffness_resists_it() {
    let lean = |stiffness: f64| {
        let mut s = stack_of(hair_rig(), &[("Adobe.RigDynamics.js", json!({"windStrength": 5, "windDirection": 90, "stiffness": stiffness}))]);
        play(&mut s, &Script::new(60.0, 600)).last().unwrap().pose.drag_of("head/Hair")
    };
    let normal = lean(100.0);
    assert!(normal.x > 0.5 && normal.x < 3.0 && normal.y.abs() < 1e-6, "{normal:?}");
    assert!(lean(25.0).x > normal.x * 3.0, "a floppy lock leans four times as far");
    assert!(lean(400.0).x < normal.x / 3.0);
}

#[test]
fn no_wind_and_no_motion_keeps_the_pose_neutral() {
    let mut s = stack_of(hair_rig(), &[("Adobe.RigDynamics.js", json!({}))]);
    assert!(play(&mut s, &Script::new(60.0, 60)).iter().all(|f| f.pose.is_neutral()));
}

// ---- Transform, Breathe, Walk, Triggers --------------------------------------------------------------------

#[test]
fn transform_blends_by_strength_and_honours_switches() {
    let pose = |params: Value| {
        let mut s = stack_of(Rig::default(), &[("Adobe.Transform.js", params)]);
        play(&mut s, &Script::new(30.0, 1))[0].pose.groups.get(ids::TRANSFORM).copied()
    };
    let full = pose(json!({"positionX": 10, "positionY": -4, "rotation": 30, "scaleX": 200})).unwrap();
    assert_eq!(full.translation, Vec2::new(10.0, -4.0));
    assert!((full.rotation - 30.0_f64.to_radians()).abs() < 1e-12);
    assert_eq!(full.scale, Vec2::new(2.0, 1.0));
    let half = pose(json!({"positionX": 10, "rotation": 30, "scaleX": 200, "strength": 50})).unwrap();
    assert_eq!(half.translation, Vec2::new(5.0, 0.0));
    assert!((half.rotation - 15.0_f64.to_radians()).abs() < 1e-12);
    assert_eq!(half.scale, Vec2::new(1.5, 1.0));
    let moved_only = pose(json!({"positionX": 10, "rotation": 30, "applyRotation": false})).unwrap();
    assert_eq!(moved_only.rotation, 0.0);
    assert_eq!(pose(json!({})), None, "the stock values change nothing");
    let uniform = pose(json!({"uniformScale": 50})).unwrap();
    assert_eq!(uniform.scale, Vec2::new(0.5, 0.5));
}

#[test]
fn an_anchor_keeps_its_point_fixed() {
    let mut s = stack_of(Rig::default(), &[("Adobe.Transform.js", json!({"rotation": 90, "scaleX": 150, "anchorPtX": 10, "anchorPtY": 5}))]);
    let x = play(&mut s, &Script::new(30.0, 1))[0].pose.groups[ids::TRANSFORM];
    let origin = Point::new(300.0, 200.0);
    let anchor = origin + Vec2::new(10.0, 5.0);
    let moved = x.about(origin) * anchor;
    assert!((moved.x - anchor.x).abs() < 1e-9 && (moved.y - anchor.y).abs() < 1e-9, "{moved:?}");
}

#[test]
fn breathing_follows_the_clock() {
    let b = behaviors::Breathe { params: behaviors::breathe::BreatheParams { rate: 60.0, strength: 100.0, inhale_ratio: 40.0 } };
    assert!(b.level(0.0).abs() < 1e-12);
    assert!((b.level(0.4) - 1.0).abs() < 1e-12);
    assert!(b.level(1.0).abs() < 1e-9);
    assert!(b.level(0.2) > 0.4 && b.level(0.2) < 0.6);
    assert_eq!(behaviors::Breathe { params: behaviors::breathe::BreatheParams { rate: 0.0, ..Default::default() } }.level(3.0), 0.0);

    let mut s = stack_of(Rig::default(), &[("Charcraft.Breathe.js", json!({"rate": 60, "strength": 50}))]);
    let frames = play(&mut s, &Script::new(100.0, 200));
    let top = frames.iter().map(|f| f.pose.breathe).fold(0.0, f64::max);
    assert!((top - 0.5).abs() < 1e-3, "{top}");
    // Seeking to the same time gives the same breath, whatever happened before.
    let again = play(&mut s, &Script::new(100.0, 41));
    assert_eq!(again[40].pose.breathe, frames[40].pose.breathe);
}

#[test]
fn walking_swings_the_limbs_and_travels() {
    let mut s = stack_of(Rig::new(Size::new(2000.0, 1000.0)), &[("Charcraft.Walk.js", json!({}))]);
    let script = Script::new(60.0, 360).with(ScriptEvent::Key { frame: 30, key: "Right".into(), down: true }).with(ScriptEvent::Key {
        frame: 150,
        key: "right".into(),
        down: false,
    });
    let frames = play(&mut s, &script);
    assert!(frames[..30].iter().all(|f| f.pose.is_neutral()));
    let mid = &frames[100].pose;
    assert!(mid.root.translation.x > 20.0 && mid.root.translation.y <= 0.0);
    assert!(mid.groups.contains_key(ids::LEFT_ARM) && mid.groups.contains_key(ids::RIGHT_ARM) && mid.groups.contains_key("left leg"));
    assert!(mid.groups[ids::LEFT_ARM].rotation * mid.groups[ids::RIGHT_ARM].rotation < 0.0, "arms swing opposite");
    let end = &frames[359].pose;
    assert!(end.groups.is_empty(), "limbs rest once stopped");
    assert!(end.root.translation.x > 100.0 && end.root.translation.y == 0.0, "the puppet stays where it stopped: {:?}", end.root);
    // Walking left goes the other way.
    let mut left = stack_of(Rig::new(Size::new(2000.0, 1000.0)), &[("Charcraft.Walk.js", json!({}))]);
    let f = play(&mut left, &Script::new(60.0, 120).with(ScriptEvent::Key { frame: 0, key: "left".into(), down: true }));
    assert!(f[119].pose.root.translation.x < -20.0);
}

#[test]
fn keys_hold_latch_and_time_triggers() {
    let rules = json!({"rules": [
        {"key": "l", "mode": "latch"},
        {"key": "t", "mode": {"timed": {"seconds": 0.3}}},
        {"key": "1", "swap": {"set": "hats", "member": "cap"}}
    ]});
    let mut s = stack_of(Rig::default(), &[("Adobe.Triggers.js", rules)]);
    let key = |frame, k: &str, down| ScriptEvent::Key { frame, key: k.into(), down };
    let script = Script::new(10.0, 14)
        .with(key(1, "a", true))
        .with(key(3, "a", false))
        .with(key(2, "L", true))
        .with(key(3, "L", false))
        .with(key(6, "l", true))
        .with(key(7, "l", false))
        .with(key(4, "t", true))
        .with(key(5, "t", false))
        .with(key(8, "1", true))
        .with(key(9, "1", false));
    let f = play(&mut s, &script);
    let on = |i: usize, k: &str| f[i].pose.triggers.contains(k);
    assert!(!on(0, "a") && on(1, "a") && on(2, "a") && !on(3, "a"), "held while down");
    assert!(!on(1, "l") && on(2, "l") && on(4, "l") && on(5, "l") && !on(6, "l"), "latched until pressed again");
    assert!(!on(3, "t") && on(4, "t") && on(6, "t") && !on(7, "t"), "timed for 300 ms from the press");
    assert!(
        f[7].pose.swaps.is_empty() && f[8].pose.swaps.get("hats").map(String::as_str) == Some("cap") && f[13].pose.swaps.get("hats").is_some(),
        "the swap choice stays"
    );
    assert_eq!(serde_json::from_value::<TriggerMode>(json!("latch")).unwrap(), TriggerMode::Latch);
}

// ---- The stack --------------------------------------------------------------------------------------------

#[test]
fn behaviors_combine_in_order_and_can_be_switched_off() {
    let parts = [
        ("Adobe.FaceTracker2.js", json!({"smoothingRate": 0, "headRotFactor": 100})),
        ("Adobe.Transform.js", json!({"positionX": 7})),
        ("Charcraft.Breathe.js", json!({"rate": 60})),
    ];
    let script = Script::new(10.0, 6).with(face(FaceInput { head_roll: 10.0, ..FaceInput::default() }));
    let mut s = stack_of(Rig::default(), &parts);
    let all = &play(&mut s, &script)[4].pose;
    assert!(all.head.rotation > 0.0 && all.groups.contains_key(ids::TRANSFORM) && all.breathe > 0.0);
    s.slot_mut(1).unwrap().enabled = false;
    s.slot_mut(2).unwrap().enabled = false;
    let only_face = &play(&mut s, &script)[4].pose;
    assert!(only_face.head.rotation > 0.0 && only_face.groups.is_empty() && only_face.breathe == 0.0);
}

#[test]
fn a_standard_stack_is_deterministic_and_a_scrub_restarts_it() {
    let script = Script::new(30.0, 300)
        .with(face(FaceInput { head_roll: 12.0, mouth_open: 0.4, ..FaceInput::default() }))
        .with(voice(30, Voice::Ee))
        .with(mouse(60, 900.0, 200.0, false))
        .with(ScriptEvent::Key { frame: 90, key: "right".into(), down: true });
    let run = |seed| play(&mut BehaviorStack::standard(Rig::default()).with_seed(seed), &script);
    assert_eq!(run(5), run(5));
    assert_ne!(run(5), run(6), "the seed shows in the blinks");

    let inputs = script.inputs().unwrap();
    let mut s = BehaviorStack::standard(Rig::default()).with_seed(5);
    let first: Vec<_> = inputs.iter().map(|i| s.step(i)).collect();
    // Scrubbing back to the start gives the first pose again, then the same run.
    let again: Vec<_> = inputs.iter().map(|i| s.step(i)).collect();
    assert_eq!(first, again);
}

#[test]
fn project_parameters_override_the_stored_defaults() {
    use charcraft_ltab::LValue;
    use charcraft_project::{BehaviorDefaults, BoundBehavior};
    let defaults =
        [BehaviorDefaults { behavior: "Adobe.Blinker.js".into(), params: vec![("rate".into(), LValue::Int(12)), ("length".into(), LValue::Int(120))] }];
    let bound = [
        BoundBehavior {
            id: "BB1".into(),
            behavior: "Adobe.Blinker.js".into(),
            ui_name: None,
            show_in_track_item_properties: false,
            params: vec![("rate".into(), LValue::float(45.5))],
        },
        BoundBehavior {
            id: "BB2".into(),
            behavior: "Adobe.Dragger3.js".into(),
            ui_name: None,
            show_in_track_item_properties: false,
            params: vec![("range".into(), LValue::Int(33)), ("onMouseUp".into(), LValue::Int(0))],
        },
        BoundBehavior { id: "BB3".into(), behavior: "Adobe.Mystery.js".into(), ui_name: None, show_in_track_item_properties: false, params: vec![] },
    ];
    let (stack, skipped) = BehaviorStack::from_bound(Rig::default(), &bound, &defaults);
    assert_eq!(skipped, ["Adobe.Mystery.js"]);
    let params: Vec<Value> = stack.slots().iter().map(|s| s.behavior.params()).collect();
    assert_eq!((params[0]["rate"].clone(), params[0]["length"].clone(), params[0]["randomness"].clone()), (json!(45.5), json!(120.0), json!(100.0)));
    assert_eq!((params[1]["range"].clone(), params[1]["onMouseUp"].clone(), params[1]["handleReleaseDuration"].clone()), (json!(33.0), json!(0.0), json!(0.2)));
}

#[test]
fn a_projects_behaviors_run_in_dependency_order() {
    use charcraft_project::BoundBehavior;
    let bind = |id: &str| BoundBehavior { id: "BB".into(), behavior: id.into(), ui_name: None, show_in_track_item_properties: false, params: vec![] };
    let bound =
        [bind("Adobe.RigDynamics.js"), bind("Adobe.Dragger3.js"), bind("Adobe.FaceTracker2.js"), bind("Adobe.HandleFixer.js"), bind("Adobe.LipSync2.js")];
    let (stack, skipped) = BehaviorStack::from_bound(Rig::default(), &bound, &[]);
    assert!(skipped.is_empty());
    let order: Vec<_> = stack.slots().iter().map(|s| s.behavior.id()).collect();
    assert_eq!(order, ["Adobe.FaceTracker2.js", "Adobe.LipSync2.js", "Adobe.HandleFixer.js", "Adobe.Dragger3.js", "Adobe.RigDynamics.js"]);
}

#[test]
fn parameter_files_parse_with_defaults_and_unknown_names() {
    let table = charcraft_ltab::parse("{ length = 90, rate = 7.5, somethingNew = { 1, 2 } }").unwrap();
    let pairs: Vec<(String, charcraft_ltab::LValue)> = table.iter().filter_map(|(k, v)| Some((k.as_str()?.to_owned(), v.clone()))).collect();
    let blinker = behaviors::create_from_pairs("Adobe.Blinker.js", &pairs).unwrap();
    assert_eq!(blinker.params(), json!({"length": 90.0, "randomness": 100.0, "rate": 7.5}));
    // Written back, the parameters read the same.
    let again = behaviors::create_from_pairs(
        "Adobe.Blinker.js",
        &behaviors::params_to_lvalue(blinker.as_ref()).iter().filter_map(|(k, v)| Some((k.as_str()?.to_owned(), v.clone()))).collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(again.params(), blinker.params());
    let _ = Tick::ZERO;
}
