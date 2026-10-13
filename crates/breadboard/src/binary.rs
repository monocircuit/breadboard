//! This binary's file as it was started, and whether a build replaced it
//! since (#348): the launcher installs a new build by renaming it over the
//! old file, so the path then names another file. A live overview and the
//! daemon go on in the new one (`exec`) instead of running the code from
//! before until someone presses `q`.

use std::ffi::OsString;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Started {
    path: PathBuf,
    /// The device and inode of the file at the start.
    id: Option<(u64, u64)>,
}

impl Started {
    /// This process's binary, by the path it was started by (herdr's `started_binary`).
    pub fn now() -> Started {
        Started::at(crate::herdr::started_binary())
    }

    pub fn at(path: PathBuf) -> Started {
        let id = id_of(&path);
        Started { path, id }
    }

    /// Whether another executable file stands at its path now: a build installed there.
    pub fn is_replaced(&self) -> bool {
        let Some(was) = self.id else { return false };
        std::fs::metadata(&self.path).is_ok_and(|meta| {
            meta.is_file() && meta.permissions().mode() & 0o111 != 0 && (meta.dev(), meta.ino()) != was
        })
    }

    /// The file at its path run in place of this process with `args` and
    /// `env` added; returns only when it couldn't (why).
    pub fn exec(&self, args: impl IntoIterator<Item = OsString>, env: &[(&str, String)]) -> std::io::Error {
        let mut command = Command::new(&self.path);
        command.args(args);
        for (name, value) in env {
            command.env(name, value);
        }
        command.exec()
    }
}

fn id_of(path: &Path) -> Option<(u64, u64)> {
    std::fs::metadata(path).ok().map(|meta| (meta.dev(), meta.ino()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sees_a_build_renamed_over_its_file_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("breadboard-binary-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("breadboard");
        let write = |path: &Path, text: &str| {
            std::fs::write(path, text).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        write(&file, "old");
        let started = Started::at(file.clone());
        assert!(!started.is_replaced());
        // Touched or rewritten in place: the same file.
        write(&file, "old again");
        assert!(!started.is_replaced());
        // Installed as the launcher does: copied beside it, renamed over it.
        let new = dir.join("breadboard.new");
        write(&new, "new");
        std::fs::rename(&new, &file).unwrap();
        assert!(started.is_replaced());
        // Gone (a build under way removed it): nothing to go on in.
        std::fs::remove_file(&file).unwrap();
        assert!(!started.is_replaced());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
