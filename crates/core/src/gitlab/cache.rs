//! What GitLab says that can't change any more (#305), kept on disk for every
//! overview of the checkout (gitlab-cache.mjs): a merged or closed MR, a
//! closed issue, the jobs of a finished pipeline (a failed one's log lines
//! with them), the deployment of a merge commit that ended. Asked once, then
//! never again: a refresh reads only what still moves.
//!
//! One file in the main checkout's .claude/, shared by all overviews, Node's
//! too: a save merges with what another one wrote meanwhile, the last
//! writer's entry wins (both hold the same facts).

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

/// Bumped when the shape of an entry changes: an older file is dropped.
const VERSION: i64 = 1;
/// Entries kept per kind, the newest: the file doesn't grow forever.
const KEEP: usize = 2000;

pub fn cache_file(main: &Path) -> PathBuf {
    main.join(".claude").join("gitlab-cache.json")
}

fn read(file: &Path) -> Map<String, Value> {
    let data = fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    match data {
        Some(Value::Object(data)) if data.get("version").and_then(Value::as_i64) == Some(VERSION) => data,
        _ => json!({ "version": VERSION }).as_object().cloned().unwrap_or_default(),
    }
}

/// The cache in a file (None: in memory only, for tests and a --once print).
#[derive(Debug, Default)]
pub struct Cache {
    file: Option<PathBuf>,
    data: Map<String, Value>,
    added: Map<String, Value>,
}

impl Cache {
    pub fn open(file: Option<PathBuf>) -> Cache {
        let data = file.as_deref().map(read).unwrap_or_else(|| read(Path::new("")));
        Cache {
            file,
            data,
            added: Map::new(),
        }
    }

    /// The raw entry of `kind` under `key`.
    pub fn raw(&self, kind: &str, key: &str) -> Option<&Value> {
        self.data.get(kind)?.get(key).filter(|value| !value.is_null())
    }

    /// The entry of `kind` under `key`, None without one or in another shape.
    pub fn get<T: DeserializeOwned>(&self, kind: &str, key: &str) -> Option<T> {
        serde_json::from_value(self.raw(kind, key)?.clone()).ok()
    }

    pub fn set<T: Serialize>(&mut self, kind: &str, key: &str, value: &T) {
        let Ok(value) = serde_json::to_value(value) else { return };
        for map in [&mut self.data, &mut self.added] {
            let entries = map.entry(kind.to_string()).or_insert_with(|| Value::Object(Map::new()));
            if let Some(entries) = entries.as_object_mut() {
                entries.insert(key.to_string(), value.clone());
            }
        }
    }

    /// What this read added, written once it is done: merged with what another
    /// overview wrote meanwhile, through a temporary file and a rename.
    pub fn save(&mut self) {
        let Some(file) = self.file.clone() else { return };
        if self.added.is_empty() {
            return;
        }
        let mut merged = read(&file);
        for (kind, entries) in &self.added {
            let mut all = merged.get(kind).and_then(Value::as_object).cloned().unwrap_or_default();
            for (key, value) in entries.as_object().into_iter().flatten() {
                // `{ ...old, ...added }`: a known key keeps its place, a new one goes last.
                all.insert(key.clone(), value.clone());
            }
            let skip = all.len().saturating_sub(KEEP);
            merged.insert(kind.clone(), Value::Object(all.into_iter().skip(skip).collect()));
        }
        let temporary = PathBuf::from(format!("{}.{}.tmp", file.display(), std::process::id()));
        let is_written = file.parent().is_some_and(|dir| fs::create_dir_all(dir).is_ok())
            && fs::write(&temporary, Value::Object(merged.clone()).to_string()).is_ok()
            && fs::rename(&temporary, &file).is_ok();
        // Not written: asked again next time, nothing lost.
        if is_written {
            self.data = merged;
            self.added.clear();
        } else {
            let _ = fs::remove_file(&temporary);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::scratch::Scratch;

    #[test]
    fn keeps_what_another_overview_wrote_meanwhile() {
        let scratch = Scratch::new();
        let file = cache_file(scratch.path());
        let mut one = Cache::open(Some(file.clone()));
        let mut two = Cache::open(Some(file.clone()));
        one.set("mrs", "1", &json!({ "iid": 1 }));
        one.save();
        two.set("issues", "5", &json!({ "iid": 5 }));
        two.save();
        let three = Cache::open(Some(file.clone()));
        assert_eq!(three.get::<Value>("mrs", "1"), Some(json!({ "iid": 1 })));
        assert_eq!(three.get::<Value>("issues", "5"), Some(json!({ "iid": 5 })));
        assert!(three.get::<Value>("mrs", "2").is_none());
    }

    #[test]
    fn drops_a_file_of_another_version() {
        let scratch = Scratch::new();
        let file = cache_file(scratch.path());
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, r#"{"version":0,"mrs":{"1":{"iid":1}}}"#).unwrap();
        assert!(Cache::open(Some(file)).raw("mrs", "1").is_none());
        let mut memory = Cache::open(None);
        memory.set("mrs", "1", &1);
        memory.save();
        assert_eq!(memory.get::<i64>("mrs", "1"), Some(1));
    }
}
