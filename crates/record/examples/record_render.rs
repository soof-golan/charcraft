//! Records a scripted performance into takes through the tool commands, plays the scene back and writes a
//! contact sheet of frames (and, with `--sidecar`, the scene file).
//!
//! ```text
//! cargo run -p charcraft-record --example record_render -- INPUT OUT.png [--frames 6] [--seconds 4] [--scale 0.5] [--seed 7] [--sidecar OUT.json]
//! ```
//!
//! `INPUT` is a `.puppet` or a project (folder or `.chproj`). A project with a scene (the news anchors sample)
//! is imported with its tracks and placements; a bare puppet gets a one-track scene. Every puppet track records
//! two takes: a first one of the head, brows and voice, then a second of just the voice over the middle of the
//! scene (so the contact sheet shows a later take overriding an earlier one, per group). Everything goes through
//! the same commands an MCP client calls.
#![allow(
    clippy::print_stdout,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    clippy::too_many_lines,
    reason = "example code"
)]

use std::error::Error;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use anim_core::{FrameRate, Size, Tick, Vec2};
use anim_export::{FrameSource, RasterOptions, render_frame};
use anim_lipsync::Viseme as Voice;
use anim_session::Session;
use charcraft_behaviors::{FaceInput, Script, ScriptEvent};
use charcraft_project::Project;
use charcraft_record::{Cast, MediaStore, PuppetTrack, RecordDoc, Scene, SceneFrames, import_scene, load_sounds, mix_scene, registry, store};
use serde_json::json;

fn performance(seed: f64, fps: f64, frames: u32) -> Script {
    let mut script = Script::new(fps, frames);
    let shapes = [Voice::Ah, Voice::Oh, Voice::Ee, Voice::M, Voice::Uh, Voice::WOo, Voice::S, Voice::Neutral];
    for frame in (0..frames).step_by(3) {
        let t = f64::from(frame) / fps + seed;
        let face = FaceInput {
            head_roll: 12.0 * (t * 1.3).sin(),
            head_translation: Vec2::new(0.2 * (t * 0.9).sin(), 0.08 * (t * 2.1).cos()),
            eyebrow_left: 0.7 * (t * 1.7).sin().max(0.0),
            eyebrow_right: 0.7 * (t * 1.7).sin().max(0.0),
            ..FaceInput::default()
        };
        script = script.with(ScriptEvent::Face { frame, face });
    }
    for frame in (0..frames).step_by(5) {
        let shape = shapes[((frame / 5) as usize + seed as usize) % shapes.len()];
        script = script.with(ScriptEvent::Voice { frame, viseme: Some(shape) });
    }
    script
}

