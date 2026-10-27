//! The locks Node and Rust share while both run (rewrite-plan.md): a
//! `<file>.lock` directory around a change of a file (mkdir is atomic), and
//! pid files in `$TMPDIR` for one process of a kind per user.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

/// Whether the process `pid` runs: `kill(pid, 0)`, EPERM counts (it runs as
/// someone else). Where there is a `/proc`, a thread's id is none (#445):
/// `kill` answers yes for it too, and after a reboot the pid of a file
/// written before may be a thread of another process now.
pub fn is_alive(pid: i64) -> bool {
    if pid <= 0 || pid > i64::from(i32::MAX) {
        return false;
    }
    if Path::new("/proc/self/status").exists() {
        return tgid_of(pid) == Some(pid);
    }
    // SAFETY: signal 0 only asks whether the process exists.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The process the task `pid` belongs to (`Tgid` of `/proc/<pid>/status`):
/// itself for a process, another for a thread; None when there is none.
fn tgid_of(pid: i64) -> Option<i64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Tgid:"))
        .and_then(|tgid| tgid.trim().parse().ok())
}

/// Who a process is beyond its pid: this boot's id and when it started
/// (`/proc/<pid>/stat`'s ticks since boot), `<boot> <start>`; None without a `/proc`.
fn identity_of(pid: i64) -> Option<String> {
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The name in parentheses may hold spaces: the fields count from its end.
    let start = stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.to_string();
    Some(format!("{} {start}", boot.trim()))
}

/// How old the lock directory is, None when it is gone.
pub fn age_of(lock: &Path) -> Option<Duration> {
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
    /// The pid file `file` for this process, None while another live one
    /// holds it: its pid alive and, where written (#445), the same boot and
    /// start time, so a pid reused after a reboot holds nothing.
    pub fn hold(file: &Path) -> Option<PidFile> {
        let pid = i64::from(std::process::id());
        let own = match identity_of(pid) {
            Some(identity) => format!("{pid} {identity}"),
            None => pid.to_string(),
        };
        for _ in 0..2 {
            match fs::OpenOptions::new().write(true).create_new(true).open(file) {
                Ok(mut created) => {
                    use std::io::Write;
                    created.write_all(own.as_bytes()).ok()?;
                    return Some(PidFile {
                        file: file.to_path_buf(),
                    });
                }
                Err(_) => {
                    if holds(file) {
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
    fs::read_to_string(file)
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

/// Whether the process a pid file names still holds it: alive, and the one
/// that wrote it when the file says who (`<pid> <boot> <start>`).
fn holds(file: &Path) -> bool {
    let text = fs::read_to_string(file).unwrap_or_default();
    let mut parts = text.split_whitespace();
    let Some(pid) = parts.next().and_then(|pid| pid.parse::<i64>().ok()) else {
        return false;
    };
    let written: Vec<&str> = parts.collect();
    is_alive(pid) && (written.is_empty() || identity_of(pid).is_none_or(|now| now == written.join(" ")))
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
    fn a_stale_pid_file_naming_a_thread_or_another_boot_is_taken_over() {
        let scratch = scratch::Scratch::new();
        let file = scratch.join("engine.pid");
        // A thread of this very process: `kill(tid, 0)` says yes, it is no process.
        let (sender, receiver) = std::sync::mpsc::channel();
        let (_stop, wait) = std::sync::mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            // SAFETY: gettid has no arguments and only reads.
            sender.send(unsafe { libc::gettid() } as i64).unwrap();
            let _ = wait.recv();
        });
        let tid = receiver.recv().unwrap();
        assert!(!is_alive(tid), "a thread id counts as a process");
        assert!(is_alive(i64::from(std::process::id())));
        fs::write(&file, tid.to_string()).unwrap();
        let held = PidFile::hold(&file).expect("not taken over from a thread id");
        let written = fs::read_to_string(&file).unwrap();
        assert_eq!(written.split_whitespace().count(), 3, "{written}");
        drop(held);
        // This process, but written in another boot: a pid reused.
        fs::write(&file, format!("{} 00000000-0000 1", std::process::id())).unwrap();
        assert!(PidFile::hold(&file).is_some(), "a pid of another boot holds it");
        drop(_stop);
        thread.join().unwrap();
    }

    #[test]
    fn this_process_is_alive() {
        assert!(is_alive(i64::from(std::process::id())));
        assert!(!is_alive(0));
    }
}
