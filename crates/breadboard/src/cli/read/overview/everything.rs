//! The view of everything printed (render.mjs, read.mjs `readRemote`): the
//! title band with develop's pipeline, every worktree with its app, stack,
//! work and MR, the stacks without a worktree, the open MRs and the open
//! issues, claimed first. Every row whole: a long one goes on under its last
//! column, nothing is cut with `…`.

use super::clock::{band_time, minute};
use super::frame::Frame;
use super::paint::{bold, cyan, dim, green, link, red, table_rows, yellow};
use crate::cli::read::gitlab;
use crate::cli::read::list::{offset_of, port_answers};
use breadboard_core::git::{self, is_inside};
use breadboard_core::json::{int_of, str_of};
use breadboard_core::record::{ORCHESTRATOR, origin_of};
use breadboard_core::time::now_ms;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

/// A checkout as the view of everything shows it.
#[derive(Debug, Clone, Default)]
struct Tree {
    dir: PathBuf,
    name: String,
    branch: String,
    origin: Option<String>,
    is_here: bool,
    has_env: bool,
    offset: i64,
    has_own_port: bool,
    is_up: bool,
    /// Its compose project's state (`running(5)`), None when docker knows none.
    stack: Option<String>,
    changed: usize,
    ahead: u64,
    check: super::orchestration::Check,
}

/// The `key=value` lines of an env file, the last of a key winning (stack.mjs `readEnv`).
pub(super) fn env_of(file: &Path) -> Option<HashMap<String, String>> {
    let text = std::fs::read_to_string(file).ok()?;
    Some(
        text.split('\n')
            .filter(|line| line.contains('=') && !line.trim_start().starts_with('#'))
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
    )
}

