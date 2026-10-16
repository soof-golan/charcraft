//! Predict, render, inspect: speech to mouth over the frame clock, takes and blends, punch-in audio, the mix and
//! determinism. Predictions are in `tests/fixtures/visual/*.expect.md`.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::indexing_slicing,
    clippy::many_single_char_names,
    clippy::missing_const_for_fn,
    clippy::panic,
    clippy::similar_names,
    clippy::too_many_lines,
    clippy::unwrap_used,
    clippy::needless_range_loop,
    clippy::iter_on_single_items,
    clippy::assert_is_empty,
    clippy::suboptimal_flops,
    clippy::default_trait_access,
    reason = "test code"
)]

use std::collections::BTreeSet;
use std::io::BufWriter;
use std::path::PathBuf;

use anim_artwork::{ArtNode, ArtworkTree};
use anim_audio::{MixFormat, PcmBuffer};
use anim_core::{FrameRate, Point, Size, TICKS_PER_SECOND, Tick, TimeRange, Vec2};
use anim_display::Image;
use anim_export::{FrameSource, RasterOptions, RgbaFrame, render_frame};
use anim_lipsync::{VisemeKey, analyze, viseme_at};
use charcraft_behaviors::api::StackOp;
use charcraft_behaviors::{FaceInput, Inputs, Script, ScriptEvent};
use charcraft_puppet::{Puppet, Viseme as PuppetViseme};
use charcraft_record::{
    AudioClip, Cast, CastMember, Evaluator, Frame, MediaStore, ParamGroup, PuppetTrack, RecordDoc, Recorder, Sample, Scene, SceneFrames, Take, mix_scene,
    scene_soundtrack, store, track_inputs,
};

const SIZE: u32 = 200;
const SKIN: [u8; 3] = [240, 190, 160];

// ---- output helpers --------------------------------------------------------------------------------------

fn scratch() -> Option<PathBuf> {
    std::env::var_os("ANICRAFT_VISUAL_OUT").map(|root| PathBuf::from(root).join("charcraft-record"))
}

fn write_png(name: &str, w: u32, h: u32, rgba: &[u8]) {
    let Some(dir) = scratch() else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(dir.join(format!("{name}.png"))).unwrap();
    let mut encoder = png::Encoder::new(BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(rgba).unwrap();
}

/// Frames side by side, `columns` per row, on a light grey ground.
fn save_sheet(name: &str, frames: &[&RgbaFrame], columns: usize) {
    let (tw, th) = (frames[0].width as usize, frames[0].height as usize);
    let rows = frames.len().div_ceil(columns);
    let (w, h) = (tw * columns, th * rows);
    let mut out = vec![200_u8; w * h * 4];
    for (i, f) in frames.iter().enumerate() {
        let (ox, oy) = ((i % columns) * tw, (i / columns) * th);
        for y in 0..th {
            let at = ((oy + y) * w + ox) * 4;
            out[at..at + tw * 4].copy_from_slice(&f.rgba[y * tw * 4..(y + 1) * tw * 4]);
        }
    }
    write_png(name, w as u32, h as u32, &out);
}

/// A crop scaled up with nearest-neighbour sampling.
fn save_zoom(name: &str, f: &RgbaFrame, x: u32, y: u32, w: u32, h: u32, k: u32) {
    let mut out = vec![0_u8; (w * k * h * k * 4) as usize];
    for oy in 0..h * k {
        for ox in 0..w * k {
            let (sx, sy) = (x + ox / k, y + oy / k);
            let dst = ((oy * w * k + ox) * 4) as usize;
            if sx < f.width && sy < f.height {
                let src = ((sy * f.width + sx) * 4) as usize;
                out[dst..dst + 4].copy_from_slice(&f.rgba[src..src + 4]);
            } else {
                out[dst..dst + 4].copy_from_slice(&[255, 0, 255, 255]);
            }
        }
    }
    write_png(name, w * k, h * k, &out);
}

fn pixel(frame: &RgbaFrame, x: u32, y: u32) -> [u8; 4] {
    let row = frame.row(y).unwrap();
    let at = x as usize * 4;
    [row[at], row[at + 1], row[at + 2], row[at + 3]]
}

fn rgb(frame: &RgbaFrame, x: u32, y: u32) -> [u8; 3] {
    let p = pixel(frame, x, y);
    [p[0], p[1], p[2]]
}

// ---- the puppet: a face of rectangles with 14 flat mouth shapes ---------------------------------------------

fn rect(name: &str, x: f64, y: f64, w: u32, h: u32, rgb: [u8; 3]) -> ArtNode {
    let n = (w * h) as usize;
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
            rect("White", x, 50.0, 30, 20, [255, 255, 255]),
            rect(&format!("+ {side} Pupil"), x + 10.0, 55.0, 10, 10, [10, 10, 10]),
            hidden(rect(&format!("{side} Blink"), x, 58.0, 30, 4, [30, 60, 220])),
        ],
    )
}

/// The flat colour of a mouth shape.
fn shape_colour(v: PuppetViseme) -> [u8; 3] {
    let i = PuppetViseme::ALL.iter().position(|x| *x == v).unwrap() as u8;
    [20 + i * 15, 220 - i * 12, 40 + i * 5]
}

