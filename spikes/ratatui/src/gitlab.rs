//! GitLab in one request (#305): the open MRs with their head pipeline and
//! every job of it, through GraphQL with glab's token. No process per call.

use serde::Deserialize;
use std::time::{Duration, Instant};

const PROJECT: &str = "monocircuit/platform";
const QUERY: &str = "query($p:ID!){project(fullPath:$p){mergeRequests(state:opened,first:50,sort:CREATED_DESC){nodes{iid title webUrl draft sourceBranch headPipeline{status path jobs(first:100){nodes{name status}}}}}}}";

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Mr {
    pub iid: String,
    pub title: String,
    pub web_url: String,
    pub draft: bool,
    pub source_branch: String,
    pub head_pipeline: Option<Pipeline>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Pipeline {
    pub status: String,
    pub path: String,
    pub jobs: Nodes<Job>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Job {
    pub name: String,
    pub status: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Nodes<T> {
    pub nodes: Vec<T>,
}

#[derive(Deserialize)]
struct Answer {
    data: Option<Data>,
    errors: Option<Vec<serde_json::Value>>,
}
#[derive(Deserialize)]
struct Data {
    project: Project,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    merge_requests: Nodes<Mr>,
}

/// The MRs of a GraphQL answer (live or recorded).
pub fn parse(json: &str) -> Result<Vec<Mr>, String> {
    let answer: Answer = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if let Some(errors) = answer.errors {
        return Err(format!("GitLab: {}", serde_json::Value::from(errors)));
    }
    Ok(answer.data.ok_or("GitLab: no data")?.project.merge_requests.nodes)
}

/// glab's token, asked once per run: `glab config get token`.
pub fn token() -> Option<String> {
    if let Ok(token) = std::env::var("GITLAB_TOKEN") {
        return Some(token);
    }
    let out = std::process::Command::new("glab")
        .args(["config", "get", "token", "--host", "gitlab.com"])
        .output()
        .ok()?;
    let token = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!token.is_empty()).then_some(token)
}

/// Where the MRs come from: GitLab itself or a recorded answer.
#[derive(Clone)]
pub enum Source {
    Live { client: reqwest::Client, token: String },
    Fixture(String),
}

impl Source {
    pub async fn read(&self) -> (Result<Vec<Mr>, String>, Duration) {
        let start = Instant::now();
        let result = match self {
            Source::Fixture(path) => std::fs::read_to_string(path)
                .map_err(|e| format!("{path}: {e}"))
                .and_then(|json| parse(&json)),
            Source::Live { client, token } => read_live(client, token).await,
        };
        (result, start.elapsed())
    }
}

async fn read_live(client: &reqwest::Client, token: &str) -> Result<Vec<Mr>, String> {
    let body = serde_json::json!({ "query": QUERY, "variables": { "p": PROJECT } });
    let response = client
        .post("https://gitlab.com/api/graphql")
        .bearer_auth(token)
        .json(&body)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("GitLab answered {}", response.status()));
    }
    parse(&response.text().await.map_err(|e| e.to_string())?)
}
