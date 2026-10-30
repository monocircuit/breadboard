//! A node's side of lab web (#501): its login and what it sends.
//!
//! `breadboard lab login` signs the user in with the device flow (RFC 8628)
//! against lab's OpenID provider, which lab web names (`GET /api/node/login`).
//! The provider's token goes to `POST /api/node/register` once, which answers
//! lab's own node token, bound to the user and this machine (`machine`, an
//! id the node keeps). The node keeps only that token (`config_file`, 0600),
//! a new login replaces it.
//!
//! The daemon pushes with it, outbound only, so a node behind NAT works:
//! the full snapshot on connect (`POST /api/node/snapshot`), then batched
//! events (`POST /api/node/events`), an empty batch at least every
//! HEARTBEAT so lab web sees the node online. A push that fails is retried
//! after `backoff`, and the next one after a failure is a snapshot again.
//!
//! Every body carries `protocol` (PROTOCOL); its payloads are core's
//! `node::Snapshot`, `node::Event` (their own protocol inside) and, over
//! lab's stream (#503), `command::Command`. lab web's zod contract
//! (`products/lab/web/src/server/node/protocol.ts`) mirrors this file.

use crate::node::{Event, Snapshot};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The version of the envelope: a node and a lab web of another refuse each other (409).
pub const PROTOCOL: u32 = 1;
/// The longest a node stays silent: lab web marks a node offline after three of them.
pub const HEARTBEAT: Duration = Duration::from_secs(30);
/// How long a request may take.
const TIMEOUT: Duration = Duration::from_secs(20);
/// The OAuth grant of the device flow.
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// What the node keeps of its login.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// lab web's origin, no trailing slash.
    pub server: String,
    /// This machine's id, kept across logins.
    pub machine: String,
    /// lab's id of the node, its token and who signed in; None once logged out.
    pub node: Option<String>,
    pub token: Option<String>,
    pub user: Option<String>,
    /// The main checkouts it pushes, besides those an overview shows.
    #[serde(default)]
    pub repos: Vec<PathBuf>,
}

impl Config {
    /// Logged in: a node token to push with.
    pub fn is_logged_in(&self) -> bool {
        self.token.is_some()
    }
}

/// Where the login is kept: `$XDG_CONFIG_HOME/breadboard/lab.json`.
pub fn config_file() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("breadboard").join("lab.json")
}

/// The kept login, None while there is none.
pub fn read() -> Option<Config> {
    read_at(&config_file())
}

pub fn read_at(file: &Path) -> Option<Config> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

/// The login kept, readable by the user alone.
pub fn write(config: &Config) -> std::io::Result<()> {
    write_at(&config_file(), config)
}

pub fn write_at(file: &Path, config: &Config) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let beside = file.with_extension(format!("json.{}", std::process::id()));
    let text = serde_json::to_string_pretty(config).map_err(std::io::Error::other)?;
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&beside)?;
    out.write_all(format!("{text}\n").as_bytes())?;
    drop(out);
    std::fs::rename(&beside, file)
}

/// A new machine id: 32 hex digits.
pub fn new_machine() -> String {
    crate::conversations::new_session_id().replace('-', "")
}

/// `GET /api/node/login`: where the node signs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Login {
    pub protocol: u32,
    /// The OpenID provider's issuer.
    pub issuer: String,
    pub client_id: String,
}

/// `POST /api/node/register`, with the provider's access token as its bearer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Register {
    pub protocol: u32,
    pub machine: String,
    pub host: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registered {
    pub protocol: u32,
    pub node: String,
    pub token: String,
    /// Who signed in, as lab web names them.
    pub user: String,
}

/// `POST /api/node/snapshot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotPush {
    pub protocol: u32,
    pub snapshot: Snapshot,
}

/// An event with its place: per node, rising; one lab web holds already is skipped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sequenced {
    pub seq: i64,
    pub event: Event,
}

/// `POST /api/node/events`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsPush {
    pub protocol: u32,
    pub events: Vec<Sequenced>,
}

/// lab web's answer to a snapshot or events: the last seq it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pushed {
    pub protocol: u32,
    pub seq: i64,
}