fn puppet() -> Puppet {
    let mouth = ArtNode::group(
        "mouth",
        PuppetViseme::ALL
            .iter()
            .map(|v| {
                let node = rect(v.label(), 85.0, 90.0, 30, 15, shape_colour(*v));
                if *v == PuppetViseme::Neutral { node } else { hidden(node) }
            })
            .collect(),
    );
    let head = ArtNode::group(
        "Head",
        vec![
            rect("Face", 50.0, 20.0, 100, 100, SKIN),
            eye("Left", 110.0),
            eye("Right", 60.0),
            rect("+Left Eyebrow", 110.0, 36.0, 30, 10, [90, 40, 10]),
            rect("+ right eyebrow", 60.0, 36.0, 30, 10, [90, 40, 10]),
            mouth,
        ],
    );
    let body = ArtNode::group("Body", vec![rect("Torso", 60.0, 120.0, 80, 80, [120, 120, 130])]);
    let tree = ArtworkTree { size: Size::new(f64::from(SIZE), f64::from(SIZE)), root: ArtNode::group("", vec![body, head]), warnings: Vec::new() };
    Puppet::from_artwork(&tree).unwrap()
}

fn cast() -> Cast {
    let mut cast = Cast::new();
    let mut member = CastMember::new(puppet());
    member.skin = Vec2::new(-100.0, -100.0);
    cast.insert("Hero", member);
    cast
}

fn seconds(s: f64) -> Tick {
    Tick::from_seconds_f64(s)
}

fn secs(t: Tick) -> f64 {
    t.0 as f64 / TICKS_PER_SECOND as f64
}

fn scene_at(fps: FrameRate, duration: f64) -> Scene {
    Scene::new("visual", Size::new(f64::from(SIZE), f64::from(SIZE)), fps, seconds(duration))
}

fn voice_take(groups: &[ParamGroup], start: Tick, duration: Tick, shapes: &[(Tick, anim_lipsync::Viseme)]) -> Take {
    let mut take = Take::new(0, groups.iter().copied().collect(), start, duration);
    for (t, v) in shapes {
        take.samples.push(Sample { t: *t, frame: Frame { viseme: Some(*v), ..Frame::default() } });
    }
    take
}

fn mouth_colour(frames: &SceneFrames<'_>, f: u32) -> [u8; 3] {
    rgb(&render_frame(frames, f, &RasterOptions::default()).unwrap(), 100, 97)
}

// ---- synthetic speech ------------------------------------------------------------------------------------

fn n(seconds: f32, rate: u32) -> usize {
    (seconds * rate as f32).round() as usize
}

fn resonate(x: &mut [f32], rate: u32, hz: f32, bw: f32) {
    let r = (-std::f32::consts::PI * bw / rate as f32).exp();
    let c1 = 2.0 * r * (std::f32::consts::TAU * hz / rate as f32).cos();
    let c2 = -r * r;
    let (mut y1, mut y2) = (0.0, 0.0);
    for v in x.iter_mut() {
        let y = *v + c1 * y1 + c2 * y2;
        y2 = y1;
        y1 = y;
        *v = y;
    }
}

/// A vowel: a 120 Hz glottal pulse train with a spectral tilt through a formant cascade, rms `level`, 10 ms fades.
fn vowel(formants: &[f32], seconds: f32, rate: u32, level: f32) -> Vec<f32> {
    let len = n(seconds, rate);
    let mut src = vec![0.0_f32; len];
    let mut phase = 0.0_f32;
    for v in &mut src {
        phase += 120.0 / rate as f32;
        if phase >= 1.0 {
            phase -= 1.0;
            *v = 1.0;
        }
    }
    let mut y = 0.0;
    for v in &mut src {
        y = 0.5 * y + *v;
        *v = y;
    }
    for &f in formants {
        resonate(&mut src, rate, f, 70.0 + f * 0.04);
    }
    let rms = (src.iter().map(|v| v * v).sum::<f32>() / len.max(1) as f32).sqrt().max(1e-9);
    for v in &mut src {
        *v *= level / rms;
    }
    let k = n(0.01, rate).min(len / 2);
    for i in 0..k {
        let g = i as f32 / k as f32;
        src[i] *= g;
        let last = len - 1 - i;
        src[last] *= g;
    }
    src
}

const AH: [f32; 3] = [800.0, 1300.0, 2600.0];
const EE: [f32; 3] = [300.0, 2250.0, 3000.0];
const OH: [f32; 3] = [540.0, 880.0, 2500.0];

fn speech(rate: u32) -> Vec<f32> {
    let mut x = vec![0.0; n(0.30, rate)];
    x.extend(vowel(&AH, 0.5, rate, 0.1));
    x.extend(vowel(&EE, 0.5, rate, 0.1));
    x.extend(vec![0.0; n(0.40, rate)]);
    x.extend(vowel(&OH, 0.5, rate, 0.1));
    x.extend(vec![0.0; n(0.30, rate)]);
    x
}

// ---- speech-to-mouth -------------------------------------------------------------------------------------

fn puppet_viseme(v: anim_lipsync::Viseme) -> PuppetViseme {
    PuppetViseme::from_label(v.ca_layer_name()).unwrap()
}

fn speech_scene(fps: FrameRate, keys: &[VisemeKey]) -> Scene {
    let mut doc = RecordDoc::new(scene_at(fps, 2.5));
    doc.scene.add_track(PuppetTrack::new("Hero", "Hero")).unwrap();
    doc.arming.groups.insert(ParamGroup::Voice);
    let frames = doc.scene.frame_count();
    let inputs: Vec<Inputs> = (0..frames)
        .map(|f| {
            let t = doc.scene.frame_time(i64::from(f));
            Inputs::at(t).with_viseme(viseme_at(keys, t))
        })
        .collect();
    let recording = doc.recorder().record_all(&inputs, None).unwrap();
    doc.scene.add_take(0, recording.take.unwrap()).unwrap();
    doc.scene
}

