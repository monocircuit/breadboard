//! The repo's changelog kept current (#261, read.mjs `keepOverall`): after
//! each GitLab read of the view of everything, when develop's head on origin
//! moved since `.claude/changelog.json` was written, or `.claude/changelog.md`
//! is missing, `breadboard changelog` rewrites it in the background. Once per
//! head; a failure lets the next read try again. This is how the tab
//! `changelog` of the main space stays current.

use breadboard_core::json;
use serde_json::Value;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};

/// The head of develop the rewrite was last started for (read.mjs `overallTried`).
type Tried = Arc<Mutex<Option<String>>>;

fn tried() -> &'static Tried {
    static TRIED: OnceLock<Tried> = OnceLock::new();
    TRIED.get_or_init(Tried::default)
}

fn overall_file(main: &Path) -> PathBuf {
    main.join(".claude").join("changelog.md")
}

fn state_file(main: &Path) -> PathBuf {
    main.join(".claude").join("changelog.json")
}

/// Whether the changelog was written for another head than `develop`, or not at all (`isOverallBehind`).
pub fn is_behind(main: &Path, develop: &str) -> bool {
    !overall_file(main).exists()
        || json::read(&state_file(main))
            .and_then(|state| json::str_of(&state, "develop").map(String::from))
            .as_deref()
            != Some(develop)
}

/// After a read: the rewrite started when develop moved (`keepOverall`).
pub fn keep(main: &Path, develop: Option<&str>) {
    let dir = main.to_path_buf();
    start(tried(), main, develop, move || rewrite(&dir));
}

/// `write` on a thread of its own when the changelog is behind `develop` and
/// it wasn't tried for that head yet; the head recorded once it wrote.
fn start(
    tried: &Tried,
    main: &Path,
    develop: Option<&str>,
    write: impl FnOnce() -> bool + Send + 'static,
) -> Option<JoinHandle<()>> {
    let develop = develop.filter(|develop| !develop.is_empty())?;
    {
        let mut was = tried.lock().ok()?;
        if was.as_deref() == Some(develop) || !is_behind(main, develop) {
            return None;
        }
        *was = Some(develop.to_string());
    }
    let (tried, main, develop) = (tried.clone(), main.to_path_buf(), develop.to_string());
    Some(thread::spawn(move || {
        if write() && record(&main, &develop).is_ok() {
            return;
        }
        if let Ok(mut was) = tried.lock()
            && was.as_deref() == Some(develop.as_str())
        {
            *was = None;
        }
    }))
}

/// `breadboard changelog` (`pnpm changelog`): the writer of the CLI, its output unseen.
fn rewrite(main: &Path) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    Command::new(exe)
        .arg("changelog")
        .current_dir(main)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// `develop` into `.claude/changelog.json` beside what the writer put there
/// (`writeOverall({ develop })`), written whole or not at all.
fn record(main: &Path, develop: &str) -> io::Result<()> {
    if !overall_file(main).exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no changelog written"));
    }
    let file = state_file(main);
    let mut state = json::read_object(&file);
    state.insert("develop".into(), Value::from(develop));
    let temporary = file.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&temporary, format!("{}\n", json::pretty(&Value::Object(state))))?;
    std::fs::rename(&temporary, &file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn main_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bb-overall-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        dir
    }

    /// A writer that counts its runs and writes the changelog when `is_ok`.
    fn writer(main: &Path, runs: &Arc<AtomicUsize>, is_ok: bool) -> impl FnOnce() -> bool + Send + 'static {
        let (main, runs) = (main.to_path_buf(), runs.clone());
        move || {
            runs.fetch_add(1, Ordering::SeqCst);
            if is_ok {
                std::fs::write(overall_file(&main), "# changelog\n").unwrap();
                std::fs::write(
                    state_file(&main),
                    "{\n  \"since\": \"2w\",\n  \"develop\": null,\n  \"mrs\": {}\n}\n",
                )
                .unwrap();
            }
            is_ok
        }
    }

    #[test]
    fn writes_once_per_head_and_records_it() {
        let main = main_dir("once");
        let (tried, runs) = (Tried::default(), Arc::new(AtomicUsize::new(0)));
        assert!(is_behind(&main, "aaa"));
        start(&tried, &main, Some("aaa"), writer(&main, &runs, true))
            .unwrap()
            .join()
            .unwrap();
        assert!(!is_behind(&main, "aaa"));
        let state = std::fs::read_to_string(state_file(&main)).unwrap();
        assert_eq!(
            state,
            "{\n  \"since\": \"2w\",\n  \"develop\": \"aaa\",\n  \"mrs\": {}\n}\n"
        );
        assert!(start(&tried, &main, Some("aaa"), writer(&main, &runs, true)).is_none());
        // develop moved: written again.
        start(&tried, &main, Some("bbb"), writer(&main, &runs, true))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        assert!(start(&tried, &main, None, writer(&main, &runs, true)).is_none());
        let _ = std::fs::remove_dir_all(&main);
    }

    #[test]
    fn a_failure_is_tried_again_by_the_next_read() {
        let main = main_dir("failure");
        let (tried, runs) = (Tried::default(), Arc::new(AtomicUsize::new(0)));
        start(&tried, &main, Some("aaa"), writer(&main, &runs, false))
            .unwrap()
            .join()
            .unwrap();
        assert!(tried.lock().unwrap().is_none());
        start(&tried, &main, Some("aaa"), writer(&main, &runs, true))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        assert!(!is_behind(&main, "aaa"));
        let _ = std::fs::remove_dir_all(&main);
    }

    #[test]
    fn a_missing_changelog_is_behind_even_for_its_head() {
        let main = main_dir("missing");
        std::fs::write(state_file(&main), "{\"develop\": \"aaa\"}\n").unwrap();
        assert!(is_behind(&main, "aaa"));
        std::fs::write(overall_file(&main), "# changelog\n").unwrap();
        assert!(!is_behind(&main, "aaa"));
        assert!(is_behind(&main, "bbb"));
        let _ = std::fs::remove_dir_all(&main);
    }
}
