//! Prints every binding with its tags (and the behavior-defined tags) for a project or puppet file (exploration aid).
#![allow(clippy::print_stdout, clippy::unwrap_used, clippy::expect_used)]

use charcraft_project::Project;

fn main() {
    let path = std::env::args().nth(1).expect("path");
    let brief = std::env::args().nth(2).is_some();
    let project = Project::open(path).expect("open");
    let model = project.model().expect("model");
    for b in &model.behaviors {
        for t in &b.tags {
            println!("TAGDEF {} {} kind={:?} label={:?} art={:?} groups={:?}", b.id, t.id, t.kind, t.label, t.art_matches, t.groups);
        }
    }
    if brief {
        return;
    }
    for p in &model.puppets {
        println!("PUPPET {} ({} bindings) art={:?}", p.path, p.bindings.len(), p.artwork);
        for b in &p.bindings {
            let tags: Vec<&str> = b.tags.iter().map(|t| t.name.as_str()).collect();
            println!(
                "  {:?} {:?} name={:?} active={} tags={:?} key={:?} attach={:?} xf={:?}",
                b.kind, b.ppath, b.name, b.active, tags, b.key_trigger.key, b.attach_to, b.transform
            );
        }
        if let Some(o) = &p.origin {
            println!("  origin {} tags={:?} children={}", o.name, o.tags, o.children.len());
            for c in &o.children {
                println!("    handle {} loc={:?} tags={:?} children={}", c.name, c.location, c.tags, c.children.len());
            }
        }
        println!("  locations {:?}", p.tagged_locations);
    }
}
