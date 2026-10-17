//! Scenes on a stage: frames for the exporters, merged pictures, the audio mix, and a sidecar round trip.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::float_cmp,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]
#![allow(clippy::many_single_char_names, clippy::missing_const_for_fn, clippy::assert_is_empty, reason = "test code")]

use anim_artwork::synthetic::face;
use anim_audio::{MixFormat, PcmBuffer};
use anim_core::{FrameRate, Rgba8, Size, Tick, Vec2};
use anim_display::{DisplayList, Node};
use anim_export::{FrameSource, RasterOptions, render_frame};
use charcraft_behaviors::{FaceInput, Script, ScriptEvent};
use charcraft_puppet::{Puppet, Viseme};
use charcraft_record::{AudioClip, Cast, CastMember, MediaStore, ParamGroup, PuppetTrack, RecordDoc, Scene, SceneFrames, mix_scene, store};

const SIZE: u32 = 200;

/// A face of rectangles named like a Character Animator puppet: a head, two eyes, brows and 14 mouth shapes.
fn puppet() -> Puppet {
    let labels: Vec<&str> = Viseme::ALL.iter().map(|v| v.label()).collect();
    Puppet::from_artwork(&face(&labels, Viseme::Neutral.label())).unwrap()
}

fn cast() -> Cast {
    let mut cast = Cast::new();
    let mut member = CastMember::new(puppet());
    member.skin = Vec2::new(-100.0, -100.0);
    cast.insert("Hero", member);
    cast
}

fn performance() -> Script {
    Script::new(10.0, 20)
        .with(ScriptEvent::Face { frame: 0, face: FaceInput { head_roll: 12.0, ..FaceInput::default() } })
        .with(ScriptEvent::Voice { frame: 10, viseme: Some(anim_lipsync::Viseme::Oh) })
}

/// A 2 s scene at 10 fps with the hero performing the script, recorded through the document like a tool would.
fn scene() -> Scene {
    let scene = Scene::new("Stage", Size::new(f64::from(SIZE), f64::from(SIZE)), FrameRate::new(10, 1), Tick::from_seconds_f64(2.0));
    let mut doc = RecordDoc::new(scene);
    doc.scene.add_track(PuppetTrack::new("Hero", "Hero")).unwrap();
    doc.arming.groups.extend([ParamGroup::Head, ParamGroup::Voice]);
    let inputs = performance().inputs().unwrap();
    let recording = doc.recorder().record_all(&inputs, None).unwrap();
    doc.scene.add_take(0, recording.take.unwrap()).unwrap();
    doc.scene
}

fn pixel(frame: &anim_export::RgbaFrame, x: u32, y: u32) -> [u8; 4] {
    let row = frame.row(y).unwrap();
    let at = usize::try_from(x).unwrap() * 4;
    [row[at], row[at + 1], row[at + 2], row[at + 3]]
}

#[test]
fn the_frames_follow_the_recorded_performance() {
    let cast = cast();
    let frames = SceneFrames::new(&scene(), &cast).unwrap();
    assert_eq!(frames.frame_count(), 20);
    assert_eq!(frames.fps(), 10.0);
    assert_eq!(frames.size(), Size::new(200.0, 200.0));
    assert!(frames.missing().is_empty());
    let options = RasterOptions::default();
    let before = render_frame(&frames, 3, &options).unwrap();
    let after = render_frame(&frames, 15, &options).unwrap();
    assert_eq!((before.width, before.height), (200, 200));
    // The mouth rectangle sits near (85..115, 90..105) of the puppet, which is centred on the stage; the head tilt
    // shifts it, so compare the whole lower-face region rather than one pixel.
    let changed = (80..125).flat_map(|y| (70..130).map(move |x| (x, y))).filter(|&(x, y)| pixel(&before, x, y) != pixel(&after, x, y)).count();
    assert!(changed > 100, "the mouth shape changed ({changed} pixels)");
    assert_eq!(pixel(&before, 100, 97)[3], 255);
    // The face is drawn at the right place, on the white background.
    assert_eq!(pixel(&before, 2, 2), [255, 255, 255, 255]);
    // The recorded head tilt shows up in the pose once the smoothing has settled.
    let tilted = frames.pose(0, 8).unwrap();
    assert!(tilted.head.rotation.abs() > 0.02, "{}", tilted.head.rotation);
}

#[test]
fn frames_are_pure_functions_of_the_index() {
    let cast = cast();
    let scene = scene();
    let a = SceneFrames::new(&scene, &cast).unwrap();
    let b = SceneFrames::new(&scene, &cast).unwrap();
    let forward: Vec<DisplayList> = (0..20).map(|i| a.display_list(i)).collect();
    let backward: Vec<DisplayList> = (0..20).rev().map(|i| b.display_list(i)).collect();
    for (i, list) in forward.iter().enumerate() {
        assert_eq!(list, &backward[19 - i], "frame {i}");
    }
    assert_eq!(a.display_list(7), a.display_list(7));
}

