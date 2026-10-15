//! Prints what is inside a Character Animator project store.
//!
//! ```text
//! cargo run -p charcraft-rope --example rope_dump -- PATH [--json] [--commits N] [--cat FILE]
//! ```
//!
//! `PATH` is a `.puppet` zip, a project or extracted-puppet folder, a `.chproj` marker file, a `repo.noindex`
//! folder or a bare `projectdata.rope`. `--json` prints the store summary as JSON for tools.

use std::process::ExitCode;

use charcraft_rope::{Oid, RefTarget, RopeStore, class_name};

fn short(id: &Oid) -> String {
    id.to_hex().chars().take(10).collect()
}

fn run() -> Result<(), charcraft_rope::Error> {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let mut json = false;
    let mut commits = 12_usize;
    let mut cat: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--commits" => commits = args.next().and_then(|n| n.parse().ok()).unwrap_or(commits),
            "--cat" => cat = args.next(),
            _ => path = Some(arg),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: rope_dump PATH [--json] [--commits N]");
        return Ok(());
    };
    let store = RopeStore::open(&path)?;
    if let Some(file) = cat {
        let files = store.head_tree()?;
        match files.get(file.as_str()) {
            Some(bytes) => {
                use std::io::Write as _;
                let _ = std::io::stdout().write_all(bytes);
            }
            None => eprintln!("no file `{file}` at HEAD"),
        }
        return Ok(());
    }
    if json {
        match serde_json::to_string_pretty(&store.summary()) {
            Ok(text) => println!("{text}"),
            Err(error) => eprintln!("could not serialise the summary: {error}"),
        }
        return Ok(());
    }

    let summary = store.summary();
    println!("store: {path}");
    println!("log entries: {}", summary.log_len);
    for (kind, count) in &summary.objects {
        println!("  {count:>6} {kind} objects");
    }
    for warning in &store.warnings {
        println!("warning: {warning}");
    }

    println!("\nrefs ({}):", store.refs.len());
    for (name, target) in store.refs.iter().filter(|(n, _)| !n.starts_with("refs/tags/")) {
        match target {
            RefTarget::Direct(id) => println!("  {name} = {id}"),
            RefTarget::Symbolic(to) => println!("  {name} -> {to}"),
        }
    }
    let tag_count = store.refs.keys().filter(|n| n.starts_with("refs/tags/")).count();
    if tag_count > 0 {
        println!("  ({tag_count} refs under refs/tags/)");
    }

    let head = store.head_commit()?;
    let history = store.history(head, usize::MAX)?;
    println!("\nhistory of HEAD: {} commits, newest first", history.len());
    for (id, commit) in history.iter().take(commits) {
        let message = commit.message.lines().next().unwrap_or_default();
        println!("  {} {} {message}", short(id), commit.committer);
    }
    if history.len() > commits {
        println!("  ...");
    }

    if let Some(tag) = store.refs.iter().find_map(|(name, target)| match target {
        RefTarget::Direct(id) if name.starts_with("refs/tags/") => store.tag(id).ok().map(|t| (name, t)),
        _ => None,
    }) {
        println!("\nexample tag {}: object {} ({})", tag.0, short(&tag.1.object), tag.1.kind.name());
        for line in tag.1.message.lines().take(8) {
            println!("  | {line}");
        }
    }

    println!("\nfiles at HEAD:");
    let files = store.list_files(head)?;
    let mut total = 0_u64;
    for file in &files {
        let size = file.size.unwrap_or(0);
        total = total.saturating_add(size);
        let class = store.object(&file.id).and_then(|o| class_name(&o.data)).map(|c| format!("  [{c}]")).unwrap_or_default();
        println!("  {size:>9}  {:o}  {}{class}", file.mode, file.path);
    }
    println!("{} files, {total} bytes", files.len());
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
