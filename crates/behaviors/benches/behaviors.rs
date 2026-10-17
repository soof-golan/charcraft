//! Behavior benchmarks: a scripted run of the standard stack over the sample puppets and a synthetic stress rig
//! (many draggable and dangling handles, pointers pulling far away, a restless face).
//!
//! With `BENCH_CHECKSUM=1` each scene prints a checksum of its poses before timing, so that an optimisation can be
//! checked to leave the output unchanged: compare the `checksum` lines of two runs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::print_stderr,
    clippy::missing_const_for_fn,
    clippy::suboptimal_flops,
    clippy::many_single_char_names,
    reason = "benchmark code over synthetic scenes"
)]

use std::cell::OnceCell;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::hint::black_box;

use anim_artwork::ArtNode;
use anim_artwork::synthetic::{rect, tree};
use anim_core::Point;
use charcraft_behaviors::{BehaviorStack, FaceInput, Rig, Script, ScriptEvent, simulate};
use charcraft_project::Project;
use charcraft_puppet::{HandleKind, Puppet};
use criterion::{Criterion, criterion_group, criterion_main};

const FRAMES: u32 = 600;

/// A torso with `groups` independent arms and tails, each with a draggable and a dangling handle.
fn stress_puppet(groups: u32) -> Puppet {
    let mut kids = vec![ArtNode::group("Body", vec![rect("Torso", 300.0, 400.0, 400, 800, [120, 120, 130])])];
    for i in 0..groups {
        let x = 20.0 + f64::from(i % 8) * 120.0;
        let y = 100.0 + f64::from(i / 8) * 150.0;
        kids.push(ArtNode::group(format!("+ Limb {i}"), vec![rect("Limb", x, y, 100, 120, [30, 60, 220])]));
    }
    let mut p = Puppet::from_artwork(&tree(1000, 1400, kids)).unwrap();
    for i in 0..groups {
        let x = 20.0 + f64::from(i % 8) * 120.0;
        let y = 100.0 + f64::from(i / 8) * 150.0;
        let id = p.find_label(&format!("Limb {i}")).unwrap();
        p.add_handle(id, "Fixed", HandleKind::Fixed, Point::new(x + 50.0, y + 5.0)).unwrap();
        p.add_handle(id, "Hand", HandleKind::Draggable, Point::new(x + 50.0, y + 60.0)).unwrap();
        p.add_handle(id, "Tip", HandleKind::Dangle, Point::new(x + 50.0, y + 115.0)).unwrap();
    }
    p
}

/// Faces moving, a voice, and pointers grabbing handles then flinging them far away.
fn script(rig: &Rig) -> Script {
    let mut s = Script::new(30.0, FRAMES);
    for f in (0..FRAMES).step_by(7) {
        let t = f64::from(f);
        s = s.with(ScriptEvent::Face {
            frame: f,
            face: FaceInput { head_roll: 20.0 * (t / 11.0).sin(), mouth_open: (t / 5.0).sin().abs(), ..FaceInput::default() },
        });
    }
    for (n, h) in rig.handles.iter().filter(|h| h.kind == HandleKind::Draggable).take(8).enumerate() {
        let id = u32::try_from(n).unwrap();
        let start = u32::try_from(n * 20).unwrap();
        s = s.with(ScriptEvent::Pointer { frame: start, id, x: h.position.x, y: h.position.y, down: true });
        for k in 1..=12_u32 {
            let t = f64::from(k);
            s = s.with(ScriptEvent::Pointer {
                frame: start + k * 15,
                id,
                x: h.position.x + 900.0 * (t / 2.0).sin(),
                y: h.position.y - 700.0 * (t / 3.0).cos(),
                down: true,
            });
        }
    }
    s
}

fn checksum(stack: &mut BehaviorStack, script: &Script) -> u64 {
    let mut h = DefaultHasher::new();
    let frames = simulate(stack, script, 1).unwrap();
    eprintln!("handles pulled in the last frame: {}", frames.last().map_or(0, |f| f.pose.drag.len()));
    for f in frames {
        format!("{:?}", f.pose).hash(&mut h);
    }
    h.finish()
}

/// Benchmarks a puppet, built only when one of its benchmarks runs (a filtered run is not dominated by the setup of
/// the others).
fn bench_puppet(c: &mut Criterion, name: &str, make: impl Fn() -> Puppet) {
    let puppet = OnceCell::new();
    let puppet = || puppet.get_or_init(&make);
    c.bench_function(&format!("simulate_{FRAMES}_frames_{name}"), |b| {
        let rig = Rig::from_puppet(puppet());
        let script = script(&rig);
        let mut stack = BehaviorStack::standard(rig).with_seed(1);
        if std::env::var_os("BENCH_CHECKSUM").is_some() {
            eprintln!("checksum {name} {:016x}", checksum(&mut stack, &script));
        }
        b.iter(|| black_box(simulate(&mut stack, black_box(&script), 1).unwrap()));
    });
    c.bench_function(&format!("rig_from_puppet_{name}"), |b| {
        let puppet = puppet();
        b.iter(|| black_box(Rig::from_puppet(black_box(puppet))));
    });
}

fn bench_behaviors(c: &mut Criterion) {
    for groups in [8, 32] {
        bench_puppet(c, &format!("stress_{groups}"), || stress_puppet(groups));
    }
    let Some(root) = anim_testkit::samples_dir() else {
        eprintln!("ANICRAFT_SAMPLES is not set: skipping the sample puppets");
        return;
    };
    for file in ["Boy.puppet", "blood_cells.puppet"] {
        let path = root.join(file);
        bench_puppet(c, file.trim_end_matches(".puppet"), || Puppet::from_project(&Project::open(&path).unwrap(), None).unwrap());
    }
}

criterion_group!(benches, bench_behaviors);
criterion_main!(benches);
