//! GitLab through lab web (#505): while the user is logged in to lab
//! (`crate::lab`), the node asks lab web instead of GitLab. lab web reads
//! each project once for every node with a group token and keeps its MRs,
//! issues and pipelines in GraphQL's shape (`products/lab/web/src/server/gitlab`);
//! the node takes them (`GET /api/node/gitlab`, only what changed since the
//! version it has) and answers its own queries from them (`answer`): the
//! views' reads stay as they are, only where the answer comes from changes.
//! A REST read goes through lab's proxy (`GET /api/node/gitlab/rest`).
//!
//! Writes never come here: a merge, a pipeline started, a push run on the
//! node as the user (glab). A project lab web doesn't read (404), a lab web
//! that doesn't answer, a query the resolver can't answer: the node reads
//! GitLab itself, as before (`client.rs`); `$BREADBOARD_GITLAB_DIRECT`
//! always.

use super::client::GitlabError;
use super::read::id_of;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

/// How old lab's state may be for another read of the same project.
const FRESH: Duration = Duration::from_secs(5);
/// How long a fresh read may take: lab web reads GitLab first.
const TIMEOUT: Duration = Duration::from_secs(20);
/// How long the node reads GitLab itself after lab web said it doesn't read the project.
const NOT_SERVED: Duration = Duration::from_secs(300);
/// How long after lab web didn't answer.
const UNREACHABLE: Duration = Duration::from_secs(30);

/// lab web, where the node's login says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub server: String,
    pub token: String,
}

/// lab web while the user is logged in to it, None for GitLab itself.
pub fn source() -> Option<Source> {
    if std::env::var_os("BREADBOARD_GITLAB_DIRECT").is_some_and(|value| !value.is_empty()) {
        return None;
    }
    let config = crate::lab::read()?;
    Some(Source {
        token: config.token?,
        server: config.server.trim_end_matches('/').to_string(),
    })
}

/// What lab web keeps of a project: its nodes by iid or id.
#[derive(Debug, Clone, Default)]
pub struct State {
    pub version: i64,
    pub merge_requests: BTreeMap<i64, Value>,
    pub issues: BTreeMap<i64, Value>,
    pub pipelines: BTreeMap<i64, Value>,
}

impl State {
    /// `body` (lab's answer) taken in: a delta over this one, or all anew.
    pub fn take(&mut self, body: &Value) {
        if body.get("isDelta").and_then(Value::as_bool) != Some(true) {
            *self = State::default();
        }
        let list = |key: &str| body.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
        for node in list("mergeRequests") {
            self.merge_requests.insert(id_of(node.get("iid")), node);
        }
        for node in list("issues") {
            self.issues.insert(id_of(node.get("iid")), node);
        }
        for node in list("pipelines") {
            self.pipelines.insert(id_of(node.get("id")), node);
        }
        self.version = body.get("version").and_then(Value::as_i64).unwrap_or(0);
    }
}

#[derive(Debug, Default)]
struct Held {
    state: Option<Arc<State>>,
    at: Option<Instant>,
    /// Until when the node reads GitLab itself.
    direct_until: Option<Instant>,
}

/// Per project; one fetch at a time, the others wait for it and take its answer.
static HELD: LazyLock<tokio::sync::Mutex<BTreeMap<String, Held>>> = LazyLock::new(Default::default);

fn http() -> &'static reqwest::Client {
    static HTTP: LazyLock<reqwest::Client> =
        LazyLock::new(|| reqwest::Client::builder().timeout(TIMEOUT).build().unwrap_or_default());
    &HTTP
}