/// Why a request to lab web or its provider failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failed {
    /// 401: the token is unknown or revoked: `lab login` again.
    Unauthorized,
    /// 409: lab web speaks another protocol: another breadboard is needed.
    Protocol(String),
    /// Anything else: unreachable, a 5xx, a body it didn't take; tried again.
    Other(String),
}

impl std::fmt::Display for Failed {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failed::Unauthorized => write!(out, "lab web doesn't know this node's token: run lab login again"),
            Failed::Protocol(why) => write!(out, "lab web speaks another protocol: {why}"),
            Failed::Other(why) => write!(out, "{why}"),
        }
    }
}

/// How long to wait after `failures` failed pushes in a row: 1 s doubling, at most a minute.
pub fn backoff(failures: u32) -> Duration {
    Duration::from_secs(1u64 << failures.saturating_sub(1).min(6)).min(Duration::from_secs(60))
}

/// The snapshot as it is pushed: without the top processes (#513's list, the
/// sums stay), so a push stays small.
pub fn slim(mut snapshot: Snapshot) -> Snapshot {
    if let Some(resources) = snapshot.resources.as_mut() {
        resources.top.clear();
    }
    snapshot
}

/// An event as it is pushed, as `slim`.
pub fn slim_event(mut event: Event) -> Event {
    if let Event::Resources { resources, .. } = &mut event {
        resources.top.clear();
    }
    event
}

/// The most of a run's log one push carries, in bytes: a run that wrote
/// more goes on in the next ones.
pub const LOG_BATCH: u64 = 256 << 10;
/// A log line longer than this goes cut, with `…` (a tool's whole file).
pub const LOG_LINE: usize = 16 << 10;

/// Where lab web stands in a run's log (`runs::log_file`): the run, the
/// bytes and the lines it has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LogMark {
    pub since: i64,
    pub bytes: u64,
    pub lines: u64,
}

/// The whole lines of `file` after `mark`, at most LOG_BATCH of them, and
/// the mark after them; None while there is nothing new. A run other than
/// the mark's, or a log shorter than it (written anew), starts at its top.
pub fn log_after(file: &Path, since: i64, mark: Option<LogMark>) -> Option<(LogMark, Vec<String>)> {
    use std::io::{Read, Seek, SeekFrom};
    let mark = mark.filter(|mark| mark.since == since).unwrap_or(LogMark {
        since,
        bytes: 0,
        lines: 0,
    });
    let mut log = std::fs::File::open(file).ok()?;
    let length = log.metadata().ok()?.len();
    let mark = if length < mark.bytes {
        LogMark {
            since,
            bytes: 0,
            lines: 0,
        }
    } else {
        mark
    };
    if length == mark.bytes {
        return None;
    }
    log.seek(SeekFrom::Start(mark.bytes)).ok()?;
    let mut read = Vec::new();
    log.take(LOG_BATCH).read_to_end(&mut read).ok()?;
    // Only whole lines: the rest is still being written.
    let whole = read.iter().rposition(|byte| *byte == b'\n').map(|at| at + 1)?;
    let lines: Vec<String> = String::from_utf8_lossy(&read[..whole])
        .lines()
        .map(|line| cut(line, LOG_LINE))
        .collect();
    let next = LogMark {
        since,
        bytes: mark.bytes + whole as u64,
        lines: mark.lines + lines.len() as u64,
    };
    Some((next, lines))
}

fn cut(line: &str, most: usize) -> String {
    if line.len() <= most {
        return line.to_string();
    }
    let mut end = most;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &line[..end])
}

/// The HTTP client of lab web and its OpenID provider.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    server: String,
}

impl Client {
    pub fn new(server: &str) -> Client {
        Client {
            http: reqwest::Client::builder().timeout(TIMEOUT).build().unwrap_or_default(),
            server: server.trim_end_matches('/').to_string(),
        }
    }

    /// Where the node signs in.
    pub async fn login(&self) -> Result<Login, Failed> {
        let login: Login = answer(self.http.get(format!("{}/api/node/login", self.server)).send().await).await?;
        same_protocol(login.protocol)?;
        Ok(login)
    }

