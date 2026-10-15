//! Runs the real face models on one image and prints what the tracker would hand to a puppet.
//!
//! ```text
//! cargo run -p charcraft-tracking --features onnx --example probe -- PHOTO.jpg [OVERLAY.png]
//! ```
//!
//! The models are downloaded into the platform cache on first use (see `CHARCRAFT_MODELS_DIR`). The optional
//! overlay shows the 478 landmarks on the photo and a bar per tracked value, a stand-in for the app's live view.

use std::path::PathBuf;

use anim_core::Tick;
use anim_core::num::round_f64_to_u32;
use charcraft_tracking::frame::Frame;
use charcraft_tracking::input::FaceInput;
use charcraft_tracking::landmarks::{Landmarks, idx};
use charcraft_tracking::onnx::OnnxRuntime;
use charcraft_tracking::solver::{FaceSolver, Smoothing};
use charcraft_tracking::source::LandmarkBackend;
use image::{Rgb, RgbImage};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(photo) = args.next().map(PathBuf::from) else {
        return Err("usage: probe PHOTO [OVERLAY.png]".into());
    };
    let overlay = args.next().map(PathBuf::from);

    let picture = image::open(&photo)?.to_rgb8();
    let frame = Frame::new(picture.width(), picture.height(), picture.as_raw().clone())?;

    let runtime = OnnxRuntime::with_default_store()?;
    let mut tracker = runtime.tracker()?;
    let started = std::time::Instant::now();
    let Some(landmarks) = tracker.landmarks(&frame)? else {
        println!("no face found in {}", photo.display());
        return Ok(());
    };
    let first = started.elapsed();
    let started = std::time::Instant::now();
    let again = tracker.landmarks(&frame)?;
    let follow = started.elapsed();

    let mut solver = FaceSolver::default().with_smoothing(Smoothing::off());
    let input = solver.solve(Tick::ZERO, Some(&landmarks)).input;
    println!("{}", serde_json::to_string_pretty(&input)?);
    eprintln!(
        "{}x{}  score {:.3}  first frame {:.0} ms (detector + mesh x2), followed frame {:.0} ms{}",
        frame.width(),
        frame.height(),
        landmarks.score,
        first.as_secs_f64() * 1000.0,
        follow.as_secs_f64() * 1000.0,
        if again.is_some() { "" } else { " (lost)" }
    );

    if let Some(path) = overlay {
        draw(picture, &landmarks, &input).save(&path)?;
        eprintln!("overlay written to {}", path.display());
    }
    Ok(())
}

fn dot(img: &mut RgbImage, x: f32, y: f32, radius: i32, colour: Rgb<u8>) {
    for dy in radius.saturating_neg()..=radius {
        for dx in radius.saturating_neg()..=radius {
            if dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy)) > radius.saturating_mul(radius) {
                continue;
            }
            let (px, py) = (f64::from(x) + f64::from(dx), f64::from(y) + f64::from(dy));
            if px < 0.0 || py < 0.0 {
                continue;
            }
            if let Some(p) = img.get_pixel_mut_checked(round_f64_to_u32(px), round_f64_to_u32(py)) {
                *p = colour;
            }
        }
    }
}

fn bar(img: &mut RgbImage, row: u32, value: f32, signed: bool, colour: Rgb<u8>) {
    let (x0, width, height) = (img.width().saturating_sub(190), 170_u32, 14_u32);
    let y0 = 12_u32.saturating_add(row.saturating_mul(22));
    for y in y0..y0.saturating_add(height) {
        for x in x0..x0.saturating_add(width) {
            if let Some(p) = img.get_pixel_mut_checked(x, y) {
                *p = Rgb([30, 30, 36]);
            }
        }
    }
    let unit = f64::from(value.clamp(if signed { -1.0 } else { 0.0 }, 1.0));
    let (from, to) = if signed { (f64::from(width) / 2.0, f64::from(width) / 2.0 + unit * f64::from(width) / 2.0) } else { (0.0, unit * f64::from(width)) };
    let (lo, hi) = (from.min(to), from.max(to));
    for y in y0.saturating_add(2)..y0.saturating_add(height).saturating_sub(2) {
        for x in round_f64_to_u32(lo)..round_f64_to_u32(hi) {
            if let Some(p) = img.get_pixel_mut_checked(x0.saturating_add(x), y) {
                *p = colour;
            }
        }
    }
}

fn draw(mut img: RgbImage, l: &Landmarks, input: &FaceInput) -> RgbImage {
    let scale = i32::from(img.width().max(img.height()) > 700);
    for (i, p) in l.points.iter().enumerate() {
        let key = [idx::MOUTH_L, idx::MOUTH_R, idx::LIP_UPPER_INNER, idx::LIP_LOWER_INNER, idx::NOSE_TIP].contains(&i);
        let colour = if i >= 468 {
            Rgb([0, 230, 255])
        } else if key {
            Rgb([255, 80, 60])
        } else {
            Rgb([120, 255, 120])
        };
        dot(&mut img, p.x(), p.y(), if key || i >= 468 { scale.saturating_add(2) } else { scale }, colour);
    }
    let green = Rgb([90, 220, 120]);
    let amber = Rgb([255, 190, 60]);
    let rows = [
        (input.brow_l, true, amber),
        (input.brow_r, true, amber),
        (input.eye_open_l, false, green),
        (input.eye_open_r, false, green),
        (input.gaze[0], true, Rgb([120, 180, 255])),
        (input.gaze[1], true, Rgb([120, 180, 255])),
        (input.mouth_open, false, Rgb([255, 120, 120])),
        (input.mouth_wide, true, Rgb([255, 120, 120])),
        (input.smile, true, Rgb([255, 120, 200])),
        (input.head.yaw / 0.8, true, Rgb([200, 200, 255])),
        (input.head.pitch / 0.8, true, Rgb([200, 200, 255])),
        (input.head.roll / 0.8, true, Rgb([200, 200, 255])),
    ];
    for (row, (value, signed, colour)) in (0_u32..).zip(rows) {
        bar(&mut img, row, value, signed, colour);
    }
    img
}