/// lab's state of `project`, `is_fresh` after GitLab was read again
/// (a push, `r`); None: the node reads GitLab itself.
pub async fn state(source: &Source, project: &str, is_fresh: bool) -> Option<Arc<State>> {
    let mut held = HELD.lock().await;
    let entry = held.entry(project.to_string()).or_default();
    let now = Instant::now();
    if entry.direct_until.is_some_and(|until| now < until) {
        return None;
    }
    if let (Some(state), Some(at)) = (&entry.state, entry.at)
        && now.duration_since(at) < FRESH
        && !is_fresh
    {
        return Some(state.clone());
    }
    let mut url = format!(
        "{}/api/node/gitlab?protocol={}&project={}",
        source.server,
        crate::lab::PROTOCOL,
        encode(project)
    );
    if let Some(state) = &entry.state {
        url.push_str(&format!("&since={}", state.version));
    }
    if is_fresh {
        url.push_str("&fresh=1");
    }
    let mut request = http().get(&url).bearer_auth(&source.token);
    if let Some(state) = &entry.state {
        request = request.header("if-none-match", format!("\"{}\"", state.version));
    }
    let answer = match request.send().await {
        Ok(response) if response.status().as_u16() == 304 => {
            entry.at = Some(now);
            return entry.state.clone();
        }
        Ok(response) if response.status().is_success() => response.json::<Value>().await.ok(),
        // Not read there (404), a login lab no longer takes (401), another protocol (409).
        Ok(response) if response.status().is_client_error() => {
            entry.direct_until = Some(now + NOT_SERVED);
            return None;
        }
        _ => None,
    };
    let Some(body) = answer else {
        entry.direct_until = Some(now + UNREACHABLE);
        return None;
    };
    let mut next = entry.state.as_deref().cloned().unwrap_or_default();
    next.take(&body);
    let next = Arc::new(next);
    entry.state = Some(next.clone());
    entry.at = Some(now);
    Some(next)
}

/// A REST GET of `endpoint` under `project` through lab's proxy, its body;
/// None: the node asks GitLab itself (lab doesn't read the project, refused
/// the path, or didn't answer).
pub async fn rest(source: &Source, project: &str, endpoint: &str) -> Option<Result<String, GitlabError>> {
    {
        let held = HELD.lock().await;
        if held
            .get(project)
            .and_then(|entry| entry.direct_until)
            .is_some_and(|until| Instant::now() < until)
        {
            return None;
        }
    }
    let url = format!(
        "{}/api/node/gitlab/rest?protocol={}&project={}&path={}",
        source.server,
        crate::lab::PROTOCOL,
        encode(project),
        encode(endpoint)
    );
    let response = http().get(&url).bearer_auth(&source.token).send().await.ok()?;
    let status = response.status().as_u16();
    let text = response.text().await.ok()?;
    if status == 200 {
        return Some(Ok(text));
    }
    // lab's own refusal carries its correlation id; GitLab's answers don't.
    let refusal = serde_json::from_str::<Value>(&text)
        .ok()
        .filter(|body| body.get("correlationId").is_some());
    match refusal {
        Some(body) if status == 429 => Some(Err(GitlabError {
            reason: format!(
                "GitLab's rate limit ({}), through lab web",
                body.get("why").and_then(Value::as_str).unwrap_or("HTTP 429")
            ),
            status: Some(429),
        })),
        Some(_) => None,
        None => Some(Err(GitlabError {
            reason: format!("HTTP {status} ({endpoint})"),
            status: Some(status),
        })),
    }
}

/// `text` as a query parameter.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The node's queries answered from lab's state
// ---------------------------------------------------------------------------

/// A token of GraphQL's text, the part of it the views' queries use.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Name(String),
    Variable(String),
    Text(String),
    Number(i64),
    Punct(char),
}

fn lex(query: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = query.chars().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    let word = |at: &mut usize| {
        let start = *at;
        while *at < chars.len() && (chars[*at].is_alphanumeric() || chars[*at] == '_') {
            *at += 1;
        }
        chars[start..*at].iter().collect::<String>()
    };
    while at < chars.len() {
        let char = chars[at];
        match char {
            _ if char.is_whitespace() || char == ',' => at += 1,
            '{' | '}' | '(' | ')' | '[' | ']' | ':' | '!' | '=' => {
                tokens.push(Token::Punct(char));
                at += 1;
            }
            '$' => {
                at += 1;
                tokens.push(Token::Variable(word(&mut at)));
            }
            '"' => {
                at += 1;
                let mut text = String::new();
                while at < chars.len() && chars[at] != '"' {
                    if chars[at] == '\\' && at + 1 < chars.len() {
                        at += 1;
                    }
                    text.push(chars[at]);
                    at += 1;
                }
                at += 1;
                tokens.push(Token::Text(text));
            }
            '-' | '0'..='9' => {
                let start = at;
                at += 1;
                while at < chars.len() && chars[at].is_ascii_digit() {
                    at += 1;
                }
                let number: String = chars[start..at].iter().collect();
                tokens.push(Token::Number(
                    number.parse().map_err(|_| format!("a number: {number}"))?,
                ));
            }
            _ if char.is_alphabetic() || char == '_' => tokens.push(Token::Name(word(&mut at))),
            _ => return Err(format!("unexpected {char:?}")),
        }
    }
    Ok(tokens)
}

