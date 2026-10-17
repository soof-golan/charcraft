//! Runs the real binary. Tests that need Adobe sample files read `ANICRAFT_SAMPLES` and return when it is unset.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "test code"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anim_testkit::samples_dir as samples;

const SVG_PUPPET: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200" viewBox="0 0 200 200">
  <g id="Head">
    <circle id="face" cx="100" cy="90" r="70" fill="#f5c9a5"/>
    <g id="Left Eye"><circle cx="75" cy="75" r="12" fill="#fff"/><circle id="Left Pupil" cx="75" cy="75" r="5" fill="#000"/></g>
    <g id="Right Eye"><circle cx="125" cy="75" r="12" fill="#fff"/><circle id="Right Pupil" cx="125" cy="75" r="5" fill="#000"/></g>
    <g id="Mouth">
      <ellipse id="Neutral" cx="100" cy="125" rx="20" ry="3" fill="#a33"/>
      <ellipse id="Ah" cx="100" cy="128" rx="18" ry="14" fill="#a33"/>
      <ellipse id="Oh" cx="100" cy="128" rx="9" ry="11" fill="#a33"/>
      <ellipse id="Ee" cx="100" cy="125" rx="24" ry="5" fill="#a33"/>
      <ellipse id="S" cx="100" cy="125" rx="16" ry="7" fill="#a33"/>
      <ellipse id="F" cx="100" cy="126" rx="14" ry="4" fill="#a33"/>
      <ellipse id="D" cx="100" cy="127" rx="12" ry="9" fill="#a33"/>
    </g>
  </g>
</svg>
"##;

fn cli(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_charcraft-cli")).args(args).output().unwrap()
}

fn run<const N: usize>(args: [&dyn AsRef<std::ffi::OsStr>; N]) -> Output {
    let os: Vec<&std::ffi::OsStr> = args.iter().map(|a| (**a).as_ref()).collect();
    cli(&os)
}

