//! `_archive_info`: the manifest at the root of every `.puppet` zip.

use charcraft_ltab::{LValue, parse_bytes, to_string_pretty};
use serde::Serialize;

use crate::error::{Error, Result};

/// The default parameter values of one stock behavior, as stored in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BehaviorDefaults {
    /// The behavior id (`Adobe.Blinker.js`).
    pub behavior: String,
    /// Parameter id and value, in file order.
    pub params: Vec<(String, LValue)>,
}

/// The parsed `_archive_info` of a `.puppet`.
///
/// Anything the typed accessors do not cover stays in [`ArchiveInfo::value`]. A manifest nobody edited is written
/// back byte for byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveInfo {
    /// The whole table. Edit it freely and call [`ArchiveInfo::mark_edited`] (the setters do that themselves).
    pub value: LValue,
    original: Vec<u8>,
    edited: bool,
}

impl ArchiveInfo {
    /// Parses the manifest text.
    ///
    /// # Errors
    ///
    /// [`Error::Ltab`] for invalid text, [`Error::Malformed`] when the root is not a table.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let value = parse_bytes(bytes)?;
        if value.as_table().is_none() {
            return Err(Error::malformed("_archive_info", "the manifest is not a table"));
        }
        Ok(Self { value, original: bytes.to_vec(), edited: false })
    }

    /// A manifest for a new puppet whose root puppet lives at `root_puppet_ppath` (`/ProjectItems/Name`).
    #[must_use]
    pub fn new(root_puppet_ppath: &str, unique_id: &str) -> Self {
        let mut value = LValue::empty_table();
        let entries = [
            ("bContainsArtworkCache", LValue::Bool(false)),
            ("iVersion", LValue::Int(1)),
            ("rootPuppetPPath", LValue::string(root_puppet_ppath)),
            ("sType", LValue::string("puppet")),
            ("sUniqueID", LValue::string(unique_id)),
            ("tBehaviorDefaults", LValue::empty_table()),
        ];
        for (key, entry) in entries {
            // Setting a key on a table cannot fail.
            let _ = value.set(key, entry);
        }
        Self { value, original: Vec::new(), edited: true }
    }

    /// `iVersion`.
    #[must_use]
    pub fn version(&self) -> Option<i64> {
        self.value.get_path_i64("iVersion")
    }

    /// `rootPuppetPPath`, the project path of the puppet the file contains.
    #[must_use]
    pub fn root_puppet_ppath(&self) -> Option<&str> {
        self.value.get_path_str("rootPuppetPPath")
    }

    /// `sType` (`"puppet"`).
    #[must_use]
    pub fn kind(&self) -> Option<&str> {
        self.value.get_path_str("sType")
    }

    /// `sUniqueID`.
    #[must_use]
    pub fn unique_id(&self) -> Option<&str> {
        self.value.get_path_str("sUniqueID")
    }

    /// `bContainsArtworkCache`.
    #[must_use]
    pub fn contains_artwork_cache(&self) -> bool {
        self.value.get_path_bool("bContainsArtworkCache").unwrap_or(false)
    }

    /// The stock behaviors' default parameter values.
    #[must_use]
    pub fn behavior_defaults(&self) -> Vec<BehaviorDefaults> {
        let Some(table) = self.value.get("tBehaviorDefaults") else {
            return Vec::new();
        };
        table
            .named()
            .map(|(behavior, params)| BehaviorDefaults {
                behavior: behavior.to_owned(),
                params: params.named().map(|(name, value)| (name.to_owned(), value.clone())).collect(),
            })
            .collect()
    }

    /// One default value.
    #[must_use]
    pub fn behavior_default(&self, behavior: &str, param: &str) -> Option<&LValue> {
        self.value.get("tBehaviorDefaults")?.get(behavior)?.get(param)
    }

    /// Sets the default of one parameter, adding the behavior when it is not listed.
    ///
    /// # Errors
    ///
    /// [`Error::Malformed`] when `tBehaviorDefaults` exists but is not a table.
    pub fn set_behavior_default(&mut self, behavior: &str, param: &str, value: LValue) -> Result<()> {
        if self.value.get("tBehaviorDefaults").is_none() {
            self.value.set("tBehaviorDefaults", LValue::empty_table())?;
        }
        let defaults = self.value.get_mut("tBehaviorDefaults").ok_or_else(|| Error::malformed("_archive_info", "tBehaviorDefaults is missing"))?;
        if defaults.as_table().is_none() {
            return Err(Error::malformed("_archive_info", "tBehaviorDefaults is not a table"));
        }
        if defaults.get(behavior).is_none() {
            defaults.set(behavior, LValue::empty_table())?;
        }
        let table = defaults.get_mut(behavior).ok_or_else(|| Error::malformed("_archive_info", "behavior table is missing"))?;
        table.set(param, value)?;
        self.value.sort_keys();
        self.edited = true;
        Ok(())
    }

    /// Changes `rootPuppetPPath`.
    ///
    /// # Errors
    ///
    /// Never fails for a parsed manifest; the error is for tables that lost their shape.
    pub fn set_root_puppet_ppath(&mut self, ppath: &str) -> Result<()> {
        self.value.set("rootPuppetPPath", LValue::string(ppath))?;
        self.edited = true;
        Ok(())
    }

    /// Records that `value` was edited directly, so the next save prints it instead of the original text.
    pub const fn mark_edited(&mut self) {
        self.edited = true;
    }

    /// The text to write: the original bytes when nothing changed, otherwise the table in Character Animator's
    /// layout.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        if self.edited || self.original.is_empty() { to_string_pretty(&self.value).into_bytes() } else { self.original.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "{\n\tbContainsArtworkCache = false,\n\tiVersion = 1,\n\trootPuppetPPath = \"/ProjectItems/Hero\",\n\tsType = \"puppet\",\n\tsUniqueID = \"abc\",\n\ttBehaviorDefaults = {\n\t\t[\"Adobe.Blinker.js\"] = {\n\t\t\tlength = 80,\n\t\t\trate = 30,\n\t\t},\n\t\t[\"Adobe.Dragger3.js\"] = {\n\t\t\thandleReleaseDuration = 0.20000000000000001,\n\t\t},\n\t},\n}\n";

    #[test]
    fn typed_accessors_read_the_manifest() {
        let info = ArchiveInfo::parse(SAMPLE.as_bytes()).expect("parses");
        assert_eq!(info.version(), Some(1));
        assert_eq!(info.root_puppet_ppath(), Some("/ProjectItems/Hero"));
        assert_eq!(info.kind(), Some("puppet"));
        assert_eq!(info.unique_id(), Some("abc"));
        assert!(!info.contains_artwork_cache());
        let defaults = info.behavior_defaults();
        assert_eq!(defaults.len(), 2);
        assert_eq!(info.behavior_default("Adobe.Blinker.js", "rate").and_then(LValue::as_i64), Some(30));
    }

    #[test]
    fn untouched_manifest_is_written_back_verbatim() {
        let info = ArchiveInfo::parse(SAMPLE.as_bytes()).expect("parses");
        assert_eq!(info.to_bytes(), SAMPLE.as_bytes());
    }

    #[test]
    fn edits_reprint_in_ca_layout() {
        let mut info = ArchiveInfo::parse(SAMPLE.as_bytes()).expect("parses");
        info.set_behavior_default("Adobe.Blinker.js", "rate", LValue::Int(45)).expect("sets");
        info.set_behavior_default("Adobe.Walk.js", "speed", LValue::float(0.5)).expect("sets");
        let text = String::from_utf8(info.to_bytes()).expect("utf8");
        assert!(text.contains("rate = 45,"));
        assert!(text.contains("[\"Adobe.Walk.js\"] = {\n\t\t\tspeed = 0.5,"));
        let again = ArchiveInfo::parse(text.as_bytes()).expect("reparses");
        assert_eq!(again.behavior_default("Adobe.Walk.js", "speed").and_then(LValue::as_f64), Some(0.5));
    }

    #[test]
    fn a_new_manifest_has_the_standard_keys() {
        let info = ArchiveInfo::new("/ProjectItems/Hero", "id-1");
        let reparsed = ArchiveInfo::parse(&info.to_bytes()).expect("reparses");
        assert_eq!(reparsed.kind(), Some("puppet"));
        assert_eq!(reparsed.root_puppet_ppath(), Some("/ProjectItems/Hero"));
    }

    #[test]
    fn non_tables_are_rejected() {
        assert!(ArchiveInfo::parse(b"42").is_err());
        assert!(ArchiveInfo::parse(b"{{{").is_err());
    }
}
