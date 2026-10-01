//! A url into the browser, in the tab group of its herdr space (browser.mjs):
//! the Chromium extension in packages/breadboard/chromium does the grouping,
//! from a copy outside the repo; without it `xdg-open` opens the url.
//! `pnpm browser` puts the copy in place and says how to load it.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::sync::OnceLock;

/// Fixed by the `key` of packages/breadboard/chromium/manifest.json.
pub const EXTENSION_ID: &str = "aikcnbeljldocaafkokjbngkboecmoli";
const BROWSERS: [&str; 4] = ["chromium", "chromium-browser", "google-chrome-stable", "google-chrome"];

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn install_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local").join("share"))
        .join("monocircuit")
        .join("chromium-extension")
}

fn source() -> PathBuf {
    super::proc::package_dir(&super::repo::main_dir()).join("chromium")
}

/// Whether `command` is on the PATH.
fn on_path(command: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(command).is_file()))
}

/// Where a browser keeps its profiles and its flags file.
fn config_of(browser: &str) -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"));
    config.join(if browser.starts_with("chromium") {
        "chromium"
    } else {
        "google-chrome"
    })
}

fn flags_of(browser: &str) -> PathBuf {
    PathBuf::from(format!("{}-flags.conf", config_of(browser).display()))
}

/// The copy made the same as the checkout's extension; true when something changed.
fn sync_extension() -> bool {
    let Ok(entries) = fs::read_dir(source()) else {
        return false;
    };
    let target = install_dir();
    if fs::create_dir_all(&target).is_err() {
        return false;
    }
    let mut is_changed = false;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == "README.md" || !entry.path().is_file() {
            continue;
        }
        let Ok(from) = fs::read(entry.path()) else { continue };
        let to = target.join(&name);
        if fs::read(&to).is_ok_and(|known| known == from) {
            continue;
        }
        is_changed |= fs::write(to, from).is_ok();
    }
    is_changed
}

fn is_in_flags(browser: &str) -> bool {
    fs::read_to_string(flags_of(browser)).is_ok_and(|text| text.contains(&*install_dir().to_string_lossy()))
}

fn has_extension(browser: &str) -> bool {
    if is_in_flags(browser) {
        return true;
    }
    let Ok(profiles) = fs::read_dir(config_of(browser)) else {
        return false;
    };
    profiles.flatten().any(|profile| {
        ["Preferences", "Secure Preferences"]
            .iter()
            .any(|file| fs::read_to_string(profile.path().join(file)).is_ok_and(|text| text.contains(EXTENSION_ID)))
    })
}

/// The browser that can group tabs, looked up once (the copy synced with it).
fn grouping_browser() -> Option<&'static str> {
    static FOUND: OnceLock<Option<&'static str>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        let found = BROWSERS
            .into_iter()
            .find(|browser| on_path(browser) && has_extension(browser));
        if found.is_some() {
            sync_extension();
        }
        found
    })
}

/// Whether a url can go into a tab group.
pub fn can_group_tabs() -> bool {
    grouping_browser().is_some()
}

fn detached(command: &str, args: &[&str]) {
    let _ = Command::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// `encodeURIComponent` as URLSearchParams writes it (space as `+`).
fn form(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => (byte as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// `url` in the tab group `group`, without waiting for the browser. True when something was opened.
pub fn open_in_group(url: &str, group: &str) -> bool {
    if url.is_empty() {
        return false;
    }
    match grouping_browser().filter(|_| !group.is_empty()) {
        Some(browser) => {
            let page = format!(
                "chrome-extension://{EXTENSION_ID}/open.html?url={}&group={}",
                form(url),
                form(group)
            );
            detached(browser, &[&page]);
        }
        None => detached(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }, &[url]),
    }
    true
}

/// `pnpm browser`.
pub fn command() -> ExitCode {
    let is_changed = sync_extension();
    let dir = install_dir();
    println!(
        "\n✓ the extension is in {}{}",
        dir.display(),
        if is_changed {
            " (updated: restart the browser)"
        } else {
            ""
        }
    );
    match BROWSERS.into_iter().find(|browser| on_path(browser)) {
        None => println!("  no Chromium or Chrome found: urls open with xdg-open, without a group\n"),
        Some(browser) if is_in_flags(browser) => println!("  {browser} loads it ({})\n", flags_of(browser).display()),
        Some(browser) if has_extension(browser) => println!("  {browser} has it installed\n"),
        Some(browser) => println!(
            "\n  {browser} doesn't load it yet. Either\n    add {dir} to the --load-extension= list in {flags} and restart it, or\n    chrome://extensions → Developer mode → Load unpacked → {dir}\n",
            dir = dir.display(),
            flags = flags_of(browser).display()
        ),
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_url_search_params() {
        assert_eq!(form("http://localhost:3002"), "http%3A%2F%2Flocalhost%3A3002");
        assert_eq!(form("a b"), "a+b");
        assert!(config_of("chromium-browser").ends_with("chromium"));
    }
}
