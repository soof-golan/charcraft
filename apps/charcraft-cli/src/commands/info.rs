//! `info`: what is in a puppet, project, PSD or AI file.

use std::fmt::Write as _;

use charcraft_project::ProjectSummary;
use charcraft_puppet::PuppetSummary;
use color_eyre::eyre::Result;
use serde::Serialize;

use crate::cli::InfoArgs;
use crate::source::{Kind, detect, load_puppet, open_project};

/// Everything `info` can report; absent parts are left out of the JSON.
#[derive(Debug, Serialize)]
pub struct InfoReport {
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub puppet: Option<PuppetSummary>,
    /// Scenes with their tracks, clips and behaviors (JSON only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scene_details: Vec<charcraft_project::Scene>,
    /// The project's top-level puppets with bindings, handles and behaviors (JSON only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub puppet_details: Vec<charcraft_project::PuppetItem>,
    /// Why the rig could not be built, when it could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub puppet_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<String>,
}

/// Collects the report.
pub fn report(args: &InfoArgs) -> Result<InfoReport> {
    let kind = detect(&args.input)?;
    let (project, scene_details, puppet_details) = match kind {
        Kind::Puppet | Kind::Project => {
            let opened = open_project(&args.input)?;
            let summary = opened.summary()?;
            if args.json {
                let model = opened.model()?;
                let puppets = model.puppets.iter().filter(|p| !p.is_part).cloned().collect();
                (Some(summary), model.scenes, puppets)
            } else {
                (Some(summary), Vec::new(), Vec::new())
            }
        }
        Kind::Artwork => (None, Vec::new(), Vec::new()),
    };
    let (puppet, outline, puppet_error) = match load_puppet(&args.input, None) {
        Ok(p) => (Some(p.summary()), args.outline.then(|| p.outline()), None),
        Err(e) => (None, None, Some(format!("{e:#}"))),
    };
    let kind = match kind {
        Kind::Puppet => "puppet",
        Kind::Project => "project",
        Kind::Artwork => "artwork",
    };
    Ok(InfoReport { kind, project, puppet, scene_details, puppet_details, puppet_error, outline })
}

/// Human-readable text.
pub fn render_text(report: &InfoReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "kind: {}", report.kind);
    if let Some(p) = &report.project {
        let _ = writeln!(out, "name: {}", p.name);
        if let Some(build) = &p.build {
            let _ = writeln!(out, "build: {build}");
        }
        let _ =
            writeln!(out, "puppets: {}  scenes: {}  media: {}  behaviors installed: {}", p.puppets.len(), p.scenes.len(), p.media.len(), p.behaviors_installed);
        for pup in p.puppets.iter().filter(|x| !x.is_part) {
            let _ = writeln!(
                out,
                "  puppet {} ({}): {} layers, {} triggers, behaviors: {}",
                pup.name,
                pup.path,
                pup.layers,
                pup.triggers,
                pup.behaviors.join(", ")
            );
        }
        for s in &p.scenes {
            let _ =
                writeln!(out, "  scene {:?}: {}x{} @ {} fps, {:.1} s, {} tracks, {} takes", s.name, s.width, s.height, s.fps, s.duration, s.tracks, s.takes);
        }
        for m in p.media.iter().filter(|m| m.kind != "Other") {
            let _ =
                writeln!(out, "  media {} {} {}", m.kind, m.file_name.as_deref().unwrap_or(&m.path), if m.in_package { "" } else { "(not in the package)" });
        }
        for w in p.problems.iter().chain(&p.warnings) {
            let _ = writeln!(out, "  warning: {w}");
        }
    }
    if let Some(p) = &report.puppet {
        let _ = writeln!(out, "rig: {}x{} px, {} layers, {} independent groups", p.size[0], p.size[1], p.layers, p.groups.len());
        let _ = writeln!(out, "  mouth groups: {}", if p.mouth_groups.is_empty() { "none".to_owned() } else { p.mouth_groups.join(", ") });
        let _ = writeln!(out, "  mouth shapes: {}", if p.visemes.is_empty() { "none".to_owned() } else { p.visemes.join(", ") });
        for g in &p.groups {
            let _ = writeln!(
                out,
                "  group {} [{}]{}{}",
                g.path,
                g.tags.join(", "),
                if g.handles.is_empty() { String::new() } else { format!(" handles: {}", g.handles.len()) },
                if g.warped { " warped" } else { "" }
            );
        }
        for w in &p.warnings {
            let _ = writeln!(out, "  warning: {w}");
        }
    }
    if let Some(e) = &report.puppet_error {
        let _ = writeln!(out, "no rig: {e}");
    }
    if let Some(o) = &report.outline {
        let _ = writeln!(out, "\nlayers:\n{o}");
    }
    out
}

/// Runs the command.
pub fn run(args: &InfoArgs) -> Result<()> {
    let report = report(args)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_text(&report));
    }
    Ok(())
}
