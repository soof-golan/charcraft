//! The commands that bring puppets into the document and keep them up to date:
//!
//! * `puppet.reimport` reads the artwork file again and carries the rig over by layer identity,
//! * `puppet.edit_original` opens the artwork file in its own application,
//! * `puppet.make` builds a puppet with the Puppet Maker (from a template or from a spec),
//! * `puppet.templates` lists the starter puppets.
//!
//! All are ordinary registry commands, so the menus, the CLI and MCP share them and undo restores the document.
//! The pictures live in the [world](crate::world): commands hand puppets to it through [`Delivery`] and the hooks
//! move them in on their next tick.

use std::path::PathBuf;

use anim_session::{CommandError, CommandSpec, param_opt};
use charcraft_maker::Spec;
use charcraft_record::{Cast, CastMember};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::reimport::carry;
use crate::session::{AppDoc, Delivery, RigEdit, Skeleton};
use crate::world::{World, placement_track};

/// Longest puppet name accepted.
const MAX_NAME: usize = 64;

fn failed(e: impl std::fmt::Display) -> CommandError {
    CommandError::failed(e.to_string())
}

// --- re-import ---------------------------------------------------------------------------------------------------

/// A different file for a document that holds one artwork puppet: that puppet keeps its key.
fn keyed_like_the_document(doc: &AppDoc, fresh: Cast) -> Cast {
    let existing: Vec<&String> = doc.rig.skeletons.keys().filter(|k| !doc.rig.made.contains_key(*k)).collect();
    let fresh_keys: Vec<String> = fresh.keys().map(str::to_owned).collect();
    match (existing.as_slice(), fresh_keys.as_slice()) {
        ([old], [new]) if old.as_str() != new.as_str() => {
            let mut renamed = Cast::new();
            if let Some(member) = fresh.get(new) {
                renamed.insert(old, member.clone());
            }
            renamed
        }
        _ => fresh,
    }
}

fn puppet_reimport(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let path: Option<String> = param_opt(params, "path")?;
    let source = path
        .map(PathBuf::from)
        .or_else(|| doc.rig.source.clone())
        .ok_or_else(|| CommandError::failed("this document was not made from a file, so there is no artwork to read again (pass `path` to read a file)"))?;
    if !source.exists() {
        return Err(CommandError::failed(format!("{} does not exist", source.display())));
    }
    let opened = World::open(&source).map_err(failed)?;
    let fresh = keyed_like_the_document(doc, opened.world.cast);
    let keys: Vec<String> = fresh.keys().map(str::to_owned).collect();

    let mut skeletons = doc.rig.skeletons.clone();
    let mut edits: Vec<RigEdit> = doc.rig.edits.iter().filter(|e| !keys.contains(&e.puppet)).cloned().collect();
    let mut dropped = Vec::new();
    let mut puppets = Vec::new();
    for key in &keys {
        let Some(member) = fresh.get(key) else { continue };
        let new = Skeleton::of(&member.puppet);
        let carried = doc.rig.skeletons.get(key).map(|old| carry(&doc.rig.edits, key, old, &new));
        puppets.push(match &carried {
            Some(c) => json!({
                "puppet": key, "layers": new.layers.len(), "matched": c.matched, "removed": c.removed, "added": c.added,
                "edits_kept": c.edits.len(), "edits_dropped": c.dropped.len(),
            }),
            None => json!({ "puppet": key, "layers": new.layers.len(), "new": true }),
        });
        if let Some(c) = carried {
            edits.extend(c.edits);
            dropped.extend(c.dropped);
        }
        skeletons.insert(key.clone(), new);
    }
    if edits.len() > crate::session::MAX_RIG_EDITS {
        return Err(CommandError::failed("too many rig edits"));
    }
    let from = doc.rig.revision;
    let revision = from.saturating_add(1);
    doc.links
        .puppets
        .send(Delivery::Reimport { from, revision, cast: fresh, warnings: opened.world.warnings })
        .map_err(|_| CommandError::failed("the world is gone"))?;
    doc.rig.skeletons = skeletons;
    doc.rig.edits = edits;
    doc.rig.revision = revision;
    doc.rig.source = Some(source.clone());
    Ok(json!({ "source": source.display().to_string(), "revision": revision, "puppets": puppets, "dropped": dropped }))
}