fn ok(out: &Output) {
    assert!(out.status.success(), "stdout: {}\nstderr: {}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

/// Where sample renders go for the owner to look at: `$ANICRAFT_VISUAL_OUT/charcraft-cli`, else the temp folder (the
/// tests read the files back, so they need somewhere to write).
fn scratch() -> PathBuf {
    let dir = std::env::var_os("ANICRAFT_VISUAL_OUT")
        .map_or_else(|| std::env::temp_dir().join("charcraft-cli-tests"), |root| PathBuf::from(root).join("charcraft-cli"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Png {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn read_png(path: &Path) -> Png {
    let png = anim_testkit::read_png(path).unwrap();
    Png { width: png.width, height: png.height, rgba: png.rgba }
}

/// Fraction of pixels that are not plain white or fully transparent.
fn inked(png: &Png) -> f64 {
    let total = png.rgba.len() / 4;
    let ink = png.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] > 0 && !(p[0] == 255 && p[1] == 255 && p[2] == 255)).count();
    ink as f64 / total as f64
}

fn wav_bytes(rate: u32, samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// A second of silence, a second of loud hiss (an "s"), a second of silence.
fn hiss_wav(dir: &Path) -> PathBuf {
    let rate = 16_000_u32;
    let mut state = 12_345_u32;
    let mut samples = vec![0_i16; rate as usize];
    for _ in 0..rate {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        samples.push(i16::try_from((i32::try_from(state >> 16).unwrap() - 32_768) / 2).unwrap());
    }
    samples.extend(std::iter::repeat_n(0, rate as usize));
    let path = dir.join("hiss.wav");
    std::fs::write(&path, wav_bytes(rate, &samples)).unwrap();
    path
}

fn svg_puppet(dir: &Path) -> PathBuf {
    let path = dir.join("toy.svg");
    std::fs::write(&path, SVG_PUPPET).unwrap();
    path
}

#[test]
fn render_draws_the_pose_and_the_mouth_shape_changes_it() {
    let dir = tempfile::tempdir().unwrap();
    let svg = svg_puppet(dir.path());
    let (neutral, oh) = (dir.path().join("neutral.png"), dir.path().join("oh.png"));
    ok(&run([&"render", &svg, &"-o", &neutral]));
    ok(&run([&"render", &svg, &"--viseme", &"Oh", &"-o", &oh]));
    let (a, b) = (read_png(&neutral), read_png(&oh));
    assert_eq!((a.width, a.height), (200, 200));
    assert!(inked(&a) > 0.2);
    assert_ne!(a.rgba, b.rgba, "the Oh mouth should differ from the neutral one");
}

#[test]
fn render_reads_a_pose_file_and_scales() {
    let dir = tempfile::tempdir().unwrap();
    let svg = svg_puppet(dir.path());
    let pose = dir.path().join("pose.json");
    std::fs::write(&pose, r#"{"viseme":"Ah","blink_left":1.0}"#).unwrap();
    let out = dir.path().join("posed.png");
    ok(&run([&"render", &svg, &"--pose", &pose, &"--scale", &"2", &"-o", &out]));
    let png = read_png(&out);
    assert_eq!((png.width, png.height), (400, 400));
}

#[test]
fn render_transparent_leaves_the_corner_clear() {
    let dir = tempfile::tempdir().unwrap();
    let svg = svg_puppet(dir.path());
    let out = dir.path().join("t.png");
    ok(&run([&"render", &svg, &"--transparent", &"-o", &out]));
    let png = read_png(&out);
    assert_eq!(png.rgba[3], 0, "the top-left pixel should be transparent");
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let svg = svg_puppet(dir.path());
    let out = dir.path().join("x.png");
    assert!(!run([&"render", &dir.path().join("missing.psd"), &"-o", &out]).status.success());
    assert!(!run([&"render", &svg, &"--viseme", &"Banana", &"-o", &out]).status.success());
    assert!(!run([&"render", &svg, &"--scale", &"-1", &"-o", &out]).status.success());
    let junk = dir.path().join("junk.puppet");
    std::fs::write(&junk, b"definitely not a zip").unwrap();
    let result = run([&"info", &junk]);
    assert!(!result.status.success());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("panicked"));
    let notes = dir.path().join("notes.txt");
    std::fs::write(&notes, b"hello").unwrap();
    assert!(!run([&"info", &notes]).status.success());
}

#[test]
fn info_lists_the_rig_as_text_and_json() {
    let dir = tempfile::tempdir().unwrap();
    let svg = svg_puppet(dir.path());
    let text = run([&"info", &svg, &"--outline"]);
    ok(&text);
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("mouth shapes: Neutral, Ah, D, Ee, F, Oh, S"), "{text}");
    assert!(text.contains("Left Eye"), "{text}");
    let json = run([&"info", &svg, &"--json"]);
    ok(&json);
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["kind"], "artwork");
    assert_eq!(value["puppet"]["layers"], 17);
    assert_eq!(value["puppet"]["mouth_groups"][0], "Head/Mouth");
}

#[test]
fn lipsync_writes_keys_that_follow_the_sound() {
    let dir = tempfile::tempdir().unwrap();
    let wav = hiss_wav(dir.path());
    let out = dir.path().join("visemes.json");
    ok(&run([&"lipsync", &wav, &"-o", &out]));
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(value["sample_rate"], 16_000);
    assert!((value["duration_seconds"].as_f64().unwrap() - 3.0).abs() < 0.01);
    let keys = value["keys"].as_array().unwrap();
    assert_eq!(keys[0]["viseme"], "Neutral");
    assert_eq!(keys[0]["time_seconds"], 0.0);
    let spoken: Vec<_> = keys.iter().filter(|k| k["viseme"] != "Neutral").collect();
    assert!(!spoken.is_empty(), "the hiss should make a mouth shape: {keys:?}");
    // Everything spoken happens during the second second.
    for k in spoken {
        let t = k["time_seconds"].as_f64().unwrap();
        assert!((0.9..2.2).contains(&t), "{k}");
    }
    assert_eq!(keys.last().unwrap()["viseme"], "Neutral");
}

#[test]
fn animate_writes_an_mp4_with_sound() {
    let dir = tempfile::tempdir().unwrap();
    let svg = svg_puppet(dir.path());
    let wav = hiss_wav(dir.path());
    let out = dir.path().join("talk.mp4");
    let result = run([&"animate", &svg, &"--audio", &wav, &"--fps", &"12", &"-o", &out]);
    ok(&result);
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(&bytes[4..8], b"ftyp");
    assert!(bytes.len() > 2_000);
    let said = String::from_utf8_lossy(&result.stdout);
    assert!(said.contains("36 frames") && said.contains("200x200") && said.contains("audio yes"), "{said}");
}

#[test]
fn animate_to_a_png_sequence_moves_the_mouth() {
    let dir = tempfile::tempdir().unwrap();
    let svg = svg_puppet(dir.path());
    let wav = hiss_wav(dir.path());
    let out = dir.path().join("frames");
    ok(&run([&"animate", &svg, &"--audio", &wav, &"--fps", &"10", &"--no-blink", &"--no-breathe", &"-o", &out]));
    let frames: Vec<_> = (1..=30).map(|n| read_png(&out.join(format!("frame_{n:05}.png")))).collect();
    // Silence at the start, hiss in the middle: the picture changes.
    assert_ne!(frames[0].rgba, frames[15].rgba);
    assert_eq!(frames[0].rgba, frames[2].rgba);
}

#[test]
fn animate_is_deterministic_for_a_seed() {
    let dir = tempfile::tempdir().unwrap();
    let svg = svg_puppet(dir.path());
    let wav = hiss_wav(dir.path());
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    for (out, seed) in [(&a, "3"), (&b, "3")] {
        ok(&run([&"animate", &svg, &"--audio", &wav, &"--fps", &"8", &"--seed", &seed, &"-o", out]));
    }
    for n in 1..=24 {
        let name = format!("frame_{n:05}.png");
        assert_eq!(std::fs::read(a.join(&name)).unwrap(), std::fs::read(b.join(&name)).unwrap(), "{name}");
    }
}

#[test]
fn convert_refuses_what_is_not_a_puppet() {
    let dir = tempfile::tempdir().unwrap();
    let svg = svg_puppet(dir.path());
    assert!(!run([&"convert", &svg, &dir.path().join("x.puppet")]).status.success());
}

// ---- sample-gated -------------------------------------------------------------------------------------------------

#[test]
fn sample_puppets_render_and_look_right() {
    let Some(samples) = samples() else { return };
    let out = scratch();
    for name in ["Boy", "blood_cells"] {
        let input = samples.join(format!("{name}.puppet"));
        let neutral = out.join(format!("{name}_neutral.png"));
        ok(&run([&"render", &input, &"-o", &neutral]));
        let png = read_png(&neutral);
        assert_eq!((png.width, png.height), (1080, 1080), "{name}");
        // A standing character: some ink, but most of the canvas is background.
        let ink = inked(&png);
        assert!((0.05..0.6).contains(&ink), "{name} ink fraction {ink}");
        let ah = out.join(format!("{name}_ah_blink.png"));
        let pose = out.join(format!("{name}_pose.json"));
        std::fs::write(&pose, r#"{"viseme":"Ah","blink_left":1.0,"blink_right":1.0}"#).unwrap();
        ok(&run([&"render", &input, &"--pose", &pose, &"-o", &ah]));
        assert_ne!(read_png(&ah).rgba, png.rgba, "{name}: Ah plus blink should differ from neutral");
    }
}

#[test]
fn sample_puppets_convert_and_keep_their_rig() {
    let Some(samples) = samples() else { return };
    let dir = tempfile::tempdir().unwrap();
    for name in ["Boy", "blood_cells"] {
        let input = samples.join(format!("{name}.puppet"));
        let copy = dir.path().join(format!("{name}_copy.puppet"));
        ok(&run([&"convert", &input, &copy]));
        let again = dir.path().join(format!("{name}_again.puppet"));
        ok(&run([&"convert", &copy, &again]));
        let summary = |p: &Path| -> serde_json::Value {
            let out = run([&"info", &p, &"--json"]);
            ok(&out);
            let mut v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            // The path differs by design.
            v["project"]["path"] = serde_json::Value::Null;
            v
        };
        let (first, second, third) = (summary(&input), summary(&copy), summary(&again));
        assert_eq!(first["puppet"], second["puppet"], "{name}: rig changed by conversion");
        assert_eq!(first["project"]["puppets"], second["project"]["puppets"], "{name}");
        assert_eq!(second, third, "{name}: converting twice changed something");
        // And the copy still draws the same picture.
        let (a, b) = (dir.path().join("a.png"), dir.path().join("b.png"));
        ok(&run([&"render", &input, &"-o", &a]));
        ok(&run([&"render", &copy, &"-o", &b]));
        assert_eq!(read_png(&a).rgba, read_png(&b).rgba, "{name}: the converted puppet renders differently");
    }
}

#[test]
fn sample_boy_talks_to_the_news_audio() {
    let Some(samples) = samples() else { return };
    let audio = samples.join("chproj_full/news anchors/Ch Media/Gathered Media/news_song_vocals_mixdown_short.wav");
    if !audio.is_file() {
        return;
    }
    let out = scratch().join("boy_talk.mp4");
    let result = run([&"animate", &samples.join("Boy.puppet"), &"--audio", &audio, &"--seconds", &"3", &"--scale", &"0.5", &"-o", &out]);
    ok(&result);
    let said = String::from_utf8_lossy(&result.stdout);
    assert!(said.contains("90 frames") && said.contains("540x540") && said.contains("audio yes"), "{said}");
    let keys = scratch().join("news_visemes.json");
    ok(&run([&"lipsync", &audio, &"-o", &keys]));
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&keys).unwrap()).unwrap();
    assert!(value["keys"].as_array().unwrap().len() > 100);
}

#[test]
fn sample_news_scene_exports_to_png_and_mp4() {
    let Some(samples) = samples() else { return };
    let project = samples.join("chproj_full/news anchors/news anchors.chproj");
    if !project.is_file() {
        return;
    }
    let out = scratch();
    let frames = out.join("news_frames");
    let _ = std::fs::remove_dir_all(&frames);
    let result = run([&"export", &project, &"--scene", &"anchors_01", &"--start", &"12", &"--seconds", &"0.25", &"--scale", &"0.5", &"-o", &frames]);
    ok(&result);
    let first = read_png(&frames.join("frame_00001.png"));
    assert_eq!((first.width, first.height), (960, 540));
    assert_eq!(first.rgba[3], 0, "the corner of the stage is empty");
    assert!(inked(&first) > 0.2, "the anchors and the desk should cover much of the frame");
    assert!(frames.join("frame_00006.png").is_file());
    assert!(!frames.join("frame_00007.png").is_file());

    let video = out.join("news_scene.mp4");
    let result = run([&"export", &project, &"--scene", &"anchors_01", &"--start", &"12", &"--seconds", &"1", &"--scale", &"0.5", &"-o", &video]);
    ok(&result);
    let said = String::from_utf8_lossy(&result.stdout);
    assert!(said.contains("24 frames") && said.contains("960x540") && said.contains("audio yes"), "{said}");
    assert!(!run([&"export", &project, &"--scene", &"no such scene", &"-o", &video]).status.success());
}
