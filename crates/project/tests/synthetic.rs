//! Round trips on projects built by this crate: no sample files needed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "integration test helpers"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use charcraft_ltab::LValue;
use charcraft_project::{Change, ClassedDoc, Error, LayerKind, Project, ProjectKind};
use proptest::prelude::*;

const HERO: &str = "Puppet{
	_aBoundBehaviors = IDArray_001{
		_aIDArray = {
			{
				id = \"BB1\",
				v = BoundBehavior{
					_bShowInTrackItemProperties = true,
					_behaviorPPath = PPath{
						p = \"/$Behaviors/Adobe.Blinker.js\",
					},
					_sparseParams = {
						rate = 45,
					},
					_uiName = \"Adobe.Blinker.js\",
				},
			},
			{
				id = \"BB2\",
				v = BoundBehavior{
					_bShowInTrackItemProperties = true,
					_behaviorPPath = PPath{
						p = \"/$Behaviors/Adobe.Dragger3.js\",
					},
					_sparseParams = {
					},
					_uiName = \"Adobe.Dragger3.js\",
				},
			},
		},
		_lastID = 2,
		_sPrefix = \"BB\",
	},
	_idaBindings = IDArray_001{
		_aIDArray = {
			{
				id = \"B1\",
				v = SkinBinding{
					_active = true,
					_name = \"Locations\",
					_opacity = 1,
					_ppath = PPath{
						p = \"/ProjectItems/.Parts/Hero/Hero.psd/Ch\",
					},
					_tTags = {
					},
				},
			},
			{
				id = \"B2\",
				v = SkinBinding{
					_active = false,
					_keyTrigger = BindingKeyTrigger{
						_bHideOthersWhenTriggered = false,
						_triggerKey = \"o\",
						_triggerLatch = 1,
					},
					_name = \"שלום\",
					_opacity = 0.5,
					_ppath = PPath{
						p = \"/ProjectItems/.Parts/Hero/Hero.psd/Ch/Mouth/Oh\",
					},
					_tTags = {
						[\"Adobe.LipSynch.Oh\"] = true,
						[\"Adobe.Face.Front\"] = false,
					},
				},
			},
		},
		_lastID = 2,
		_sPrefix = \"B\",
	},
	_isRootInPuppetFile = true,
	_origin = Handle{
		_aChildren = {
			Handle{
				_aChildren = {
				},
				_bUserEditable = true,
				_sName = \"Hand\",
				_tTags = {
					[\"Adobe.Dragger3.Draggable\"] = true,
				},
			},
		},
		_bUserEditable = true,
		_sName = \"origin\",
		_tTags = {
		},
	},
	_originatingSkinPPath0 = PPath{
		p = \"/ProjectItems/.Parts/Hero/Hero.psd/Ch\",
	},
	_sGuid = \"11111111-2222-4333-8444-555555555555\",
	_sUiName = false,
	_tOptions = {
		warpWith = \"contour\",
	},
	_tTaggedLocations = {
		origin = {
			1.5,
			-2,
		},
	},
}
";

fn hero_project() -> Project {
    let mut project = Project::new_puppet("Hero").expect("new puppet");
    project.commit_at(vec![Change::Put("ProjectItems/Hero".to_owned(), HERO.as_bytes().to_vec())], "Import", 1_700_000_000, "+0200").expect("commit");
    project
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
}

#[test]
fn a_new_puppet_has_the_standard_layout() {
    let project = Project::new_puppet("Hero").expect("new puppet");
    assert_eq!(project.kind(), ProjectKind::Puppet);
    let names: Vec<&str> = project.package().entries().iter().map(charcraft_project::PackageEntry::name).collect();
    for expected in [
        "Hero.tmp.chproj",
        "Ch Data/repo.noindex/config",
        "Ch Data/repo.noindex/HEAD",
        "Ch Data/repo.noindex/projectdata.rope",
        "Ch Data/repo.noindex/hooks/README.sample",
        "Ch Data/repo.noindex/info/exclude",
        "Ch Data/repo.noindex/refs/heads/main",
        "Ch Data/repo.noindex/refs/heads/v190_25.0",
        "Ch Media/Gathered Media/",
        "_archive_info",
    ] {
        assert!(names.contains(&expected), "{expected} missing from {names:?}");
    }
    let model = project.model().unwrap();
    assert_eq!(model.root_puppets().count(), 1);
    assert_eq!(project.manifest().and_then(|m| m.root_puppet_ppath()), Some("/ProjectItems/Hero"));
}

#[test]
fn a_puppet_round_trips_through_a_zip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Hero.puppet");
    let mut project = hero_project();
    project.save_puppet(&path).expect("saves");

    let again = Project::open(&path).expect("opens");
    assert_eq!(again.kind(), ProjectKind::Puppet);
    assert_eq!(again.name(), "Hero");
    assert!(again.warnings().is_empty(), "{:?}", again.warnings());
    assert_eq!(again.model().unwrap(), project.model().unwrap());
    assert_eq!(again.store().objects, project.store().objects);
    assert_eq!(again.store().refs, project.store().refs);

    let model = again.model().unwrap();
    let hero = model.puppet("ProjectItems/Hero").expect("hero");
    assert_eq!(hero.guid.as_deref(), Some("11111111-2222-4333-8444-555555555555"));
    assert_eq!(hero.behaviors.iter().map(|b| b.behavior.as_str()).collect::<Vec<_>>(), ["Adobe.Blinker.js", "Adobe.Dragger3.js"]);
    assert_eq!(hero.behavior("Adobe.Blinker.js").unwrap().params, vec![("rate".to_owned(), LValue::Int(45))]);
    let mouth = hero.binding("B2").expect("B2");
    assert_eq!(mouth.name.as_deref(), Some("שלום"));
    assert!(!mouth.active);
    assert!((mouth.opacity - 0.5).abs() < 1e-9);
    assert_eq!(mouth.key_trigger.key.as_deref(), Some("o"));
    assert_eq!(hero.tag_counts(), vec![("Adobe.LipSynch.Oh".to_owned(), 1), ("Adobe.Face.Front".to_owned(), 1)]);
    assert!(mouth.tags.iter().any(|t| t.name == "Adobe.LipSynch.Oh" && t.user_set));
    assert!(mouth.tags.iter().any(|t| t.name == "Adobe.Face.Front" && !t.user_set));
    let origin = hero.origin.as_ref().expect("origin");
    assert_eq!(origin.children.first().map(|h| h.name.as_str()), Some("Hand"));
    assert_eq!(hero.tagged_locations, vec![("origin".to_owned(), vec![1.5, -2.0])]);

    let tree = model.layer_tree("ProjectItems/Hero").expect("tree");
    assert_eq!(tree.kind, LayerKind::Puppet);
    assert_eq!(tree.children.len(), 1, "the self binding is not a child");
    assert_eq!(tree.children.first().map(|c| c.name.as_str()), Some("שלום"));
    assert_eq!(tree.with_tag("Adobe.LipSynch.Oh").len(), 1);
}

#[test]
fn saving_twice_without_changes_gives_identical_files() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("a.puppet");
    let second = dir.path().join("b.puppet");
    hero_project().save_puppet(&first).unwrap();
    let mut project = Project::open(&first).unwrap();
    project.save_puppet(&second).unwrap();
    assert_eq!(read(&first), read(&second));
}