/// The compose projects docker knows, by name: their state; None when docker doesn't answer.
pub(super) fn compose_projects() -> Option<HashMap<String, String>> {
    let output = Command::new("docker")
        .args(["compose", "ls", "--all", "--format", "json"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let list: Vec<Value> = serde_json::from_slice(&output.stdout).ok()?;
    Some(
        list.iter()
            .filter_map(|project| {
                Some((
                    str_of(project, "Name")?.to_string(),
                    str_of(project, "Status")?.to_string(),
                ))
            })
            .collect(),
    )
}

fn trees_of(main: &Path, here: &Path, projects: Option<&HashMap<String, String>>) -> Vec<Tree> {
    let worktrees = git::worktrees(main);
    let main_name = main
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    thread::scope(|scope| {
        let handles: Vec<_> = worktrees
            .iter()
            .enumerate()
            .map(|(index, worktree)| {
                let main_name = &main_name;
                scope.spawn(move || {
                    let dir = &worktree.dir;
                    let is_main = index == 0;
                    let env_file = dir.join("products/vita/web/.env.local");
                    let env = env_of(&env_file);
                    let offset = if is_main { 0 } else { offset_of(&env_file) };
                    let has_own_port = is_main || offset > 0;
                    let project = env
                        .as_ref()
                        .and_then(|env| env.get("COMPOSE_PROJECT_NAME").cloned())
                        .unwrap_or_else(|| "vita".into());
                    let ahead = git::count(dir, "origin/develop..HEAD").unwrap_or(0);
                    let base = dir
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    Tree {
                        name: if is_main {
                            main_name.clone()
                        } else {
                            base.replacen(&format!("{main_name}-"), "", 1)
                        },
                        branch: worktree.branch.clone().unwrap_or_else(|| "(detached)".into()),
                        origin: if is_main {
                            None
                        } else {
                            origin_of(dir).map(|origin| origin.name)
                        },
                        is_here: is_inside(here, dir),
                        has_env: env.is_some(),
                        is_up: env.is_some() && has_own_port && port_answers(3000 + offset),
                        offset,
                        has_own_port,
                        stack: projects.and_then(|projects| projects.get(&project).cloned()),
                        changed: git::changed_count(dir),
                        check: super::orchestration::check_of(dir, ahead),
                        ahead,
                        dir: dir.clone(),
                    }
                })
            })
            .collect();
        handles.into_iter().filter_map(|handle| handle.join().ok()).collect()
    })
}

/// An open MR as the view shows it.
struct Mr {
    iid: i64,
    title: String,
    branch: String,
    is_draft: bool,
    who: String,
    url: Option<String>,
    pipeline: Option<(String, Option<String>)>,
}

/// An open issue with the branch on origin that claims it.
struct Issue {
    iid: i64,
    title: String,
    url: Option<String>,
    labels: Vec<String>,
    who: String,
    branch: Option<String>,
    mr: Option<i64>,
}

struct Remote {
    mrs: Vec<Mr>,
    issues: Vec<Issue>,
    develop: Option<(String, Option<String>)>,
}

fn names_of(list: Option<&Value>, key: &str) -> Vec<String> {
    list.and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|each| str_of(each, key).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Open MRs, the last pipelines, open issues and the heads on origin, side by side.
fn read_remote(main: &Path) -> Result<Remote, String> {
    let endpoints = [
        "projects/:fullpath/merge_requests?state=opened&per_page=50".to_string(),
        "projects/:fullpath/pipelines?per_page=100".to_string(),
        "projects/:fullpath/issues?state=opened&per_page=100&order_by=updated_at".to_string(),
    ];
    let (lists, heads) = thread::scope(|scope| {
        let heads = scope.spawn(|| git::remote_heads(main));
        let lists = gitlab::lists(main, &endpoints);
        (lists, heads.join().unwrap_or_default())
    });
    let mut lists = lists.into_iter();
    let mrs = lists.next().unwrap_or(Ok(Vec::new()))?;
    let pipelines = lists.next().unwrap_or(Ok(Vec::new()))?;
    let issues = lists.next().unwrap_or(Ok(Vec::new()))?;
    // Newest first: the first pipeline of a ref is its state.
    let mut pipeline_of: HashMap<String, (String, Option<String>)> = HashMap::new();
    for pipeline in &pipelines {
        if let (Some(reference), Some(status)) = (str_of(pipeline, "ref"), str_of(pipeline, "status")) {
            pipeline_of
                .entry(reference.to_string())
                .or_insert_with(|| (status.to_string(), str_of(pipeline, "web_url").map(String::from)));
        }
    }
    let mrs: Vec<Mr> = mrs
        .iter()
        .filter_map(|mr| {
            let iid = int_of(mr, "iid")?;
            let branch = str_of(mr, "source_branch").unwrap_or_default().to_string();
            let title = str_of(mr, "title").unwrap_or_default();
            let assignees = names_of(mr.get("assignees"), "username");
            Some(Mr {
                iid,
                title: title
                    .strip_prefix("Draft:")
                    .map(str::trim_start)
                    .unwrap_or(title)
                    .to_string(),
                is_draft: mr.get("draft").and_then(Value::as_bool).unwrap_or(false),
                who: if assignees.is_empty() {
                    mr.get("author")
                        .and_then(|author| str_of(author, "username"))
                        .unwrap_or_default()
                        .to_string()
                } else {
                    assignees.join(",")
                },
                url: str_of(mr, "web_url").map(String::from),
                pipeline: pipeline_of
                    .get(&format!("refs/merge-requests/{iid}/head"))
                    .or_else(|| pipeline_of.get(&branch))
                    .cloned(),
                branch,
            })
        })
        .collect();
    let claim_of = |number: i64| {
        heads
            .iter()
            .map(|(_, branch)| branch)
            .find(|branch| {
                branch
                    .rsplit('/')
                    .next()
                    .is_some_and(|last| last.starts_with(&format!("{number}-")))
            })
            .cloned()
    };
    let mut issues: Vec<Issue> = issues
        .iter()
        .filter_map(|issue| {
            let iid = int_of(issue, "iid")?;
            let branch = claim_of(iid);
            Some(Issue {
                iid,
                title: str_of(issue, "title").unwrap_or_default().to_string(),
                url: str_of(issue, "web_url").map(String::from),
                labels: issue
                    .get("labels")
                    .and_then(Value::as_array)
                    .map(|labels| {
                        labels
                            .iter()
                            .filter_map(|label| label.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                who: names_of(issue.get("assignees"), "username").join(","),
                mr: branch
                    .as_ref()
                    .and_then(|branch| mrs.iter().find(|mr| &mr.branch == branch).map(|mr| mr.iid)),
                branch,
            })
        })
        .collect();
    // Claimed ones first: that is the work in flight.
    issues.sort_by_key(|issue| issue.branch.is_none());
    Ok(Remote {
        mrs,
        issues,
        develop: pipeline_of.get("develop").cloned(),
    })
}

/// A pipeline's state as a mark, linked to it (boxes.mjs `statusMark`).
pub(super) fn mark_of(pipeline: Option<&(String, Option<String>)>) -> String {
    let Some((status, url)) = pipeline else {
        return dim("no pipeline");
    };
    let mark = match status.as_str() {
        "success" => green("✓ passed"),
        "failed" => red("✗ failed"),
        "running" => yellow("◐ running"),
        "pending" => yellow("… pending"),
        "created" => yellow("… created"),
        "canceled" => dim("⊘ canceled"),
        "skipped" => dim("skipped"),
        "manual" => dim("▸ manual"),
        other => dim(other),
    };
    link(url.as_deref(), &mark)
}

/// A worktree's stack (boxes.mjs `stackCell`).
fn stack_cell(tree: &Tree) -> String {
    if !tree.has_env {
        return dim("no .env.local");
    }
    if !tree.has_own_port {
        return dim("shared");
    }
    let label = if tree.offset > 0 {
        format!("+{}", tree.offset)
    } else {
        "default".into()
    };
    match &tree.stack {
        None => dim(&format!("{label} down")),
        Some(state) if state.starts_with("running") => green(&format!("{label} {state}")),
        Some(state) => yellow(&format!("{label} {state}")),
    }
}

pub fn everything(frame: &mut Frame, main: &Path, here: &Path) {
    let (projects, remote) = thread::scope(|scope| {
        let remote = scope.spawn(|| read_remote(main));
        let projects = compose_projects();
        (
            projects,
            remote.join().unwrap_or_else(|_| Err("the read broke off".into())),
        )
    });
    let trees = trees_of(main, here, projects.as_ref());
    let (remote, error) = match remote {
        Ok(remote) => (Some(remote), None),
        Err(reason) => (None, Some(reason)),
    };
    let develop = remote
        .as_ref()
        .map(|remote| format!("develop {}", super::paint::strip(&mark_of(remote.develop.as_ref()))))
        .unwrap_or_default();
    frame.band(&format!("platform  {develop}"), &band_time(now_ms()));

    let mr_of = |branch: &str| {
        remote
            .as_ref()
            .and_then(|remote| remote.mrs.iter().find(|mr| mr.branch == branch))
    };
    let title = format!("{} {}", bold("WORKTREES"), dim("click one to go to its space"));
    frame.boxed("WORKTREES", &title, |width| {
        let rows: Vec<Vec<String>> = trees
            .iter()
            .map(|tree| {
                let app = if !tree.has_env {
                    dim("—")
                } else if !tree.has_own_port {
                    dim("⇒ :3000")
                } else if tree.is_up {
                    let url = format!("http://localhost:{}", 3000 + tree.offset);
                    link(Some(&url), &green(&format!("● :{}", 3000 + tree.offset)))
                } else {
                    dim(&format!("○ :{}", 3000 + tree.offset))
                };
                let check = super::orchestration::check_fact(&tree.check);
                let work: Vec<String> = [
                    (tree.ahead > 0).then(|| format!("{} ahead", tree.ahead)),
                    (tree.changed > 0).then(|| yellow(&format!("{} changed", tree.changed))),
                    (!check.is_empty()).then_some(check),
                ]
                .into_iter()
                .flatten()
                .collect();
                // feature/<name> says nothing the name doesn't; a branch that differs does.
                let name = if tree.branch.rsplit('/').next() == Some(tree.name.as_str()) {
                    bold(&tree.name)
                } else {
                    format!("{} {}", bold(&tree.name), dim(&tree.branch))
                };
                let origin = tree
                    .origin
                    .as_ref()
                    .map(|origin| format!(" {}", cyan(&format!("{ORCHESTRATOR} {origin}"))))
                    .unwrap_or_default();
                let mr = mr_of(&tree.branch)
                    .map(|mr| {
                        format!(
                            "{}{} {}",
                            link(mr.url.as_deref(), &format!("!{}", mr.iid)),
                            if mr.is_draft { dim(" draft") } else { String::new() },
                            mark_of(mr.pipeline.as_ref())
                        )
                    })
                    .unwrap_or_default();
                vec![
                    if tree.is_here { cyan("→") } else { " ".into() },
                    format!("{name}{origin}"),
                    app,
                    stack_cell(tree),
                    if work.is_empty() {
                        dim("clean")
                    } else {
                        work.join(" · ")
                    },
                    mr,
                ]
            })
            .collect();
        let mut lines: Vec<String> = table_rows(&rows, width).into_iter().flatten().collect();
        let main_tree = trees.first();
        if projects.is_some()
            && let Some(main_tree) = main_tree.filter(|tree| tree.has_env)
            && !main_tree
                .stack
                .as_deref()
                .is_some_and(|state| state.contains("running"))
        {
            lines.push(String::new());
            lines.push(format!(
                "{} {}",
                super::paint::button("▶ start the default stack", "cyan"),
                dim("pnpm stack:up")
            ));
        }
        if projects.is_none() {
            lines.push(String::new());
            lines.push(yellow("docker does not answer: the stacks are unknown"));
        }
        lines
    });
    let known: Vec<String> = trees
        .iter()
        .map(|tree| {
            env_of(&tree.dir.join("products/vita/web/.env.local"))
                .and_then(|env| env.get("COMPOSE_PROJECT_NAME").cloned())
                .unwrap_or_else(|| "vita".into())
        })
        .collect();
    let mut orphans: Vec<(String, String)> = projects
        .iter()
        .flatten()
        .filter(|(name, _)| name.starts_with("vita") && !known.contains(name))
        .map(|(name, state)| (name.clone(), state.clone()))
        .collect();
    orphans.sort();
    if !orphans.is_empty() {
        frame.boxed(
            "STACKS WITHOUT A WORKTREE",
            &bold("STACKS WITHOUT A WORKTREE"),
            |width| {
                let rows: Vec<Vec<String>> = orphans
                    .iter()
                    .map(|(name, state)| {
                        vec![
                            name.clone(),
                            yellow(state),
                            dim(&format!("docker compose -p {name} down -v")),
                        ]
                    })
                    .collect();
                table_rows(&rows, width).into_iter().flatten().collect()
            },
        );
    }
    if let Some(error) = &error {
        frame.blank();
        let reason = error.strip_prefix("GitLab: ").unwrap_or(error);
        let reason = reason.rfind(" (projects/").map_or(reason, |at| &reason[..at]);
        frame.text_at(
            0,
            &red(&format!(
                "GitLab didn't answer {} ({reason}), tried again",
                minute(now_ms())
            )),
        );
    }
    if let Some(remote) = &remote {
        let title = format!(
            "{} {}",
            bold("MERGE REQUESTS"),
            dim(&format!("{} open", remote.mrs.len()))
        );
        frame.boxed("MERGE REQUESTS", &title, |width| {
            let rows: Vec<Vec<String>> = remote
                .mrs
                .iter()
                .map(|mr| {
                    vec![
                        link(mr.url.as_deref(), &format!("!{}", mr.iid)),
                        if mr.is_draft { dim("draft") } else { green("ready") },
                        mark_of(mr.pipeline.as_ref()),
                        format!("{}  {}", mr.title, dim(&format!("{} · {}", mr.branch, mr.who))),
                    ]
                })
                .collect();
            table_rows(&rows, width).into_iter().flatten().collect()
        });
        let title = format!(
            "{} {}",
            bold("ISSUES"),
            dim(&format!("{} open, claimed first", remote.issues.len()))
        );
        frame.boxed("ISSUES", &title, |width| {
            let rows: Vec<Vec<String>> = remote
                .issues
                .iter()
                .map(|issue| {
                    let claim = match &issue.branch {
                        Some(branch) => cyan(&format!(
                            "←\u{a0}{branch}{}",
                            issue.mr.map(|mr| format!(" !{mr}")).unwrap_or_default()
                        )),
                        None => dim("unclaimed"),
                    };
                    let tags: Vec<String> = [
                        (!issue.labels.is_empty()).then(|| issue.labels.join(",")),
                        (!issue.who.is_empty()).then(|| format!("@{}", issue.who)),
                    ]
                    .into_iter()
                    .flatten()
                    .collect();
                    let tags = if tags.is_empty() {
                        String::new()
                    } else {
                        format!("  {}", dim(&tags.join(" ")))
                    };
                    // The title first, what claims it after: one line for most.
                    vec![
                        link(issue.url.as_deref(), &format!("#{}", issue.iid)),
                        format!("{}  {claim}{tags}", issue.title),
                    ]
                })
                .collect();
            table_rows(&rows, width).into_iter().flatten().collect()
        });
    }
    frame.blank();
    frame.push(dim(
        " pnpm stack:up|down|reset --port <p> · pnpm wt:new <n-feature> · pnpm commands",
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stack_reads_as_its_state() {
        let tree = |has_env: bool, offset: i64, stack: Option<&str>| Tree {
            has_env,
            offset,
            has_own_port: offset > 0,
            stack: stack.map(String::from),
            ..Tree::default()
        };
        assert_eq!(stack_cell(&tree(false, 0, None)), "no .env.local");
        assert_eq!(stack_cell(&tree(true, 0, None)), "shared");
        assert_eq!(stack_cell(&tree(true, 2, None)), "+2 down");
        assert_eq!(stack_cell(&tree(true, 2, Some("running(5)"))), "+2 running(5)");
    }

    #[test]
    fn reads_an_env_file_as_node_does() {
        let dir = std::env::temp_dir().join(format!("bb-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(".env.local");
        std::fs::write(&file, "# x=1\nA=1\nB=x=y\nA=2\n").unwrap();
        let env = env_of(&file).unwrap();
        assert_eq!((env["A"].as_str(), env["B"].as_str()), ("2", "x=y"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