fn puppet_edit_original(doc: &mut AppDoc, _: &Value) -> Result<Value, CommandError> {
    let source = doc
        .rig
        .source
        .clone()
        .ok_or_else(|| CommandError::failed("this document was not made from a file (the demo and Puppet Maker puppets have no original)"))?;
    if !source.exists() {
        return Err(CommandError::failed(format!("{} does not exist any more", source.display())));
    }
    doc.links.opens.send(source.clone()).map_err(|_| CommandError::failed("nothing can open files"))?;
    Ok(json!({ "opening": source.display().to_string() }))
}

// --- Puppet Maker ------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Make {
    template: Option<String>,
    spec: Option<Value>,
    name: Option<String>,
    add_track: bool,
}

impl Default for Make {
    fn default() -> Self {
        Self { template: None, spec: None, name: None, add_track: true }
    }
}

/// The spec a request asks for: a template (or the classic character) with the given fields laid over it.
fn requested_spec(make: &Make) -> Result<Spec, CommandError> {
    let base = match make.template.as_deref() {
        Some(id) => {
            charcraft_maker::template(id)
                .ok_or_else(|| {
                    let ids: Vec<&str> = charcraft_maker::templates().iter().map(|t| t.id).collect();
                    CommandError::invalid_params(format!("no template {id:?}; the templates are {ids:?}"))
                })?
                .spec
        }
        None => Spec::default(),
    };
    let mut value = serde_json::to_value(&base).map_err(failed)?;
    match (&make.spec, value.as_object_mut()) {
        (Some(Value::Object(patch)), Some(target)) => target.extend(patch.iter().map(|(k, v)| (k.clone(), v.clone()))),
        (Some(_), _) => return Err(CommandError::invalid_params("`spec` must be an object")),
        (None, _) => {}
    }
    let mut spec: Spec = serde_json::from_value(value).map_err(|e| CommandError::invalid_params(format!("not a character spec: {e}")))?;
    if let Some(name) = &make.name {
        spec.name.clone_from(name);
    }
    let name = spec.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(CommandError::invalid_params(format!("the name must be 1 to {MAX_NAME} characters")));
    }
    spec.name = name.to_owned();
    Ok(spec)
}

/// A cast key that no puppet of the document has: the name, then `Name 2`, `Name 3`, ...
fn free_key(doc: &AppDoc, name: &str) -> Result<String, CommandError> {
    if !doc.rig.skeletons.contains_key(name) {
        return Ok(name.to_owned());
    }
    (2_u32..1000)
        .map(|n| format!("{name} {n}"))
        .find(|k| !doc.rig.skeletons.contains_key(k))
        .ok_or_else(|| CommandError::failed("too many puppets with that name"))
}

fn puppet_make(doc: &mut AppDoc, params: &Value) -> Result<Value, CommandError> {
    let make: Make = serde_json::from_value(params.clone()).map_err(|e| CommandError::invalid_params(e.to_string()))?;
    let spec = requested_spec(&make)?;
    let key = free_key(doc, &spec.name)?;
    let puppet = charcraft_maker::puppet(&spec).map_err(failed)?;
    let member = CastMember::new(puppet);
    let track = if make.add_track {
        let index = doc.rec.scene.tracks.len();
        let track = placement_track(&spec.name, &key, &member, doc.rec.scene.size, index);
        Some(doc.rec.scene.add_track(track).map_err(failed)?)
    } else {
        None
    };
    doc.rig.skeletons.insert(key.clone(), Skeleton::of(&member.puppet));
    doc.rig.made.insert(key.clone(), spec.clone());
    doc.rec.templates.insert(key.clone(), member.template.clone());
    doc.links.puppets.send(Delivery::Made { key: key.clone(), member: Box::new(member) }).map_err(|_| CommandError::failed("the world is gone"))?;
    Ok(json!({ "puppet": key, "track": track, "spec": spec }))
}