#[test]
fn edits_keep_history_and_class_names() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Hero.puppet");
    hero_project().save_puppet(&path).unwrap();
    let mut project = Project::open(&path).unwrap();
    let history_before = project.store().history(project.store().head_commit().unwrap(), 100).unwrap().len();

    project.rename_puppet("ProjectItems/Hero", "Heroine").unwrap();
    project.set_behavior_param("ProjectItems/Hero", "Adobe.Dragger3.js", "range", LValue::Int(80)).unwrap();
    project.set_behavior_param("ProjectItems/Hero", "Adobe.Blinker.js", "rate", LValue::float(31.5)).unwrap();
    project.set_layer_tag("ProjectItems/Hero", "B2", "Adobe.LipSynch.Oh", false).unwrap();
    project.remove_layer_tag("ProjectItems/Hero", "B2", "Adobe.Face.Front").unwrap();
    project.set_layer_tag("ProjectItems/Hero", "B1", "Adobe.Face.Front", true).unwrap();
    assert!(project.remove_layer_tag("ProjectItems/Hero", "B2", "Adobe.Nothing").is_err());
    project.manifest_mut().unwrap().set_behavior_default("Adobe.Blinker.js", "rate", LValue::Int(40)).unwrap();
    assert!(project.set_behavior_param("ProjectItems/Hero", "Adobe.Walk.js", "speed", LValue::Int(1)).is_err());
    assert!(project.set_behavior_param("ProjectItems/Hero", "Adobe.Walk.js", "speed", LValue::empty_table()).is_err());
    assert!(project.set_layer_tag("ProjectItems/Hero", "B9", "x", true).is_err());
    let (head, main) = project.heads();
    assert_ne!(head, main, "main only moves on save");
    project.save().unwrap();
    assert_eq!(project.heads().0, project.heads().1);

    let again = Project::open(&path).unwrap();
    let model = again.model().unwrap();
    let hero = model.puppet("ProjectItems/Hero").unwrap();
    assert_eq!(hero.ui_name.as_deref(), Some("Heroine"));
    assert_eq!(hero.display_name(), "Heroine");
    assert_eq!(hero.behavior("Adobe.Dragger3.js").unwrap().params, vec![("range".to_owned(), LValue::Int(80))]);
    assert_eq!(hero.behavior("Adobe.Blinker.js").unwrap().params.first().and_then(|(_, v)| v.as_f64()), Some(31.5));
    assert_eq!(hero.tag_counts(), vec![("Adobe.Face.Front".to_owned(), 1), ("Adobe.LipSynch.Oh".to_owned(), 1)]);
    assert!(hero.binding("B1").unwrap().tags.iter().any(|t| t.name == "Adobe.Face.Front" && t.user_set));
    assert!(hero.binding("B2").unwrap().tags.iter().any(|t| t.name == "Adobe.LipSynch.Oh" && !t.user_set));
    assert_eq!(again.manifest().unwrap().behavior_default("Adobe.Blinker.js", "rate").and_then(LValue::as_i64), Some(40));
    let history_after = again.store().history(again.store().head_commit().unwrap(), 100).unwrap().len();
    assert_eq!(history_after, history_before + 6);

    let text = String::from_utf8(again.file("ProjectItems/Hero").unwrap().to_vec()).unwrap();
    assert!(text.starts_with("Puppet{\n"));
    assert!(text.contains("_aBoundBehaviors = IDArray_001{"));
    assert!(text.contains("v = SkinBinding{"));
    assert!(text.contains("שלום"));
    let doc = again.doc("ProjectItems/Hero").unwrap();
    assert_eq!(doc.to_bytes().unwrap(), text.as_bytes());
}

