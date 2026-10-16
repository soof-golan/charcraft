//! Real projects: Boy.puppet performing a recorded take, and the news anchors project imported as a scene.
//! Skipped (passing) when `ANICRAFT_SAMPLES` is not set.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::float_cmp,
    clippy::indexing_slicing,
    clippy::print_stdout,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]
#![allow(clippy::assert_is_empty, reason = "test code")]

use std::path::PathBuf;

use anim_core::{FrameRate, Size, Tick};
use anim_export::{FrameSource, RasterOptions, render_frame};
use charcraft_behaviors::{FaceInput, Script, ScriptEvent};
use charcraft_project::Project;
use charcraft_record::{Cast, MediaStore, ParamGroup, PuppetTrack, RecordDoc, Scene, SceneFrames, import_scene, load_sounds, mix_scene};

fn samples() -> Option<PathBuf> {
    std::env::var_os("ANICRAFT_SAMPLES").map(PathBuf::from)
}

fn opaque_pixels(frame: &anim_export::RgbaFrame, background: [u8; 4]) -> usize {
    frame.rgba.as_chunks::<4>().0.iter().filter(|p| **p != background).count()
}

#[test]
fn boy_performs_a_recorded_take() {
    let Some(dir) = samples() else { return };
    let project = Project::open(dir.join("Boy.puppet")).unwrap();
    let model = project.model().unwrap();
    let path = model.root_puppets().next().unwrap().path.clone();
    let (cast, warnings) = Cast::from_project(&project, &model, [path.as_str()]);
    assert_eq!(cast.len(), 1, "{warnings:?}");
    let member = cast.get(&path).unwrap();
    let size = member.puppet.size;

    let mut scene = Scene::new("Boy", size, FrameRate::FPS_24, Tick::from_seconds_f64(2.0));
    let mut track = PuppetTrack::new("Boy", &path);
    track.placement.position = anim_core::Vec2::new(0.0, 0.0);
    scene.add_track(track).unwrap();
    let mut doc = RecordDoc::new(scene);
    doc.templates = cast.templates();
    doc.arming.groups.extend([ParamGroup::Head, ParamGroup::Brows, ParamGroup::Voice]);
    let script = Script::new(24.0, 48)
        .with(ScriptEvent::Face { frame: 0, face: FaceInput { head_roll: 14.0, eyebrow_left: 0.8, eyebrow_right: 0.8, ..FaceInput::default() } })
        .with(ScriptEvent::Voice { frame: 24, viseme: Some(anim_lipsync::Viseme::Ah) });
    let recording = doc.recorder().record_all(&script.inputs().unwrap(), None).unwrap();
    doc.scene.add_take(0, recording.take.unwrap()).unwrap();

    let frames = SceneFrames::new(&doc.scene, &cast).unwrap();
    assert_eq!(frames.frame_count(), 48);
    let first = render_frame(&frames, 5, &RasterOptions::default()).unwrap();
    let last = render_frame(&frames, 40, &RasterOptions::default()).unwrap();
    // The scene is centred on the stage but not on the artwork's top-left, so only check that Boy is drawn
    // and that the performance changed the picture.
    assert!(opaque_pixels(&first, [255, 255, 255, 255]) > 1000, "the puppet is drawn");
    assert_ne!(first.rgba, last.rgba, "the recorded mouth and head change the picture");
    let pose = frames.pose(0, 40).unwrap();
    assert!(pose.head.rotation.abs() > 0.05, "the head tilts: {}", pose.head.rotation);
    assert!(pose.viseme.is_some());
}

#[test]
fn the_news_anchors_scene_imports_and_plays() {
    let Some(dir) = samples() else { return };
    let path = dir.join("chproj_full").join("news anchors");
    if !path.exists() {
        return;
    }
    let project = Project::open(&path).unwrap();
    let model = project.model().unwrap();
    let imported = import_scene(&model, None).unwrap();
    println!("{} puppet tracks, {} audio tracks, warnings {:?}", imported.scene.tracks.len(), imported.scene.audio.len(), imported.warnings);
    assert!(!imported.scene.tracks.is_empty());
    assert!(imported.scene.size.width >= 640.0);
    let (cast, cast_warnings) = Cast::from_project(&project, &model, imported.puppets.iter().map(String::as_str));
    println!("cast {:?} warnings {cast_warnings:?}", cast.keys().collect::<Vec<_>>());
    assert!(!cast.is_empty(), "{cast_warnings:?}");

    // Play one second of it.
    let mut scene = imported.scene.clone();
    scene.duration = Tick::from_seconds_f64(1.0);
    scene.seed = 3;
    let frames = SceneFrames::new(&scene, &cast).unwrap();
    assert!(frames.frame_count() > 10);
    let frame = render_frame(&frames, 5, &RasterOptions::default()).unwrap();
    assert_eq!((f64::from(frame.width), f64::from(frame.height)), (scene.size.width, scene.size.height));
    assert!(opaque_pixels(&frame, [0, 0, 0, 0]) > 10_000, "the anchors are on the stage");
    // Same scene, same frames.
    let again = SceneFrames::new(&scene, &cast).unwrap();
    assert_eq!(frames.display_list(7), again.display_list(7));

    let mut media = MediaStore::new();
    let warnings = load_sounds(&project, &imported, &mut media);
    println!("sounds {:?} warnings {warnings:?}", media.names().collect::<Vec<_>>());
    let (mixed, missing) = mix_scene(&imported.scene, &media, anim_audio::MixFormat::STEREO_48K).unwrap();
    println!("mixed {} frames, {} missing", mixed.frames(), missing.len());
    let _ = Size::new(1.0, 1.0);
}