#[test]
fn speech_to_mouth() {
    let keys = analyze(&speech(16_000), 16_000);
    // The analysis itself is checked in anim-lipsync; here only the sequence the picture must follow.
    let shown: Vec<_> = keys.iter().map(|k| k.viseme).collect();
    assert_eq!(
        shown,
        [
            anim_lipsync::Viseme::Neutral,
            anim_lipsync::Viseme::Ah,
            anim_lipsync::Viseme::Ee,
            anim_lipsync::Viseme::Neutral,
            anim_lipsync::Viseme::Oh,
            anim_lipsync::Viseme::Neutral
        ],
        "{keys:?}"
    );
    let cast = cast();
    let mut sheet: Vec<RgbaFrame> = Vec::new();
    for (label, fps) in [("23976", FrameRate::FPS_23_976), ("24", FrameRate::FPS_24), ("2997", FrameRate::FPS_29_97), ("30", FrameRate::FPS_30)] {
        let scene = speech_scene(fps, &keys);
        let frames = SceneFrames::new(&scene, &cast).unwrap();
        let count = frames.frame_count();
        let mut previous: Option<PuppetViseme> = None;
        for f in 0..count {
            let t = scene.frame_time(i64::from(f));
            let ts = secs(t);
            let expected = puppet_viseme(viseme_at(&keys, t));
            let pose = frames.pose(0, f).unwrap();
            assert_eq!(pose.viseme, Some(expected), "{label} fps frame {f} at {ts:.4}");
            let colour = mouth_colour(&frames, f);
            assert_eq!(colour, shape_colour(expected), "{label} fps frame {f}");
            // Zone predictions derived from the audio, with a margin for the detector's latency.
            let zone = |lo: f64, hi: f64| ts >= lo && ts <= hi;
            if zone(0.0, 0.28) || zone(1.45, 1.65) || ts > 2.3 {
                assert_eq!(colour, shape_colour(PuppetViseme::Neutral), "{label} frame {f} at {ts:.3} must be silent");
            }
            if zone(0.42, 0.68) {
                assert_eq!(colour, shape_colour(PuppetViseme::Ah), "{label} frame {f} at {ts:.3}");
            }
            if zone(0.92, 1.18) {
                assert_eq!(colour, shape_colour(PuppetViseme::Ee), "{label} frame {f} at {ts:.3}");
            }
            if zone(1.82, 2.08) {
                assert_eq!(colour, shape_colour(PuppetViseme::Oh), "{label} frame {f} at {ts:.3}");
            }
            previous = Some(expected);
        }
        assert!(previous.is_some());
        // The key boundary lands on the first frame at or after the key's start tick.
        for key in keys.iter().skip(1) {
            let first = (0..count).find(|f| scene.frame_time(i64::from(*f)) >= key.start).unwrap();
            let before = viseme_at(&keys, scene.frame_time(i64::from(first) - 1));
            assert_ne!(before, key.viseme, "{label}: the frame before key {key:?} still shows the old shape");
            assert_eq!(frames.pose(0, first).unwrap().viseme, Some(puppet_viseme(key.viseme)), "{label}: first frame of {key:?}");
        }
        // Determinism: two independent builds draw the same bytes.
        let again = SceneFrames::new(&scene, &cast).unwrap();
        let f = count / 2;
        let a = render_frame(&frames, f, &RasterOptions::default()).unwrap();
        let b = render_frame(&again, f, &RasterOptions::default()).unwrap();
        assert_eq!(a.rgba, b.rgba, "{label}: frame {f} rendered twice");
        if label == "23976" || label == "2997" {
            let picks = [0.5_f64, 1.0, 1.5, 2.0].map(|t| (t * scene.fps_f64()) as u32);
            for p in picks {
                sheet.push(render_frame(&frames, p, &RasterOptions::default()).unwrap());
            }
        }
        // Mouth rectangle edges (the Ah frame): x 85..=114, y 90..=104.
        let ah_frame = (0..count).find(|f| secs(scene.frame_time(i64::from(*f))) > 0.55).unwrap();
        let img = render_frame(&frames, ah_frame, &RasterOptions::default()).unwrap();
        let ah = shape_colour(PuppetViseme::Ah);
        assert_eq!(pixel(&img, 100, 97)[3], 255);
        assert_eq!(rgb(&img, 85, 97), ah);
        assert_eq!(rgb(&img, 114, 97), ah);
        assert_eq!(rgb(&img, 84, 97), SKIN);
        assert_eq!(rgb(&img, 115, 97), SKIN);
        assert_eq!(rgb(&img, 100, 90), ah);
        assert_eq!(rgb(&img, 100, 104), ah);
        assert_eq!(rgb(&img, 100, 89), SKIN);
        assert_eq!(rgb(&img, 100, 105), SKIN);
        if label == "2997" {
            save_zoom("speech-to-mouth-edge", &img, 78, 84, 44, 28, 8);
        }
    }
    let refs: Vec<&RgbaFrame> = sheet.iter().collect();
    save_sheet("speech-to-mouth", &refs, 4);
}

// ---- frame-tick-maths ------------------------------------------------------------------------------------