fn write_sheet(path: &str, tiles: &[anim_export::RgbaFrame], columns: usize) -> Result<(), Box<dyn Error>> {
    let (tw, th) = (tiles[0].width as usize, tiles[0].height as usize);
    let rows = tiles.len().div_ceil(columns);
    let (w, h) = (tw * columns, th * rows);
    let mut pixels = vec![255_u8; w * h * 4];
    for (i, tile) in tiles.iter().enumerate() {
        let (ox, oy) = ((i % columns) * tw, (i / columns) * th);
        for y in 0..th {
            let src = &tile.rgba[y * tw * 4..(y + 1) * tw * 4];
            let at = ((oy + y) * w + ox) * 4;
            pixels[at..at + tw * 4].copy_from_slice(src);
        }
    }
    if let Some(parent) = Path::new(path).parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&pixels)?;
    println!("wrote {path} ({w}x{h})");
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut positional = Vec::new();
    let (mut count, mut seconds, mut scale, mut seed, mut sidecar) = (6_usize, 4.0_f64, 0.5_f64, 7_u64, None);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--frames" => count = value("--frames")?.parse()?,
            "--seconds" => seconds = value("--seconds")?.parse()?,
            "--scale" => scale = value("--scale")?.parse()?,
            "--seed" => seed = value("--seed")?.parse()?,
            "--sidecar" => sidecar = Some(value("--sidecar")?),
            other => positional.push(other.to_owned()),
        }
    }
    let input = positional.first().ok_or("usage: record_render INPUT OUT.png [options]")?;
    let output = positional.get(1).ok_or("usage: record_render INPUT OUT.png [options]")?;

    let project = Project::open(input)?;
    let model = project.model()?;
    let mut media = MediaStore::new();
    let (mut scene, cast) = if let Some(imported) = import_scene(&project, &model, None).ok().filter(|i| !i.puppets.is_empty()) {
        println!("imported scene {:?}: {} puppet tracks; {:?}", imported.scene.name, imported.scene.tracks.len(), imported.warnings);
        let (cast, warnings) = Cast::from_project(&project, &model, imported.puppets.iter().map(String::as_str));
        println!("cast: {:?}; {warnings:?}", cast.keys().collect::<Vec<_>>());
        println!("sound warnings: {:?}", load_sounds(&project, &imported, &mut media));
        (imported.scene, cast)
    } else {
        let path = model.root_puppets().next().ok_or("the input has no puppet")?.path.clone();
        let (cast, warnings) = Cast::from_project(&project, &model, [path.as_str()]);
        println!("cast: {:?}; {warnings:?}", cast.keys().collect::<Vec<_>>());
        let size = cast.get(&path).ok_or("the puppet could not be built")?.puppet.size;
        let mut scene = Scene::new("Demo", Size::new(size.width, size.height), FrameRate::FPS_24, Tick::from_seconds_f64(seconds));
        let mut track = PuppetTrack::new("Puppet", &path);
        track.placement.position = Vec2::ZERO;
        scene.add_track(track)?;
        (scene, cast)
    };
    scene.duration = Tick::from_seconds_f64(seconds);
    scene.seed = seed;

    // Drive the editor the way a tool client does: JSON commands on a session, undo history included.
    let mut doc = RecordDoc::new(scene);
    doc.templates = cast.templates();
    let mut session = Session::new(doc, registry()?);
    let fps = session.doc().scene.fps_f64();
    let total = (seconds * fps) as u32;
    let tracks = session.doc().scene.tracks.len();
    for track in 0..tracks {
        if session.doc().scene.tracks[track].puppet.contains("bg") || session.doc().scene.tracks[track].puppet.contains("desk") {
            continue;
        }
        session.execute("record.arm", &json!({ "groups": ["head", "brows", "voice"], "only": true }))?;
        let script = performance(track as f64 * 1.7, fps, total);
        session.execute("record.script", &json!({ "track": track, "script": script }))?;
        // A second, later take of only the voice over the middle third replaces the first take's mouth there.
        session.execute("record.arm", &json!({ "groups": ["voice"], "only": true }))?;
        let mut second = Script::new(fps, total / 3);
        for frame in (0..total / 3).step_by(4) {
            second = second.with(ScriptEvent::Voice { frame, viseme: Some(if (frame / 4) % 2 == 0 { Voice::Oh } else { Voice::Ee }) });
        }
        session.execute("record.script", &json!({ "track": track, "script": second, "start_seconds": seconds / 3.0 }))?;
        session.execute("take.set", &json!({ "track": track, "take": 1, "blend_in_seconds": 0.1, "blend_out_seconds": 0.1 }))?;
    }
    println!("{}", serde_json::to_string_pretty(&session.execute("scene.info", &json!({}))?)?);

    let scene = session.doc().scene.clone();
    let frames = SceneFrames::new(&scene, &cast)?;
    println!("{} frames at {} fps, {} not in the cast", frames.frame_count(), frames.fps(), frames.missing().len());
    let options = RasterOptions::new().with_scale(scale);
    let mut tiles = Vec::new();
    for i in 0..count {
        let at = (frames.frame_count() as usize - 1) * i / count.saturating_sub(1).max(1);
        tiles.push(render_frame(&frames, at as u32, &options)?);
    }
    write_sheet(output, &tiles, 3.min(count))?;
    if !media.is_empty() {
        let (mixed, missing) = mix_scene(&scene, &media, anim_audio::MixFormat::STEREO_48K)?;
        println!("soundtrack: {:.1} s, missing {missing:?}", mixed.seconds());
    }
    if let Some(path) = sidecar {
        store::save(Path::new(&path), &scene, &media)?;
        println!("wrote {path}");
    }
    Ok(())
}
