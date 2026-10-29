//! The commands of lab web (#503): the node holds one stream to the server
//! (`GET /api/node/stream`, Server-Sent Events), every command on it is
//! done as the command table does one asked over the socket (`acts::command`),
//! and its result goes back (`POST /api/node/commands/{id}/ack`).
//!
//! The node only ever calls out: the stream is its request, the ack another.
//! A command the server sent again (no ack reached it, the stream broke) is
//! done once: the table keeps its result by id and gives that one.

use super::serve::State;
use breadboard_core::command::{Asked, Command, Done, PROTOCOL};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// The first wait after the stream broke, doubled up to LONGEST.
const FIRST: Duration = Duration::from_secs(2);
const LONGEST: Duration = Duration::from_secs(60);
/// A refusal (token revoked, another protocol) is asked again this seldom.
const REFUSED: Duration = Duration::from_secs(300);
/// How often a node logged out (or not the one that pushes) looks again.
const LOOK: Duration = Duration::from_secs(5);
/// The server says something at least this often (its keep-alive); silent longer, the stream is dead.
const SILENT: Duration = Duration::from_secs(90);
/// An ack is tried this many times, FIRST apart.
const ACKS: u32 = 5;

/// One command as the stream carries it: `data: {protocol, id, main, command}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Streamed {
    pub protocol: u32,
    pub id: String,
    /// The main checkout of the repo it is for, as the node's snapshot names it.
    pub main: PathBuf,
    pub command: Command,
}

/// What came of it, as the ack carries it: `{protocol, result: {id, isOk, text}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ack {
    pub protocol: u32,
    pub result: AckResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AckResult {
    pub id: String,
    pub is_ok: bool,
    pub text: String,
}

impl From<Done> for AckResult {
    fn from(done: Done) -> AckResult {
        AckResult {
            id: done.id,
            is_ok: done.is_ok,
            text: done.text,
        }
    }
}

/// Where the node reaches lab web and who it is there: its login (`lab login`, #501).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Login {
    /// The server's origin, `https://lab.monocircuit.net`.
    server: String,
    token: String,
}

impl Login {
    /// The login of now, None while logged out.
    async fn now() -> Option<Login> {
        let config = tokio::task::spawn_blocking(breadboard_core::lab::read).await.ok().flatten()?;
        Some(Login {
            token: config.token.clone().filter(|_| config.is_logged_in())?,
            server: config.server,
        })
    }
}

/// How a stream ended.
#[derive(Debug, PartialEq, Eq)]
enum Ended {
    /// It broke, or never opened: again soon.
    Broke(String),
    /// The server refuses this node (401, 409): again seldom.
    Refused(String),
}

/// The stream, as long as the daemon runs: held by the daemon that pushes
/// (`lab::holds`), while the user is logged in.
pub(super) async fn follow(state: Arc<State>) {
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let mut wait = FIRST;
    loop {
        let login = match Login::now().await {
            Some(login) if super::lab::holds() => login,
            _ => {
                tokio::time::sleep(LOOK).await;
                continue;
            }
        };
        let opened = std::time::Instant::now();
        let ended = once(&state, &http, &login).await;
        let pause = match &ended {
            Ended::Refused(why) => {
                eprintln!("serve: lab refuses the stream: {why}");
                // Asked again after a while, at once after another login.
                let until = tokio::time::Instant::now() + REFUSED;
                while tokio::time::Instant::now() < until && Login::now().await.as_ref() == Some(&login) {
                    tokio::time::sleep(LOOK).await;
                }
                wait = FIRST;
                continue;
            }
            Ended::Broke(why) => {
                // A stream that held a while starts over from the first wait.
                if opened.elapsed() > LONGEST {
                    wait = FIRST;
                }
                eprintln!("serve: lab stream ended: {why}");
                let pause = wait;
                wait = (wait * 2).min(LONGEST);
                pause
            }
        };
        tokio::time::sleep(pause).await;
    }
}

/// One stream, followed until it ends.
async fn once(state: &Arc<State>, http: &reqwest::Client, login: &Login) -> Ended {
    let url = format!("{}/api/node/stream?protocol={PROTOCOL}", login.server.trim_end_matches('/'));
    let mut response = match http
        .get(&url)
        .bearer_auth(&login.token)
        .header("accept", "text/event-stream")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => return Ended::Broke(error.to_string()),
    };
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::CONFLICT {
        let body = response.text().await.unwrap_or_default();
        return Ended::Refused(format!("{status} {body}"));
    }
    if !status.is_success() {
        return Ended::Broke(status.to_string());
    }
    let mut blocks = Blocks::default();
    loop {
        let chunk = match tokio::time::timeout(SILENT, response.chunk()).await {
            Err(_) => return Ended::Broke(format!("silent for {}s", SILENT.as_secs())),
            Ok(Err(error)) => return Ended::Broke(error.to_string()),
            Ok(Ok(None)) => return Ended::Broke("closed by the server".into()),
            Ok(Ok(Some(chunk))) => chunk,
        };
        for data in blocks.push(&chunk) {
            let Some(streamed) = command_of(&data) else {
                continue;
            };
            let (state, http, login) = (state.clone(), http.clone(), login.clone());
            tokio::spawn(async move { run(&state, &http, &login, streamed).await });
        }
    }
}