#[test]
fn frame_tick_maths() {
    let rates = [FrameRate::FPS_23_976, FrameRate::FPS_24, FrameRate::FPS_29_97, FrameRate::FPS_30, FrameRate::new(11, 1), FrameRate::FPS_59_94];
    // The first whole tick inside frame f (ceil of the exact start).
    let exact = |r: FrameRate, f: i64| {
        let scaled = i128::from(f) * i128::from(TICKS_PER_SECOND) * i128::from(r.den);
        (scaled.div_euclid(i128::from(r.num)) + i128::from(scaled.rem_euclid(i128::from(r.num)) != 0)) as i64
    };
    let frame1: [(FrameRate, i64); 5] = [
        (FrameRate::FPS_23_976, 10_594_584_000),
        (FrameRate::FPS_24, 10_584_000_000),
        (FrameRate::FPS_29_97, 8_475_667_200),
        (FrameRate::FPS_30, 8_467_200_000),
        (FrameRate::FPS_59_94, 4_237_833_600),
    ];
    for (rate, want) in frame1 {
        assert_eq!(scene_at(rate, 10.0).frame_time(1).0, want, "{rate:?}");
    }
    for rate in rates {
        let scene = scene_at(rate, 10.0);
        for f in (0..=1_000_000_i64).step_by(997).chain([999_999, 1_000_000]) {
            let t = scene.frame_time(f);
            assert_eq!(t.0, exact(rate, f), "{rate:?} frame {f}");
            assert_eq!(scene.frame_at(t), f, "{rate:?} frame {f}");
            if f > 0 {
                assert_eq!(scene.frame_at(Tick(t.0 - 1)), f - 1, "{rate:?} one tick before frame {f}");
            }
        }
        // A scene lasting exactly N frames has N frames; one tick more has N+1.
        let mut s = scene_at(rate, 0.0);
        for frames in [1_i64, 2, 239, 240, 1001] {
            s.duration = s.frame_time(frames);
            assert_eq!(i64::from(s.frame_count()), frames, "{rate:?} exactly {frames}");
            s.duration = Tick(s.duration.0 + 1);
            assert_eq!(i64::from(s.frame_count()), frames + 1, "{rate:?} {frames} + 1 tick");
        }
    }
    for (rate, count) in [
        (FrameRate::FPS_23_976, 240),
        (FrameRate::FPS_24, 240),
        (FrameRate::FPS_29_97, 300),
        (FrameRate::FPS_30, 300),
        (FrameRate::new(11, 1), 110),
        (FrameRate::FPS_59_94, 600),
    ] {
        assert_eq!(scene_at(rate, 10.0).frame_count(), count, "{rate:?}");
    }
    // The script clock is the scene clock.
    for rate in [FrameRate::FPS_23_976, FrameRate::FPS_24, FrameRate::FPS_29_97, FrameRate::FPS_30, FrameRate::FPS_59_94, FrameRate::FPS_119_88] {
        let scene = scene_at(rate, 10.0);
        let inputs = Script::new(scene.fps_f64(), 3000).inputs().unwrap();
        for (f, input) in inputs.iter().enumerate() {
            assert_eq!(input.time, scene.frame_time(f as i64), "{rate:?} frame {f}");
        }
    }
    // A take starting at a frame's time covers it and not the frame before; one ending there does not cover it.
    for rate in [FrameRate::FPS_23_976, FrameRate::FPS_29_97] {
        let scene = scene_at(rate, 10.0);
        for k in [1_i64, 7, 100, 239] {
            let take = Take::new(1, [ParamGroup::Head].into_iter().collect(), scene.frame_time(k), Tick(scene.frame_time(k + 5).0 - scene.frame_time(k).0));
            assert_eq!(take.weight_at(scene.frame_time(k)), 1.0, "{rate:?} k={k}");
            assert_eq!(take.weight_at(scene.frame_time(k - 1)), 0.0);
            assert_eq!(take.weight_at(scene.frame_time(k + 4)), 1.0);
            assert_eq!(take.weight_at(scene.frame_time(k + 5)), 0.0);
        }
    }
    // The mixer's start sample for a clip on frame 7.
    let mut media = MediaStore::new();
    media.insert("one", PcmBuffer::new(8000, 1, vec![1.0; 100])).unwrap();
    for rate in [8000_u32, 44_100, 48_000] {
        let mut scene = scene_at(FrameRate::FPS_23_976, 2.0);
        let start = scene.frame_time(7);
        scene.add_clip(0, AudioClip::new("c", "one", start)).unwrap();
        let (mixed, _) = mix_scene(&scene, &media, MixFormat { sample_rate: rate, channels: 1 }).unwrap();
        let first = mixed.samples.iter().position(|s| s.abs() > 0.5).unwrap();
        let want = (i128::from(start.0) * i128::from(rate)).div_euclid(i128::from(TICKS_PER_SECOND)) as usize;
        assert!(first.abs_diff(want) <= 1, "{rate} Hz: clip starts at output sample {first}, expected {want}");
    }
}

// ---- ramp-take-resample ----------------------------------------------------------------------------------

fn roll_of(k: i64) -> f64 {
    if k <= 30 {
        k as f64
    } else if k <= 70 {
        30.0
    } else {
        30.0 - (k - 70) as f64
    }
}

