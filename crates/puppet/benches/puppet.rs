//! Puppet benchmarks: building and evaluating the sample puppets and a synthetic stress puppet (the hardest
//! verification fixtures, scaled up: a welded head on a meshed body, arms with handles, nested warps).
//!
//! With `BENCH_CHECKSUM=1` each scene prints a checksum of its display lists before timing, so that an optimisation
//! can be checked to leave the output unchanged: compare the `checksum` lines of two runs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::missing_const_for_fn,
    clippy::suboptimal_flops,
    clippy::many_single_char_names,
    reason = "benchmark code over synthetic scenes"
)]

use std::hash::{DefaultHasher, Hash, Hasher};
use std::hint::black_box;
use std::path::PathBuf;

use anim_artwork::{ArtNode, ArtworkTree};
use anim_core::{Point, Size, Vec2};
use anim_display::Image;
use charcraft_project::Project;
use charcraft_puppet::{HandleKind, Puppet, PuppetPose, Viseme, Xform};
use criterion::{Criterion, criterion_group, criterion_main};

fn rect(name: &str, x: f64, y: f64, w: u32, h: u32, rgb: [u8; 3]) -> ArtNode {
    let n = usize::try_from(w * h).unwrap();
    let px: Vec<u8> = [rgb[0], rgb[1], rgb[2], 255].iter().copied().cycle().take(n * 4).collect();
    ArtNode::raster(name, Image::new(w, h, px).unwrap(), Point::new(x, y))
}

fn hidden(mut node: ArtNode) -> ArtNode {
    node.visible = false;
    node
}

fn eye(side: &str, x: f64) -> ArtNode {
    ArtNode::group(
        format!("{side} Eye"),
        vec![
            rect("White", x, 160.0, 120, 70, [255, 255, 255]),
            rect(&format!("+ {side} Pupil"), x + 40.0, 175.0, 36, 36, [10, 10, 10]),
            hidden(rect(&format!("{side} Blink"), x - 10.0, 190.0, 140, 12, [30, 60, 220])),
        ],
    )
}

fn mouth() -> ArtNode {
    let shapes = Viseme::ALL
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let node = rect(v.label(), 300.0, 300.0, 120, 60, [20 + u8::try_from(i).unwrap() * 12, 90, 60]);
            if *v == Viseme::Neutral { node } else { hidden(node) }
        })
        .collect();
    ArtNode::group("mouth", shapes)
}

/// A 1000x1400 puppet: a torso with a neck under a welded head (eyes, brows, mouth) and `arms` arm groups
/// that overlap the torso and carry handles.
fn stress_tree(arms: u32) -> ArtworkTree {
    let head = ArtNode::group(
        "Head",
        vec![
            rect("Face", 250.0, 40.0, 500, 420, [240, 190, 160]),
            eye("Left", 520.0),
            eye("Right", 330.0),
            rect("+Left Eyebrow", 520.0, 110.0, 120, 24, [60, 40, 30]),
            rect("+ right eyebrow", 330.0, 110.0, 120, 24, [60, 40, 30]),
            mouth(),
        ],
    );
    let mut body = vec![rect("Torso", 300.0, 480.0, 400, 700, [120, 120, 130]), rect("Neck", 440.0, 420.0, 120, 120, [220, 170, 140])];
    for i in 0..arms {
        let side = if i % 2 == 0 { 0.0 } else { 600.0 };
        let y = 520.0 + f64::from(i / 2) * 130.0;
        body.push(ArtNode::group(
            format!("+ Arm {i}"),
            vec![rect("Sleeve", 200.0 + side, y, 200, 110, [30, 60, 220]), rect("Hand", 150.0 + side, y + 20.0, 70, 70, [240, 190, 160])],
        ));
    }
    let body = ArtNode::group("Body", body);
    ArtworkTree { size: Size::new(1000.0, 1400.0), root: ArtNode::group("", vec![body, head]), warnings: Vec::new() }
}

fn stress_puppet(arms: u32) -> Puppet {
    let mut p = Puppet::from_artwork(&stress_tree(arms)).unwrap();
    for i in 0..arms {
        let id = p.find_label(&format!("Arm {i}")).unwrap();
        let side = if i % 2 == 0 { 0.0 } else { 600.0 };
        let y = 520.0 + f64::from(i / 2) * 130.0;
        p.add_handle(id, "Fixed", HandleKind::Fixed, Point::new(300.0 + side, y + 55.0)).unwrap();
        p.add_handle(id, "Hand", HandleKind::Draggable, Point::new(185.0 + side, y + 55.0)).unwrap();
    }
    p
}

