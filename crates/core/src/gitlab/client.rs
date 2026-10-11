//! GitLab's API with glab's login (lib/glab.mjs): one request over a kept-alive
//! connection, never a glab process per call. The host and project come from
//! origin, the API's base from `$BREADBOARD_GITLAB_URL` (the gauntlet's fake),
//! else the host; the token from `GITLAB_TOKEN` or `glab config get token`.
//! A call that fails for a reason that passes (a 429, a 5xx, a timeout, the
//! network) is tried again after a pause; what still fails is an error whose
//! reason says why in a few words, for the view to draw.

use regex::Regex;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::OnceCell;

/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(15);
/// The pauses before the second and third try.
const BACKOFF: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(4)];

/// A call that failed, with why in a few words: `HTTP 429, rate limit`, `timeout`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitlabError {
    pub reason: String,
    pub status: Option<u16>,
}

impl std::fmt::Display for GitlabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GitLab: {}", self.reason)
    }
}

impl std::error::Error for GitlabError {}

impl GitlabError {
    pub fn new(reason: impl Into<String>) -> GitlabError {
        GitlabError {
            reason: reason.into(),
            status: None,
        }
    }
}

pub type Result<T> = std::result::Result<T, GitlabError>;

/// `(host, project)` of origin's url, ssh (`user@host:path.git`) or https (glab.mjs `remoteOf`).
pub fn remote_of(url: &str) -> Option<(String, String)> {
    static REMOTE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?:[a-z+]+://)?(?:[^@/]+@)?([^:/]+)(?::\d+)?[:/](.+?)(?:\.git)?/?$").unwrap());
    let found = REMOTE.captures(url.trim())?;
    Some((found[1].to_string(), found[2].to_string()))
}

/// GitLab's word on the minute of this user, all its clients together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Rate {
    pub limit: u64,
    pub observed: u64,
    pub remaining: u64,
}

/// Where a read stands, for the view (`GitLab: 4/7 · pipelines`): calls asked
/// and answered since it began, the phase its reader named.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Progress {
    pub asked: u64,
    pub answered: u64,
    pub phase: Option<String>,
}

/// Host, project, base and token, found once.
#[derive(Debug, Clone)]
struct Connection {
    project: String,
    base: String,
    token: Option<String>,
}

#[derive(Debug, Default)]
struct Shared {
    connection: OnceCell<Connection>,
    calls: AtomicU64,
    rate: Mutex<Option<Rate>>,
    progress: Mutex<Progress>,
}

/// A GitLab client of the repo whose main checkout is `main`; cheap to clone.
#[derive(Debug, Clone)]
pub struct Gitlab {
    main: PathBuf,
    http: reqwest::Client,
    shared: Arc<Shared>,
}

impl Gitlab {
    pub fn new(main: &Path) -> Gitlab {
        let http = reqwest::Client::builder().timeout(TIMEOUT).build().unwrap_or_default();
        Gitlab {
            main: main.to_path_buf(),
            http,
            shared: Arc::new(Shared::default()),
        }
    }

    /// The repo's default branch (breadboard.toml): the pipelines of `develop`.
    pub fn branch(&self) -> String {
        crate::git::branch(&self.main)
    }

    /// `query` with develop's pipelines asked of the repo's default branch:
    /// the text stays the same where the branch is `develop`.
    pub fn on_branch(&self, query: &str) -> String {
        let branch = serde_json::to_string(&self.branch()).unwrap_or_else(|_| "\"develop\"".into());
        query.replace("ref: \"develop\"", &format!("ref: {branch}"))
    }

    /// Calls made so far by this client, tries included.
    pub fn calls(&self) -> u64 {
        self.shared.calls.load(Ordering::Relaxed)
    }