fn ramp_scene() -> (Scene, i64) {
    let mut doc = RecordDoc::new(scene_at(FrameRate::FPS_23_976, 4.0));
    doc.scene.add_track(PuppetTrack::new("Hero", "Hero")).unwrap();
    doc.arming.groups.insert(ParamGroup::Head);
    let mut script = Script::new(30000.0 / 1001.0, 101);
    for k in 0..=100_u32 {
        script = script.with(ScriptEvent::Face { frame: k, face: FaceInput { head_roll: roll_of(i64::from(k)), ..FaceInput::default() } });
    }
    let offset = seconds(0.5);
    let mut inputs = script.inputs().unwrap();
    let last = inputs.last().unwrap().time.0;
    for i in &mut inputs {
        i.time = Tick(i.time.0 + offset.0);
    }
    let recording = doc.recorder().record_all(&inputs, None).unwrap();
    doc.scene.add_take(0, recording.take.unwrap()).unwrap();
    (doc.scene, last)
}

#[test]
fn ramp_take_resample() {
    let (scene, last_tick) = ramp_scene();
    let take = &scene.tracks[0].takes[0];
    assert_eq!(take.samples.len(), 62);
    assert_eq!(take.start, seconds(0.5));
    assert_eq!(take.duration.0, last_tick + 1);
    let mut covered = 0;
    for f in 0..i64::from(scene.frame_count()) {
        let t = scene.frame_time(f);
        let face = track_inputs(&scene, &scene.tracks[0], f).face;
        let s = secs(Tick(t.0 - take.start.0));
        let inside = t.0 >= take.start.0 && t.0 < take.start.0 + take.duration.0;
        match (inside, face) {
            (false, None) => {}
            (true, Some(face)) => {
                covered += 1;
                let k = s * 30000.0 / 1001.0;
                let want = if k <= 30.0 {
                    k
                } else if k <= 70.0 {
                    30.0
                } else {
                    30.0 - (k - 70.0)
                };
                assert!((face.head_roll - want).abs() < 1e-6, "scene frame {f} at {t:?}: roll {} want {want}", face.head_roll);
                let neutral = FaceInput { head_roll: face.head_roll, ..FaceInput::default() };
                assert_eq!(face, neutral, "frame {f}: only the head was recorded");
            }
            (inside, face) => panic!("frame {f}: inside={inside} face={face:?}"),
        }
    }
    assert_eq!(covered, 80, "frames 12..=91 of the 23.976 scene");
    assert!(track_inputs(&scene, &scene.tracks[0], 11).face.is_none());
    assert!(track_inputs(&scene, &scene.tracks[0], 12).face.is_some());
    assert!(track_inputs(&scene, &scene.tracks[0], 91).face.is_some());
    assert!(track_inputs(&scene, &scene.tracks[0], 92).face.is_none());

    // The picture follows the curve.
    let cast = cast();
    let mut ev = Evaluator::new(&scene, &cast.templates()).unwrap();
    let rot = |ev: &mut Evaluator, f: i64| ev.pose(0, f).unwrap().head.rotation;
    let (r0, r22, r60, r66) = (rot(&mut ev, 0), rot(&mut ev, 22), rot(&mut ev, 60), rot(&mut ev, 66));
    assert!(r0.abs() < 1e-9);
    assert!(r22.abs() < r66.abs(), "mid-ramp {r22} against plateau {r66}");
    assert!((r60 - r66).abs() < 1e-3, "plateau settles: {r60} vs {r66}");
    let frames = SceneFrames::new(&scene, &cast).unwrap();
    let opts = RasterOptions::default();
    let rest = render_frame(&frames, 0, &opts).unwrap();
    let mid = render_frame(&frames, 22, &opts).unwrap();
    let top = render_frame(&frames, 66, &opts).unwrap();
    let end = render_frame(&frames, 95, &opts).unwrap();
    save_sheet("ramp-take-resample", &[&rest, &mid, &top, &end], 4);
    assert_eq!(rgb(&rest, 125, 41), [90, 40, 10], "left eyebrow at rest");
    assert_ne!(rgb(&top, 125, 41), [90, 40, 10], "tilted head moves the eyebrow");
    let r95 = rot(&mut ev, 95);
    assert!(r95.abs() < 0.1 * r66.abs(), "after the take ends the head is back near rest: {r95} vs {r66}");
    assert_eq!(rgb(&end, 125, 41), [90, 40, 10], "and the eyebrow is home again");

    // The sidecar gives the same numbers.
    let dir = tempfile::tempdir().unwrap();
    let path = store::sidecar_path(&dir.path().join("show.chproj"));
    store::save(&path, &scene, &MediaStore::new()).unwrap();
    let (loaded, _) = store::load(&path).unwrap();
    for f in 0..i64::from(scene.frame_count()) {
        assert_eq!(track_inputs(&loaded, &loaded.tracks[0], f), track_inputs(&scene, &scene.tracks[0], f));
    }
}

// ---- voice-take-blend ------------------------------------------------------------------------------------

fn blend_scene(blend: f64, second_enabled: bool) -> Scene {
    let mut scene = scene_at(FrameRate::new(10, 1), 3.0);
    scene.add_track(PuppetTrack::new("Hero", "Hero")).unwrap();
    let voice = [ParamGroup::Voice];
    scene.add_take(0, voice_take(&voice, Tick::ZERO, seconds(3.0), &[(Tick::ZERO, anim_lipsync::Viseme::Ah)])).unwrap();
    let mut upper = voice_take(&voice, seconds(1.0), seconds(1.0), &[(Tick::ZERO, anim_lipsync::Viseme::Oh)]);
    upper.blend_in = seconds(blend);
    upper.blend_out = seconds(blend);
    upper.enabled = second_enabled;
    scene.add_take(0, upper).unwrap();
    scene
}