/// A field asked of the project: `alias: name(args)`, its args resolved.
#[derive(Debug, Clone, PartialEq)]
struct Field {
    alias: String,
    name: String,
    args: Map<String, Value>,
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
    variables: &'a Value,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Result<Token, String> {
        let token = self.tokens.get(self.at).cloned().ok_or("the query ends early")?;
        self.at += 1;
        Ok(token)
    }

    fn expect(&mut self, punct: char) -> Result<(), String> {
        match self.next()? {
            Token::Punct(found) if found == punct => Ok(()),
            other => Err(format!("{punct} expected, {other:?} found")),
        }
    }

    fn is(&self, punct: char) -> bool {
        self.peek() == Some(&Token::Punct(punct))
    }

    /// Past the next `open` and everything up to its closing `close`.
    fn skip(&mut self, open: char, close: char) -> Result<(), String> {
        self.expect(open)?;
        let mut depth = 1;
        while depth > 0 {
            match self.next()? {
                Token::Punct(found) if found == open => depth += 1,
                Token::Punct(found) if found == close => depth -= 1,
                _ => {}
            }
        }
        Ok(())
    }

    fn value(&mut self) -> Result<Value, String> {
        Ok(match self.next()? {
            Token::Variable(name) => self.variables.get(&name).cloned().unwrap_or(Value::Null),
            Token::Text(text) => Value::String(text),
            Token::Number(number) => json!(number),
            Token::Name(name) => match name.as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                "null" => Value::Null,
                _ => Value::String(name),
            },
            Token::Punct('[') => {
                let mut items = Vec::new();
                while !self.is(']') {
                    items.push(self.value()?);
                }
                self.expect(']')?;
                Value::Array(items)
            }
            other => return Err(format!("a value expected, {other:?} found")),
        })
    }

    fn args(&mut self) -> Result<Map<String, Value>, String> {
        let mut args = Map::new();
        if !self.is('(') {
            return Ok(args);
        }
        self.expect('(')?;
        while !self.is(')') {
            let Token::Name(name) = self.next()? else {
                return Err("an argument expected".into());
            };
            self.expect(':')?;
            args.insert(name, self.value()?);
        }
        self.expect(')')?;
        Ok(args)
    }

    /// The fields of `project(fullPath: $project) { … }`.
    fn project_fields(&mut self) -> Result<Vec<Field>, String> {
        while self.peek().is_some_and(|token| *token != Token::Name("project".into())) {
            self.at += 1;
        }
        self.next()?;
        self.skip('(', ')')?;
        self.expect('{')?;
        let mut fields = Vec::new();
        while !self.is('}') {
            let Token::Name(first) = self.next()? else {
                return Err("a field expected".into());
            };
            let (alias, name) = if self.is(':') {
                self.expect(':')?;
                let Token::Name(name) = self.next()? else {
                    return Err("a field expected after its alias".into());
                };
                (first, name)
            } else {
                (first.clone(), first)
            };
            let args = self.args()?;
            if self.is('{') {
                self.skip('{', '}')?;
            }
            fields.push(Field { alias, name, args });
        }
        Ok(fields)
    }
}

