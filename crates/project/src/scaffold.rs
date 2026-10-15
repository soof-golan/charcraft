//! Building a project from nothing: the repository boilerplate, the opening records and a minimal puppet.

use std::fmt::Write;

use charcraft_rope::{Commit, ObjectKind, Oid, Record, RefTarget, RopeStore};

use crate::classed::ClassedDoc;
use crate::error::Result;
use crate::package::Package;

/// The Character Animator build a new project claims to come from: project format 190, application 25.0.
pub const DEFAULT_BUILD: &str = "190 25.0";

pub const CONFIG: &str = "[core]\n\tbare = true\n\trepositoryformatversion = 0\n\tfilemode = false\n\tsymlinks = false\n\tignorecase = true\n";
pub const DESCRIPTION: &str = "Animal project repository";
pub const HOOKS_README: &str = "#!/bin/sh\n#\n# Place appropriately named executable hook scripts into this directory\n# to intercept various actions that git takes.  See `git help hooks` for\n# more information.\n";
pub const EXCLUDE: &str = "# File patterns to ignore; see `git help ignore` for more information.\n# Lines that start with '#' are comments.\n";
pub const REPO: &str = "Ch Data/repo.noindex/";

/// The branch name Character Animator creates for a build (`v190_25.0`).
#[must_use]
pub fn build_branch(build: &str) -> String {
    format!("refs/heads/v{}", build.replace(' ', "_"))
}

/// A GUID-shaped id derived from `seed` (FNV-1a, version-4 and variant bits set). Stable for a given seed, so a
/// scripted project is reproducible.
#[must_use]
pub fn make_guid(seed: &str) -> String {
    let mut state = 0xcbf2_9ce4_8422_2325_u64;
    let mut bytes = [0_u8; 16];
    for (round, chunk) in bytes.chunks_mut(8).enumerate() {
        for byte in seed.bytes().chain(round.to_le_bytes()) {
            state ^= u64::from(byte);
            state = state.wrapping_mul(0x0100_0000_01b3);
        }
        chunk.copy_from_slice(&state.to_be_bytes());
    }
    if let Some(b) = bytes.get_mut(6) {
        *b = (*b & 0x0f) | 0x40;
    }
    if let Some(b) = bytes.get_mut(8) {
        *b = (*b & 0x3f) | 0x80;
    }
    let hex = bytes.iter().fold(String::new(), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    });
    let part = |from: usize, to: usize| hex.get(from..to).unwrap_or_default().to_owned();
    format!("{}-{}-{}-{}-{}", part(0, 8), part(8, 12), part(12, 16), part(16, 20), part(20, 32))
}

/// The records every rope starts with: `HEAD` points at `main`, the empty tree, `.FIRST COMMIT FOR VERSION.`, the
/// `main` ref and the build's branch pointing at `HEAD`. Returns the first commit.
pub fn opening_records(store: &mut RopeStore, build: &str, time: i64, zone: &str) -> Result<Oid> {
    store.apply(Record::SetRef { name: "HEAD".to_owned(), target: RefTarget::Symbolic("refs/heads/main".to_owned()) })?;
    store.put_object(ObjectKind::Tree, Vec::new())?;
    let line = format!("user <red@monster> {time} {zone}");
    let commit = Commit { tree: Oid::EMPTY_TREE, parents: Vec::new(), author: line.clone(), committer: line, message: ".FIRST COMMIT FOR VERSION.".to_owned() };
    let id = store.put_object(ObjectKind::Commit, commit.to_bytes())?;
    store.set_ref("refs/heads/main", RefTarget::Direct(id));
    store.set_ref(&build_branch(build), RefTarget::Symbolic("HEAD".to_owned()));
    Ok(id)
}

