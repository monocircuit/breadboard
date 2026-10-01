//! The locks Node and Rust share while both run (rewrite-plan.md): a
//! `<file>.lock` directory around a change of a file (mkdir is atomic), and
//! pid files in `$TMPDIR` for one process of a kind per user.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

/// Whether the process `pid` runs: `kill(pid, 0)`, EPERM counts (it runs as
/// someone else).
pub fn is_alive(pid: i64) -> bool {
    if pid <= 0 || pid > i64::from(i32::MAX) {
        return false;
    }
    // SAFETY: signal 0 only asks whether the process exists.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// How old the lock directory is, None when it is gone.
fn age_of(lock: &Path) -> Option<Duration> {
    let modified = fs::metadata(lock).ok()?.modified().ok()?;
    Some(SystemTime::now().duration_since(modified).unwrap_or_default())
}

/// `run` as one change of `file`: the `<file>.lock` directory next to it held
/// meanwhile, one older than `stale` taken over (its holder died). The
/// overview, the Stop hook and wt:action change `.claude/actions.json` from
/// processes of their own: a read between another's read and write would
/// undo it (actions.mjs `changeActions`, lock.mjs `exclusive`).
pub fn with_dir_lock<T>(file: &Path, stale: Duration, run: impl FnOnce() -> T) -> io::Result<T> {
    let lock = lock_of(file);
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut waited = Duration::ZERO;
    loop {
        match fs::create_dir(&lock) {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if age_of(&lock).is_some_and(|age| age > stale) || waited > stale * 2 {
                    let _ = fs::remove_dir_all(&lock);
                    continue;
                }
                thread::sleep(Duration::from_millis(10));
                waited += Duration::from_millis(10);
            }
            Err(error) => return Err(error),
        }
    }
    let result = run();
    let _ = fs::remove_dir_all(&lock);
    Ok(result)
}

/// The lock directory of a file: `<file>.lock`.
pub fn lock_of(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

/// The lock of herdr's layout, one for every session of the user
/// (lock.mjs `exclusive`): `$TMPDIR/platform-herdr-layout-<uid>.lock`.
pub fn layout_lock() -> PathBuf {
    // SAFETY: getuid never fails.
    let uid = unsafe { libc::getuid() };
    std::env::temp_dir().join(format!("platform-herdr-layout-{uid}.lock"))
}

/// A pid file this process holds (lock.mjs `holds`): the first to ask gets
/// it and keeps it while it runs, one whose process is gone is taken over.
/// Dropped, the file goes, if it is still ours.
pub struct PidFile {
    file: PathBuf,
}

impl PidFile {
    /// The pid file `file` for this process, None while another live one holds it.
    pub fn hold(file: &Path) -> Option<PidFile> {
        let own = std::process::id().to_string();
        for _ in 0..2 {
            match fs::OpenOptions::new().write(true).create_new(true).open(file) {
                Ok(mut created) => {
                    use std::io::Write;
                    created.write_all(own.as_bytes()).ok()?;
                    return Some(PidFile { file: file.to_path_buf() });
                }
                Err(_) => {
                    let pid = pid_in(file);
                    if pid != 0 && is_alive(pid) {
                        return None;
                    }
                    let _ = fs::remove_file(file);
                }
            }
        }
        None
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        if pid_in(&self.file) == i64::from(std::process::id()) {
            let _ = fs::remove_file(&self.file);
        }
    }
}

/// The pid in a pid file, 0 when there is none.
pub fn pid_in(file: &Path) -> i64 {
    fs::read_to_string(file).ok().and_then(|text| text.trim().parse().ok()).unwrap_or(0)
}

#[cfg(test)]
pub(crate) mod scratch {
    //! A directory of its own per test, gone after it.
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    pub struct Scratch(pub PathBuf);

    impl Scratch {
        pub fn new() -> Scratch {
            let dir = std::env::temp_dir().join(format!(
                "breadboard-core-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
        pub fn join(&self, part: &str) -> PathBuf {
            self.0.join(part)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::scratch::Scratch;
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn one_change_at_a_time() {
        let scratch = Scratch::new();
        let file = Arc::new(scratch.join("actions.json"));
        let inside = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let (file, inside) = (file.clone(), inside.clone());
                thread::spawn(move || {
                    with_dir_lock(&file, Duration::from_secs(5), || {
                        assert_eq!(inside.fetch_add(1, Ordering::SeqCst), 0);
                        thread::sleep(Duration::from_millis(5));
                        inside.fetch_sub(1, Ordering::SeqCst);
                    })
                    .unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert!(!lock_of(&file).exists());
    }

    #[test]
    fn a_stale_lock_is_taken_over() {
        let scratch = Scratch::new();
        let file = scratch.join("actions.json");
        fs::create_dir(lock_of(&file)).unwrap();
        let ran = with_dir_lock(&file, Duration::from_millis(30), || true).unwrap();
        assert!(ran);
    }

    #[test]
    fn a_pid_file_is_held_once_and_given_back() {
        let scratch = Scratch::new();
        let file = scratch.join("one.pid");
        let held = PidFile::hold(&file).unwrap();
        assert!(PidFile::hold(&file).is_none());
        drop(held);
        assert!(!file.exists());
        fs::write(&file, "999999999").unwrap();
        assert!(PidFile::hold(&file).is_some());
    }

    #[test]
    fn this_process_is_alive() {
        assert!(is_alive(i64::from(std::process::id())));
        assert!(!is_alive(0));
    }
}