    /// The node registered with the provider's `access` token: its node token.
    pub async fn register(&self, access: &str, register: &Register) -> Result<Registered, Failed> {
        let request = self
            .http
            .post(format!("{}/api/node/register", self.server))
            .bearer_auth(access)
            .json(register);
        let registered: Registered = answer(request.send().await).await?;
        same_protocol(registered.protocol)?;
        Ok(registered)
    }

    pub async fn snapshot(&self, token: &str, snapshot: Snapshot) -> Result<Pushed, Failed> {
        let body = SnapshotPush {
            protocol: PROTOCOL,
            snapshot: slim(snapshot),
        };
        self.push(token, "snapshot", &body).await
    }

    pub async fn events(&self, token: &str, events: Vec<Sequenced>) -> Result<Pushed, Failed> {
        let body = EventsPush {
            protocol: PROTOCOL,
            events,
        };
        self.push(token, "events", &body).await
    }

    async fn push(&self, token: &str, path: &str, body: &impl Serialize) -> Result<Pushed, Failed> {
        let request = self
            .http
            .post(format!("{}/api/node/{path}", self.server))
            .bearer_auth(token)
            .json(body);
        let pushed: Pushed = answer(request.send().await).await?;
        same_protocol(pushed.protocol)?;
        Ok(pushed)
    }

    /// The device flow started at the realm of `login`.
    pub async fn device(&self, login: &Login) -> Result<(Endpoints, Device), Failed> {
        let discovery = format!(
            "{}/.well-known/openid-configuration",
            login.issuer.trim_end_matches('/')
        );
        let endpoints: Endpoints = answer(self.http.get(discovery).send().await).await?;
        let request = self
            .http
            .post(&endpoints.device_authorization_endpoint)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form(&[("client_id", &login.client_id), ("scope", "openid")]));
        let device: Device = answer(request.send().await).await?;
        Ok((endpoints, device))
    }

    /// One poll of the device flow's token: None while the user hasn't
    /// answered yet (`slow_down` lengthens `interval`).
    pub async fn poll(
        &self,
        endpoints: &Endpoints,
        login: &Login,
        device: &Device,
        interval: &mut Duration,
    ) -> Result<Option<String>, Failed> {
        let response = self
            .http
            .post(&endpoints.token_endpoint)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form(&[
                ("grant_type", DEVICE_GRANT),
                ("device_code", &device.device_code),
                ("client_id", &login.client_id),
            ]))
            .send()
            .await
            .map_err(|error| Failed::Other(format!("the login provider didn't answer: {error}")))?;
        let is_ok = response.status().is_success();
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|error| Failed::Other(format!("the login provider's answer: {error}")))?;
        if is_ok {
            return crate::json::str_of(&body, "access_token")
                .map(|token| Some(token.to_string()))
                .ok_or_else(|| Failed::Other("the login provider's answer has no access_token".into()));
        }
        match crate::json::str_of(&body, "error").unwrap_or_default() {
            "authorization_pending" => Ok(None),
            "slow_down" => {
                *interval += Duration::from_secs(5);
                Ok(None)
            }
            "access_denied" => Err(Failed::Other("the login was denied".into())),
            "expired_token" => Err(Failed::Other("the code expired: run lab login again".into())),
            other => Err(Failed::Other(format!("the login provider refused: {other}"))),
        }
    }
}

/// The endpoints of the realm (its OpenID discovery).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Endpoints {
    pub device_authorization_endpoint: String,
    pub token_endpoint: String,
}

/// A started device flow: the code the user confirms and where.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Device {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    #[serde(default = "five")]
    pub interval: u64,
}

fn five() -> u64 {
    5
}

fn same_protocol(protocol: u32) -> Result<(), Failed> {
    if protocol == PROTOCOL {
        Ok(())
    } else {
        Err(Failed::Protocol(format!("{protocol}, this node {PROTOCOL}")))
    }
}

