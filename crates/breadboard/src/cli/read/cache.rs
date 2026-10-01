//! What GitLab says that can't change any more (#305), on disk for every
//! overview of the checkout (gitlab-cache.mjs): a merged or closed MR, a
//! closed issue, the state of a merged or closed MR, the jobs of a finished
//! pipeline. Asked once, then never again.
//!
//! One file, `<main>/.claude/gitlab-cache.json`, shared with the Node
//! overview while both run (rewrite rule 5): its kinds, keys and shapes. A
//! save merges with what another overview wrote meanwhile, the last writer's
//! entry wins (both hold the same facts). Core's GitLab client has the same
//! cache (`gitlab/cache.rs` of the branch); once the snapshot carries it, this
//! file goes.

use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

/// Bumped when the shape of an entry changes: an older file is dropped.
const VERSION: i64 = 1;
/// Entries kept per kind, the newest: the file doesn't grow forever.
const KEEP: usize = 2000;

fn read(file: &Path) -> Map<String, Value> {
    let data = fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    match data {
        Some(Value::Object(data)) if data.get("version").and_then(Value::as_i64) == Some(VERSION) => data,
        _ => Map::from_iter([("version".to_string(), json!(VERSION))]),
    }
}

/// The cache of the checkout whose main checkout is `main`.
#[derive(Debug)]
pub struct Cache {
    file: PathBuf,
    data: Map<String, Value>,
    added: Map<String, Value>,
}

impl Cache {
    pub fn open(main: &Path) -> Cache {
        let file = main.join(".claude").join("gitlab-cache.json");
        Cache {
            data: read(&file),
            file,
            added: Map::new(),
        }
    }

    /// The entry of `kind` under `key`, None without one.
    pub fn get(&self, kind: &str, key: &str) -> Option<&Value> {
        self.data.get(kind)?.get(key).filter(|value| !value.is_null())
    }

    pub fn set(&mut self, kind: &str, key: &str, value: Value) {
        for map in [&mut self.data, &mut self.added] {
            if let Value::Object(entries) = map.entry(kind).or_insert_with(|| json!({})) {
                entries.insert(key.to_string(), value.clone());
            }
        }
    }

    /// What this read added, merged with what another overview wrote
    /// meanwhile, through a temporary file and a rename. Not written: asked
    /// again next time, nothing lost.
    pub fn save(&mut self) {
        if self.added.is_empty() {
            return;
        }
        let mut merged = read(&self.file);
        for (kind, entries) in &self.added {
            let mut all = merged.get(kind).and_then(Value::as_object).cloned().unwrap_or_default();
            // `{ ...old, ...added }`: a known key keeps its place, a new one goes last.
            for (key, value) in entries.as_object().into_iter().flatten() {
                all.insert(key.clone(), value.clone());
            }
            let skip = all.len().saturating_sub(KEEP);
            merged.insert(kind.clone(), Value::Object(all.into_iter().skip(skip).collect()));
        }
        let temporary = PathBuf::from(format!("{}.{}.tmp", self.file.display(), std::process::id()));
        let is_written = self.file.parent().is_some_and(|dir| fs::create_dir_all(dir).is_ok())
            && fs::write(&temporary, Value::Object(merged.clone()).to_string()).is_ok()
            && fs::rename(&temporary, &self.file).is_ok();
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

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("breadboard-read-cache-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn keeps_what_another_overview_wrote_meanwhile() {
        let main = scratch("merge");
        let mut one = Cache::open(&main);
        let mut two = Cache::open(&main);
        one.set("mrs", "1", json!({ "iid": 1 }));
        one.save();
        two.set("issues", "5", json!({ "iid": 5 }));
        two.save();
        let three = Cache::open(&main);
        assert_eq!(three.get("mrs", "1"), Some(&json!({ "iid": 1 })));
        assert_eq!(three.get("issues", "5"), Some(&json!({ "iid": 5 })));
        assert_eq!(three.get("mrs", "2"), None);
        let _ = fs::remove_dir_all(&main);
    }

    #[test]
    fn drops_a_file_of_another_version_and_writes_nothing_unasked() {
        let main = scratch("version");
        let file = main.join(".claude/gitlab-cache.json");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, r#"{"version":0,"mrs":{"1":{"iid":1}}}"#).unwrap();
        let mut cache = Cache::open(&main);
        assert_eq!(cache.get("mrs", "1"), None);
        cache.save();
        assert!(fs::read_to_string(&file).unwrap().contains(r#""version":0"#));
        let _ = fs::remove_dir_all(&main);
    }
}
