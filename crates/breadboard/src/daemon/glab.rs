//! `glab api` as the user, from the main checkout: what the daemon's tasks
//! (the CI queue, the job watcher, ready, keep up) ask GitLab directly.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// `glab api <endpoint>` as the user, from the main checkout: its JSON.
pub fn glab_get(main: &Path, glab: &str, endpoint: &str) -> Result<serde_json::Value, String> {
    glab_api(main, glab, &["api", endpoint])
}

/// `glab api --method POST <endpoint>` as the user, from the main checkout: its JSON.
pub fn glab_post(main: &Path, glab: &str, endpoint: &str) -> Result<serde_json::Value, String> {
    glab_api(main, glab, &["api", "--method", "POST", endpoint])
}

/// `glab api --method PUT <endpoint>` as the user, from the main checkout: its JSON.
pub fn glab_put(main: &Path, glab: &str, endpoint: &str) -> Result<serde_json::Value, String> {
    glab_api(main, glab, &["api", "--method", "PUT", endpoint])
}

/// A read the daemon polls (the CI job watcher's): through lab web while the
/// user is logged in to lab (#505), every node's the same answer; else, or
/// where lab doesn't serve it, `glab api` as the user. Never a read a write
/// decides on: those stay the user's own (`glab_get`).
pub fn polled_get(main: &Path, glab: &str, endpoint: &str) -> Result<serde_json::Value, String> {
    match through_lab(main, glab, endpoint) {
        Some(answer) => answer.map(|text| serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)),
        None => glab_get(main, glab, endpoint),
    }
}

/// `polled_get` of a text answer (a job's log).
pub fn polled_text(main: &Path, glab: &str, endpoint: &str) -> Result<String, String> {
    through_lab(main, glab, endpoint).unwrap_or_else(|| glab_text(main, glab, endpoint))
}

/// `projects/:fullpath/<path>` through lab's proxy; None: glab asks GitLab.
/// The real glab only: a test's fake glab is its GitLab.
fn through_lab(main: &Path, glab: &str, endpoint: &str) -> Option<Result<String, String>> {
    let path = endpoint.strip_prefix("projects/:fullpath/")?;
    if glab != "glab" {
        return None;
    }
    let source = breadboard_core::gitlab::lab::source()?;
    let url = breadboard_core::git::git(main, &["config", "--get", "remote.origin.url"]);
    let (_, project) = breadboard_core::gitlab::client::remote_of(url.stdout.trim()).filter(|_| url.is_ok)?;
    let handle = tokio::runtime::Handle::try_current().ok()?;
    let answer = handle.block_on(breadboard_core::gitlab::lab::rest(&source, &project, path))?;
    Some(answer.map_err(|error| error.reason))
}

/// `glab api <endpoint>` of a text answer (a job's log), as the user, from the main checkout.
pub fn glab_text(main: &Path, glab: &str, endpoint: &str) -> Result<String, String> {
    glab_out(main, glab, &["api", endpoint]).map(|out| String::from_utf8_lossy(&out).into_owned())
}

pub fn glab_api(main: &Path, glab: &str, args: &[&str]) -> Result<serde_json::Value, String> {
    glab_out(main, glab, args).map(|out| serde_json::from_slice(&out).unwrap_or(serde_json::Value::Null))
}

fn glab_out(main: &Path, glab: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    // A 429 paused GitLab for every caller (#462): nothing is asked before it ends.
    if let Some(pause) = breadboard_core::gitlab::budget::glab_may("daemon glab", args) {
        return Err(pause);
    }
    let spawn = || {
        Command::new(glab)
            .args(args)
            .current_dir(main)
            .stdin(Stdio::null())
            .output()
    };
    // A glab just written (a test's) is busy while another thread forks: a moment later it runs.
    let mut output = spawn();
    for _ in 0..20 {
        if !matches!(&output, Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy) {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
        output = spawn();
    }
    let output = output.map_err(|error| format!("glab: {error}"))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let said = if stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        stderr
    };
    Err(breadboard_core::gitlab::budget::glab_failed(&said).unwrap_or(said))
}

/// A glab answering from `answers` (`$*` → its JSON), each call logged to `<scratch>/glab.log`.
#[cfg(test)]
pub fn fake_glab(scratch: &Path, answers: &[(&str, &str)]) -> String {
    use std::os::unix::fs::PermissionsExt;
    let bin = scratch.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = scratch.join("glab.log");
    let cases: String = answers
        .iter()
        .map(|(args, json)| format!("  '{args}') echo '{json}' ;;\n"))
        .collect();
    let glab = bin.join("glab");
    std::fs::write(
        &glab,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$*\" in\n{cases}  *) echo '{{}}' ;;\nesac\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o755)).unwrap();
    glab.to_string_lossy().into_owned()
}
