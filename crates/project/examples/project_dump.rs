//! Lists what is inside a Character Animator project or puppet.
//!
//! ```text
//! cargo run -p charcraft-project --example project_dump -- PATH [--json] [--tree PUPPET_PATH] [--save OUT]
//! ```
//!
//! `PATH` is a `.puppet` zip, a project folder, or the `.chproj` marker inside one. `--json` prints the summary a
//! tool would get. `--tree` prints a puppet's layer tree. `--save` writes the project back (a `.puppet` extension
//! makes a zip, anything else a folder).

use std::process::ExitCode;

use charcraft_project::{LayerNode, Project};

fn print_tree(node: &LayerNode, depth: usize) {
    let tags: Vec<&str> = node.tags.iter().map(|t| t.name.as_str()).collect();
    let tags = if tags.is_empty() { String::new() } else { format!("  [{}]", tags.join(", ")) };
    let key = node.trigger_key.as_deref().map_or_else(String::new, |k| format!("  key={k}"));
    println!("{}{}{}{}", "  ".repeat(depth), node.name, tags, key);
    for child in &node.children {
        print_tree(child, depth.saturating_add(1));
    }
}

#[allow(clippy::too_many_lines, reason = "a flat listing")]
fn run() -> Result<(), charcraft_project::Error> {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let mut json = false;
    let mut tree: Option<String> = None;
    let mut save: Option<String> = None;
    let mut reprint: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--reprint" => reprint = args.next(),
            "--tree" => tree = args.next(),
            "--save" => save = args.next(),
            _ => path = Some(arg),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: project_dump PATH [--json] [--tree PUPPET_PATH] [--save OUT]");
        return Ok(());
    };
    let mut project = Project::open(&path)?;
    if let Some(file) = reprint {
        let bytes = project.doc(&file)?.to_bytes()?;
        println!("{}", String::from_utf8_lossy(&bytes));
        return Ok(());
    }
    if let Some(out) = save {
        if out.ends_with(".puppet") {
            project.save_puppet(&out)?;
        } else {
            project.save_folder(&out)?;
        }
        println!("saved {out}");
        return Ok(());
    }
    let summary = project.summary()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&summary).unwrap_or_default());
        return Ok(());
    }
    let model = project.model()?;
    if let Some(root) = tree {
        match model.layer_tree(&root) {
            Some(node) => print_tree(&node, 0),
            None => eprintln!("no puppet at {root}"),
        }
        return Ok(());
    }
    println!("{:?} `{}`  build {}  {} log entries", summary.kind, summary.name, summary.build.as_deref().unwrap_or("?"), summary.store.log_len);
    for warning in &summary.warnings {
        println!("warning: {warning}");
    }
    if let Some(info) = project.manifest() {
        println!("root puppet {}  id {}", info.root_puppet_ppath().unwrap_or("?"), info.unique_id().unwrap_or("?"));
        for defaults in info.behavior_defaults() {
            let params: Vec<String> = defaults.params.iter().map(|(k, v)| format!("{k}={}", charcraft_ltab::to_string_compact(v))).collect();
            println!("  default {}: {}", defaults.behavior, params.join(" "));
        }
    }
    println!("\npuppets ({}):", model.puppets.len());
    for puppet in model.puppets.iter().filter(|p| !p.is_part) {
        println!("  {}  ({} layers, guid {})", puppet.path, puppet.bindings.len(), puppet.guid.as_deref().unwrap_or("?"));
        for behavior in &puppet.behaviors {
            let params: Vec<String> = behavior.params.iter().map(|(k, v)| format!("{k}={}", charcraft_ltab::to_string_compact(v))).collect();
            println!("    {} {}", behavior.behavior, params.join(" "));
        }
        for group in &puppet.trigger_groups {
            println!("    trigger group {} ({} triggers)", group.name.as_deref().unwrap_or(&group.id), group.triggers.len());
        }
    }
    println!("  + {} rig parts", model.puppets.iter().filter(|p| p.is_part).count());
    if !model.scenes.is_empty() {
        println!("\nscenes ({}):", model.scenes.len());
        for scene in &model.scenes {
            println!(
                "  {}  {}x{} @ {} fps, {:.2}s, {} tracks, {} markers",
                scene.name,
                scene.width,
                scene.height,
                scene.fps,
                scene.duration,
                scene.tracks.len(),
                scene.markers.len()
            );
            for track in &scene.tracks {
                let items: Vec<String> = track.items.iter().map(|i| i.source.clone().unwrap_or_default()).collect();
                println!("    {} {} {}", track.id, track.kind.as_deref().unwrap_or("?"), items.join(", "));
                for item in &track.items {
                    for behavior in &item.behaviors {
                        for take in &behavior.takes {
                            let at = take
                                .time
                                .as_ref()
                                .map_or_else(String::new, |t| format!(" at {:.3}s for {:.3}s trim {:.3}", t.parent_offset, t.duration, t.trim_in));
                            println!(
                                "      take {} {:?} {} param {:?} on {} ({}){} <- {}",
                                take.take_id,
                                take.name.as_deref().unwrap_or(""),
                                if take.enabled { "on" } else { "off" },
                                take.param.as_deref().unwrap_or(""),
                                behavior.behavior,
                                behavior.instance_id.as_deref().unwrap_or("?"),
                                at,
                                take.source.as_deref().unwrap_or("?")
                            );
                        }
                    }
                }
            }
        }
    }
    println!("\nmedia ({}):", model.media.len());
    for item in &model.media {
        println!(
            "  {:?} {}  file {} ({})",
            item.kind,
            item.path,
            item.file_name.as_deref().unwrap_or("?"),
            if item.file.is_some() { "in package" } else { "not in package" }
        );
    }
    println!("\nbehaviors installed: {}", model.behaviors.len());
    for behavior in model.behaviors.iter().take(8) {
        println!("  {}  {}  ({} parameters)", behavior.id, behavior.name.as_deref().unwrap_or(""), behavior.params.len());
    }
    if !model.problems.is_empty() {
        println!("\nproblems:");
        for problem in &model.problems {
            println!("  {problem}");
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
