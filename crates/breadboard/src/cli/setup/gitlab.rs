//! GitLab's REST API, read only, for the setup commands: `glab api
//! <endpoint>` (`projects/:fullpath/…`), glab's login and host. One process
//! per call, as lib/glab.mjs does without a token; several side by side
//! where a command reads many. The kept-alive HTTP client with the token and
//! GitLab's rate limit belongs in breadboard-core (rewrite-plan.md), shared
//! with the overview; these calls move to it once the halves meet.
//!
//! A call that fails for a reason that passes (a 5xx, a timeout, the
//! network) is tried again after 1 s and 4 s, as lib/glab.mjs does; what
//! still fails is a `Failure` whose `reason` says why in a few words.

use regex::Regex;
use serde_json::Value;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// How long one call may take.
const TIMEOUT: Duration = Duration::from_secs(15);
/// The pauses before the second and third try.
const BACKOFF: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(4)];

/// A call that failed: why in a few words (`HTTP 502 Bad Gateway`,
/// `HTTP 429, rate limit`, `timeout`) and what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub reason: String,
    pub endpoint: String,
    /// Whether another try may pass: a 5xx, a timeout, the network; never a
    /// 429, which pauses every caller until GitLab's `retry-after` (#462).
    pub is_passing: bool,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GitLab: {} ({})", self.reason, self.endpoint)
    }
}

impl From<Failure> for String {
    fn from(failure: Failure) -> String {
        failure.to_string()
    }
}

/// `glab api <endpoint>` parsed, tried again while it fails for a reason that passes.
pub fn api(endpoint: &str) -> Result<Value, Failure> {
    with_tries(&BACKOFF, || once(endpoint))
}

/// `api` of an endpoint that answers a list.
pub fn list(endpoint: &str) -> Result<Vec<Value>, Failure> {
    api(endpoint).map(|value| value.as_array().cloned().unwrap_or_default())
}

/// `call` until it passes or fails for good, `backoff` the pauses between tries.
fn with_tries<T>(backoff: &[Duration], call: impl Fn() -> Result<T, Failure>) -> Result<T, Failure> {
    let mut last = call();
    for pause in backoff {
        match &last {
            Err(failure) if failure.is_passing => thread::sleep(*pause),
            _ => break,
        }
        last = call();
    }
    last
}

/// What a pipe of a child said, read on a thread of its own so neither fills up.
fn drain(pipe: Option<impl Read + Send + 'static>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

/// One `glab api`, killed after TIMEOUT.
fn once(endpoint: &str) -> Result<Value, Failure> {
    let failed = |reason: String, is_passing: bool| Failure {
        reason,
        endpoint: endpoint.to_string(),
        is_passing,
    };
    if let Some(pause) = breadboard_core::gitlab::budget::glab_may("glab", &["api", endpoint]) {
        return Err(failed(pause, false));
    }
    let mut child = Command::new("glab")
        .args(["api", endpoint])
        .current_dir(super::repo::main_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => failed(
                "glab is not installed (https://gitlab.com/gitlab-org/cli)".into(),
                false,
            ),
            _ => failed(format!("glab did not start: {error}"), false),
        })?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < TIMEOUT => thread::sleep(Duration::from_millis(2)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    let Some(status) = status else {
        return Err(failed("timeout".into(), true));
    };
    if !status.success() {
        let said = String::from_utf8_lossy(&stderr);
        if let Some(pause) = breadboard_core::gitlab::budget::glab_failed(&said) {
            return Err(failed(pause, false));
        }
        let (reason, is_passing) = reason_of(&said);
        return Err(failed(reason, is_passing));
    }
    serde_json::from_slice(&stdout).map_err(|_| failed("its answer was no JSON".into(), false))
}

/// What a failed `glab api` said, in the words of lib/glab.mjs (`HTTP 502
/// Bad Gateway`, `HTTP 429, rate limit`, `timeout`), the network's in a few
/// words (`connection refused`, anywhere in what it said: a crash names it
/// above its last line), and whether another try may pass.
fn reason_of(stderr: &str) -> (String, bool) {
    static STATUS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:HTTP )?([1-5]\d\d)\b(?: ([A-Za-z][A-Za-z' ]*[A-Za-z]))?|\(HTTP ([1-5]\d\d)\)").unwrap()
    });
    static NETWORK: [(&str, &str); 6] = [
        ("connection refused", "connection refused"),
        ("econnrefused", "connection refused"),
        ("connection reset", "connection reset"),
        ("econnreset", "connection reset"),
        ("no such host", "host not found"),
        ("enotfound", "host not found"),
    ];
    static NETWORK_ELSE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)network is unreachable|enetunreach|unexpected EOF|tls handshake|fetch failed").unwrap()
    });
    static TIMEOUT_SAID: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)timeout|timed out|deadline exceeded").unwrap());
    let line = stderr
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("glab failed");
    let said = line.strip_prefix("glab: ").unwrap_or(line);
    if let Some(found) = STATUS.captures(said) {
        let status: u16 = found
            .get(1)
            .or_else(|| found.get(3))
            .and_then(|status| status.as_str().parse().ok())
            .unwrap_or_default();
        let reason = match (status, found.get(2)) {
            (429, _) => "HTTP 429, rate limit".to_string(),
            (_, Some(text)) => format!("HTTP {status} {}", text.as_str()),
            (_, None) => format!("HTTP {status}"),
        };
        return (reason, status >= 500);
    }
    if TIMEOUT_SAID.is_match(said) {
        return ("timeout".to_string(), true);
    }
    let all = stderr.to_lowercase();
    if let Some((_, kind)) = NETWORK.iter().find(|(said, _)| all.contains(said)) {
        return (kind.to_string(), true);
    }
    // `fetch failed`: what Node's fetch says without a connection.
    if NETWORK_ELSE.is_match(stderr) {
        return ("no connection".to_string(), true);
    }
    (said.to_string(), false)
}

