//! Behavior definitions: the `$Behaviors/Adobe.*.js` files Character Animator keeps in every project.

use charcraft_ltab::LValue;
use serde::Serialize;

use super::util::{boolean, last_segment, localized, opt_number, string};

/// One parameter a behavior declares.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParamDef {
    /// The parameter id (`rate`).
    pub id: String,
    /// The control type (`slider`, `checkbox`, `layer`, `dropdown`, ...).
    pub kind: String,
    /// Label shown in the UI.
    pub label: Option<String>,
    /// Unit shown after the value (`ms`, `%`).
    pub units: Option<String>,
    /// The default value (`dephault`).
    pub default: LValue,
    /// Smallest allowed value.
    pub min: Option<f64>,
    /// Largest allowed value.
    pub max: Option<f64>,
    /// Digits after the decimal point shown.
    pub precision: Option<i64>,
    /// The parameter table as stored, for fields the typed view does not cover.
    pub raw: LValue,
}

/// A tag a behavior defines (`Adobe.Face.LeftBlink`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TagDef {
    /// The tag id.
    pub id: String,
    /// `layertag` or `handletag`.
    pub kind: Option<String>,
    /// Label shown in the UI.
    pub label: Option<String>,
    /// Layer names that get the tag automatically.
    pub art_matches: Vec<String>,
    /// UI groups the tag is listed in.
    pub groups: Vec<String>,
}

/// A behavior definition.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BehaviorDef {
    /// The store path (`$Behaviors/Adobe.Blinker.js`).
    pub path: String,
    /// The behavior id (`Adobe.Blinker.js`).
    pub id: String,
    /// Display name (`Auto Blink`).
    pub name: Option<String>,
    /// Description.
    pub description: Option<String>,
    /// Armed for recording by default.
    pub default_armed: bool,
    /// Declared parameters.
    pub params: Vec<ParamDef>,
    /// Declared tags.
    pub tags: Vec<TagDef>,
    /// Size of the JavaScript in bytes.
    pub js_len: usize,
    /// Compilation failed when Character Animator last built it.
    pub compile_failed: bool,
    /// The behavior is missing its source.
    pub missing: bool,
}

impl BehaviorDef {
    /// A parameter by id.
    #[must_use]
    pub fn param(&self, id: &str) -> Option<&ParamDef> {
        self.params.iter().find(|p| p.id == id)
    }
}

fn string_list(value: Option<&LValue>) -> Vec<String> {
    value.map(|v| v.iter().filter_map(|(_, s)| s.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

/// Decodes `Behavior{...}` stored at `path`.
#[must_use]
pub fn decode(path: &str, value: &LValue) -> BehaviorDef {
    let attributes = value.get("_attributes");
    let params = value
        .get("_aParams")
        .map(|list| {
            list.iter()
                .filter_map(|(_, p)| {
                    Some(ParamDef {
                        id: string(p, "id")?,
                        kind: string(p, "type").unwrap_or_default(),
                        label: string(p, "uiName").map(|l| localized(&l).to_owned()),
                        units: string(p, "uiUnits"),
                        default: p.get("dephault").cloned().unwrap_or(LValue::Nil),
                        min: opt_number(p, "min"),
                        max: opt_number(p, "max"),
                        precision: p.get("precision").and_then(LValue::as_i64),
                        raw: p.clone(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let tags = value
        .get("_aBehaviorDefinedTags")
        .map(|list| {
            list.iter()
                .filter_map(|(_, t)| {
                    Some(TagDef {
                        id: string(t, "id")?,
                        kind: string(t, "tagType"),
                        label: string(t, "uiName").map(|l| localized(&l).to_owned()),
                        art_matches: string_list(t.get("artMatches")),
                        groups: t.get("uiGroups").map(|g| g.iter().filter_map(|(_, group)| string(group, "id")).collect()).unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    BehaviorDef {
        path: path.to_owned(),
        id: last_segment(path).to_owned(),
        name: attributes.and_then(|a| string(a, "uiName")).map(|n| localized(&n).to_owned()),
        description: attributes.and_then(|a| string(a, "description")).map(|n| localized(&n).to_owned()),
        default_armed: attributes.is_some_and(|a| boolean(a, "defaultArmedForRecordOn")),
        params,
        tags,
        js_len: value.get("_jsCode").and_then(LValue::as_bytes).map_or(0, <[u8]>::len),
        compile_failed: boolean(value, "_bCompileFailed"),
        missing: boolean(value, "_bMissing"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLINKER: &str = r#"{
	_aBehaviorDefinedTags = { { artMatches = { "left blink" }, id = "Adobe.Face.LeftBlink", tagType = "layertag",
		uiGroups = { { id = "Adobe.TagGroup.Face" } }, uiName = "$$$/x/LeftBlink=Left Blink" } },
	_aParams = { { dephault = 30, id = "rate", max = 500, min = 0, precision = 1, type = "slider", uiName = "$$$/x/rate=Blinks per Minute" },
		{ dephault = { match = "//Adobe.Face.LeftBlink" }, id = "blinkLayers", type = "layer" } },
	_attributes = { description = "$$$/x/Desc=Trigger Blink-tagged layers", uiName = "$$$/x/UIName=Auto Blink", defaultArmedForRecordOn = true },
	_bCompileFailed = false,
	_jsCode = "define()",
}"#;

    #[test]
    fn a_behavior_definition_is_decoded() {
        let value = charcraft_ltab::parse(BLINKER).expect("parses");
        let def = decode("$Behaviors/Adobe.Blinker.js", &value);
        assert_eq!(def.id, "Adobe.Blinker.js");
        assert_eq!(def.name.as_deref(), Some("Auto Blink"));
        assert_eq!(def.params.len(), 2);
        let rate = def.param("rate").expect("rate");
        assert_eq!(rate.default.as_i64(), Some(30));
        assert_eq!(rate.label.as_deref(), Some("Blinks per Minute"));
        assert_eq!(rate.max, Some(500.0));
        assert_eq!(def.tags.first().map(|t| t.art_matches.clone()), Some(vec!["left blink".to_owned()]));
        assert_eq!(def.tags.first().map(|t| t.groups.clone()), Some(vec!["Adobe.TagGroup.Face".to_owned()]));
        assert_eq!(def.js_len, 8);
        assert!(def.default_armed);
    }
}