#[test]
fn voice_take_blend() {
    let cast = cast();
    let (ah, oh) = (shape_colour(PuppetViseme::Ah), shape_colour(PuppetViseme::Oh));
    let run = |scene: &Scene| -> Vec<[u8; 3]> {
        let frames = SceneFrames::new(scene, &cast).unwrap();
        (0..frames.frame_count()).map(|f| mouth_colour(&frames, f)).collect()
    };
    let blended = run(&blend_scene(0.4, true));
    assert_eq!(blended.len(), 30);
    for (f, c) in blended.iter().enumerate() {
        let want = if (12..=18).contains(&f) { oh } else { ah };
        assert_eq!(*c, want, "frame {f}");
    }
    assert!(run(&blend_scene(0.4, false)).iter().all(|c| *c == ah));
    let hard = run(&blend_scene(0.0, true));
    for (f, c) in hard.iter().enumerate() {
        assert_eq!(*c, if (10..=19).contains(&f) { oh } else { ah }, "hard frame {f}");
    }
    // A head take with the same blends does not touch the mouth.
    let mut scene = blend_scene(0.4, true);
    let mut head = Take::new(0, [ParamGroup::Head].into_iter().collect(), seconds(1.0), seconds(1.0));
    head.blend_in = seconds(0.4);
    head.blend_out = seconds(0.4);
    head.samples.push(Sample { t: Tick::ZERO, frame: Frame { face: Some(FaceInput { head_roll: 10.0, ..FaceInput::default() }), ..Frame::default() } });
    scene.add_take(0, head).unwrap();
    let with_head = run(&scene);
    assert_eq!(with_head, blended);
    let frames = SceneFrames::new(&blend_scene(0.4, true), &cast).unwrap();
    let opts = RasterOptions::default();
    let picks: Vec<RgbaFrame> = [10_u32, 11, 12, 17, 18, 19].iter().map(|f| render_frame(&frames, *f, &opts).unwrap()).collect();
    let refs: Vec<&RgbaFrame> = picks.iter().collect();
    save_sheet("voice-take-blend", &refs, 6);
}

// ---- punch-audio-alignment -------------------------------------------------------------------------------

fn signal(range: std::ops::Range<usize>) -> PcmBuffer {
    PcmBuffer::new(8000, 1, range.map(|i| i as f32 / 8000.0).collect())
}

fn at_sample(i: usize) -> Tick {
    Tick::from_units(i as i64, 8000)
}

#[test]
fn punch_audio_alignment() {
    // Chunks of 2000 samples at sample offsets 1000 + 2000k straddle the punch edges (8000 and 16000).
    let mut r = Recorder::new();
    r.set_audio_armed(true);
    r.arm_only([ParamGroup::Head]);
    r.set_punch(Some(TimeRange::from_bounds(seconds(1.0), seconds(2.0))));
    r.start(Tick::ZERO).unwrap();
    for k in 0..8 {
        let from = 1000 + 2000 * k;
        r.push_audio(at_sample(from), &signal(from..from + 2000));
    }
    let audio = r.finish(seconds(3.0)).unwrap().audio.unwrap();
    assert_eq!(audio.start, seconds(1.0), "audio starts at the punch-in");
    assert_eq!(audio.buffer.frames(), 8000, "and lasts exactly the punch");
    for (i, s) in audio.buffer.samples.iter().enumerate() {
        let want = (8000 + i) as f32 / 8000.0;
        assert!((s - want).abs() < 1e-6, "sample {i}: {s} vs {want}");
    }

    // A hole is filled with silence and an overlap is not repeated.
    let mut r = Recorder::new();
    r.set_audio_armed(true);
    r.start(Tick::ZERO).unwrap();
    r.push_audio(at_sample(0), &signal(0..2000));
    r.push_audio(at_sample(2400), &signal(2400..4400));
    r.push_audio(at_sample(4300), &signal(4300..6300));
    let audio = r.finish(seconds(1.0)).unwrap().audio.unwrap();
    assert_eq!(audio.start, Tick::ZERO);
    assert_eq!(audio.buffer.frames(), 6300);
    for (i, s) in audio.buffer.samples.iter().enumerate() {
        let want = if (2000..2400).contains(&i) { 0.0 } else { i as f32 / 8000.0 };
        assert!((s - want).abs() < 1e-6, "sample {i}: {s} vs {want}");
    }

    // A chunk before the start is dropped and one straddling it is cut.
    let mut r = Recorder::new();
    r.set_audio_armed(true);
    r.start(seconds(0.5)).unwrap();
    r.push_audio(at_sample(0), &signal(0..2000));
    r.push_audio(at_sample(3500), &signal(3500..5500));
    let audio = r.finish(seconds(1.0)).unwrap().audio.unwrap();
    assert_eq!(audio.start, seconds(0.5));
    assert_eq!(audio.buffer.frames(), 1500);
    assert!((audio.buffer.samples[0] - 0.5).abs() < 1e-6, "first kept sample is the one at 0.5 s");

    // Mixed back at its start it reproduces the original signal.
    let mut doc = RecordDoc::new(scene_at(FrameRate::FPS_24, 3.0));
    doc.scene.add_track(PuppetTrack::new("Hero", "Hero")).unwrap();
    doc.arming.audio = true;
    doc.arming.punch = Some(TimeRange::from_bounds(seconds(1.0), seconds(2.0)));
    let mut rec = doc.recorder();
    rec.start(Tick::ZERO).unwrap();
    for k in 0..8 {
        let from = 1000 + 2000 * k;
        rec.push_audio(at_sample(from), &signal(from..from + 2000));
    }
    let recording = rec.finish(seconds(3.0)).unwrap();
    let mut media = MediaStore::new();
    let committed = doc.commit(0, recording, 0, &mut media).unwrap();
    let (name, _) = committed.clip.unwrap();
    assert!(media.get(&name).is_some());
    let (mixed, missing) = mix_scene(&doc.scene, &media, MixFormat { sample_rate: 8000, channels: 1 }).unwrap();
    assert!(missing.is_empty());
    for i in (8000..16_000).step_by(37) {
        let want = i as f32 / 8000.0;
        assert!((mixed.samples[i] - want).abs() < 1e-5, "mixed sample {i}");
    }
    assert!(mixed.samples[7999].abs() < 1e-9 && mixed.samples[16_000].abs() < 1e-9, "nothing outside the punch");
}

