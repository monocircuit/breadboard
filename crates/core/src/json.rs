//! JSON files as the Node code reads and writes them: `JSON.parse` of the
//! whole file, `JSON.stringify(value, null, 2)` and a newline back, the keys
//! in the order they came (serde_json's `preserve_order`). A field Rust
//! doesn't know survives a write: the files are changed as values, not
//! rebuilt from structs.

use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::path::Path;

/// The parsed file, None when it is missing or no JSON.
pub fn read(file: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(file).ok()?).ok()
}

/// The file as an object, empty when it is missing, broken or no object
/// (actions.mjs `readFile`).
pub fn read_object(file: &Path) -> Map<String, Value> {
    match read(file) {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

/// `JSON.stringify(value, null, 2)`.
pub fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".into())
}

/// `value` into `file` as `JSON.stringify(value, null, 2)` and a newline,
/// its folder made first.
pub fn write_pretty(file: &Path, value: &Value) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(file, format!("{}\n", pretty(value)))
}

/// `value` into `file` as `JSON.stringify(value)` and a newline (`.claude/origin`).
pub fn write_compact(file: &Path, value: &Value) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(file, format!("{value}\n"))
}

/// `{ ...object, ...fields }`: a key there keeps its place, a new one goes
/// last; a null in `fields` stays a null (JSON.stringify writes it).
pub fn merge(object: &mut Map<String, Value>, fields: Map<String, Value>) {
    for (key, value) in fields {
        object.insert(key, value);
    }
}

/// A string field, None when it is missing or no string.
pub fn str_of<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

/// A whole number field, None when it is missing or no number.
pub fn int_of(value: &Value, key: &str) -> Option<i64> {
    let field = value.get(key)?;
    field
        .as_i64()
        .or_else(|| field.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
}

/// A list of whole numbers, the others left out.
pub fn ints_of(value: &Value, key: &str) -> Vec<i64> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}

/// A list of strings, the others left out.
pub fn strs_of(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|item| item.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

/// JavaScript's truthiness of a field: missing, null, false, 0 and "" are false.
pub fn is_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn writes_what_json_stringify_writes() {
        let value = json!({ "actions": [], "answered": [{ "id": 1, "text": "ä → ✓", "url": null }], "empty": {} });
        assert_eq!(
            pretty(&value),
            "{\n  \"actions\": [],\n  \"answered\": [\n    {\n      \"id\": 1,\n      \"text\": \"ä → ✓\",\n      \"url\": null\n    }\n  ],\n  \"empty\": {}\n}"
        );
    }

    #[test]
    fn keeps_the_order_of_the_keys_it_read() {
        let text = "{\n  \"zeta\": 1,\n  \"alpha\": 2\n}";
        let value: Value = serde_json::from_str(text).unwrap();
        assert_eq!(pretty(&value), text);
        let mut object = value.as_object().unwrap().clone();
        merge(
            &mut object,
            json!({ "alpha": 3, "beta": null }).as_object().unwrap().clone(),
        );
        assert_eq!(Value::Object(object).to_string(), r#"{"zeta":1,"alpha":3,"beta":null}"#);
    }

    #[test]
    fn reads_truthiness_as_javascript() {
        let value = json!({ "a": 0, "b": "", "c": "x", "d": [], "e": null });
        assert!(!is_truthy(value.get("a")));
        assert!(!is_truthy(value.get("b")));
        assert!(is_truthy(value.get("c")));
        assert!(is_truthy(value.get("d")));
        assert!(!is_truthy(value.get("e")));
        assert!(!is_truthy(value.get("missing")));
    }
}