/// A response's body as `T`, or why not.
async fn answer<T: serde::de::DeserializeOwned>(sent: reqwest::Result<reqwest::Response>) -> Result<T, Failed> {
    let response = sent.map_err(|error| Failed::Other(format!("not reached: {error}")))?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(Failed::Unauthorized);
    }
    let text = response.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::CONFLICT {
        let why = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|body| {
                let why = crate::json::str_of(&body, "message").or_else(|| crate::json::str_of(&body, "error"));
                why.map(str::to_string)
            })
            .unwrap_or_else(|| text.clone());
        return Err(Failed::Protocol(why));
    }
    if !status.is_success() {
        return Err(Failed::Other(format!(
            "{status}: {}",
            text.chars().take(200).collect::<String>()
        )));
    }
    serde_json::from_str(&text).map_err(|error| Failed::Other(format!("an answer it didn't take: {error}")))
}

/// `pairs` as a form body.
fn form(pairs: &[(&str, &str)]) -> String {
    let encode = |text: &str| {
        text.bytes()
            .map(|byte| match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (byte as char).to_string(),
                _ => format!("%{byte:02X}"),
            })
            .collect::<String>()
    };
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn backs_off_doubling_up_to_a_minute() {
        let waits: Vec<u64> = (1..=9).map(|failures| backoff(failures).as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
    }

    #[test]
    fn encodes_a_form() {
        assert_eq!(
            form(&[("grant_type", DEVICE_GRANT), ("client_id", "lab-node")]),
            "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code&client_id=lab-node"
        );
    }

    /// The shapes lab web's zod contract reads: a change here is a change of `PROTOCOL`.
    #[test]
    fn pins_the_envelopes() {
        let register = Register {
            protocol: PROTOCOL,
            machine: "m1".into(),
            host: "box".into(),
        };
        assert_eq!(
            serde_json::to_value(&register).unwrap(),
            json!({ "protocol": 1, "machine": "m1", "host": "box" })
        );
        let registered: Registered =
            serde_json::from_value(json!({ "protocol": 1, "node": "n1", "token": "t", "user": "ada" })).unwrap();
        assert_eq!(registered.node, "n1");
        let pushed: Pushed = serde_json::from_value(json!({ "protocol": 1, "seq": 7 })).unwrap();
        assert_eq!(pushed.seq, 7);
        let login: Login = serde_json::from_value(
            json!({ "protocol": 1, "issuer": "https://id.example/realms/lab", "clientId": "lab-node" }),
        )
        .unwrap();
        assert_eq!(login.client_id, "lab-node");
    }

    #[test]
    fn keeps_the_login_readable_by_the_user_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("lab-config-{}", std::process::id()));
        let file = dir.join("breadboard/lab.json");
        let config = Config {
            server: "http://127.0.0.1:1".into(),
            machine: new_machine(),
            node: Some("n1".into()),
            token: Some("t".into()),
            user: Some("ada".into()),
            repos: vec![PathBuf::from("/r/platform")],
        };
        write_at(&file, &config).unwrap();
        assert_eq!(read_at(&file), Some(config));
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reads_a_runs_log_on_from_its_mark() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("lab-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("s-1.jsonl");
        std::fs::write(&file, "{\"type\":\"system\"}\n{\"type\":\"assistant\"}\n{\"type\":").unwrap();
        let (mark, lines) = log_after(&file, 10, None).unwrap();
        assert_eq!(lines, ["{\"type\":\"system\"}", "{\"type\":\"assistant\"}"]);
        assert_eq!((mark.lines, mark.since), (2, 10));
        assert_eq!(log_after(&file, 10, Some(mark)), None, "a half line waits");
        let mut log = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        log.write_all(b"\"result\"}\n").unwrap();
        let (next, lines) = log_after(&file, 10, Some(mark)).unwrap();
        assert_eq!(lines, ["{\"type\":\"result\"}"]);
        assert_eq!(next.lines, 3);
        // The next run writes its log anew: from its top.
        std::fs::write(&file, "{\"type\":\"system\"}\n").unwrap();
        let (fresh, lines) = log_after(&file, 20, Some(next)).unwrap();
        assert_eq!((fresh.lines, lines.len()), (1, 1));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cuts_a_long_line_on_a_char() {
        assert_eq!(cut("aéb", 2), "a…");
        assert_eq!(cut("ab", 2), "ab");
    }
}