/// The command of a block, None for what isn't one (`ready`, another protocol's).
fn command_of(data: &str) -> Option<Streamed> {
    match serde_json::from_str::<Streamed>(data) {
        Ok(streamed) => Some(streamed),
        Err(error) => {
            if data.contains("\"command\"") {
                eprintln!("serve: lab sent a command this node can't read: {error}");
            }
            None
        }
    }
}

/// A command done and acked.
async fn run(state: &Arc<State>, http: &reqwest::Client, login: &Login, streamed: Streamed) {
    let Streamed {
        protocol,
        id,
        main,
        command,
    } = streamed;
    let done = super::acts::command(
        state,
        Asked {
            id: id.clone(),
            protocol,
            main,
            command,
        },
    )
    .await;
    let ack = Ack {
        protocol: PROTOCOL,
        result: done.into(),
    };
    let url = format!("{}/api/node/commands/{id}/ack", login.server.trim_end_matches('/'));
    for attempt in 0..ACKS {
        if attempt > 0 {
            tokio::time::sleep(FIRST * attempt).await;
        }
        match http
            .post(&url)
            .bearer_auth(&login.token)
            .timeout(Duration::from_secs(20))
            .json(&ack)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => return,
            // Refused for good (another's command, gone): trying again changes nothing.
            Ok(response) if response.status().is_client_error() => {
                eprintln!("serve: lab refused the ack of {id}: {}", response.status());
                return;
            }
            Ok(_) | Err(_) => {}
        }
    }
    // The server sends it again on the next stream; the table answers it with this result.
    eprintln!("serve: the ack of {id} didn't reach lab");
}

/// The blocks of an event stream as they come, cut at an empty line: the
/// `data:` lines of each, joined. Comments (`: keep-alive`) and the other
/// fields (`event:`, `retry:`) carry nothing for the node.
#[derive(Default)]
struct Blocks {
    pending: String,
}

impl Blocks {
    fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.pending.push_str(&String::from_utf8_lossy(chunk));
        if self.pending.contains('\r') {
            self.pending = self.pending.replace("\r\n", "\n").replace('\r', "\n");
        }
        let mut found = Vec::new();
        while let Some(end) = self.pending.find("\n\n") {
            let block: String = self.pending.drain(..end + 2).collect();
            let data: Vec<&str> = block
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|value| value.strip_prefix(' ').unwrap_or(value))
                .collect();
            if !data.is_empty() {
                found.push(data.join("\n"));
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_the_stream_into_its_data_wherever_the_chunks_end() {
        let mut blocks = Blocks::default();
        assert!(blocks.push(b"retry: 3000\nevent: ready\ndata: {}\n").is_empty());
        assert_eq!(blocks.push(b"\n: keep-alive\n\nevent: command\r\ndata: {\"a\":"), vec!["{}"]);
        assert_eq!(blocks.push(b"1}\r\n\r\n"), vec!["{\"a\":1}"]);
        assert_eq!(blocks.push(b"data: one\ndata:two\n\n"), vec!["one\ntwo"]);
        assert!(blocks.pending.is_empty());
    }

    #[test]
    fn reads_a_command_and_says_its_result_in_camel_case() {
        let line = r#"{"protocol":1,"id":"c1","main":"/repo","command":{"kind":"tell","dir":"/repo-x","line":"go on"}}"#;
        let streamed = command_of(line).unwrap();
        assert_eq!(
            streamed,
            Streamed {
                protocol: 1,
                id: "c1".into(),
                main: "/repo".into(),
                command: Command::Tell {
                    dir: "/repo-x".into(),
                    line: "go on".into(),
                },
            }
        );
        assert_eq!(command_of(r#"{}"#), None);
        let ack = Ack {
            protocol: PROTOCOL,
            result: Done::of("c1", Err("no agent".into())).into(),
        };
        assert_eq!(
            serde_json::to_string(&ack).unwrap(),
            r#"{"protocol":1,"result":{"id":"c1","isOk":false,"text":"no agent"}}"#
        );
    }
}
