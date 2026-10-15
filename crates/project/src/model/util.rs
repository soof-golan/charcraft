//! Small readers over `LValue` shared by the decoders. Everything is optional: hostile or unexpected shapes give
//! `None` or a default, never a panic.

use charcraft_ltab::LValue;

/// A string field. Character Animator writes `false` for "no value", which reads as `None`.
pub fn string(value: &LValue, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}

/// A boolean field (`false` when missing).
pub fn boolean(value: &LValue, key: &str) -> bool {
    value.get(key).and_then(LValue::as_bool).unwrap_or(false)
}

/// A boolean field with a default for missing ones.
pub fn boolean_or(value: &LValue, key: &str, default: bool) -> bool {
    value.get(key).and_then(LValue::as_bool).unwrap_or(default)
}

/// A number field (`0` when missing).
pub fn number(value: &LValue, key: &str) -> f64 {
    value.get(key).and_then(LValue::as_f64).unwrap_or(0.0)
}

/// A number field that may be absent.
pub fn opt_number(value: &LValue, key: &str) -> Option<f64> {
    value.get(key).and_then(LValue::as_f64)
}

/// An integer field (`0` when missing).
pub fn int(value: &LValue, key: &str) -> i64 {
    value.get(key).and_then(LValue::as_i64).unwrap_or(0)
}

/// The path inside a `PPath{ p = "..." }` or `CPath{ p = "..." }` table.
pub fn ppath(value: &LValue, key: &str) -> Option<String> {
    string(value.get(key)?, "p")
}

/// The `{ id = ..., v = ... }` entries of an `IDArray_001`.
pub fn id_items(array: &LValue) -> Vec<(String, &LValue)> {
    let Some(items) = array.get("_aIDArray") else {
        return Vec::new();
    };
    items.iter().filter_map(|(_, entry)| Some((entry.get("id")?.as_str()?.to_owned(), entry.get("v")?))).collect()
}

/// The numbers of an array-style table.
pub fn numbers(value: &LValue) -> Vec<f64> {
    value.iter().filter_map(|(_, v)| v.as_f64()).collect()
}

/// A pair of numbers from an array-style table (missing entries read as `fallback`).
pub fn pair(value: Option<&LValue>, fallback: f64) -> [f64; 2] {
    let list = value.map(numbers).unwrap_or_default();
    [list.first().copied().unwrap_or(fallback), list.get(1).copied().unwrap_or(fallback)]
}

/// The text after the `=` of a localisation key (`$$$/animal/.../Name=Left Blink` gives `Left Blink`). Text that
/// is not a localisation key comes back unchanged.
pub fn localized(text: &str) -> &str {
    if text.starts_with("$$$/") { text.split_once('=').map_or(text, |(_, shown)| shown) } else { text }
}

/// The last `/` separated segment of a path.
pub fn last_segment(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

/// A project path (`/ProjectItems/x`) as a store tree path (`ProjectItems/x`).
pub fn tree_path(ppath: &str) -> &str {
    ppath.trim_start_matches('/')
}