fn extreme_pose(p: &Puppet) -> PuppetPose {
    let mut pose = PuppetPose::neutral().with_viseme(Viseme::Ah).with_blink(1.0);
    pose.head = Xform { translation: Vec2::new(60.0, -50.0), rotation: 40.0_f64.to_radians(), scale: Vec2::new(1.2, 0.8) };
    pose.gaze = Vec2::new(1.0, -1.0);
    pose.eyebrow_left = 1.0;
    pose.breathe = 1.0;
    for layer in p.layers().iter().filter(|l| l.independent) {
        for h in layer.handles.iter().filter(|h| h.kind == HandleKind::Draggable) {
            pose.drag.insert(h.key.clone(), Vec2::new(-120.0, 90.0));
        }
    }
    pose
}

fn checksum(p: &Puppet, pose: &PuppetPose) -> u64 {
    let mut h = DefaultHasher::new();
    format!("{:?}", p.evaluate(pose)).hash(&mut h);
    h.finish()
}

fn bench_stress(c: &mut Criterion) {
    let checks = std::env::var_os("BENCH_CHECKSUM").is_some();
    let mut g = c.benchmark_group("puppet_stress");
    g.sample_size(20);
    for arms in [2, 8] {
        let tree = stress_tree(arms);
        g.bench_function(format!("build_{arms}_arms"), |b| b.iter(|| black_box(Puppet::from_artwork(black_box(&tree)).unwrap())));
        let p = stress_puppet(arms);
        let extreme = extreme_pose(&p);
        let mut moved = PuppetPose::neutral();
        moved.head = Xform::moved(0.0, -40.0);
        if checks {
            eprintln!("checksum stress_{arms}_extreme {:016x}", checksum(&p, &extreme));
            eprintln!("checksum stress_{arms}_head {:016x}", checksum(&p, &moved));
        }
        g.bench_function(format!("evaluate_neutral_{arms}_arms"), |b| b.iter(|| black_box(p.evaluate(black_box(&PuppetPose::neutral())))));
        g.bench_function(format!("evaluate_head_moved_{arms}_arms"), |b| b.iter(|| black_box(p.evaluate(black_box(&moved)))));
        g.bench_function(format!("evaluate_extreme_{arms}_arms"), |b| b.iter(|| black_box(p.evaluate(black_box(&extreme)))));
        g.bench_function(format!("visibility_{arms}_arms"), |b| b.iter(|| black_box(p.visibility(black_box(&extreme)))));
    }
    g.finish();
}

fn bench_samples(c: &mut Criterion) {
    let Some(root) = std::env::var_os("ANICRAFT_SAMPLES").map(PathBuf::from) else {
        eprintln!("ANICRAFT_SAMPLES is not set: skipping the sample puppets");
        return;
    };
    let checks = std::env::var_os("BENCH_CHECKSUM").is_some();
    let mut g = c.benchmark_group("puppet_samples");
    g.sample_size(20);
    for file in ["Boy.puppet", "blood_cells.puppet"] {
        let name = file.trim_end_matches(".puppet");
        let Ok(project) = Project::open(root.join(file)) else { continue };
        g.bench_function(format!("{name}_build"), |b| b.iter(|| black_box(Puppet::from_project(black_box(&project), None).unwrap())));
        let p = Puppet::from_project(&project, None).unwrap();
        let extreme = extreme_pose(&p);
        let tilt = PuppetPose::neutral().with_viseme(Viseme::Oh).with_head_rotation(15.0);
        if checks {
            eprintln!("checksum {name}_extreme {:016x}", checksum(&p, &extreme));
            eprintln!("checksum {name}_tilt {:016x}", checksum(&p, &tilt));
        }
        g.bench_function(format!("{name}_evaluate_neutral"), |b| b.iter(|| black_box(p.evaluate(black_box(&PuppetPose::neutral())))));
        g.bench_function(format!("{name}_evaluate_tilt"), |b| b.iter(|| black_box(p.evaluate(black_box(&tilt)))));
        g.bench_function(format!("{name}_evaluate_extreme"), |b| b.iter(|| black_box(p.evaluate(black_box(&extreme)))));
    }
    g.finish();
}

criterion_group!(benches, bench_stress, bench_samples);
criterion_main!(benches);