    pub fn rate(&self) -> Option<Rate> {
        *self.shared.rate.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn progress(&self) -> Progress {
        self.shared.progress.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// A read starts: its calls are counted from here.
    pub fn begin_read(&self) {
        *self.shared.progress.lock().unwrap_or_else(|e| e.into_inner()) = Progress::default();
    }

    /// What the read waits for now, in a word or two.
    pub fn phase(&self, phase: Option<&str>) {
        self.shared.progress.lock().unwrap_or_else(|e| e.into_inner()).phase = phase.map(String::from);
    }

    fn step(&self, is_answer: bool) {
        let mut progress = self.shared.progress.lock().unwrap_or_else(|e| e.into_inner());
        if is_answer {
            progress.answered += 1;
        } else {
            progress.asked += 1;
        }
    }

    async fn connection(&self) -> &Connection {
        self.shared
            .connection
            .get_or_init(|| async {
                let url = run(&self.main, "git", &["config", "--get", "remote.origin.url"])
                    .await
                    .unwrap_or_default();
                let (host, project) = remote_of(&url).unwrap_or_else(|| ("gitlab.com".into(), String::new()));
                let base = std::env::var("BREADBOARD_GITLAB_URL")
                    .ok()
                    .filter(|base| !base.is_empty())
                    .unwrap_or_else(|| format!("https://{host}"));
                let token = match std::env::var("GITLAB_TOKEN").ok().filter(|token| !token.is_empty()) {
                    Some(token) => Some(token),
                    None => run(&self.main, "glab", &["config", "get", "token", "--host", &host]).await,
                };
                Connection {
                    project,
                    base: base.trim_end_matches('/').to_string(),
                    token,
                }
            })
            .await
    }

    /// The token again after glab renewed it: `glab api user` renews an expired OAuth token.
    async fn renewed_token(&self) -> Option<String> {
        let _ = run(&self.main, "glab", &["api", "user"]).await;
        let url = run(&self.main, "git", &["config", "--get", "remote.origin.url"])
            .await
            .unwrap_or_default();
        let host = remote_of(&url)
            .map(|(host, _)| host)
            .unwrap_or_else(|| "gitlab.com".into());
        run(&self.main, "glab", &["config", "get", "token", "--host", &host]).await
    }

    /// One request with its tries, the body as text.
    async fn request(&self, build: impl Fn(&str) -> reqwest::RequestBuilder, what: &str) -> Result<String> {
        let connection = self.connection().await;
        let mut token = connection
            .token
            .clone()
            .ok_or_else(|| GitlabError::new("no token: glab auth login"))?;
        let mut last = GitlabError::new("no answer");
        let mut has_renewed = false;
        let mut attempt = 0;
        while attempt <= BACKOFF.len() {
            if attempt > 0 {
                tokio::time::sleep(BACKOFF[attempt - 1]).await;
            }
            self.shared.calls.fetch_add(1, Ordering::Relaxed);
            match build(&token).send().await {
                Ok(response) => {
                    self.note_rate(response.headers());
                    let status = response.status();
                    if status.is_success() {
                        return response
                            .text()
                            .await
                            .map_err(|error| GitlabError::new(format!("network: {error}")));
                    }
                    // A token GitLab no longer takes: renewed once, the same try again.
                    if status.as_u16() == 401 && !has_renewed {
                        has_renewed = true;
                        if let Some(renewed) = self.renewed_token().await {
                            token = renewed;
                            continue;
                        }
                    }
                    let reason = if status.as_u16() == 429 {
                        "HTTP 429, rate limit".to_string()
                    } else {
                        format!("HTTP {}", status.as_u16())
                    };
                    last = GitlabError {
                        reason: format!("{reason} ({what})"),
                        status: Some(status.as_u16()),
                    };
                    if !(status.as_u16() == 429 || status.is_server_error()) {
                        break;
                    }
                }
                Err(error) => {
                    let reason = if error.is_timeout() {
                        "timeout".to_string()
                    } else {
                        format!("network: {error}")
                    };
                    last = GitlabError::new(reason);
                }
            }
            attempt += 1;
        }
        Err(last)
    }

    fn note_rate(&self, headers: &reqwest::header::HeaderMap) {
        let number = |name: &str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok()?.parse::<u64>().ok())
        };
        if let Some(limit) = number("ratelimit-limit").filter(|limit| *limit > 0) {
            *self.shared.rate.lock().unwrap_or_else(|e| e.into_inner()) = Some(Rate {
                limit,
                observed: number("ratelimit-observed").unwrap_or(0),
                remaining: number("ratelimit-remaining").unwrap_or(0),
            });
        }
    }

    /// A GraphQL query of the project, its `data.project` (None: GitLab knows no such project).
    pub async fn graphql(&self, query: &str, variables: Value) -> Result<Value> {
        let connection = self.connection().await;
        let url = format!("{}/api/graphql", connection.base);
        let mut variables = variables;
        if let Some(object) = variables.as_object_mut() {
            object.insert("project".into(), json!(connection.project));
        }
        let body = json!({ "query": query, "variables": variables });
        self.step(false);
        let text = self
            .request(|token| self.http.post(&url).bearer_auth(token).json(&body), "graphql")
            .await;
        self.step(true);
        let answer: Value =
            serde_json::from_str(&text?).map_err(|error| GitlabError::new(format!("GraphQL answer: {error}")))?;
        if let Some(errors) = answer
            .get("errors")
            .and_then(Value::as_array)
            .filter(|errors| !errors.is_empty())
        {
            let first = errors[0].get("message").and_then(Value::as_str).unwrap_or("an error");
            return Err(GitlabError::new(format!("GraphQL: {first}")));
        }
        Ok(answer
            .get("data")
            .and_then(|data| data.get("project"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// A GET of `endpoint` under the project (`merge_requests/5`, `jobs/9/trace`), the body as text.
    pub async fn get_text(&self, endpoint: &str) -> Result<String> {
        let connection = self.connection().await;
        let project = connection.project.replace('/', "%2F");
        let url = format!("{}/api/v4/projects/{project}/{endpoint}", connection.base);
        self.step(false);
        let text = self
            .request(|token| self.http.get(&url).bearer_auth(token), endpoint)
            .await;
        self.step(true);
        text
    }

    /// A GET of `endpoint` under the project, parsed.
    pub async fn get_json(&self, endpoint: &str) -> Result<Value> {
        serde_json::from_str(&self.get_text(endpoint).await?)
            .map_err(|error| GitlabError::new(format!("{endpoint}: {error}")))
    }
}

/// The trimmed output of a command in `dir`, None when it fails or says nothing.
async fn run(dir: &Path, program: &str, args: &[&str]) -> Option<String> {
    let output = tokio::process::Command::new(program)
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_host_and_project_of_origin() {
        assert_eq!(
            remote_of("git@gitlab.com:monocircuit/platform.git"),
            Some(("gitlab.com".into(), "monocircuit/platform".into()))
        );
        assert_eq!(
            remote_of("https://gitlab.example.com:8443/group/sub/project.git"),
            Some(("gitlab.example.com".into(), "group/sub/project".into()))
        );
        assert_eq!(
            remote_of("ssh://git@gitlab.com/monocircuit/platform"),
            Some(("gitlab.com".into(), "monocircuit/platform".into()))
        );
        assert_eq!(remote_of(""), None);
    }
}
