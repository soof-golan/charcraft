//! Hostile and random input: landmarks, frames, recordings and model bytes never panic, and the solver's output is
//! always inside its documented ranges.
#![expect(clippy::disallowed_types, reason = "proptest's prop_oneof! shares its strategies through an Arc")]
#![allow(clippy::arithmetic_side_effects, reason = "test code")]

use anim_core::Tick;
use charcraft_tracking::frame::Frame;
use charcraft_tracking::landmarks::{Landmarks, V3};
use charcraft_tracking::measure::measure;
use charcraft_tracking::model::{FACE_DETECTOR, ModelStore, verify};
use charcraft_tracking::pipeline::{Border, Roi, crop};
use charcraft_tracking::solver::FaceSolver;
use charcraft_tracking::source::ScriptedSource;
use charcraft_tracking::synthetic::SyntheticFace;
use proptest::prelude::*;

fn any_float() -> impl Strategy<Value = f32> {
    prop_oneof![
        8 => -2000.0_f32..2000.0,
        1 => Just(f32::NAN),
        1 => Just(f32::INFINITY),
        1 => Just(f32::NEG_INFINITY),
        1 => Just(0.0),
        1 => Just(f32::MAX),
    ]
}

fn any_landmarks() -> impl Strategy<Value = Landmarks> {
    (prop::collection::vec([any_float(), any_float(), any_float()], 0..500), 0_u32..5000, 0_u32..5000, any_float())
        .prop_map(|(points, frame_width, frame_height, score)| Landmarks { points: points.into_iter().map(V3).collect(), frame_width, frame_height, score })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn measuring_random_points_never_panics(l in any_landmarks()) {
        let _ = measure(&l);
    }

    #[test]
    fn the_solver_output_is_always_valid(frames in prop::collection::vec(any_landmarks(), 1..6)) {
        let mut solver = FaceSolver::default();
        solver.begin_calibration(2);
        for (i, l) in frames.iter().enumerate() {
            let out = solver.solve(Tick(i64::try_from(i).unwrap_or(0).saturating_mul(Tick::from_units(1, 30).0)), Some(l)).input;
            prop_assert!(out.is_valid(), "{out:?}");
        }
    }

    #[test]
    fn perturbed_real_faces_stay_valid(
        yaw in -1.5_f32..1.5, pitch in -1.5_f32..1.5, roll in -3.2_f32..3.2,
        open in -1.0_f32..2.0, mouth in -0.5_f32..2.0, brow in -1.0_f32..1.0, smile in -1.0_f32..1.0,
        gx in -3.0_f32..3.0, px in 5.0_f32..800.0,
    ) {
        let face = SyntheticFace {
            yaw, pitch, roll, eye_open: [open, 1.0 - open], mouth_open: mouth, brow_raise: [brow, -brow], smile, gaze: [gx, -gx], eye_distance_px: px,
            ..SyntheticFace::default()
        };
        let out = FaceSolver::default().solve(Tick::ZERO, Some(&face.landmarks())).input;
        prop_assert!(out.is_valid(), "{out:?}");
    }

    #[test]
    fn frames_of_any_size_are_checked(w in any::<u32>(), h in any::<u32>(), len in 0_usize..64) {
        let result = Frame::new(w, h, vec![0; len]);
        if let Ok(f) = result {
            prop_assert_eq!(usize::try_from(f.width()).unwrap() * usize::try_from(f.height()).unwrap() * 3, len);
        }
    }

    #[test]
    fn crops_of_any_roi_are_finite(cx in any_float(), cy in any_float(), size in any_float(), angle in any_float(), replicate in any::<bool>()) {
        let frame = Frame::solid(8, 6, [10, 120, 250]).unwrap();
        let border = if replicate { Border::Replicate } else { Border::Zero };
        let out = crop(&frame, &Roi { center: [cx, cy], size, angle }, 8, [-1.0, 1.0], border);
        prop_assert_eq!(out.len(), 8 * 8 * 3);
        prop_assert!(out.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn recordings_of_random_text_never_panic(text in ".{0,400}") {
        let _ = ScriptedSource::from_json_lines("fuzz", &text);
    }

    #[test]
    fn random_model_bytes_are_rejected(bytes in prop::collection::vec(any::<u8>(), 0..2000)) {
        prop_assert!(verify(&FACE_DETECTOR, &bytes).is_err());
        let dir = tempfile::tempdir().unwrap();
        prop_assert!(ModelStore::new(dir.path()).install(&FACE_DETECTOR, &bytes).is_err());
    }
}