fn texts(value: Option<&Value>) -> Option<Vec<String>> {
    value.and_then(Value::as_array).map(|items| {
        items
            .iter()
            .map(|item| match item {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .collect()
    })
}

fn text_at<'a>(node: &'a Value, key: &str) -> &'a str {
    node.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// A connection of `nodes`, `first` of them after the cursor `after` (an offset).
fn page(nodes: Vec<Value>, args: &Map<String, Value>) -> Value {
    let first = args.get("first").and_then(Value::as_u64).unwrap_or(100) as usize;
    let after = args
        .get("after")
        .and_then(Value::as_str)
        .and_then(|cursor| cursor.parse::<usize>().ok())
        .unwrap_or(0);
    let end = (after + first).min(nodes.len());
    let has_next = end < nodes.len();
    let taken: Vec<Value> = nodes.into_iter().skip(after).take(first).collect();
    json!({
        "nodes": taken,
        "pageInfo": { "hasNextPage": has_next, "endCursor": has_next.then(|| end.to_string()) },
    })
}

/// The MR with the head pipeline lab web read later, where it did: an MR
/// isn't updated when its pipeline moves.
fn with_head(state: &State, node: &Value) -> Value {
    let head = node.get("headPipeline").filter(|head| head.is_object());
    let later = head
        .and_then(|head| state.pipelines.get(&id_of(head.get("id"))))
        .filter(|later| text_at(later, "updatedAt") > head.map(|head| text_at(head, "updatedAt")).unwrap_or(""));
    match later {
        Some(later) => {
            let mut node = node.clone();
            node["headPipeline"] = later.clone();
            node
        }
        None => node.clone(),
    }
}

fn merge_requests(state: &State, args: &Map<String, Value>) -> Value {
    let wanted = args
        .get("state")
        .and_then(Value::as_str)
        .filter(|state| *state != "all");
    let sources = texts(args.get("sourceBranches"));
    let targets = texts(args.get("targetBranches"));
    let iids = texts(args.get("iids"));
    let mut nodes: Vec<&Value> = state
        .merge_requests
        .values()
        .filter(|node| wanted.is_none_or(|wanted| text_at(node, "state") == wanted))
        .filter(|node| {
            sources
                .as_ref()
                .is_none_or(|sources| sources.iter().any(|branch| branch == text_at(node, "sourceBranch")))
        })
        .filter(|node| {
            targets
                .as_ref()
                .is_none_or(|targets| targets.iter().any(|branch| branch == text_at(node, "targetBranch")))
        })
        .filter(|node| {
            iids.as_ref().is_none_or(|iids| {
                iids.iter()
                    .any(|iid| id_of(Some(&json!(iid))) == id_of(node.get("iid")))
            })
        })
        .collect();
    match args.get("sort").and_then(Value::as_str) {
        Some("UPDATED_DESC") => nodes.sort_by(|a, b| text_at(b, "updatedAt").cmp(text_at(a, "updatedAt"))),
        Some("MERGED_AT_DESC") => nodes.sort_by(|a, b| text_at(b, "mergedAt").cmp(text_at(a, "mergedAt"))),
        _ => nodes.sort_by_key(|node| std::cmp::Reverse(id_of(node.get("iid")))),
    }
    page(nodes.into_iter().map(|node| with_head(state, node)).collect(), args)
}

fn issues(state: &State, args: &Map<String, Value>) -> Value {
    let wanted = args
        .get("state")
        .and_then(Value::as_str)
        .filter(|state| *state != "all");
    let iids = texts(args.get("iids"));
    let nodes: Vec<Value> = state
        .issues
        .values()
        .rev()
        .filter(|node| wanted.is_none_or(|wanted| text_at(node, "state") == wanted))
        .filter(|node| {
            iids.as_ref().is_none_or(|iids| {
                iids.iter()
                    .any(|iid| id_of(Some(&json!(iid))) == id_of(node.get("iid")))
            })
        })
        .cloned()
        .collect();
    page(nodes, args)
}

fn pipelines(state: &State, args: &Map<String, Value>) -> Value {
    let reference = args.get("ref").and_then(Value::as_str);
    let sha = args.get("sha").and_then(Value::as_str);
    let nodes: Vec<Value> = state
        .pipelines
        .values()
        .rev()
        .filter(|node| reference.is_none_or(|reference| text_at(node, "ref") == reference))
        .filter(|node| sha.is_none_or(|sha| text_at(node, "sha") == sha))
        .cloned()
        .collect();
    page(nodes, args)
}

/// `query`'s `data.project` as GitLab would answer it, from lab's state;
/// an error for a field it can't answer (the node asks GitLab then).
pub fn answer(query: &str, variables: &Value, state: &State) -> Result<Value, String> {
    let tokens = lex(query)?;
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
        variables,
    };
    let mut project = Map::new();
    for field in parser.project_fields()? {
        let value = match field.name.as_str() {
            "mergeRequests" => merge_requests(state, &field.args),
            "issues" => issues(state, &field.args),
            "pipelines" => pipelines(state, &field.args),
            "issue" => {
                let iid = id_of(field.args.get("iid"));
                state.issues.get(&iid).cloned().unwrap_or(Value::Null)
            }
            other => return Err(format!("lab web keeps no {other}")),
        };
        project.insert(field.alias, value);
    }
    Ok(Value::Object(project))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab::read::{self, CI_QUERY, NOTIFY_QUERY, open_mrs_of};

    fn mr(iid: i64, state: &str, branch: &str, updated: &str, head: Value) -> Value {
        json!({
            "iid": iid.to_string(), "title": format!("MR {iid}"), "state": state, "draft": false,
            "webUrl": format!("https://gitlab.com/g/p/-/merge_requests/{iid}"),
            "sourceBranch": branch, "targetBranch": "develop", "updatedAt": updated,
            "mergedAt": if state == "merged" { json!(updated) } else { Value::Null },
            "diffHeadSha": "abc", "headPipeline": head,
            "labels": { "nodes": [] },
            "pipelines": { "nodes": [ { "id": "gid://gitlab/Ci::Pipeline/9", "status": "FAILED", "ref": "refs/merge-requests/1/head", "sha": "abc" } ] },
        })
    }

    fn pipeline(id: i64, reference: &str, status: &str, updated: &str) -> Value {
        json!({
            "id": format!("gid://gitlab/Ci::Pipeline/{id}"), "status": status, "ref": reference,
            "sha": format!("sha{id}"), "path": format!("/g/p/-/pipelines/{id}"), "updatedAt": updated,
            "jobs": { "nodes": [ { "id": "gid://gitlab/Ci::Build/1", "name": "lint", "status": status } ] },
        })
    }

    fn state() -> State {
        let mut state = State::default();
        state.take(&json!({
            "version": 3,
            "mergeRequests": [
                mr(1, "opened", "feature/1-a", "2026-10-30T08:00:00Z", pipeline(9, "refs/merge-requests/1/head", "RUNNING", "2026-10-30T08:00:00Z")),
                mr(2, "merged", "feature/2-b", "2026-10-29T08:00:00Z", Value::Null),
                mr(3, "opened", "feature/3-c", "2026-10-30T09:00:00Z", Value::Null),
            ],
            "issues": [
                { "iid": "7", "title": "seven", "state": "closed", "description": "- [x] done" },
                { "iid": "8", "title": "eight", "state": "opened", "description": "" },
            ],
            "pipelines": [
                pipeline(9, "refs/merge-requests/1/head", "SUCCESS", "2026-10-30T08:10:00Z"),
                pipeline(10, "develop", "SUCCESS", "2026-10-30T07:00:00Z"),
                pipeline(11, "develop", "RUNNING", "2026-10-30T09:00:00Z"),
            ],
        }));
        state
    }

    #[test]
    fn takes_a_delta_over_what_it_has() {
        let mut state = state();
        state.take(&json!({ "version": 4, "isDelta": true, "issues": [ { "iid": "8", "state": "closed" } ] }));
        assert_eq!(state.version, 4);
        assert_eq!(state.issues.len(), 2);
        assert_eq!(state.issues[&8]["state"], "closed");
        assert_eq!(state.merge_requests.len(), 3);
        state.take(&json!({ "version": 1, "issues": [] }));
        assert!(state.issues.is_empty(), "a full answer replaces it");
    }

    #[test]
    fn answers_the_orchestration_mrs_by_branch_with_the_later_head() {
        let query = read::queries().into_iter().find(|(name, _)| *name == "mrs").unwrap().1;
        let project = answer(
            &query,
            &json!({ "branches": ["feature/1-a", "feature/2-b"], "epics": [], "after": null }),
            &state(),
        )
        .unwrap();
        let nodes = project["mrs"]["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0]["iid"], "1", "UPDATED_DESC");
        assert_eq!(nodes[0]["headPipeline"]["status"], "SUCCESS", "the pipeline read later");
        assert_eq!(project["mrs"]["pageInfo"]["hasNextPage"], false);
        assert!(project["epicMrs"]["nodes"].as_array().unwrap().is_empty());
    }

    #[test]
    fn answers_issues_by_iid_and_develop_pipelines() {
        let query = read::queries()
            .into_iter()
            .find(|(name, _)| *name == "orchestration")
            .unwrap()
            .1;
        let project = answer(
            &query,
            &json!({ "iids": ["7"], "awaited": ["2"], "epic": "epic/20", "epicTargets": ["epic/20"] }),
            &state(),
        )
        .unwrap();
        assert_eq!(project["issues"]["nodes"][0]["title"], "seven");
        assert_eq!(project["awaited"]["nodes"][0]["state"], "merged");
        let develop = project["develop"]["nodes"].as_array().unwrap();
        assert_eq!(develop.len(), 2);
        assert_eq!(develop[0]["status"], "RUNNING", "newest first");
        assert!(project["epicPipelines"]["nodes"].as_array().unwrap().is_empty());
    }

    #[test]
    fn answers_the_ci_queue_and_the_notifications() {
        let project = answer(CI_QUERY, &json!({}), &state()).unwrap();
        let open = open_mrs_of(&project);
        assert_eq!(open.iter().map(|mr| mr.iid).collect::<Vec<_>>(), vec![3, 1]);
        let project = answer(NOTIFY_QUERY, &json!({}), &state()).unwrap();
        let now = read::notify_of(&project);
        assert_eq!(now.merged.len(), 1);
        assert_eq!(now.issues.first().map(|issue| issue.0), Some(8), "newest issue first");
        assert_eq!(now.open.len(), 2);
    }

    #[test]
    fn answers_every_query_of_the_views() {
        for (name, query) in read::queries() {
            let variables = json!({ "iids": ["7"], "iid": "8", "sha": "sha10", "branches": ["feature/1-a"], "awaited": ["1"], "epic": "e", "epicTargets": ["e"], "epics": ["e"] });
            assert!(answer(&query, &variables, &state()).is_ok(), "{name}");
        }
    }

    #[test]
    fn pages_by_offset() {
        let project = answer(
            "query($project: ID!, $after: String) { project(fullPath: $project) { mergeRequests(first: 2, after: $after) { nodes { iid } pageInfo { hasNextPage endCursor } } } }",
            &json!({ "after": null }),
            &state(),
        )
        .unwrap();
        assert_eq!(project["mergeRequests"]["pageInfo"]["endCursor"], "2");
        let project = answer(
            "query($project: ID!, $after: String) { project(fullPath: $project) { mergeRequests(first: 2, after: $after) { nodes { iid } pageInfo { hasNextPage endCursor } } } }",
            &json!({ "after": "2" }),
            &state(),
        )
        .unwrap();
        assert_eq!(project["mergeRequests"]["nodes"][0]["iid"], "1");
        assert_eq!(project["mergeRequests"]["pageInfo"]["hasNextPage"], false);
    }

    #[test]
    fn refuses_what_it_keeps_none_of() {
        assert!(
            answer(
                "query($project: ID!) { project(fullPath: $project) { milestones { nodes { id } } } }",
                &json!({}),
                &state()
            )
            .is_err()
        );
    }

    #[test]
    fn encodes_a_parameter() {
        assert_eq!(encode("monocircuit/platform"), "monocircuit%2Fplatform");
        assert_eq!(encode("jobs/9/trace?a=b&c"), "jobs%2F9%2Ftrace%3Fa%3Db%26c");
    }
}
