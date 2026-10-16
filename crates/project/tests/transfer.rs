//! Moving puppets between projects and saving a project as a `.puppet`: no sample files needed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "integration test helpers")]

use charcraft_project::{Project, ProjectKind};

#[test]
fn a_puppet_imports_into_a_project_and_exports_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut hero = Project::new_puppet("Hero").unwrap();
    hero.add_media("hero.png", vec![1, 2, 3]).unwrap();
    let mut show = Project::new_project("Show").unwrap();
    let added = show.import_from(&hero).unwrap();
    assert!(added.iter().any(|p| p == "ProjectItems/Hero"), "{added:?}");
    assert!(show.model().unwrap().puppet("ProjectItems/Hero").is_some());
    assert!(show.package().exists("Ch Media/Gathered Media/hero.png"));
    // Importing twice changes nothing.
    assert_eq!(show.import_from(&hero).unwrap(), Vec::<String>::new());

    let folder = dir.path().join("Show");
    show.save_folder(&folder).unwrap();
    let reopened = Project::open(&folder).unwrap();
    assert!(reopened.model().unwrap().puppet("ProjectItems/Hero").is_some());
    assert_eq!(reopened.package().get("Ch Media/Gathered Media/hero.png").map(charcraft_project::PackageEntry::size), Some(3));

    let mut again = Project::open(&folder).unwrap();
    again.make_puppet("ProjectItems/Hero").unwrap();
    assert_eq!(again.kind(), ProjectKind::Puppet);
    let file = dir.path().join("Hero.puppet");
    again.save_puppet(&file).unwrap();
    let back = Project::open(&file).unwrap();
    assert_eq!(back.kind(), ProjectKind::Puppet);
    assert_eq!(back.manifest().and_then(|m| m.root_puppet_ppath()), Some("/ProjectItems/Hero"));
    assert!(again.make_puppet("ProjectItems/Nobody").is_err());
}

#[test]
fn a_different_file_at_the_same_path_is_refused() {
    let mut a = Project::new_puppet("Same").unwrap();
    let mut b = Project::new_puppet("Same").unwrap();
    b.rename_puppet("ProjectItems/Same", "Changed").unwrap();
    assert!(a.import_from(&b).is_err(), "two different puppets cannot share a path");
}
