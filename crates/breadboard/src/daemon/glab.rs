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
