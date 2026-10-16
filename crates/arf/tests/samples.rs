//! The recordings of the `news anchors` sample project, byte for byte. Skipped without `ANICRAFT_SAMPLES`.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::suboptimal_flops, reason = "integration test")]

use std::path::{Path, PathBuf};

use charcraft_arf::{EventGraph, Role};

fn project() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("ANICRAFT_SAMPLES")?).join("chproj_full").join("news anchors");
    dir.exists().then_some(dir)
}

fn arf_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = read.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            arf_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "arf") {
            out.push(path);
        }
    }
}

fn all() -> Vec<PathBuf> {
    let Some(dir) = project() else { return Vec::new() };
    let mut files = Vec::new();
    arf_files(&dir, &mut files);
    files
}

#[test]
fn every_recording_reads_and_writes_back_byte_for_byte() {
    let files = all();
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        let graph = EventGraph::from_bytes(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(graph.to_bytes(), bytes, "{} does not write back exactly", path.display());
    }
    println!("{} recordings round-trip", files.len());
}

#[test]
fn recorded_channels_have_a_known_role_and_curves_are_in_time_order() {
    for path in all() {
        let graph = EventGraph::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
        for track in graph.tracks() {
            assert_ne!(track.role, Role::Other, "{}: unknown channel {}", path.display(), track.name);
            let mut last = f64::NEG_INFINITY;
            for s in &track.curve.segments {
                assert!(s.start() >= last, "{}: {} goes back in time", path.display(), track.name);
                assert!(s.t.windows(2).all(|w| w[0] <= w[1]), "{}: {} has a segment that runs backwards", path.display(), track.name);
                last = s.start();
            }
        }
    }
}

#[test]
fn frames_are_quarter_seconds_from_the_start() {
    for path in all() {
        let graph = EventGraph::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
        for (k, frame) in (0_u32..).zip(&graph.frames) {
            let expect = graph.start + f64::from(k) * charcraft_arf::FRAME_SECONDS;
            assert!((frame.start - expect).abs() < 1e-6, "{}: frame {k} starts at {} not {expect}", path.display(), frame.start);
            assert!(frame.end >= frame.start && frame.end - frame.start <= charcraft_arf::FRAME_SECONDS + 1e-9);
        }
        if let Some(last) = graph.frames.last() {
            assert!((last.end - graph.end).abs() < 1e-6, "{}: the last frame ends at the end of the take", path.display());
        }
    }
}

#[test]
fn rebuilding_a_graph_from_its_curves_keeps_every_value() {
    let Some(dir) = project() else { return };
    let path = dir.join("Ch Media/Recordings/2024-07-10/take 02_00_05 pm.arf");
    let graph = EventGraph::from_bytes(&std::fs::read(path).unwrap()).unwrap();
    let tracks = graph.tracks();
    let channels: Vec<(String, charcraft_arf::Curve)> = tracks.iter().map(|t| (t.name.clone(), t.curve.clone())).collect();
    let rebuilt = EventGraph::from_curves(graph.start, graph.end, &channels);
    let again = EventGraph::from_bytes(&rebuilt.to_bytes()).unwrap().tracks();
    for (a, b) in tracks.iter().zip(&again) {
        let n = 400;
        for i in 0..=n {
            let t = graph.start + (graph.end - graph.start) * f64::from(i) / f64::from(n);
            let (x, y) = (a.curve.value_at(t).unwrap(), b.curve.value_at(t).unwrap());
            assert!((x - y).abs() < 1e-5 * (1.0 + x.abs()), "{} at {t}: {x} vs {y}", a.name);
        }
    }
}