/// `run` for each of `items`, at most `at_once` side by side (a pool, as
/// `inTurns`: a slow call holds up none of the others), the results in their order.
pub fn in_turns<T: Sync, R: Send>(items: &[T], at_once: usize, run: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let mut done: Vec<(usize, R)> = thread::scope(|scope| {
        let workers: Vec<_> = (0..at_once.clamp(1, items.len().max(1)))
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let at = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(at) else { break mine };
                        mine.push((at, run(item)));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("a GitLab read panicked"))
            .collect()
    });
    done.sort_by_key(|(at, _)| *at);
    done.into_iter().map(|(_, result)| result).collect()
}

/// `in_turns` of reads worth nothing without each other: the first that
/// fails for good is the answer, and no read starts after it (a GitLab that
/// is gone costs one round of tries, not one per read).
pub fn all_in_turns<T: Sync, V: Send, E: Send>(
    items: &[T],
    at_once: usize,
    run: impl Fn(&T) -> Result<V, E> + Sync,
) -> Result<Vec<V>, E> {
    let is_failed = AtomicBool::new(false);
    in_turns(items, at_once, |item| {
        if is_failed.load(Ordering::Relaxed) {
            return None;
        }
        let result = run(item);
        if result.is_err() {
            is_failed.store(true, Ordering::Relaxed);
        }
        Some(result)
    })
    .into_iter()
    .flatten()
    .collect()
}

/// `encodeURIComponent`.
pub fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn encodes_a_branch() {
        assert_eq!(encode("feature/40-x"), "feature%2F40-x");
    }

    #[test]
    fn runs_in_turns_in_order() {
        assert_eq!(in_turns(&[1, 2, 3, 4, 5], 2, |n| n * 2), vec![2, 4, 6, 8, 10]);
        assert!(in_turns(&[] as &[i32], 4, |n| *n).is_empty());
    }

    #[test]
    fn stops_at_the_first_read_that_fails() {
        let asked = AtomicUsize::new(0);
        let read = all_in_turns(&[1, 2, 3, 4, 5], 1, |n| {
            asked.fetch_add(1, Ordering::Relaxed);
            if *n == 2 { Err(*n) } else { Ok(*n) }
        });
        assert_eq!((read, asked.load(Ordering::Relaxed)), (Err(2), 2));
        assert_eq!(all_in_turns(&[1, 2], 2, |n| Ok::<_, ()>(n * 3)), Ok(vec![3, 6]));
    }

    #[test]
    fn tells_what_passes_from_what_does_not() {
        assert_eq!(
            reason_of("glab: 502 Bad Gateway\n"),
            ("HTTP 502 Bad Gateway".to_string(), true)
        );
        assert_eq!(
            reason_of("glab: 503 Service Unavailable (HTTP 503)"),
            ("HTTP 503 Service Unavailable".to_string(), true)
        );
        assert_eq!(
            reason_of("glab: 429 Too Many Requests (HTTP 429)"),
            ("HTTP 429, rate limit".to_string(), false)
        );
        assert_eq!(
            reason_of("glab: 404 Not Found"),
            ("HTTP 404 Not Found".to_string(), false)
        );
        assert_eq!(
            reason_of("Get \"https://gitlab.com/api/v4/x\": dial tcp: i/o timeout"),
            ("timeout".to_string(), true)
        );
        assert_eq!(
            reason_of("dial tcp: lookup gitlab.com: no such host"),
            ("host not found".to_string(), true)
        );
        // A crash of what answers for glab: its cause, not its last line.
        assert_eq!(
            reason_of(
                "TypeError: fetch failed\n  [cause]: Error: connect ECONNREFUSED 127.0.0.1:9\n}\n\nNode.js v25.8.0\n"
            ),
            ("connection refused".to_string(), true)
        );
        assert_eq!(
            reason_of("[TypeError: fetch failed] {\n  [cause]: Error: bad port\n}\n\nNode.js v25.8.0\n"),
            ("no connection".to_string(), true)
        );
        assert_eq!(
            reason_of("no merge request for x"),
            ("no merge request for x".to_string(), false)
        );
    }

    #[test]
    fn tries_again_only_what_passes() {
        let failure = |is_passing| Failure {
            reason: "r".into(),
            endpoint: "e".into(),
            is_passing,
        };
        let calls = Cell::new(0);
        let passing = with_tries(&[Duration::ZERO; 2], || {
            calls.set(calls.get() + 1);
            if calls.get() < 3 {
                Err(failure(true))
            } else {
                Ok(calls.get())
            }
        });
        assert_eq!(passing, Ok(3));
        calls.set(0);
        let lasting: Result<(), Failure> = with_tries(&[Duration::ZERO; 2], || {
            calls.set(calls.get() + 1);
            Err(failure(false))
        });
        assert_eq!((lasting.is_err(), calls.get()), (true, 1));
        calls.set(0);
        let _ = with_tries::<()>(&[Duration::ZERO; 2], || {
            calls.set(calls.get() + 1);
            Err(failure(true))
        });
        assert_eq!(calls.get(), 3);
        assert_eq!(failure(true).to_string(), "GitLab: r (e)");
    }
}
