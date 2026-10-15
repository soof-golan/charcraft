//! Telling input files apart and loading puppets from them.

use std::path::Path;

use anim_artwork::Limits;
use charcraft_project::Project;
use charcraft_puppet::Puppet;
use color_eyre::eyre::{Result, WrapErr, bail};

/// What an input path holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A `.puppet` zip.
    Puppet,
    /// A `.chproj` marker or the project folder.
    Project,
    /// A PSD, AI, SVG or other artwork file.
    Artwork,
}

/// Decides by extension (and by being a directory).
pub fn detect(path: &Path) -> Result<Kind> {
    if path.is_dir() {
        return Ok(Kind::Project);
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default();
    match ext.as_str() {
        "puppet" => Ok(Kind::Puppet),
        "chproj" => Ok(Kind::Project),
        "psd" | "psb" | "ai" | "svg" | "pdf" | "eps" => Ok(Kind::Artwork),
        _ => bail!("{}: not a .puppet, .chproj, .psd or .ai file", path.display()),
    }
}

/// Opens a puppet file or project.
pub fn open_project(path: &Path) -> Result<Project> {
    Project::open(path).wrap_err_with(|| format!("opening {}", path.display()))
}

/// Builds the puppet of any input. `puppet_path` picks one puppet of a project.
pub fn load_puppet(path: &Path, puppet_path: Option<&str>) -> Result<Puppet> {
    match detect(path)? {
        Kind::Puppet | Kind::Project => {
            let project = open_project(path)?;
            if puppet_path.is_some() {
                return Puppet::from_project(&project, puppet_path).wrap_err_with(|| format!("building the puppet of {}", path.display()));
            }
            // A project holds backgrounds and props next to its characters: take the first item that builds.
            let model = project.model()?;
            let mut first_error = None;
            for item in model.root_puppets() {
                match Puppet::from_project(&project, Some(&item.path)) {
                    Ok(puppet) => return Ok(puppet),
                    Err(e) => {
                        first_error.get_or_insert_with(|| format!("{}: {e}", item.path));
                    }
                }
            }
            bail!("{} has no puppet that can be built ({})", path.display(), first_error.unwrap_or_else(|| "it has none".to_owned()))
        }
        Kind::Artwork => {
            let tree = anim_artwork::import_path(path, &Limits::default()).wrap_err_with(|| format!("reading artwork {}", path.display()))?;
            Puppet::from_artwork(&tree).wrap_err_with(|| format!("building a puppet from {}", path.display()))
        }
    }
}
