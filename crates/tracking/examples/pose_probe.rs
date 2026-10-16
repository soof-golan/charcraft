//! Runs the real body pose model on one image and prints the 17 keypoints and what the tracker would hand to a
//! puppet, with an optional overlay.
//!
//! ```text
//! cargo run -p charcraft-tracking --features onnx --example pose_probe -- PHOTO.jpg [OVERLAY.png]
//! ```
//!
//! The model is downloaded into the platform cache on first use (see `CHARCRAFT_MODELS_DIR`).

use std::path::PathBuf;

use anim_core::Tick;
use anim_core::num::round_f64_to_u32;
use charcraft_tracking::body::{BodyBackend, BodySolver, Joint};
use charcraft_tracking::frame::Frame;
use charcraft_tracking::onnx::OnnxRuntime;
use image::{Rgb, RgbImage};

/// The bones drawn on the overlay.
const BONES: [(Joint, Joint); 12] = [
    (Joint::LeftShoulder, Joint::RightShoulder),
    (Joint::LeftShoulder, Joint::LeftElbow),
    (Joint::LeftElbow, Joint::LeftWrist),
    (Joint::RightShoulder, Joint::RightElbow),
    (Joint::RightElbow, Joint::RightWrist),
    (Joint::LeftShoulder, Joint::LeftHip),
    (Joint::RightShoulder, Joint::RightHip),
    (Joint::LeftHip, Joint::RightHip),
    (Joint::LeftHip, Joint::LeftKnee),
    (Joint::LeftKnee, Joint::LeftAnkle),
    (Joint::RightHip, Joint::RightKnee),
    (Joint::RightKnee, Joint::RightAnkle),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(photo) = args.next().map(PathBuf::from) else {
        return Err("usage: pose_probe PHOTO [OVERLAY.png]".into());
    };
    let overlay = args.next().map(PathBuf::from);

    let picture = image::open(&photo)?.to_rgb8();
    let frame = Frame::new(picture.width(), picture.height(), picture.as_raw().clone())?;
    let runtime = OnnxRuntime::with_default_store()?;
    let mut tracker = runtime.body_tracker()?;
    let started = std::time::Instant::now();
    let Some(keypoints) = tracker.keypoints(&frame)? else {
        println!("no body found in {}", photo.display());
        return Ok(());
    };
    let first = started.elapsed();
    let started = std::time::Instant::now();
    let _ = tracker.keypoints(&frame)?;
    eprintln!(
        "{}x{}  first frame {:.0} ms, followed frame {:.0} ms",
        frame.width(),
        frame.height(),
        first.as_secs_f64() * 1e3,
        started.elapsed().as_secs_f64() * 1e3
    );
    for joint in Joint::ALL {
        if let Some(k) = keypoints.get(joint) {
            println!("{joint:?}: x {:.3} y {:.3} score {:.2}", k.x, k.y, k.score);
        }
    }
    let mut solver = BodySolver::default();
    let body = solver.solve(Tick::ZERO, Some(&keypoints)).input;
    println!("{}", serde_json::to_string_pretty(&body)?);

    if let Some(path) = overlay {
        let mut canvas: RgbImage = picture;
        let (w, h) = (f64::from(canvas.width()), f64::from(canvas.height()));
        let at = |j: Joint| keypoints.seen(j, 0.3).map(|k| (f64::from(k.x) * w, f64::from(k.y) * h));
        let mut dot = |x: f64, y: f64, colour: [u8; 3]| {
            for dy in -3_i32..=3 {
                for dx in -3_i32..=3 {
                    let (px, py) = (round_f64_to_u32(x) as i64 + i64::from(dx), round_f64_to_u32(y) as i64 + i64::from(dy));
                    if let (Ok(px), Ok(py)) = (u32::try_from(px), u32::try_from(py)) {
                        if px < canvas.width() && py < canvas.height() {
                            canvas.put_pixel(px, py, Rgb(colour));
                        }
                    }
                }
            }
        };
        for (a, b) in BONES {
            if let (Some((x0, y0)), Some((x1, y1))) = (at(a), at(b)) {
                for i in 0..=40 {
                    let t = f64::from(i) / 40.0;
                    dot(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, [255, 220, 0]);
                }
            }
        }
        for j in Joint::ALL {
            if let Some((x, y)) = at(j) {
                dot(x, y, [255, 40, 40]);
            }
        }
        canvas.save(path)?;
    }
    Ok(())
}