#[test]
fn two_tracks_share_one_merged_picture_store() {
    let cast = cast();
    let mut scene = scene();
    let mut second = PuppetTrack::new("Twin", "Hero");
    second.placement.position = Vec2::new(40.0, 0.0);
    second.placement.scale = Vec2::new(0.5, 0.5);
    second.placement.opacity = 0.5;
    scene.add_track(second).unwrap();
    let mut ghost = PuppetTrack::new("Ghost", "nobody");
    ghost.enabled = true;
    scene.add_track(ghost).unwrap();
    let frames = SceneFrames::new(&scene, &cast).unwrap();
    assert_eq!(frames.missing(), ["nobody"]);
    let list = frames.display_list(4);
    assert_eq!(list.nodes.len(), 2, "the missing puppet draws nothing");
    let mut bitmaps = 0;
    list.visit(&mut |node| {
        if let Node::Bitmap(b) = node {
            bitmaps += 1;
            assert!(frames.images().get(b.image).is_some(), "image {:?} must be in the merged store", b.image);
        }
    });
    assert!(bitmaps > 8);
    let frame = render_frame(&frames, 4, &RasterOptions::default()).unwrap();
    assert_ne!(pixel(&frame, 100, 97), [255, 255, 255, 255]);
    // Switching a track off removes it.
    scene.tracks[1].enabled = false;
    let one = SceneFrames::new(&scene, &cast).unwrap();
    assert_eq!(one.display_list(4).nodes.len(), 1);
}

#[test]
fn the_sidecar_reproduces_the_same_frames() {
    let cast = cast();
    let scene = scene();
    let dir = tempfile::tempdir().unwrap();
    let path = store::sidecar_path(&dir.path().join("show.chproj"));
    store::save(&path, &scene, &MediaStore::new()).unwrap();
    let (loaded, media) = store::load(&path).unwrap();
    assert!(media.is_empty());
    assert_eq!(loaded, scene);
    let a = SceneFrames::new(&scene, &cast).unwrap();
    let b = SceneFrames::new(&loaded, &cast).unwrap();
    for i in 0..20 {
        assert_eq!(a.display_list(i), b.display_list(i), "frame {i}");
    }
}

#[test]
fn a_later_take_overrides_the_mouth_for_its_range() {
    let cast = cast();
    let mut doc = RecordDoc::new(scene());
    // Record a second performance over the voice only, starting at 1 s with a different mouth shape.
    doc.arming.groups.clear();
    doc.arming.groups.insert(ParamGroup::Voice);
    let script = Script::new(10.0, 5).with(ScriptEvent::Voice { frame: 0, viseme: Some(anim_lipsync::Viseme::Ee) });
    let mut inputs = script.inputs().unwrap();
    for i in &mut inputs {
        i.time = Tick(i.time.0 + Tick::from_seconds_f64(1.0).0);
    }
    let recording = doc.recorder().record_all(&inputs, None).unwrap();
    doc.scene.add_take(0, recording.take.unwrap()).unwrap();
    let frames = SceneFrames::new(&doc.scene, &cast).unwrap();
    let viseme = |f: u32| frames.pose(0, f).unwrap().viseme;
    assert_eq!(viseme(12), Some(Viseme::Ee), "the new take wins inside 1.0 - 1.5 s");
    assert_eq!(viseme(16), Some(Viseme::Oh), "the first take comes back after it");
    assert_eq!(viseme(5), None, "before the voice speaks the mouth is the face's");
}

#[test]
fn the_audio_mix_matches_the_scene() {
    let mut scene = scene();
    let mut media = MediaStore::new();
    media.insert("beep", PcmBuffer::new(8000, 1, vec![0.4; 8000])).unwrap();
    let mut clip = AudioClip::new("beep", "beep", Tick::from_seconds_f64(0.5));
    clip.gain = 0.5;
    scene.add_clip(0, clip).unwrap();
    let (mixed, missing) = mix_scene(&scene, &media, MixFormat { sample_rate: 8000, channels: 1 }).unwrap();
    assert!(missing.is_empty());
    assert_eq!(mixed.frames(), 16_000);
    assert!(mixed.samples[1000].abs() < 1e-6);
    assert!(mixed.samples[5000] > 0.1 && mixed.samples[5000] < 0.4);
    let soundtrack = charcraft_record::scene_soundtrack(&scene, &media, MixFormat::STEREO_48K).unwrap().unwrap();
    assert_eq!(soundtrack.frames(), 96_000);
}

#[test]
fn background_colour_reaches_the_frame() {
    let cast = cast();
    let mut scene = scene();
    scene.background = Some(Rgba8 { r: 10, g: 20, b: 30, a: 255 });
    let frames = SceneFrames::new(&scene, &cast).unwrap();
    assert_eq!(frames.display_list(0).background, scene.background);
    let frame = render_frame(&frames, 0, &RasterOptions::default()).unwrap();
    assert_eq!(pixel(&frame, 1, 1), [10, 20, 30, 255]);
}
