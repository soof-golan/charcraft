//! Predict, render, inspect: eyes, blinks, mouth shapes and head transforms. Predictions are in
//! `tests/fixtures/visual/*.expect.md`.
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
    reason = "test code"
)]

mod common;

use anim_core::{Point, Vec2};
use charcraft_puppet::{LayerId, Puppet, PuppetPose, Viseme, Xform};
use common::*;

fn puppet(wide: bool, body: bool) -> Puppet {
    Puppet::from_artwork(&face_tree(wide, body)).unwrap()
}

/// The head turn of 90 degrees clockwise about (120,140) maps a point to this.
fn turn90(x: i32, y: i32) -> (i32, i32) {
    (260 - y, x + 20)
}

// ---- eye-gaze-limits ---------------------------------------------------------------------------------------

#[test]
fn eye_gaze_limits() {
    let p = puppet(true, false);
    let at = |gaze: Vec2| {
        let mut pose = PuppetPose::neutral();
        pose.gaze = gaze;
        pose
    };
    let a = render(&p, &at(Vec2::new(1.0, 0.0)));
    let b = render(&p, &at(Vec2::new(-1.0, 1.0)));
    let c = render(&p, &at(Vec2::new(5.0, -5.0)));
    let mut pose_d = at(Vec2::new(1.0, 0.0));
    pose_d.pupil_left = Some(Vec2::new(-1.0, 0.0));
    let d = render(&p, &pose_d);
    let mut pose_e = at(Vec2::new(1.0, 0.0));
    pose_e.head = Xform::rotated_degrees(90.0);
    let e = render(&p, &pose_e);
    let f = render(&p, &at(Vec2::new(f64::NAN, f64::INFINITY)));
    save_sheet("eye-gaze-limits", &[&a, &b, &c, &d, &e, &f]);
    save_zoom("eye-gaze-limits-A-left-eye", &a, 120, 50, 70, 45, 8);

    // The boxes of the two whites at rest.
    let whites: [[i32; 4]; 2] = [[130, 60, 170, 84], [50, 60, 90, 84]];
    // Pupil boxes by dark pixels around each eye.
    let eye_box = |img: &anim_display::Image, white: [i32; 4]| {
        let mut b: Option<[i32; 4]> = None;
        for y in 40..120 {
            for x in (white[0] - 20)..(white[2] + 20) {
                if is_dark(px(img, x, y)) {
                    b = Some(match b {
                        None => [x, y, x + 1, y + 1],
                        Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1)],
                    });
                }
            }
        }
        b.unwrap()
    };
    let left = whites[0];
    let right = whites[1];
    assert_eq!(eye_box(&a, left), [158, 66, 170, 78], "A left pupil");
    assert_eq!(eye_box(&a, right), [78, 66, 90, 78], "A right pupil");
    assert_eq!(eye_box(&b, left), [130, 72, 142, 84], "B left pupil");
    assert_eq!(eye_box(&b, right), [50, 72, 62, 84], "B right pupil");
    assert_eq!(eye_box(&c, left), [158, 60, 170, 72], "C left pupil");
    assert_eq!(eye_box(&d, left), [130, 66, 142, 78], "D left pupil");
    assert_eq!(eye_box(&d, right), [78, 66, 90, 78], "D right pupil");
    assert_eq!(eye_box(&f, left), [144, 66, 156, 78], "F left pupil");
    assert_eq!(eye_box(&f, right), [64, 66, 76, 78], "F right pupil");

    assert_px(&a, 164, 72, BLACK, "A pupil centre");
    assert_px(&a, 157, 72, WHITE, "A white left of the pupil");
    assert_px(&a, 169, 72, BLACK, "A pupil inside the white edge");
    assert_px(&a, 171, 72, SKIN, "A nothing sticks out of the white");

    // E: head turned.
    let (lx, ly) = turn90(164, 72);
    assert_px(&e, lx, ly, BLACK, "E left pupil");
    let (rx, ry) = turn90(84, 72);
    assert_px(&e, rx, ry, BLACK, "E right pupil");
    assert_px(&e, 188, 191, SKIN, "E below the turned eye");

    // Invariant: no dark pixel outside the whites (mapped through the turn for E).
    let inside = |x: i32, y: i32, rot: bool| {
        whites.iter().any(|w| {
            if rot {
                let (x0, y0) = turn90(w[0], w[3]);
                let (x1, y1) = turn90(w[2], w[1]);
                (x0 - 1..x1 + 1).contains(&x) && (y0 - 1..y1 + 1).contains(&y)
            } else {
                (w[0] - 1..w[2] + 1).contains(&x) && (w[1] - 1..w[3] + 1).contains(&y)
            }
        })
    };
    for (name, img, rot) in [("A", &a, false), ("B", &b, false), ("C", &c, false), ("D", &d, false), ("E", &e, true), ("F", &f, false)] {
        for y in 0..240 {
            for x in 0..240 {
                if is_dark(px(img, x, y)) {
                    assert!(inside(x, y, rot), "pose {name}: dark pixel ({x},{y}) outside the eye whites");
                }
            }
        }
    }
    golden("eye-gaze-limits", &[&a, &b, &c, &d, &e, &f]);
}