#[test]
fn appended_records_leave_the_old_rope_bytes_alone() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Show");
    let mut project = Project::new_project("Show").unwrap();
    project.save_folder(&root).unwrap();
    let rope_path = root.join("Ch Data/repo.noindex/projectdata.rope");
    let before = read(&rope_path);
    assert!(root.join("Show.chproj").is_file());

    let mut project = Project::open(root.join("Show.chproj")).unwrap();
    assert_eq!(project.kind(), ProjectKind::Folder);
    project.commit(vec![Change::Put("ProjectItems/x".to_owned(), b"Puppet{\n}\n".to_vec())], ".test").unwrap();
    project.save().unwrap();
    let after = read(&rope_path);
    assert!(after.len() > before.len());
    assert_eq!(&after[..before.len()], &before[..], "saving appends, it never rewrites");

    let again = Project::open(&root).unwrap();
    assert!(again.files().unwrap().contains_key("ProjectItems/x"));
    assert_eq!(again.store().objects, project.store().objects);
}

#[test]
fn a_torn_rope_tail_is_repaired_on_save() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Show");
    Project::new_project("Show").unwrap().save_folder(&root).unwrap();
    let rope_path = root.join("Ch Data/repo.noindex/projectdata.rope");
    let good = read(&rope_path);
    let mut torn = good.clone();
    torn.extend_from_slice(&good[..good.len() / 2]);
    torn.truncate(torn.len() - 3);
    fs::write(&rope_path, &torn).unwrap();

    let mut project = Project::open(&root).unwrap();
    assert_ne!(project.warnings().len(), 0);
    project.commit(vec![Change::Put("a".to_owned(), b"x".to_vec())], "after the crash").unwrap();
    project.save().unwrap();
    let again = Project::open(&root).unwrap();
    assert!(again.warnings().is_empty(), "{:?}", again.warnings());
    assert!(again.files().unwrap().contains_key("a"));
}

