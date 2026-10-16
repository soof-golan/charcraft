//! Hostile inputs and parameters never panic and never poison a pose.
#![allow(
    clippy::float_cmp,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::default_trait_access,
    clippy::redundant_clone,
    clippy::suboptimal_flops,
    clippy::missing_const_for_fn,
    reason = "test code"
)]

use anim_core::{Point, Size, Tick, Vec2};
use anim_lipsync::Viseme as Voice;
use charcraft_behaviors::{BehaviorStack, BodyInput, ChainLink, FaceInput, HandleTarget, Inputs, Joint, Pointer, Rig, RigAnchor, RigHandle, behaviors};
use charcraft_puppet::{HandleKind, PuppetPose};
use proptest::prelude::*;
use serde_json::Value;

fn rig() -> Rig {
    let head =
        ChainLink { is_root: false, is_head: true, tags: Default::default(), path: "Head".into(), label: "head".into(), origin: Point::new(100.0, 100.0) };
    let mut rig = Rig::new(Size::new(1000.0, 600.0));
    for tag in ["Charcraft.Particle", "Charcraft.Shadow", "Charcraft.Fader", "Adobe.Magnets.Magnet", "Adobe.Physics.Collide", "Adobe.MotionLines.Trails"] {
        let mut anchor = RigAnchor::new(tag, tag, Point::new(300.0, 300.0));
        anchor.bounds = Some(anim_core::Rect::new(250.0, 250.0, 350.0, 350.0));
        anchor.chain = vec![head.clone()];
        rig.anchors.push(anchor);
    }
    for (key, kind) in [("a/Drag", HandleKind::Draggable), ("a/Dangle", HandleKind::Dangle)] {
        rig.handles.push(RigHandle { chain: vec![head.clone()], reach: 80.0, ..RigHandle::new(key, kind, Point::new(150.0, 120.0)) });
    }
    rig
}

#[expect(clippy::disallowed_types, reason = "proptest's prop_oneof! builds an Arc inside the macro")]
fn any_f64() -> impl Strategy<Value = f64> {
    prop_oneof![3 => -10.0..10.0_f64, 1 => Just(f64::NAN), 1 => Just(f64::INFINITY), 1 => Just(f64::NEG_INFINITY), 1 => Just(f64::MAX), 1 => Just(0.0)]
}

fn any_face() -> impl Strategy<Value = FaceInput> {
    (prop::collection::vec(any_f64(), 12), any_f64()).prop_map(|(v, scale)| FaceInput {
        head_yaw: v[0],
        head_pitch: v[1],
        head_roll: v[2],
        head_translation: Vec2::new(v[3], v[4]),
        head_scale: scale,
        eyebrow_left: v[5],
        eyebrow_right: v[6],
        eye_open_left: v[7],
        eye_open_right: v[8],
        gaze: Vec2::new(v[9], v[10]),
        mouth_open: v[11],
        mouth_wide: v[0],
        smile: v[1],
    })
}

#[expect(clippy::disallowed_types, reason = "proptest's prop_oneof! builds an Arc inside the macro")]
fn any_inputs() -> impl Strategy<Value = Inputs> {
    let time = prop_oneof![4 => (0_i64..10_000_000_000_000).prop_map(Tick), 1 => Just(Tick(i64::MIN)), 1 => Just(Tick(i64::MAX)), 1 => Just(Tick::ZERO)];
    let pointers = prop::collection::vec((0_u32..3, any_f64(), any_f64(), any::<bool>()), 0..3);
    let keys = prop::collection::btree_set(prop::sample::select(vec!["left", "right", "up", "down", "a", "1"]), 0..3);
    let targets = prop::collection::vec((prop::sample::select(vec!["a/Drag", "a/Dangle", "nobody"]), any_f64(), any_f64(), any::<bool>()), 0..3);
    let body = prop::collection::vec((0_usize..17, any_f64(), any_f64()), 0..17).prop_map(|joints| BodyInput {
        joints: joints.into_iter().filter_map(|(i, x, y)| Some((*Joint::ALL.get(i)?, Vec2::new(x, y)))).collect(),
        rest: std::collections::BTreeMap::new(),
    });
    (time, prop::option::of(any_face()), prop::option::of(body), prop::option::of(prop::sample::select(Voice::ALL.to_vec())), pointers, keys, targets).prop_map(
        |(time, face, body, viseme, pointers, keys, targets)| Inputs {
            time,
            face,
            body,
            viseme,
            scene_viseme: viseme,
            pointers: pointers.into_iter().map(|(id, x, y, down)| Pointer { id, position: Point::new(x, y), down }).collect(),
            handle_targets: targets
                .into_iter()
                .map(|(handle, x, y, held)| HandleTarget { handle: handle.to_owned(), position: Point::new(x, y), held })
                .collect(),
            keys: keys.into_iter().map(str::to_owned).collect(),
        },
    )
}

fn finite(pose: &PuppetPose) -> bool {
    let x = |x: &charcraft_puppet::Xform| {
        x.translation.x.is_finite() && x.translation.y.is_finite() && x.rotation.is_finite() && x.scale.x.is_finite() && x.scale.y.is_finite()
    };
    x(&pose.root)
        && x(&pose.head)
        && pose.groups.values().all(x)
        && [pose.eyebrow_left, pose.eyebrow_right, pose.blink_left, pose.blink_right, pose.breathe, pose.gaze.x, pose.gaze.y].iter().all(|v| v.is_finite())
        && pose.drag.values().all(|d| d.x.is_finite() && d.y.is_finite())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn any_inputs_give_finite_poses(seq in prop::collection::vec(any_inputs(), 1..40), seed in any::<u64>()) {
        let mut stack = BehaviorStack::standard(rig()).with_seed(seed);
        for i in &seq {
            let pose = stack.step(i);
            prop_assert!(finite(&pose), "{pose:?}");
        }
    }

    #[test]
    fn identical_runs_agree(seq in prop::collection::vec(any_inputs(), 1..30), seed in any::<u64>()) {
        let run = || BehaviorStack::standard(rig()).with_seed(seed).run(&seq);
        let (a, b) = (run(), run());
        // NaN never reaches a pose, so equality is meaningful.
        prop_assert_eq!(a, b);
    }

    #[test]
    fn any_parameters_are_accepted_or_refused_without_panicking(id in prop::sample::select(behaviors::IDS.to_vec()), raw in "[ -~]{0,60}", n in any_f64()) {
        let mut b = behaviors::create(id).unwrap();
        let candidates = [serde_json::from_str::<Value>(&raw).unwrap_or(Value::Null), serde_json::json!({"rate": n, "range": n, "smoothingRate": n, "stiffness": n, "windStrength": n}), serde_json::json!({"rate": raw})];
        for c in &candidates {
            let before = b.params();
            if b.set_params(c).is_err() {
                prop_assert_eq!(b.params(), before, "a refused edit changes nothing");
            }
        }
        // Whatever it accepted, stepping with it is safe.
        let mut stack = BehaviorStack::new(rig());
        stack.push(b);
        for t in 0..10_i64 {
            let pose = stack.step(&Inputs::at(Tick(t * 4_000_000_000)).with_face(FaceInput::default()).with_pointer(0, 150.0, 120.0, t > 2));
            prop_assert!(finite(&pose));
        }
    }
}