// ---- mix-levels ------------------------------------------------------------------------------------------

#[test]
fn mix_levels() {
    let mut scene = scene_at(FrameRate::FPS_23_976, 2.0);
    let mut media = MediaStore::new();
    media.insert("a", PcmBuffer::new(8000, 1, vec![0.5; 4000])).unwrap();
    media.insert("b", PcmBuffer::new(8000, 2, [0.2_f32, 0.6].iter().copied().cycle().take(16_000).collect())).unwrap();
    media.insert("c", PcmBuffer::new(8000, 1, vec![0.9; 4000])).unwrap();
    media.insert("loud", PcmBuffer::new(8000, 1, vec![1.0; 16_000])).unwrap();
    let mut a = AudioClip::new("A", "a", Tick::ZERO);
    a.pan = -1.0;
    let mut b = AudioClip::new("B", "b", seconds(0.5));
    b.gain = 0.5;
    b.trim_in = seconds(0.25);
    b.duration = Some(seconds(0.5));
    scene.add_clip(0, a).unwrap();
    scene.add_clip(0, b).unwrap();
    scene.audio[0].gain = 0.5;
    let mut c = AudioClip::new("C", "c", scene.frame_time(24));
    c.gain = 2.0;
    c.pan = 1.0;
    scene.add_clip(1, c).unwrap();
    let mut d = AudioClip::new("D", "loud", Tick::ZERO);
    d.enabled = false;
    scene.add_clip(1, d).unwrap();
    scene.add_clip(1, AudioClip::new("E", "ghost", Tick::ZERO)).unwrap();
    scene.add_clip(2, AudioClip::new("F", "loud", Tick::ZERO)).unwrap();
    scene.audio[2].enabled = false;

    let (mixed, missing) = mix_scene(&scene, &media, MixFormat { sample_rate: 8000, channels: 2 }).unwrap();
    assert_eq!(missing, ["ghost"]);
    assert_eq!(mixed.frames(), 16_000);
    let at = |i: usize| (mixed.samples[2 * i], mixed.samples[2 * i + 1]);
    let near = |v: f32, w: f32, what: &str| assert!((v - w).abs() < 1e-5, "{what}: {v} vs {w}");
    let sqrt2 = std::f32::consts::SQRT_2;
    for i in [0, 1, 2000, 3999] {
        near(at(i).0, 0.5 * sqrt2 * 0.5, &format!("A left {i}"));
        near(at(i).1, 0.0, &format!("A right {i}"));
    }
    for i in [4000, 5000, 7999] {
        near(at(i).0, 0.05, &format!("B left {i}"));
        near(at(i).1, 0.15, &format!("B right {i}"));
    }
    for i in 8000..8008 {
        assert_eq!(at(i), (0.0, 0.0), "silence at sample {i}");
    }
    for i in [8008, 9000, 12_007] {
        near(at(i).1, 0.9 * 2.0 * sqrt2, &format!("C right {i}"));
        near(at(i).0, 0.0, &format!("C left {i}"));
    }
    for i in [12_008, 14_000, 15_999] {
        assert_eq!(at(i), (0.0, 0.0), "silence after C at {i}");
    }
    let track = scene_soundtrack(&scene, &media, MixFormat { sample_rate: 8000, channels: 2 }).unwrap().unwrap();
    assert_eq!(track.samples[2 * 9000 + 1], 1.0, "clamped for the encoder");
    assert!(track.samples.iter().all(|s| (-1.0..=1.0).contains(s)));
    // Mono output averages the sides.
    let (mono, _) = mix_scene(&scene, &media, MixFormat { sample_rate: 8000, channels: 1 }).unwrap();
    near(mono.samples[5000], 0.1, "mono B");
    near(mono.samples[100], 0.5 * sqrt2 * 0.5 / 2.0, "mono A");
    // 48 kHz: C starts at floor(1.001 * 48000) = 48048.
    let (hi, _) = mix_scene(&scene, &media, MixFormat { sample_rate: 48_000, channels: 2 }).unwrap();
    assert_eq!(hi.frames(), 96_000);
    assert!(hi.samples[2 * 48_047 + 1].abs() < 1e-9, "sample 48047 is silent");
    assert!(hi.samples[2 * 48_048 + 1] > 1.0, "sample 48048 sounds");
}

// ---- determinism-scrub -----------------------------------------------------------------------------------