/// The repository files, directories, media folders and (for a puppet) the marker of a new package.
pub fn fresh_package(name: &str, puppet: bool, first_commit: Oid, build: &str) -> Package {
    let mut package = Package::new();
    if puppet {
        package.set(&format!("{name}.tmp.chproj"), Vec::new());
    } else {
        package.set(&format!("{name}.chproj"), Vec::new());
    }
    package.add_dir("Ch Data");
    package.add_dir(REPO.trim_end_matches('/'));
    package.set(&format!("{REPO}config"), CONFIG.as_bytes().to_vec());
    package.set(&format!("{REPO}description"), DESCRIPTION.as_bytes().to_vec());
    package.set(&format!("{REPO}HEAD"), b"ref: refs/heads/main\n".to_vec());
    package.add_owned(&format!("{REPO}projectdata.rope"), false);
    package.set(&format!("{REPO}hooks/README.sample"), HOOKS_README.as_bytes().to_vec());
    package.set(&format!("{REPO}info/exclude"), EXCLUDE.as_bytes().to_vec());
    package.add_dir(&format!("{REPO}objects/info"));
    package.add_dir(&format!("{REPO}objects/pack"));
    package.set(&format!("{REPO}refs/heads/main"), format!("{first_commit}\n").into_bytes());
    package.set(&format!("{REPO}{}", build_branch(build)), b"ref: HEAD\n".to_vec());
    package.add_dir(&format!("{REPO}refs/tags"));
    if !puppet {
        package.set(&format!("{REPO}interprocess.lock"), Vec::new());
    }
    package.add_dir("Ch Media/Gathered Media");
    if puppet {
        package.add_owned("_archive_info", true);
    }
    package
}

/// The text of a minimal `Puppet` file with no layers, behaviors or triggers.
///
/// # Errors
///
/// Never in practice; the template is parsed and printed through [`ClassedDoc`] so the layout is Character
/// Animator's.
pub fn minimal_puppet(guid: &str, root: bool) -> Result<Vec<u8>> {
    let text = format!(
        "Puppet{{\n\t_aBoundBehaviors = IDArray_001{{\n\t\t_aIDArray = {{\n\t\t}},\n\t\t_lastID = 0,\n\t\t_sPrefix = \"BB\",\n\t}},\n\t_controlPanelModel = ControlPanelModel{{\n\t\taControls = {{\n\t\t}},\n\t\tbLayoutMode = false,\n\t\tbReflowMode = false,\n\t}},\n\t_defaultOriginLocTag = false,\n\t_idaBindings = IDArray_001{{\n\t\t_aIDArray = {{\n\t\t}},\n\t\t_lastID = 0,\n\t\t_sPrefix = \"B\",\n\t}},\n\t_idaHotTakes = IDArray_001{{\n\t\t_aIDArray = {{\n\t\t}},\n\t\t_lastID = 0,\n\t\t_sPrefix = \"HT\",\n\t}},\n\t_isRootInPuppetFile = {root},\n\t_oTriggerCollection = TriggerCollection{{\n\t}},\n\t_origin = Handle{{\n\t\t_aChildren = {{\n\t\t}},\n\t\t_bUserEditable = true,\n\t\t_locationTag = LocationTag{{\n\t\t\tbindingId_ = false,\n\t\t\tname_ = \"origin\",\n\t\t}},\n\t\t_manualProperties = \"\",\n\t\t_sName = \"origin\",\n\t\t_tTags = {{\n\t\t}},\n\t}},\n\t_originalTracking = OriginalLink{{\n\t\t_sItemID = \"\",\n\t}},\n\t_sGuid = \"{guid}\",\n\t_sUiName = false,\n\t_tOptions = {{\n\t\tcausewayWidth = 1,\n\t\texpansion = 0,\n\t\tminTriangleAreaHint = 4,\n\t\twarpWith = \"contour\",\n\t}},\n\t_tTaggedLocations = {{\n\t\torigin = {{\n\t\t}},\n\t}},\n\t_tTaggedPSLG = {{\n\t}},\n}}\n"
    );
    ClassedDoc::parse(text.as_bytes())?.to_bytes()
}

/// `ProjectUIData_001{ }`, the empty marker file Character Animator writes for UI state and folder data.
pub const EMPTY_UI_DATA: &[u8] = b"ProjectUIData_001{\n}\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids_are_stable_and_shaped_like_uuids() {
        let a = make_guid("Hero");
        assert_eq!(a, make_guid("Hero"));
        assert_ne!(a, make_guid("Villain"));
        assert_eq!(a.len(), 36);
        assert_eq!(a.matches('-').count(), 4);
        assert_eq!(a.chars().nth(14), Some('4'));
    }

    #[test]
    fn the_minimal_puppet_parses_and_prints_back() {
        let bytes = minimal_puppet("g-1", true).expect("builds");
        let doc = ClassedDoc::parse(&bytes).expect("parses");
        assert_eq!(doc.class(), Some("Puppet"));
        assert_eq!(doc.value.get_path_str("_sGuid"), Some("g-1"));
        assert_eq!(doc.to_bytes().expect("prints"), bytes);
    }

    #[test]
    fn build_branches_follow_the_sample_naming() {
        assert_eq!(build_branch("190 25.0"), "refs/heads/v190_25.0");
    }
}
