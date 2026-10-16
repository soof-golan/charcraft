//! `export`: a scene of a project as a video or a PNG sequence.
//!
//! The scene is opened the way the app opens it ([`charcraft_app::world::World`]): the project's recorded takes
//! play, and puppets whose scene has no recorded voice are lip-synced to the scene's sound. Frames come from the
//! same evaluator and compositor as the app's own export.

use anim_audio::MixFormat;
use anim_core::{Rgba8, Tick};
use anim_render::Renderer;
use charcraft_app::compose::Stage;
use charcraft_app::world::World;
use charcraft_record::{Evaluator, TakePolicy, mix_scene, voice_from_audio};
use color_eyre::eyre::{Result, bail, eyre};

use crate::cli::ExportArgs;
use crate::video::{Format, Plan, audio_track};

/// Runs the command.
pub fn run(args: &ExportArgs) -> Result<()> {
    if !args.start.is_finite() || args.start < 0.0 {
        bail!("--start must be zero or more, got {}", args.start);
    }
    if let Some(seconds) = args.seconds
        && (!seconds.is_finite() || seconds <= 0.0)
    {
        bail!("--seconds must be positive, got {seconds}");
    }
    if !args.scale.is_finite() || args.scale <= 0.0 || args.scale > 8.0 {
        bail!("--scale must be between 0 and 8, got {}", args.scale);
    }
    let opened = World::open_scene(&args.project, args.scene.as_deref(), TakePolicy::Perform).map_err(|e| {
        let scenes =
            crate::source::open_project(&args.project).and_then(|p| Ok(p.model()?)).map(|m| m.scenes.iter().map(|s| s.name.clone()).collect::<Vec<_>>());
        match scenes {
            Ok(names) if args.scene.is_some() => eyre!("{}; the project has the scenes: {}", e.0, names.join(", ")),
            _ => eyre!("{}", e.0),
        }
    })?;
    for warning in &opened.world.warnings {
        eprintln!("warning: {warning}");
    }
    let mut scene = opened.rec.scene;
    let media = opened.world.media;
    scene.seed = args.seed;
    let format = Format::of(&args.output);
    let transparent = format.transparent();
    scene.background = (!transparent).then_some(Rgba8::WHITE);
    // Puppets with no recorded voice speak the scene's sound.
    let spoken = voice_from_audio(&mut scene, &media);
    let stage = Stage::new(opened.world.cast);
    if scene.tracks.iter().all(|t| !t.enabled || stage.cast().get(&t.puppet).is_none()) {
        bail!("scene {:?} has no puppets that can be drawn", scene.name);
    }

    let start = Tick::from_seconds_f64(args.start);
    let mut length = Tick(scene.duration.0.saturating_sub(start.0));
    if let Some(seconds) = args.seconds {
        length = length.min(Tick::from_seconds_f64(seconds));
    }
    let pixels = |extent: f64| {
        let n = anim_core::num::round_f64_to_u32((extent * args.scale).max(1.0));
        if format == Format::PngSequence { n } else { n.saturating_add(n % 2) }
    };
    let plan = Plan::new(&args.output, pixels(scene.size.width), pixels(scene.size.height), scene.fps_f64(), length)?;
    let first_frame = scene.frame_at(start);

    let mut evaluator = Evaluator::new(&scene, &stage.cast().templates())?;
    let mut renderer = Renderer::new();
    eprintln!(
        "scene {:?}: {} puppets, {} recorded takes, {} lip-synced to the sound, {}x{} at {} fps, {:.1} s",
        scene.name,
        scene.tracks.len(),
        scene.tracks.iter().map(|t| t.takes.len()).sum::<usize>(),
        spoken,
        plan.width,
        plan.height,
        scene.fps_f64(),
        length.seconds()
    );

    // The scene's sound as heard in the exported part.
    let has_sound = scene.audio.iter().filter(|t| t.enabled).flat_map(|t| &t.clips).any(|c| c.enabled && media.get(&c.media).is_some());
    let soundtrack = if has_sound {
        let (mixed, _missing) = mix_scene(&scene, &media, MixFormat::STEREO_48K)?;
        Some(audio_track(&mixed, start, length)?)
    } else {
        None
    };

    let summary = crate::video::write(&args.output, &plan, soundtrack, |index, _| {
        let poses = evaluator.poses(first_frame.saturating_add(index));
        stage.render_scene_image(&mut renderer, &scene, &poses, args.scale, Some((plan.width, plan.height)), transparent).map_err(|e| eyre!("{e}"))
    })?;
    println!("{summary}");
    Ok(())
}