fn determinism_scene(with_b: bool) -> Scene {
    let mut scene = scene_at(FrameRate::FPS_29_97, 6.0);
    scene.seed = 7;
    let mut a = PuppetTrack::new("A", "Hero");
    a.behaviors.push(StackOp::SetParam { index: 1, name: "Trim_Ah".to_owned(), value: serde_json::json!(200.0) });
    scene.add_track(a).unwrap();
    if with_b {
        scene.add_track(PuppetTrack::new("B", "Hero")).unwrap();
    }
    let head = [ParamGroup::Head];
    let mut take = Take::new(0, head.into_iter().collect(), Tick::ZERO, seconds(6.0));
    for (i, roll) in [0.0, 12.0, -8.0, 20.0, 0.0].iter().enumerate() {
        take.samples.push(Sample {
            t: seconds(i as f64 * 1.5),
            frame: Frame { face: Some(FaceInput { head_roll: *roll, ..FaceInput::default() }), ..Frame::default() },
        });
    }
    scene.add_take(0, take).unwrap();
    let ft = |k| scene.frame_time(k);
    let voice = voice_take(
        &[ParamGroup::Voice],
        Tick::ZERO,
        seconds(6.0),
        &[(Tick::ZERO, anim_lipsync::Viseme::Ah), (ft(60), anim_lipsync::Viseme::Oh), (ft(120), anim_lipsync::Viseme::Ah)],
    );
    scene.add_take(0, voice).unwrap();
    scene
}

#[test]
fn determinism_scrub() {
    let cast = cast();
    let scene = determinism_scene(true);
    let templates = cast.templates();
    let bake = Evaluator::new(&scene, &templates).unwrap().bake(180);
    let mut ev = Evaluator::new(&scene, &templates).unwrap();
    for f in [179_i64, 61, 59, 60, 120, 121, 0, 119, 180, 100, 100, 1] {
        let pose = ev.pose(0, f).unwrap();
        let reference = bake.get(usize::try_from(f).unwrap()).map_or_else(|| ev.pose(0, f).unwrap(), |p| p[0].clone());
        assert_eq!(pose, reference, "jump to frame {f}");
        let b = ev.pose(1, f).unwrap();
        let reference_b = bake.get(usize::try_from(f).unwrap()).map_or_else(|| ev.pose(1, f).unwrap(), |p| p[1].clone());
        assert_eq!(b, reference_b, "track B at frame {f}");
    }
    // Track A does not notice B.
    let solo = Evaluator::new(&determinism_scene(false), &templates).unwrap().bake(180);
    for f in 0..180 {
        assert_eq!(solo[f][0], bake[f][0], "A with and without B, frame {f}");
    }
    let mut disabled = determinism_scene(true);
    disabled.tracks[1].enabled = false;
    let off = Evaluator::new(&disabled, &templates).unwrap().bake(180);
    for f in 0..180 {
        assert_eq!(off[f][0], bake[f][0], "A with B off, frame {f}");
    }
    // Seeds: the same seed repeats the blinks, another one moves them.
    let blinks = |scene: &Scene| -> Vec<usize> {
        let poses = Evaluator::new(scene, &templates).unwrap().bake(180);
        (0..180).filter(|f| poses[*f][1].blink_left > 0.5).collect()
    };
    let mut other = determinism_scene(true);
    other.seed = 8;
    assert_eq!(blinks(&scene), blinks(&determinism_scene(true)));
    assert_ne!(blinks(&scene), blinks(&other), "seed 7 and 8 blink at different frames");

    // Rendering one frame twice, with other frames in between.
    let frames = SceneFrames::new(&scene, &cast).unwrap();
    let opts = RasterOptions::default();
    let first = render_frame(&frames, 100, &opts).unwrap();
    let list = frames.display_list(100);
    let _ = render_frame(&frames, 3, &opts).unwrap();
    let _ = render_frame(&frames, 170, &opts).unwrap();
    let second = render_frame(&frames, 100, &opts).unwrap();
    assert_eq!(first.rgba, second.rgba);
    assert_eq!(list, frames.display_list(100));
    let rebuilt = SceneFrames::new(&scene, &cast).unwrap();
    assert_eq!(render_frame(&rebuilt, 100, &opts).unwrap().rgba, first.rgba);

    // Trim_Ah = 200 ms: Oh is wanted at frame 60; Ah was last wanted at frame 59 and stays until 200 ms later,
    // which is frame 64 + 1 = 65 (5 frames of 33.37 ms are 166.9 ms, 6 are 200.2 ms).
    let poses = &bake;
    let shown = |f: usize| poses[f][0].viseme;
    assert_eq!(shown(0), Some(PuppetViseme::Ah), "a trim never delays the first shape");
    assert_eq!(shown(60), Some(PuppetViseme::Ah));
    assert_eq!(shown(64), Some(PuppetViseme::Ah));
    assert_eq!(shown(65), Some(PuppetViseme::Oh));
    // Without the trim the switch is immediate.
    let mut plain = determinism_scene(true);
    plain.tracks[0].behaviors.clear();
    let plain_poses = Evaluator::new(&plain, &templates).unwrap().bake(180);
    assert_eq!(plain_poses[59][0].viseme, Some(PuppetViseme::Ah));
    assert_eq!(plain_poses[60][0].viseme, Some(PuppetViseme::Oh));
    // The trim holds Oh the same way when it goes back to Ah at frame 120? (Trim_Oh is 0: immediate.)
    assert_eq!(poses[120][0].viseme, Some(PuppetViseme::Ah));
    let _ = BTreeSet::<u8>::new();
}