#[allow(clippy::unnecessary_wraps, reason = "the command handler signature")]
fn puppet_templates(_: &mut AppDoc, _: &Value) -> Result<Value, CommandError> {
    let list: Vec<Value> =
        charcraft_maker::templates().into_iter().map(|t| json!({ "id": t.id, "title": t.title, "about": t.about, "spec": t.spec })).collect();
    Ok(json!({ "templates": list }))
}

macro_rules! spec_schema {
    () => {
        r#"{"type":"object","properties":{"name":{"type":"string"},"head":{"enum":["round","oval","square"]},"skin":{"type":"array","items":{"type":"integer","minimum":0,"maximum":255},"minItems":3,"maxItems":3},"hair_style":{"enum":["bald","short","fringe","long","bun"]},"hair":{"type":"array","items":{"type":"integer"},"minItems":3,"maxItems":3},"eyes":{"enum":["round","big","small"]},"iris":{"type":"array","items":{"type":"integer"},"minItems":3,"maxItems":3},"lashes":{"type":"boolean"},"ears":{"type":"boolean"},"cheeks":{"type":"boolean"},"lips":{"type":"array","items":{"type":"integer"},"minItems":3,"maxItems":3},"shirt":{"type":"array","items":{"type":"integer"},"minItems":3,"maxItems":3},"body":{"enum":["slim","regular","wide"]}},"additionalProperties":false}"#
    };
}

const MAKE_SCHEMA: &str = concat!(
    r#"{"type":"object","properties":{"template":{"type":"string","description":"A starter puppet (see puppet.templates): classic, round-pal, square-bot, long-hair, bun."},"spec":"#,
    spec_schema!(),
    r#","name":{"type":"string","description":"The puppet's name (its cast key)."},"add_track":{"type":"boolean","description":"Put the puppet on a new track of the scene (default true)."}},"additionalProperties":false}"#
);

