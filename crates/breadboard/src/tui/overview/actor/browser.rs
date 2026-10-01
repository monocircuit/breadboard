//! A url into the browser (browser.mjs, io.mjs): into the tab group named
//! after the herdr space it belongs to when a Chromium with the extension
//! is there, else `xdg-open`; an MR or issue as a popup window of its own,
//! floated in the middle under Hyprland. Never waited for.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Fixed by the `key` of packages/breadboard/chromium/manifest.json.
const EXTENSION_ID: &str = "aikcnbeljldocaafkokjbngkboecmoli";
const BROWSERS: [&str; 4] = ["chromium", "chromium-browser", "google-chrome-stable", "google-chrome"];
const POPUP: (u32, u32) = (960, 720);
/// How long a new window of the browser is looked for: a cold start takes a moment.
const POPUP_WAIT: Duration = Duration::from_millis(8000);

/// Whether `program` is on the PATH.
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            let file = dir.join(program);
            std::fs::metadata(&file).is_ok_and(|meta| meta.is_file())
        })
    })
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn config_of(browser: &str) -> PathBuf {
    let name = if browser.starts_with("chromium") {
        "chromium"
    } else {
        "google-chrome"
    };
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join(name)
}

/// Where the browser loads the extension from: a copy outside every checkout.
fn install_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local").join("share"))
        .join("monocircuit")
        .join("chromium-extension")
}

/// Whether the browser knows the extension: in its flags file, or a profile names its id.
fn has_extension(browser: &str) -> bool {
    let config = config_of(browser);
    let flags = PathBuf::from(format!("{}-flags.conf", config.display()));
    let install = install_dir();
    if std::fs::read_to_string(flags).is_ok_and(|text| text.contains(&*install.to_string_lossy())) {
        return true;
    }
    let Ok(profiles) = std::fs::read_dir(&config) else {
        return false;
    };
    profiles.flatten().any(|profile| {
        ["Preferences", "Secure Preferences"].iter().any(|file| {
            std::fs::read_to_string(profile.path().join(file)).is_ok_and(|text| text.contains(EXTENSION_ID))
        })
    })
}

/// The copy of the extension brought up to the one of the checkout `main`.
fn sync_extension(main: &Path) {
    let source = main.join("packages").join("breadboard").join("chromium");
    let Ok(files) = std::fs::read_dir(&source) else { return };
    let install = install_dir();
    if std::fs::create_dir_all(&install).is_err() {
        return;
    }
    for file in files.flatten() {
        let name = file.file_name();
        if name == "README.md" {
            continue;
        }
        let Ok(from) = std::fs::read(file.path()) else { continue };
        let to = install.join(&name);
        if std::fs::read(&to).is_ok_and(|was| was == from) {
            continue;
        }
        let _ = std::fs::write(to, from);
    }
}

/// The browser that groups tabs, looked up once, its copy of the extension synced.
fn grouping_browser(main: &Path) -> Option<&'static str> {
    static FOUND: OnceLock<Option<&'static str>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        let found = BROWSERS
            .into_iter()
            .find(|browser| on_path(browser) && has_extension(browser));
        if found.is_some() {
            sync_extension(main);
        }
        found
    })
}

fn detached(program: &str, args: &[String]) {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        // Reaped on a thread of its own: no zombie, no wait on the view.
        std::thread::spawn(move || child.wait());
    }
}

/// `url` with the characters a query must escape escaped (URLSearchParams).
fn query_value(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => out.push(byte as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// `url` in the tab group `group` (a herdr space's label), without waiting (`openInGroup`).
pub fn open_in_group(main: &Path, url: &str, group: &str) {
    if url.is_empty() {
        return;
    }
    match grouping_browser(main).filter(|_| !group.is_empty()) {
        Some(browser) => {
            let page = format!(
                "chrome-extension://{EXTENSION_ID}/open.html?url={}&group={}",
                query_value(url),
                query_value(group)
            );
            detached(browser, &[page]);
        }
        None => detached(
            if cfg!(target_os = "macos") { "open" } else { "xdg-open" },
            &[url.to_string()],
        ),
    }
}

/// `url` in a window of its own without tabs (`--app`), floated under
/// Hyprland; without Chromium or Chrome like any url (`openPopup`).
pub fn open_popup(main: &Path, url: &str, group: &str) {
    if url.is_empty() {
        return;
    }
    let Some(browser) = BROWSERS.into_iter().find(|browser| on_path(browser)) else {
        return open_in_group(main, url, group);
    };
    let before = hypr_windows();
    detached(
        browser,
        &[format!("--app={url}"), format!("--window-size={},{}", POPUP.0, POPUP.1)],
    );
    let host = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default()
        .to_string();
    std::thread::spawn(move || float_new_window(&host, &before));
}

fn hyprctl(args: &[&str]) -> String {
    Command::new("hyprctl")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

/// The windows under Hyprland: (address, class).
fn hypr_windows() -> Vec<(String, String)> {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return Vec::new();
    }
    serde_json::from_str::<serde_json::Value>(&hyprctl(&["clients", "-j"]))
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|client| {
            let field = |key: &str| client.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string();
            (field("address"), field("class"))
        })
        .collect()
}

/// The popup's new window floated at POPUP's size in the middle: only that window, no rule in the config.
fn float_new_window(host: &str, before: &[(String, String)]) {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return;
    }
    let prefix = format!("chrome-{host}__");
    let until = Instant::now() + POPUP_WAIT;
    let window = loop {
        let found = hypr_windows()
            .into_iter()
            .find(|(address, class)| class.starts_with(&prefix) && !before.iter().any(|(known, _)| known == address));
        match found {
            Some((address, _)) => break address,
            None if Instant::now() < until => std::thread::sleep(Duration::from_millis(150)),
            None => return,
        }
    };
    let target = format!("address:{window}");
    let (width, height) = POPUP;
    // Hyprland ≥ 0.55 dispatches Lua (hl.dsp.*); an older one the classic dispatchers.
    let lua = [
        format!("hl.dsp.window.float({{ action = \"enable\", window = \"{target}\" }})"),
        format!("hl.dsp.window.resize({{ x = {width}, y = {height}, window = \"{target}\" }})"),
        format!("hl.dsp.window.center({{ window = \"{target}\" }})"),
        format!("hl.dsp.focus({{ window = \"{target}\" }})"),
    ];
    if hyprctl(&["dispatch", &lua[0]]).trim() == "ok" {
        for code in &lua[1..] {
            hyprctl(&["dispatch", code]);
        }
        return;
    }
    let batch = [
        format!("dispatch setfloating {target}"),
        format!("dispatch resizewindowpixel exact {width} {height},{target}"),
        format!("dispatch focuswindow {target}"),
        "dispatch centerwindow".to_string(),
    ]
    .join(" ; ");
    hyprctl(&["--batch", &batch]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_a_query_as_url_search_params_does() {
        assert_eq!(
            query_value("http://localhost:3002/atlas?x=1"),
            "http%3A%2F%2Flocalhost%3A3002%2Fatlas%3Fx%3D1"
        );
        assert_eq!(query_value("Monocircuit/Platform"), "Monocircuit%2FPlatform");
        assert_eq!(query_value("a b"), "a+b");
    }
}
