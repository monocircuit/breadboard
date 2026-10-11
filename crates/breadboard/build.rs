//! The protocol of `breadboard serve` (src/daemon): a hash of the sources
//! of what goes over its socket (core's types and reads, the daemon itself).
//! Its socket carries it, so a build whose answers could differ never talks
//! to another build's daemon, and every build of the same sources (another
//! checkout, a copy, a rebuild that only touched the frame) shares one.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

/// What the daemon serves comes from these, relative to this crate.
const SOURCES: [&str; 2] = ["../core/src", "src/daemon"];

fn main() {
    let mut files: Vec<PathBuf> = Vec::new();
    for source in SOURCES {
        println!("cargo:rerun-if-changed={source}");
        sources_in(Path::new(source), &mut files);
    }
    files.sort();
    let mut hasher = DefaultHasher::new();
    for file in &files {
        file.hash(&mut hasher);
        std::fs::read(file).unwrap_or_default().hash(&mut hasher);
    }
    println!("cargo:rustc-env=BREADBOARD_PROTOCOL={:016x}", hasher.finish());
    // The key of the sources the launcher builds (#339, herdr/breadboard.sh):
    // in the binary, so a stored copy can be checked against the key it is
    // stored under. Empty for every other build.
    println!("cargo:rerun-if-env-changed=BREADBOARD_SOURCE_KEY");
    println!(
        "cargo:rustc-env=BREADBOARD_SOURCE_KEY={}",
        std::env::var("BREADBOARD_SOURCE_KEY").unwrap_or_default()
    );
}

/// The `.rs` files under `dir`, at any depth.
fn sources_in(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources_in(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}
