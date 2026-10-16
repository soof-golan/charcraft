//! Behaviors against the sample puppets. Skipped unless `ANICRAFT_SAMPLES` points at the sample folder.
#![allow(
    clippy::float_cmp,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::redundant_clone,
    clippy::default_trait_access,
    reason = "test code"
)]

use std::path::PathBuf;

use anim_core::Point;
use charcraft_behaviors::{BehaviorStack, FaceInput, Rig, Script, ScriptEvent, behaviors, simulate};
use charcraft_project::Project;
use charcraft_puppet::{HandleKind, Puppet, PuppetPose};

fn samples() -> Option<PathBuf> {
    std::env::var_os("ANICRAFT_SAMPLES").map(PathBuf::from)
}

#[test]
fn every_stored_default_is_a_parameter_with_the_same_name_and_value() {
    let Some(root) = samples() else { return };
    for file in ["Boy.puppet", "blood_cells.puppet"] {
        let project = Project::open(root.join(file)).unwrap();
        let defaults = project.manifest().expect("a .puppet has a manifest").behavior_defaults();
        assert!(defaults.len() >= 8, "{file}: {} behaviors", defaults.len());
        let mut checked = 0;
        for d in &defaults {
            let Ok(b) = behaviors::create_from_pairs(&d.behavior, &d.params) else {
                println!("{file}: {} is not implemented", d.behavior);
                continue;
            };
            let params = b.params();
            for (name, value) in &d.params {
                let ours = params.get(name).unwrap_or_else(|| panic!("{file}: {} has no parameter {name}", d.behavior));
                let theirs = match value {
                    charcraft_ltab::LValue::Bool(v) => serde_json::json!(*v),
                    other => serde_json::json!(other.as_f64().unwrap_or(f64::NAN)),
                };
                assert!(
                    ours == &theirs || (ours.as_f64().zip(theirs.as_f64()).is_some_and(|(a, b)| (a - b).abs() < 1e-9)),
                    "{file}: {}.{name}: {ours} vs {theirs}",
                    d.behavior
                );
                checked += 1;
            }
        }
        assert!(checked > 60, "{file}: only {checked} parameters checked");
    }
}

#[test]
fn the_boy_is_driven_by_his_own_behaviors() {
    let Some(root) = samples() else { return };
    let project = Project::open(root.join("Boy.puppet")).unwrap();
    let puppet = Puppet::from_project(&project, None).unwrap();
    let model = project.model().unwrap();
    let item = model.root_puppets().next().unwrap();
    let rig = Rig::from_puppet(&puppet);
    assert!(rig.handles_of(HandleKind::Draggable).count() >= 2 && rig.handles_of(HandleKind::Dangle).count() >= 1, "{:?}", rig.handles);

    let defaults = project.manifest().unwrap().behavior_defaults();
    let (mut stack, skipped) = BehaviorStack::from_bound(rig.clone(), &item.behaviors, &defaults);
    println!("boy behaviors: {:?}, skipped {skipped:?}", stack.slots().iter().map(|s| s.behavior.id()).collect::<Vec<_>>());
    assert!(stack.slots().len() >= 5);

    // Pull the first draggable handle, tilt the head and speak.
    let hand = rig.handles_of(HandleKind::Draggable).next().unwrap().clone();
    let at: Point = hand.driven_position(&PuppetPose::neutral());
    let script = Script::new(30.0, 90)
        .with(ScriptEvent::Face { frame: 0, face: FaceInput { head_roll: 12.0, mouth_open: 0.6, ..FaceInput::default() } })
        .with(ScriptEvent::Voice { frame: 10, viseme: Some(anim_lipsync::Viseme::Oh) })
        .with(ScriptEvent::Pointer { frame: 20, id: 0, x: at.x, y: at.y, down: true })
        .with(ScriptEvent::Pointer { frame: 40, id: 0, x: at.x + 40.0, y: at.y - 60.0, down: true });
    let frames = simulate(&mut stack, &script, 1).unwrap();
    let late = &frames[50].pose;
    assert!(late.drag_of(&hand.key).hypot() > 30.0, "{:?}", late.drag);
    assert!(late.head.rotation.abs() > 0.01);
    assert_eq!(frames[20].pose.viseme, Some(charcraft_puppet::Viseme::Oh));
    let neutral = format!("{:?}", puppet.evaluate(&PuppetPose::neutral()));
    assert_ne!(format!("{:?}", puppet.evaluate(late)), neutral);

    // The standard stack finds the same handles.
    let mut standard = BehaviorStack::standard(rig);
    let pose = simulate(&mut standard, &script, 1).unwrap()[50].pose.clone();
    assert!(pose.drag_of(&hand.key).hypot() > 30.0);
}

#[test]
fn the_news_anchor_project_has_its_dragger_and_face_defaults() {
    let Some(root) = samples() else { return };
    let project = Project::open(root.join("chproj_full/news anchors")).unwrap();
    let model = project.model().unwrap();
    let mut built = 0;
    for item in model.root_puppets() {
        let Ok(puppet) = Puppet::from_project(&project, Some(&item.path)) else { continue };
        let (stack, skipped) = BehaviorStack::from_bound(Rig::from_puppet(&puppet), &item.behaviors, &[]);
        println!("{}: {:?}, skipped {skipped:?}", item.path, stack.slots().iter().map(|s| s.behavior.name()).collect::<Vec<_>>());
        built += 1;
    }
    assert!(built >= 1);
}