#[test]
fn media_files_survive_a_save_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Hero.puppet");
    let mut project = hero_project();
    let media: Vec<u8> = (0..=255_u8).cycle().take(300_000).collect();
    project.add_media("Hero.psd", media.clone()).unwrap();
    assert!(project.add_media("../evil", vec![1]).is_err());
    let skin = "Skin{\n\t_alias = Alias{\n\t\taliasJSON = \"{\\\"fullpath\\\":\\\"/Users/a/Hero.psd\\\"}\",\n\t\tbMissing = false,\n\t},\n\t_contentBounds = {\n\t\theight = 10,\n\t\tleft = 0,\n\t\ttop = 0,\n\t\twidth = 20,\n\t},\n\t_imageOversample = 2,\n}\n";
    project.commit(vec![Change::Put("ProjectItems/.Parts/Hero/Hero.psd".to_owned(), skin.as_bytes().to_vec())], "Gather").unwrap();
    project.save_puppet(&path).unwrap();

    let again = Project::open(&path).unwrap();
    assert_eq!(again.package().read("Ch Media/Gathered Media/Hero.psd").unwrap(), media);
    let model = again.model().unwrap();
    assert_eq!(model.media_files.len(), 1);
    let item = model.media.first().expect("skin item");
    assert_eq!(item.file_name.as_deref(), Some("Hero.psd"));
    assert_eq!(item.file.as_ref().map(|f| f.size), Some(300_000));
    assert_eq!(item.content_bounds.map(|b| b.width), Some(20.0));
    assert_eq!(item.oversample, Some(2));

    // A second save keeps the media byte for byte, even when it is written over the file it was read from.
    let mut again = again;
    again.save().unwrap();
    assert_eq!(Project::open(&path).unwrap().package().read("Ch Media/Gathered Media/Hero.psd").unwrap(), media);
}

#[test]
fn hostile_inputs_are_errors_not_panics() {
    let dir = tempfile::tempdir().unwrap();
    let junk = dir.path().join("junk.puppet");
    fs::write(&junk, b"definitely not a zip").unwrap();
    assert!(Project::open(&junk).is_err());
    assert!(Project::open(dir.path().join("missing.puppet")).is_err());
    let empty = dir.path().join("empty");
    fs::create_dir(&empty).unwrap();
    assert!(matches!(Project::open(&empty), Err(Error::NotAProject(_))));
    let mut project = hero_project();
    assert!(project.commit(vec![Change::Put("../escape".to_owned(), vec![1])], "x").is_err());
    assert!(project.commit(vec![Change::Put("a//b".to_owned(), vec![1])], "x").is_err());
    assert!(matches!(project.save(), Err(Error::Missing(_))));
}

#[test]
fn removing_a_folder_removes_everything_under_it() {
    let mut project = hero_project();
    project
        .commit(vec![Change::Put("ProjectItems/.Parts/a/b".to_owned(), b"x".to_vec()), Change::Put("ProjectItems/.Parts/a/c".to_owned(), b"y".to_vec())], "add")
        .unwrap();
    project.commit(vec![Change::Remove("ProjectItems/.Parts".to_owned())], "remove").unwrap();
    let files = project.files().unwrap();
    assert!(files.keys().all(|k| !k.starts_with("ProjectItems/.Parts")));
    assert!(files.contains_key("ProjectItems/Hero"));
}

#[test]
fn the_summary_serialises_for_tools() {
    let project = hero_project();
    let summary = project.summary().unwrap();
    let json = serde_json::to_value(&summary).unwrap();
    assert_eq!(json["kind"], "Puppet");
    assert_eq!(json["name"], "Hero");
    assert_eq!(json["root_puppet"], "/ProjectItems/Hero");
    assert_eq!(json["puppets"][0]["layers"], 2);
    assert_eq!(json["puppets"][0]["behaviors"][0], "Adobe.Blinker.js");
    assert_eq!(json["unsaved_commits"], true);
    let model = serde_json::to_value(project.model().unwrap()).unwrap();
    assert_eq!(model["puppets"][0]["behaviors"][1]["behavior"], "Adobe.Dragger3.js");
}

proptest! {
    #[test]
    fn random_bytes_never_panic_the_parser(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
        let _ = ClassedDoc::parse(&bytes);
    }

    #[test]
    fn printable_documents_reparse_to_themselves(
        class in "[A-Z][A-Za-z_0-9]{0,8}",
        keys in proptest::collection::btree_map("[a-z][A-Za-z0-9_]{0,6}", -1000_i64..1000, 0..6),
        text in "[ -~]{0,20}",
    ) {
        let mut body = String::new();
        for (key, value) in &keys {
            writeln!(body, "\t{key} = {value},").unwrap();
        }
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
        let source = format!("{class}{{\n{body}\ttext = \"{escaped}\",\n}}\n");
        let doc = ClassedDoc::parse(source.as_bytes()).unwrap();
        let printed = doc.to_bytes().unwrap();
        let again = ClassedDoc::parse(&printed).unwrap();
        prop_assert_eq!(again, doc);
    }
}