/// The commands of this module.
#[must_use]
pub fn commands() -> Vec<CommandSpec<AppDoc>> {
    vec![
        CommandSpec::new("puppet.reimport", "Reimport artwork", puppet_reimport)
            .describe("Read the document's artwork file (PSD, AI, SVG, .puppet or .chproj) again, or the file in 'path', and rebuild the puppets from it, keeping the rig: every tag, handle, origin, trigger and swap set made so far follows its layer by path or, when the layer moved, by name. Edits whose layers are gone are dropped and listed. One undo step.")
            .with_schema(r#"{"type":"object","properties":{"path":{"type":"string","description":"Another file to read instead of the original (a new version of the artwork)."}},"additionalProperties":false}"#)
            .in_menu(&["Puppet"]),
        CommandSpec::new("puppet.edit_original", "Edit original", puppet_edit_original)
            .transient()
            .describe("Open the artwork file the puppet was made from in the application the system has for it (Photoshop, Illustrator, ...). Save there, then run puppet.reimport.")
            .with_schema(r#"{"type":"object","properties":{},"additionalProperties":false}"#)
            .in_menu(&["Puppet"]),
        CommandSpec::new("puppet.make", "Make a puppet", puppet_make)
            .describe("Build a rigged puppet with the Puppet Maker and add it to the document (and, by default, to a new scene track). Start from a 'template' and/or lay a partial 'spec' over it: head (round|oval|square), skin, hair_style (bald|short|fringe|long|bun), hair, eyes (round|big|small), iris, lashes, ears, cheeks, lips, shirt, body (slim|regular|wide); colours are [r,g,b]. The answer names the cast key and track. One undo step.")
            .with_schema(MAKE_SCHEMA)
            .in_menu(&["Puppet"]),
        CommandSpec::new("puppet.templates", "List starter puppets", puppet_templates)
            .transient()
            .describe("The starter puppets of the Puppet Maker with their specs.")
            .with_schema(r#"{"type":"object","properties":{},"additionalProperties":false}"#),
    ]
}

#[cfg(test)]
mod tests {
    use anim_session::Session;

    use super::*;
    use crate::live::LiveLink;
    use crate::session::{Links, registry};
    use crate::world::Opened;

    fn session_of(opened: Opened) -> (Session<AppDoc>, Links) {
        let (ui, _rx) = crossbeam_channel::unbounded();
        let links = Links::new(LiveLink::disconnected(), ui);
        let mut doc = AppDoc::new(opened.rec, opened.rig, links.clone());
        doc.rec.templates = opened.world.cast.templates();
        (Session::new(doc, registry().unwrap()), links)
    }

    #[test]
    fn making_a_puppet_adds_a_track_a_skeleton_and_one_undo_step() {
        let (mut session, links) = session_of(World::demo().unwrap());
        let done = session.execute("puppet.make", &json!({ "template": "square-bot", "spec": { "lashes": true } })).unwrap();
        assert_eq!(done["puppet"], "Square Bot");
        assert_eq!(done["track"], 1);
        let doc = session.doc();
        assert!(doc.rig.skeletons.contains_key("Square Bot") && doc.rig.made.contains_key("Square Bot"));
        assert!(doc.rec.templates.contains_key("Square Bot"));
        assert_eq!(doc.rig.made["Square Bot"].shirt, [92, 98, 112]);
        assert!(doc.rig.made["Square Bot"].lashes);
        assert!(matches!(links.puppets_in.try_recv(), Ok(Delivery::Made { key, .. }) if key == "Square Bot"));
        let again = session.execute("puppet.make", &json!({ "template": "square-bot" })).unwrap();
        assert_eq!(again["puppet"], "Square Bot 2", "keys stay unique");
        session.undo().unwrap();
        session.undo().unwrap();
        assert!(session.doc().rig.made.is_empty() && session.doc().rec.scene.tracks.len() == 1);
    }

    #[test]
    fn bad_requests_leave_the_document_alone() {
        let (mut session, links) = session_of(World::demo().unwrap());
        for params in
            [json!({ "template": "nope" }), json!({ "spec": { "head": "cube" } }), json!({ "spec": 3 }), json!({ "name": "  " }), json!({ "colour": 1 })]
        {
            assert!(session.execute("puppet.make", &params).is_err(), "{params}");
        }
        assert!(session.doc().rig.made.is_empty());
        assert!(links.puppets_in.try_recv().is_err());
        let list = session.execute("puppet.templates", &json!({})).unwrap();
        assert_eq!(list["templates"].as_array().map(Vec::len), Some(5));
    }

    #[test]
    fn the_demo_has_no_original_to_edit_or_reimport() {
        let (mut session, links) = session_of(World::demo().unwrap());
        assert!(session.execute("puppet.edit_original", &json!({})).is_err());
        assert!(session.execute("puppet.reimport", &json!({})).is_err());
        assert!(links.opens_in.try_recv().is_err());
    }

    #[test]
    fn edit_original_asks_for_the_artwork_file_to_be_opened() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hero.svg");
        std::fs::write(&file, SQUARE_SVG).unwrap();
        let (mut session, links) = session_of(World::open(&file).unwrap());
        let done = session.execute("puppet.edit_original", &json!({})).unwrap();
        assert_eq!(links.opens_in.try_recv().unwrap(), file);
        assert!(done["opening"].as_str().is_some_and(|s| s.ends_with("hero.svg")));
        std::fs::remove_file(&file).unwrap();
        assert!(session.execute("puppet.edit_original", &json!({})).is_err(), "the file is gone");
    }

    const SQUARE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><g id="Head"><g id="Left_Eye"><rect width="10" height="10" fill="#000"/></g></g><g id="Body"><rect y="50" width="100" height="50" fill="#444"/></g></svg>"##;

    #[test]
    fn reimport_keeps_the_rig_and_undo_restores_the_old_edits() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hero.svg");
        std::fs::write(&file, SQUARE_SVG).unwrap();
        let (mut session, links) = session_of(World::open(&file).unwrap());
        let key = session.doc().rig.skeletons.keys().next().cloned().unwrap();
        session.execute("rig.edit", &json!({ "op": "add_tag", "layer": "Body", "tag": "My.Tag" })).unwrap();
        // The artwork changes: the body is renamed (its edit has nowhere to go) and a new layer appears.
        std::fs::write(
            &file,
            SQUARE_SVG.replace("id=\"Body\"", "id=\"Torso\"").replace("</svg>", "<g id=\"Scarf\"><rect width=\"5\" height=\"5\"/></g></svg>"),
        )
        .unwrap();
        let done = session.execute("puppet.reimport", &json!({})).unwrap();
        assert_eq!(done["revision"], 1);
        assert_eq!(done["dropped"].as_array().map(Vec::len), Some(1), "{done}");
        assert!(session.doc().rig.edits.is_empty());
        assert!(session.doc().rig.skeletons[&key].layers.iter().any(|l| l.label == "Scarf"));
        match links.puppets_in.try_recv().unwrap() {
            Delivery::Reimport { from, revision, cast, .. } => {
                assert_eq!((from, revision), (0, 1));
                assert!(cast.get(&key).is_some());
            }
            Delivery::Made { .. } => panic!("expected a re-import"),
        }
        session.undo().unwrap();
        assert_eq!(session.doc().rig.revision, 0);
        assert_eq!(session.doc().rig.edits.len(), 1, "the edit is back with the old artwork");
        assert!(!session.doc().rig.skeletons[&key].layers.iter().any(|l| l.label == "Scarf"));
    }

    #[test]
    fn the_world_follows_reimport_undo_redo_and_made_puppets() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hero.svg");
        std::fs::write(&file, SQUARE_SVG).unwrap();
        let opened = World::open(&file).unwrap();
        let (ui, _rx) = crossbeam_channel::unbounded();
        let links = Links::new(LiveLink::disconnected(), ui);
        let mut doc = AppDoc::new(opened.rec, opened.rig, links.clone());
        doc.rec.templates = opened.world.cast.templates();
        let mut world = opened.world;
        let mut session = Session::new(doc, registry().unwrap());
        let has = |w: &World, label: &str| w.cast.keys().any(|k| w.cast.get(k).is_some_and(|m| m.puppet.find_label(label).is_some()));
        let pump = |world: &mut World, session: &Session<AppDoc>| {
            while let Ok(d) = links.puppets_in.try_recv() {
                world.deliver(d);
            }
            world.sync(&session.doc().rig)
        };

        assert!(!world.check_source(), "nothing changed yet");
        std::fs::write(&file, SQUARE_SVG.replace("</svg>", "<g id=\"Scarf\"><rect width=\"5\" height=\"5\"/></g></svg>")).unwrap();
        // A file system with coarse timestamps may not show the change; the check is only meaningful when it does.
        let _ = world.check_source();
        session.execute("puppet.reimport", &json!({})).unwrap();
        assert!(pump(&mut world, &session));
        assert!(has(&world, "Scarf") && !world.stale);
        session.undo().unwrap();
        assert!(pump(&mut world, &session));
        assert!(!has(&world, "Scarf"), "undo brings the old artwork back");
        session.redo().unwrap();
        assert!(pump(&mut world, &session));
        assert!(has(&world, "Scarf"));

        session.execute("puppet.make", &json!({ "template": "bun" })).unwrap();
        assert!(pump(&mut world, &session));
        assert!(world.cast.get("Bun").is_some());
        assert!(session.doc().rec.templates.contains_key("Bun") || world.cast.templates().contains_key("Bun"));
        session.undo().unwrap();
        assert!(pump(&mut world, &session));
        assert!(world.cast.get("Bun").is_none(), "undo takes the made puppet out of the cast");
        session.redo().unwrap();
        assert!(pump(&mut world, &session));
        assert!(world.cast.get("Bun").is_some());
        assert!(world.warning_rows().iter().all(|r| r.puppet.as_deref() != Some("Bun")), "a made puppet is a tidy rig");
    }

    #[test]
    fn made_puppets_and_the_revision_survive_save_and_open() {
        let opened = World::from_template("round-pal").unwrap();
        assert_eq!(opened.world.kind, crate::world::SourceKind::Made);
        let mut rig = opened.rig.clone();
        rig.revision = 0;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("pal.charcraft");
        std::fs::write(&file, opened.world.save_bytes(&opened.rec, &rig).unwrap()).unwrap();
        let back = World::open(&file).unwrap();
        assert_eq!(back.rig.made, opened.rig.made);
        assert!(back.world.cast.get("Round Pal").is_some() && back.world.cast.len() == 1, "no demo puppet sneaks in");
        assert_eq!(back.rec.scene.tracks.len(), 1);
        assert!(back.rec.templates.contains_key("Round Pal"));
    }
}
