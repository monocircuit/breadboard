//! The view of one worktree printed (render-worktree.mjs, read.mjs
//! `readRemoteOf`, diff.mjs, develop.mjs): WORK (origin, issue, MR, tests,
//! the delete button), WAITING FOR, ISSUE with its boxes per heading,
//! PIPELINE with its jobs and a failed one's log, DIFF against develop,
//! DEVELOP since the branch started, WHERE and STACK.

use super::agents::{Claude, agents_of, first_line, read_claude, tokens};
use super::clock::{band_time, minute};
use super::everything::{compose_projects, env_of, mark_of};
use super::frame::Frame;
use super::orchestration::{Check, check_fact, check_of};
use super::paint::{bold, button, cyan, dim, green, link, red, table_rows, yellow};
use super::theme::{Theme, paint_tone, theme};
use crate::cli::read::gitlab;
use crate::cli::read::issues::{Box as IssueBox, boxes_with_headings};
use crate::cli::read::list::{offset_of, port_answers};
use breadboard_core::git::{self, Worktree, git, issue_of_branch};
use breadboard_core::json::{int_of, str_of};
use breadboard_core::record::{ORCHESTRATOR, SPACE, origin_of};
use breadboard_core::time::{ago, now_ms};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

/// Where vita's app lives in a checkout.
const VITA: &str = "products/vita/web";
/// How many open boxes ISSUE lists before it counts the rest.
const MAX_OPEN: usize = 10;
/// How many merge requests DEVELOP lists before it counts the rest.
const MAX_MERGES: usize = 6;
/// How many areas DIFF lists, how wide its bars are.
const MAX_AREAS: usize = 6;
const BAR_WIDTH: usize = 24;
/// How many lines of a failed job's log the view shows.
const LOG_LINES: usize = 6;
/// The ports of drizzle studio and Storybook before the offset.
const STUDIO_PORT: i64 = 4983;
const STORYBOOK_PORT: i64 = 6006;

fn plural(count: usize, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

/// The MR of the branch with its pipeline and jobs, its issue, the open MRs it builds on.
#[derive(Default)]
struct Remote {
    mr: Option<Mr>,
    jobs: Vec<Job>,
    issue: Option<Issue>,
    waiting: Vec<Mr>,
}

#[derive(Clone, Default)]
struct Mr {
    iid: i64,
    title: String,
    state: String,
    is_draft: bool,
    url: Option<String>,
    branch: String,
    pipeline: Option<(String, Option<String>)>,
}

impl Mr {
    fn from_value(value: &Value) -> Option<Mr> {
        let title = str_of(value, "title").unwrap_or_default();
        Some(Mr {
            iid: int_of(value, "iid")?,
            title: title
                .strip_prefix("Draft:")
                .map(str::trim_start)
                .unwrap_or(title)
                .to_string(),
            state: str_of(value, "state").unwrap_or_default().to_string(),
            is_draft: value.get("draft").and_then(Value::as_bool).unwrap_or(false),
            url: str_of(value, "web_url").map(String::from),
            branch: str_of(value, "source_branch").unwrap_or_default().to_string(),
            pipeline: value.get("head_pipeline").and_then(|pipeline| {
                Some((
                    str_of(pipeline, "status")?.to_string(),
                    str_of(pipeline, "web_url").map(String::from),
                ))
            }),
        })
    }

    /// ` merged`, ` closed`, ` draft`, ` ready` (render-worktree.mjs `mrState`).
    fn state_text(&self) -> String {
        match self.state.as_str() {
            "merged" => cyan(" merged"),
            "closed" => dim(" closed"),
            _ if self.is_draft => dim(" draft"),
            _ => green(" ready"),
        }
    }
}

struct Job {
    name: String,
    status: String,
    url: Option<String>,
    log: Vec<String>,
}

struct Issue {
    iid: i64,
    title: String,
    url: Option<String>,
    state: String,
    boxes: Vec<IssueBox>,
}

/// `/` and the other reserved characters of a query part escaped.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// The last lines of a failed job's log, where the error usually is (gitlab.mjs `logOf`).
fn log_of(trace: &str) -> Vec<String> {
    let lines: Vec<String> = trace
        .split('\n')
        .map(|line| {
            let line = strip_noise(line);
            let line = line.rsplit('\r').next().unwrap_or_default().to_string();
            strip_stamp(&line).trim_end().to_string()
        })
        .filter(|line| !line.is_empty())
        .collect();
    const TAIL: [&str; 6] = [
        "Cleaning up ",
        "Uploading artifacts",
        "Running after_script",
        "Saving cache",
        "ERROR: Job failed",
        "WARNING: ",
    ];
    let Some(end) = lines
        .iter()
        .rposition(|line| !TAIL.iter().any(|tail| line.starts_with(tail)))
    else {
        return Vec::new();
    };
    lines[(end + 1).saturating_sub(LOG_LINES)..=end].to_vec()
}

/// A log line without GitLab's colours and section markers.
fn strip_noise(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix('\x1b') {
            let end = tail
                .char_indices()
                .find(|(at, c)| *at > 0 && c.is_ascii_alphabetic())
                .map_or(tail.len(), |(at, c)| at + c.len_utf8());
            rest = &tail[end..];
            continue;
        }
        if let Some(tail) = rest
            .strip_prefix("section_start:")
            .or_else(|| rest.strip_prefix("section_end:"))
        {
            // `<time>:<name>[options]` up to the line's end of the marker.
            let end = tail
                .find(|c: char| c == '\r' || c.is_whitespace())
                .unwrap_or(tail.len());
            rest = &tail[end..];
            rest = rest.strip_prefix('\r').unwrap_or(rest);
            continue;
        }
        let character = rest.chars().next().unwrap_or_default();
        out.push(character);
        rest = &rest[character.len_utf8()..];
    }
    out
}

/// A log line without GitLab's time and stream in front: `2026-09-28T13:02:20.886478Z 01E+`.
fn strip_stamp(line: &str) -> &str {
    let bytes = line.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[10] != b'T' {
        return line;
    }
    let Some(z) = line.find("Z ") else { return line };
    let rest = &line[z + 2..];
    let stream = rest.get(..3).filter(|part| {
        let part = part.as_bytes();
        part[0].is_ascii_digit() && part[1].is_ascii_digit() && matches!(part[2], b'O' | b'E')
    });
    match stream {
        Some(_) => {
            let rest = &rest[3..];
            let rest = rest.strip_prefix('+').unwrap_or(rest);
            rest.strip_prefix(' ').unwrap_or(rest)
        }
        None => line,
    }
}

fn read_remote(main: &Path, dir: &Path, branch: &str) -> Result<Remote, String> {
    let number = issue_of_branch(branch);
    let mut endpoints = vec![
        format!(
            "projects/:fullpath/merge_requests?source_branch={}&order_by=updated_at",
            encode(branch)
        ),
        "projects/:fullpath/merge_requests?state=opened&per_page=100".to_string(),
        "projects/:fullpath/pipelines?per_page=100".to_string(),
    ];
    let (lists, issue) = thread::scope(|scope| {
        let issue =
            scope.spawn(|| number.map(|number| gitlab::get(main, &format!("projects/:fullpath/issues/{number}"))));
        let lists = gitlab::lists(main, &endpoints);
        (lists, issue.join().ok().flatten())
    });
    endpoints.clear();
    let mut lists = lists.into_iter();
    let mrs = lists.next().unwrap_or(Ok(Vec::new()))?;
    let open = lists.next().unwrap_or(Ok(Vec::new()))?;
    let pipelines = lists.next().unwrap_or(Ok(Vec::new()))?;
    let latest = mrs
        .iter()
        .find(|mr| str_of(mr, "state") == Some("opened"))
        .or(mrs.first())
        .and_then(|mr| int_of(mr, "iid"));
    // The single MR has its head pipeline; its jobs, a failed one's log with them.
    let single = match latest {
        Some(iid) => Some(gitlab::get(main, &format!("projects/:fullpath/merge_requests/{iid}"))?),
        None => None,
    };
    let mr = single.as_ref().and_then(Mr::from_value);
    let jobs = single
        .as_ref()
        .and_then(|single| single.get("head_pipeline"))
        .and_then(|pipeline| int_of(pipeline, "id"))
        .map(|id| jobs_of(main, id))
        .unwrap_or_default();
    let issue = issue.and_then(Result::ok).and_then(|issue| {
        Some(Issue {
            iid: int_of(&issue, "iid")?,
            title: str_of(&issue, "title").unwrap_or_default().to_string(),
            url: str_of(&issue, "web_url").map(String::from),
            state: str_of(&issue, "state").unwrap_or_default().to_string(),
            boxes: boxes_with_headings(str_of(&issue, "description").unwrap_or_default()),
        })
    });
    // The open MRs it builds on, with their pipelines (waiting.mjs).
    let mut pipeline_of: HashMap<String, (String, Option<String>)> = HashMap::new();
    for pipeline in &pipelines {
        if let (Some(reference), Some(status)) = (str_of(pipeline, "ref"), str_of(pipeline, "status")) {
            pipeline_of
                .entry(reference.to_string())
                .or_insert_with(|| (status.to_string(), str_of(pipeline, "web_url").map(String::from)));
        }
    }
    let mut waiting: Vec<(u64, Mr)> = open
        .iter()
        .filter_map(Mr::from_value)
        .filter(|candidate| candidate.branch != branch)
        .filter_map(|mut candidate| {
            let reference = format!("refs/remotes/origin/{}", candidate.branch);
            let found = git(dir, &["rev-parse", "--verify", "--quiet", &reference]);
            if !found.is_ok || found.out().is_empty() {
                return None;
            }
            let base = git(dir, &["merge-base", found.out(), "HEAD"]);
            if !base.is_ok || base.out().is_empty() {
                return None;
            }
            let count = git::count(dir, &format!("origin/develop..{}", base.out())).unwrap_or(0);
            candidate.pipeline = pipeline_of
                .get(&format!("refs/merge-requests/{}/head", candidate.iid))
                .or_else(|| pipeline_of.get(&candidate.branch))
                .cloned();
            (count > 0).then_some((count, candidate))
        })
        .collect();
    waiting.sort_by_key(|(count, _)| *count);
    Ok(Remote {
        mr,
        jobs,
        issue,
        waiting: waiting.into_iter().map(|(_, mr)| mr).collect(),
    })
}

/// The jobs of a pipeline in the order they ran, a failed one with the end of its log.
fn jobs_of(main: &Path, id: i64) -> Vec<Job> {
    let Ok(mut list) = gitlab::list(
        main,
        &format!("projects/:fullpath/pipelines/{id}/jobs?per_page=100&include_retried=false"),
    ) else {
        return Vec::new();
    };
    list.sort_by_key(|job| int_of(job, "id").unwrap_or(0));
    list.iter()
        .map(|job| {
            let status = str_of(job, "status").unwrap_or_default().to_string();
            let log = match int_of(job, "id") {
                Some(id) if status == "failed" => gitlab::text(main, &format!("projects/:fullpath/jobs/{id}/trace"))
                    .map(|trace| log_of(&trace))
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            Job {
                name: str_of(job, "name").unwrap_or_default().to_string(),
                status,
                url: str_of(job, "web_url").map(String::from),
                log,
            }
        })
        .collect()
}

/// An area of the diff: where in the repo its files are.
#[derive(Default)]
struct Area {
    name: String,
    added: u64,
    removed: u64,
    files: usize,
    top: String,
    top_changed: u64,
}

/// The diff of a worktree against develop (diff.mjs `diffOf`).
struct Diff {
    ahead: u64,
    behind: u64,
    uncommitted: usize,
    added: u64,
    removed: u64,
    files: usize,
    areas: Vec<Area>,
    with_develop: Vec<String>,
    with_worktrees: Vec<(String, Vec<String>)>,
}

/// The area a file belongs to (diff.mjs `areaOf`): inside a package the
/// folder below `src`, outside them the top-level folder or file.
fn area_of(file: &str) -> String {
    let parts: Vec<&str> = file.split('/').collect();
    if !["products", "packages", "tools"].contains(&parts[0]) || parts.len() < 3 {
        return if parts.len() > 1 {
            parts[0].to_string()
        } else {
            file.to_string()
        };
    }
    let (name, inside): (String, &[&str]) = if parts[0] == "products" && parts.len() > 3 {
        (
            if parts[2] == "web" {
                parts[1].to_string()
            } else {
                format!("{}-{}", parts[1], parts[2])
            },
            &parts[3..],
        )
    } else {
        (parts[1].to_string(), &parts[2..])
    };
    let below = if inside.first() == Some(&"src") {
        &inside[1..]
    } else {
        inside
    };
    if below.len() > 1 {
        format!("{name}/{}", below[0])
    } else {
        name
    }
}

fn path_of_status(line: &str) -> String {
    git::path_of_status(line)
}

fn diff_of(dir: &Path, others: &[(String, PathBuf)]) -> Option<Diff> {
    let base = git(dir, &["merge-base", "origin/develop", "HEAD"]);
    if !base.is_ok || base.out().is_empty() {
        return None;
    }
    let base = base.out().to_string();
    let status: Vec<String> = git(dir, &["status", "--porcelain", "-uall"])
        .lines()
        .map(String::from)
        .collect();
    let mut areas: Vec<Area> = Vec::new();
    let (mut added, mut removed) = (0, 0);
    let mut files: Vec<String> = Vec::new();
    fn count(file: &str, plus: u64, minus: u64, areas: &mut Vec<Area>, files: &mut Vec<String>) {
        let name = area_of(file);
        let at = match areas.iter().position(|area| area.name == name) {
            Some(at) => at,
            None => {
                areas.push(Area {
                    name,
                    ..Area::default()
                });
                areas.len() - 1
            }
        };
        let area = &mut areas[at];
        area.added += plus;
        area.removed += minus;
        area.files += 1;
        if area.top.is_empty() || plus + minus > area.top_changed {
            area.top = file.to_string();
            area.top_changed = plus + minus;
        }
        files.push(file.to_string());
    }
    for line in git(dir, &["diff", "--numstat", &base]).lines() {
        let mut parts = line.splitn(3, '\t');
        let (plus, minus, file) = (parts.next(), parts.next(), parts.next().unwrap_or_default());
        let (plus, minus) = (
            plus.and_then(|p| p.parse().ok()).unwrap_or(0),
            minus.and_then(|m| m.parse().ok()).unwrap_or(0),
        );
        let name = if file.contains(" => ") {
            renamed(file)
        } else {
            file.to_string()
        };
        added += plus;
        removed += minus;
        count(&name, plus, minus, &mut areas, &mut files);
    }
    for file in status
        .iter()
        .filter(|line| line.starts_with("??"))
        .map(|line| path_of_status(line))
    {
        if files.contains(&file) {
            continue;
        }
        let plus: u64 = git(dir, &["diff", "--no-index", "--numstat", "/dev/null", &file])
            .lines()
            .next()
            .and_then(|line| line.split('\t').next()?.parse().ok())
            .unwrap_or(0);
        added += plus;
        count(&file, plus, 0, &mut areas, &mut files);
    }
    let on_develop: Vec<String> = git(dir, &["diff", "--name-only", &base, "origin/develop"])
        .lines()
        .map(String::from)
        .collect();
    areas.sort_by_key(|area| std::cmp::Reverse(area.added + area.removed));
    let mut with_develop: Vec<String> = files.iter().filter(|file| on_develop.contains(file)).cloned().collect();
    with_develop.sort();
    let mine = git::work_files(dir);
    let with_worktrees = others
        .iter()
        .filter_map(|(name, other)| {
            let mut shared: Vec<String> = git::work_files(other)
                .into_iter()
                .filter(|file| mine.contains(file))
                .collect();
            shared.sort();
            (!shared.is_empty()).then(|| (name.clone(), shared))
        })
        .collect();
    Some(Diff {
        ahead: git::count(dir, "origin/develop..HEAD").unwrap_or(0),
        behind: git::count(dir, "HEAD..origin/develop").unwrap_or(0),
        uncommitted: status.len(),
        added,
        removed,
        files: files.len(),
        areas,
        with_develop,
        with_worktrees,
    })
}

/// The new name of a rename numstat gives: `a/{x => y}/b` → `a/y/b`, `x => y` → `y`.
fn renamed(file: &str) -> String {
    if let (Some(open), Some(close)) = (file.find('{'), file.find('}'))
        && open < close
    {
        let inside = &file[open + 1..close];
        let new = inside.split(" => ").last().unwrap_or(inside);
        let joined = format!("{}{new}{}", &file[..open], &file[close + 1..]);
        return joined.replace("//", "/");
    }
    file.split(" => ").last().unwrap_or(file).to_string()
}

/// A bar of `width` cells for `added` and `removed`, scaled so `max` fills it.
fn bar_of(added: u64, removed: u64, max: u64, width: usize) -> (String, String) {
    if max == 0 {
        return (String::new(), String::new());
    }
    let cells = |total: u64| {
        if total == 0 {
            0
        } else {
            ((total as f64 / max as f64 * width as f64).round() as usize).max(1)
        }
    };
    ("█".repeat(cells(added)), "░".repeat(cells(removed)))
}

/// What came into develop since the branch started (develop.mjs `developOf`).
struct Develop {
    merges: Vec<(Option<i64>, String, String)>,
    direct: usize,
    conflicts: Option<Vec<String>>,
}

fn develop_of(dir: &Path) -> Develop {
    let log = git(
        dir,
        &["log", "--first-parent", "--format=%s%x1f%b%x1e", "HEAD..origin/develop"],
    );
    let entries: Vec<(String, String)> = log
        .stdout
        .split('\x1e')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (subject, body) = entry.split_once('\x1f').unwrap_or((entry, ""));
            (subject.to_string(), body.to_string())
        })
        .collect();
    let mut merges = Vec::new();
    let mut direct = 0;
    for (subject, body) in &entries {
        let Some(branch) = subject
            .strip_prefix("Merge branch '")
            .and_then(|rest| rest.split_once("' into").map(|(branch, _)| branch.to_string()))
        else {
            direct += 1;
            continue;
        };
        let mr = body.lines().rev().find_map(|line| {
            let line = line.trim_end();
            let at = line.rfind('!')?;
            line[at + 1..].parse().ok()
        });
        let title = body
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .find(|line| {
                let lower = line.to_lowercase();
                !["closes", "refs", "see merge request", "co-authored-by"]
                    .iter()
                    .any(|word| lower.starts_with(word))
            })
            .map(String::from)
            .unwrap_or_else(|| branch.clone());
        merges.push((mr, title, branch));
    }
    let conflicts = if entries.is_empty() {
        Some(Vec::new())
    } else {
        let tree = git(
            dir,
            &[
                "merge-tree",
                "--write-tree",
                "--name-only",
                "--no-messages",
                "HEAD",
                "origin/develop",
            ],
        );
        match tree.status {
            0 => Some(Vec::new()),
            1 => Some(tree.lines().skip(1).map(String::from).collect()),
            _ => None,
        }
    };
    Develop {
        merges,
        direct,
        conflicts,
    }
}

/// `prefix` and the first files by name, the rest counted (files.mjs `fileLine`).
fn file_line(prefix: &str, files: &[String], count: usize, suffix: &str) -> String {
    let named = files.iter().take(count).cloned().collect::<Vec<_>>().join(", ");
    let more = if files.len() > count {
        format!(" {}", dim(&format!("+{} more", files.len() - count)))
    } else {
        String::new()
    };
    format!("{prefix}{named}{more}{suffix}")
}

/// The containers of a checkout's stack: service, state, health.
fn containers_of(dir: &Path) -> Vec<(String, String, String)> {
    let output = Command::new("docker")
        .args([
            "compose",
            "--env-file",
            ".env.local",
            "-f",
            "docker-compose.local.yml",
            "ps",
            "--all",
            "--format",
            "json",
        ])
        .current_dir(dir.join(VITA))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else { return Vec::new() };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .map(|container| {
            (
                str_of(&container, "Service").unwrap_or_default().to_string(),
                str_of(&container, "State").unwrap_or_default().to_string(),
                str_of(&container, "Health").unwrap_or_default().to_string(),
            )
        })
        .collect()
}

/// How long the context bar is.
const BAR: usize = 20;

/// A path with the home directory as `~`.
fn home(path: &Path) -> String {
    let text = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && text.starts_with(&home) => format!("~{}", &text[home.len()..]),
        _ => text,
    }
}

/// The conversation of the space as a line (render-worktree.mjs `claudeLine`):
/// a bar of its context, then model, its last answer and its id.
fn claude_line(claude: &Claude, now: i64) -> String {
    let short: String = claude.id.chars().take(8).collect();
    let Some(at) = claude.at else {
        return dim(&format!("{short} · no answer yet"));
    };
    let share = claude.share();
    let tone = |text: &str| {
        if share >= 0.8 {
            red(text)
        } else if share >= 0.5 {
            yellow(text)
        } else {
            green(text)
        }
    };
    let filled = ((share * BAR as f64).round() as usize).min(BAR);
    let model = claude.model.as_deref().unwrap_or_default();
    format!(
        "{}{} {} {} of {}  {}",
        tone(&"█".repeat(filled)),
        dim(&"░".repeat(BAR - filled)),
        tone(&format!("{}%", (share * 100.0).round() as i64)),
        tokens(claude.used),
        tokens(claude.window),
        dim(&format!(
            "{} · {} · {short}",
            model.strip_prefix("claude-").unwrap_or(model),
            ago(at, now)
        ))
    )
}

/// AGENT (actions.mjs `pushAgent`): the conversation bound to the space, what
/// herdr says it does, its context, how to get it back when herdr or its tab is gone.
fn agent_box(frame: &mut Frame, theme: &Theme, claude: &Claude, status: Option<&str>, now: i64) {
    let state = match status {
        Some("working") => paint_tone(theme, Some("work"), "◐ at work"),
        Some("blocked") => paint_tone(theme, Some("you"), "● asks you something"),
        Some("idle") => paint_tone(theme, Some("free"), "waiting for you"),
        Some("done") => paint_tone(theme, Some("free"), "✓ done, waiting for you"),
        _ => String::new(),
    };
    frame.boxed("AGENT", &format!("{} {state}", bold("AGENT")), |width| {
        let mut rows = vec![
            vec![dim("context"), claude_line(claude, now)],
            vec![
                dim("resume"),
                dim(&format!("cd {} && claude --resume {}", home(&claude.cwd), claude.id)),
            ],
            vec![dim("stored"), dim(&home(&claude.transcript))],
        ];
        if let Some(prompt) = &claude.prompt {
            rows.push(vec![dim("asked"), first_line(prompt)]);
        }
        if let Some(answer) = &claude.answer {
            rows.push(vec![dim("said"), first_line(answer)]);
        }
        table_rows(&rows, width).into_iter().flatten().collect()
    });
}

/// The view of the worktree `dir` lies in; Err says why there is none.
pub fn worktree(frame: &mut Frame, main: &Path, dir: &Path) -> Result<(), String> {
    let now = now_ms();
    let worktrees = git::worktrees(main);
    // The worktree the directory lies in, the innermost.
    let found: Option<Worktree> = worktrees
        .iter()
        .filter(|tree| git::is_inside(dir, &tree.dir))
        .max_by_key(|tree| tree.dir.as_os_str().len())
        .cloned();
    let Some(found) = found else {
        return Err(format!(
            "{} is not a worktree of the repo: pnpm wt:list shows them",
            dir.display()
        ));
    };
    let tree_dir = found.dir.clone();
    let is_main = worktrees.first().is_some_and(|first| first.dir == tree_dir);
    let main_name = main
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let base = tree_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = if is_main {
        main_name.clone()
    } else {
        base.replacen(&format!("{main_name}-"), "", 1)
    };
    let branch = found.branch.clone().unwrap_or_else(|| "(detached)".into());
    let others: Vec<(String, PathBuf)> = worktrees
        .iter()
        .skip(1)
        .filter(|other| other.dir != tree_dir)
        .map(|other| {
            let base = other
                .dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            (base.replacen(&format!("{main_name}-"), "", 1), other.dir.clone())
        })
        .collect();

    // Local and GitLab side by side.
    let (remote, local) = thread::scope(|scope| {
        let remote = scope.spawn(|| read_remote(main, &tree_dir, &branch));
        let ahead = git::count(&tree_dir, "origin/develop..HEAD").unwrap_or(0);
        let check = check_of(&tree_dir, ahead);
        let changed = git::changed_count(&tree_dir);
        let own_commits = if is_main || found.branch.is_none() {
            0
        } else {
            git::own_commits(&tree_dir, &branch)
        };
        let diff = (!is_main).then(|| diff_of(&tree_dir, &others)).flatten();
        let develop = (!is_main).then(|| develop_of(&tree_dir));
        let remote = remote.join().unwrap_or_else(|_| Err("the read broke off".into()));
        (remote, (ahead, check, changed, own_commits, diff, develop))
    });
    let (ahead, check, changed, own_commits, diff, develop) = local;
    let (remote, error) = match remote {
        Ok(remote) => (Some(remote), None),
        Err(reason) => (None, Some(reason)),
    };

    frame.band(&format!("{name}  {branch}"), &band_time(now));
    frame.push(dim(&format!(" {}", tree_dir.display())));

    let origin = if is_main {
        None
    } else {
        origin_of(&tree_dir).map(|origin| origin.name)
    };
    frame.boxed("WORK", &bold("WORK"), |width| {
        let mut lines = Vec::new();
        if let Some(error) = &error {
            let reason = error.strip_prefix("GitLab: ").unwrap_or(error);
            let reason = reason.rfind(" (projects/").map_or(reason, |at| &reason[..at]);
            lines.push(red(&format!("GitLab didn't answer {} ({reason})", minute(now))));
        }
        let mut rows: Vec<Vec<String>> = Vec::new();
        if diff.is_none() {
            let work: Vec<String> = [
                (ahead > 0).then(|| format!("{ahead} ahead of develop")),
                (changed > 0).then(|| yellow(&format!("{changed} changed"))),
            ]
            .into_iter()
            .flatten()
            .collect();
            let work = if work.is_empty() {
                dim("clean, nothing ahead of develop")
            } else {
                work.join(" · ")
            };
            // The main checkout's work: a worktree's gets the section DIFF.
            rows.push(vec!["work".into(), work]);
        }
        if let Some(origin) = &origin {
            rows.push(vec![
                "origin".into(),
                format!(
                    "{} {}",
                    cyan(&format!("{ORCHESTRATOR} {origin}")),
                    dim("the orchestrating session, its agent reports there")
                ),
            ]);
        }
        if let Some(remote) = &remote {
            if let Some(issue) = &remote.issue {
                rows.push(vec![
                    "issue".into(),
                    format!(
                        "{} {}{}",
                        link(issue.url.as_deref(), &format!("#{}", issue.iid)),
                        issue.title,
                        if issue.state == "closed" {
                            dim(" (closed)")
                        } else {
                            String::new()
                        }
                    ),
                ]);
            }
            rows.push(vec![
                "mr".into(),
                match &remote.mr {
                    Some(mr) => format!(
                        "{}{} {}  {}",
                        link(mr.url.as_deref(), &format!("!{}", mr.iid)),
                        mr.state_text(),
                        mark_of(mr.pipeline.as_ref()),
                        dim(mr.url.as_deref().unwrap_or_default())
                    ),
                    None => dim(if is_main { "—" } else { "none yet: pnpm wt:mr" }),
                },
            ]);
        }
        let tests = check_fact(&check);
        if !tests.is_empty() {
            let more = match &check {
                Check::Tested { .. } | Check::Checking => String::new(),
                _ => format!("  {}", dim("pnpm wt:check before the push")),
            };
            rows.push(vec!["tests".into(), format!("{tests}{more}")]);
        }
        lines.extend(table_rows(&rows, width).into_iter().flatten());
        // Below issue and MR: the button is the last step of a worktree.
        let is_done = !is_main && found.branch.is_some() && changed == 0 && own_commits == 0;
        if is_done {
            lines.push(format!(
                "{} {}",
                button("✕ delete worktree", "red"),
                dim(&format!("pnpm wt:remove {name}"))
            ));
        } else if !is_main && found.branch.is_some() {
            let reasons: Vec<String> = [
                (changed > 0).then(|| plural(changed, "uncommitted file")),
                (own_commits > 0).then(|| format!("{} not in develop", plural(own_commits, "commit"))),
            ]
            .into_iter()
            .flatten()
            .collect();
            let is_stale = remote
                .as_ref()
                .and_then(|remote| remote.mr.as_ref())
                .is_some_and(|mr| mr.state == "merged")
                && own_commits > 0;
            let why = if is_stale {
                "merged on GitLab, but develop here is older: git fetch".to_string()
            } else {
                format!("not yet: {}", reasons.join(", "))
            };
            lines.push(format!("{} {}", button("✕ delete worktree", "gray"), dim(&why)));
        }
        lines
    });

    if let Some(remote) = &remote
        && let Some(next) = remote.waiting.first()
    {
        let title = format!(
            "{} {}",
            bold("WAITING FOR"),
            dim(&format!(
                "merge in this order; next: !{}, then this branch goes on",
                next.iid
            ))
        );
        frame.boxed("WAITING FOR", &title, |width| {
            let rows: Vec<Vec<String>> = remote
                .waiting
                .iter()
                .enumerate()
                .map(|(at, mr)| {
                    vec![
                        dim(&format!("{}.", at + 1)),
                        format!(
                            "{}{}",
                            link(mr.url.as_deref(), &format!("!{}", mr.iid)),
                            mr.state_text()
                        ),
                        mark_of(mr.pipeline.as_ref()),
                        mr.title.clone(),
                    ]
                })
                .collect();
            table_rows(&rows, width).into_iter().flatten().collect()
        });
    }
    if !is_main && let Some(claude) = read_claude(&tree_dir) {
        let status = agents_of().get(&claude.id).map(|agent| agent.status.clone());
        agent_box(frame, theme(main), &claude, status.as_deref(), now);
    }
    if let Some(remote) = &remote {
        if let Some(issue) = remote.issue.as_ref().filter(|issue| !issue.boxes.is_empty()) {
            let done = issue.boxes.iter().filter(|found| found.is_ticked).count();
            let total = issue.boxes.len();
            let count = format!("{done}/{total} done");
            let title = format!(
                "{} {}  {}",
                bold("ISSUE"),
                link(issue.url.as_deref(), &format!("#{}", issue.iid)),
                if done == total { green(&count) } else { count }
            );
            frame.boxed("ISSUE", &title, |width| {
                let mut lines = Vec::new();
                let mut headings: Vec<Option<String>> = Vec::new();
                for found in &issue.boxes {
                    if !headings.contains(&found.heading) {
                        headings.push(found.heading.clone());
                    }
                }
                let (mut shown, mut rest) = (0, 0);
                for heading in headings {
                    let boxes: Vec<&IssueBox> = issue.boxes.iter().filter(|found| found.heading == heading).collect();
                    let done = boxes.iter().filter(|found| found.is_ticked).count();
                    let count = if done == boxes.len() {
                        green(&format!("{done}/{} ✓", boxes.len()))
                    } else {
                        format!("{done}/{}", boxes.len())
                    };
                    lines.push(format!("{}  {count}", bold(heading.as_deref().unwrap_or("—"))));
                    for open in boxes.iter().filter(|found| !found.is_ticked) {
                        if shown < MAX_OPEN {
                            let text = format!("{} {}", dim("○"), open.text);
                            lines.extend(
                                super::paint::wrap(&text, width.saturating_sub(4).max(1))
                                    .into_iter()
                                    .enumerate()
                                    .map(|(at, line)| format!("{}{line}", if at == 0 { "  " } else { "    " })),
                            );
                        } else {
                            rest += 1;
                        }
                        shown += 1;
                    }
                }
                if rest > 0 {
                    lines.push(dim(&format!("  … {rest} more open")));
                }
                lines
            });
        }
        if let Some(mr) = remote.mr.as_ref().filter(|mr| mr.pipeline.is_some())
            && !remote.jobs.is_empty()
        {
            let jobs = &remote.jobs;
            let title = format!("{} {}", bold("PIPELINE"), mark_of(mr.pipeline.as_ref()));
            frame.boxed("PIPELINE", &title, |width| {
                let short = |status: &str| match status {
                    "success" => green("✓"),
                    "failed" => red("✗"),
                    "running" => yellow("◐"),
                    "pending" | "created" => yellow("…"),
                    "manual" => dim("▸"),
                    _ => dim("·"),
                };
                let marks: Vec<String> = jobs
                    .iter()
                    .map(|job| format!("{}\u{a0}{}", short(&job.status), link(job.url.as_deref(), &job.name)))
                    .collect();
                // Every job whole, over as many lines as it takes (#304).
                let mut lines = super::paint::wrap(&marks.join("   "), width);
                for job in jobs.iter().filter(|job| !job.log.is_empty()) {
                    lines.push(format!(
                        "{} {}  {}",
                        red("✗"),
                        bold(&job.name),
                        dim("the job's page has all of it")
                    ));
                    for line in &job.log {
                        lines.extend(
                            super::paint::wrap(&dim(line), width.saturating_sub(4).max(1))
                                .into_iter()
                                .map(|line| format!("    {line}")),
                        );
                    }
                }
                lines
            });
        }
    }
    if let Some(diff) = &diff {
        let counts: Vec<String> = [
            Some(format!("{} ahead", diff.ahead)),
            Some(if diff.behind > 0 {
                yellow(&format!("{} behind", diff.behind))
            } else {
                dim("0 behind")
            }),
            (diff.uncommitted > 0).then(|| yellow(&format!("{} uncommitted", diff.uncommitted))),
        ]
        .into_iter()
        .flatten()
        .collect();
        let title = format!(
            "{} {}  {}  {}",
            bold("DIFF"),
            dim("against develop"),
            counts.join(" · "),
            dim("click a file: nvim at the change")
        );
        frame.boxed("DIFF", &title, |width| {
            let mut lines = Vec::new();
            if diff.files == 0 {
                lines.push(dim("nothing changed yet"));
            } else {
                let (plus, minus) = bar_of(diff.added, diff.removed, diff.added + diff.removed, BAR_WIDTH);
                lines.push(format!(
                    "{} {} in {}  {}{}",
                    green(&format!("+{}", diff.added)),
                    red(&format!("−{}", diff.removed)),
                    plural(diff.files, "file"),
                    green(&plus),
                    red(&minus)
                ));
                let shown: Vec<&Area> = diff.areas.iter().take(MAX_AREAS).collect();
                let rest: Vec<&Area> = diff.areas.iter().skip(MAX_AREAS).collect();
                let max = shown.iter().map(|area| area.added + area.removed).max().unwrap_or(0);
                let mut rows: Vec<Vec<String>> = shown
                    .iter()
                    .map(|area| {
                        let (plus, minus) = bar_of(area.added, area.removed, max, BAR_WIDTH);
                        let top = area.top.rsplit('/').next().unwrap_or(&area.top);
                        vec![
                            area.name.clone(),
                            green(&format!("+{}", area.added)),
                            red(&format!("−{}", area.removed)),
                            format!("{}{}", green(&plus), red(&minus)),
                            dim(&format!("{} · {top}", plural(area.files, "file"))),
                        ]
                    })
                    .collect();
                if !rest.is_empty() {
                    let sum = |of: fn(&Area) -> u64| rest.iter().map(|area| of(area)).sum::<u64>();
                    rows.push(vec![
                        dim(&format!("… {}", plural(rest.len(), "area"))),
                        green(&format!("+{}", sum(|area| area.added))),
                        red(&format!("−{}", sum(|area| area.removed))),
                        String::new(),
                        dim(&plural(rest.iter().map(|area| area.files).sum(), "file")),
                    ]);
                }
                lines.push(String::new());
                lines.extend(table_rows(&rows, width).into_iter().flatten());
            }
            if !diff.with_develop.is_empty() || !diff.with_worktrees.is_empty() {
                lines.push(String::new());
            }
            if !diff.with_develop.is_empty() {
                lines.extend(super::paint::wrap(
                    &file_line(
                        &format!("{} also changed on develop: ", yellow("⚠")),
                        &diff.with_develop,
                        3,
                        &format!("  {}", dim("merge develop in while it is small")),
                    ),
                    width,
                ));
            }
            for (other, files) in &diff.with_worktrees {
                lines.extend(super::paint::wrap(
                    &file_line(
                        &format!("{} also changed in {}: ", yellow("⚠"), bold(other)),
                        files,
                        3,
                        "",
                    ),
                    width,
                ));
            }
            lines
        });
    }
    if let Some(develop) = develop
        .as_ref()
        .filter(|develop| !develop.merges.is_empty() || develop.direct > 0)
    {
        let base = remote
            .as_ref()
            .and_then(|remote| {
                remote
                    .issue
                    .as_ref()
                    .and_then(|issue| issue.url.clone())
                    .or_else(|| remote.mr.as_ref().and_then(|mr| mr.url.clone()))
            })
            .and_then(|url| url.split("/-/").next().map(String::from));
        let title = format!(
            "{} {}  {}",
            bold("DEVELOP"),
            dim("since the branch started"),
            plural(develop.merges.len(), "merge request")
        );
        frame.boxed("DEVELOP", &title, |width| {
            let mut lines = Vec::new();
            match &develop.conflicts {
                None => lines.push(dim("git can't tell whether a merge would conflict (git < 2.38)")),
                Some(conflicts) if conflicts.is_empty() => lines.push(format!(
                    "{} merges cleanly: {}",
                    green("✓"),
                    dim("git fetch && git merge origin/develop")
                )),
                Some(conflicts) => lines.extend(super::paint::wrap(
                    &file_line(&format!("{} a merge conflicts in ", red("✗")), conflicts, 4, ""),
                    width,
                )),
            }
            let rows: Vec<Vec<String>> = develop
                .merges
                .iter()
                .take(MAX_MERGES)
                .map(|(mr, title, branch)| {
                    let url = mr.and_then(|mr| base.as_ref().map(|base| format!("{base}/-/merge_requests/{mr}")));
                    vec![
                        mr.map(|mr| link(url.as_deref(), &format!("!{mr}"))).unwrap_or_default(),
                        title.clone(),
                        dim(branch),
                    ]
                })
                .collect();
            lines.extend(table_rows(&rows, width).into_iter().flatten());
            let more: Vec<String> = [
                (develop.merges.len() > MAX_MERGES)
                    .then(|| plural(develop.merges.len() - MAX_MERGES, "more merge request")),
                (develop.direct > 0).then(|| format!("{} without one", plural(develop.direct, "commit"))),
            ]
            .into_iter()
            .flatten()
            .collect();
            if !more.is_empty() {
                lines.push(dim(&format!("… {}", more.join(", "))));
            }
            lines
        });
    }

    let env_file = tree_dir.join(VITA).join(".env.local");
    let Some(env) = env_of(&env_file) else {
        frame.boxed("STACK", &bold("STACK"), |_| {
            vec![yellow("no .env.local: no stack, the app can't run here")]
        });
        return Ok(());
    };
    let offset = if is_main { 0 } else { offset_of(&env_file) };
    let is_shared = !is_main && offset == 0;
    let project = env
        .get("COMPOSE_PROJECT_NAME")
        .cloned()
        .unwrap_or_else(|| "vita".into());
    let projects = compose_projects();
    let state = projects.as_ref().and_then(|projects| projects.get(&project).cloned());
    let is_up = state.as_deref().is_some_and(|state| state.contains("running"));
    let port = |variable: &str, base: i64| {
        env.get(variable)
            .and_then(|value| value.trim().parse::<i64>().ok())
            .filter(|port| *port != 0)
            .unwrap_or(base + offset)
    };
    let mark = |is_on: bool| if is_on { green("●") } else { dim("○") };
    let app = format!("http://localhost:{}", 3000 + offset);
    let is_app_up = port_answers(3000 + offset);
    let is_studio_up = port_answers(STUDIO_PORT + offset);
    let is_storybook_up = port_answers(STORYBOOK_PORT + offset);
    let keycloak = format!("http://localhost:{}/admin", port("KEYCLOAK_PORT", 8080));
    let s3 = format!("http://localhost:{}", port("S3_PORT", 8333));
    let studio = format!("https://local.drizzle.studio?port={}", STUDIO_PORT + offset);
    let storybook = format!("http://localhost:{}", STORYBOOK_PORT + offset);
    let rows: Vec<Vec<String>> = vec![
        vec![
            mark(is_app_up),
            "app".into(),
            link(Some(&app), &app),
            if is_app_up {
                if is_shared {
                    dim("shared: whichever checkout runs pnpm dev")
                } else {
                    String::new()
                }
            } else {
                dim("cd products/vita/web && pnpm dev")
            },
        ],
        vec![
            mark(is_up),
            "keycloak".into(),
            link(Some(&keycloak), &keycloak),
            String::new(),
        ],
        vec![mark(is_up), "s3".into(), link(Some(&s3), &s3), String::new()],
        vec![
            mark(is_up),
            "postgres".into(),
            format!("localhost:{}", port("POSTGRES_PORT", 5432)),
            String::new(),
        ],
        vec![
            mark(is_studio_up),
            "studio".into(),
            link(Some(&studio), &studio),
            if is_studio_up {
                String::new()
            } else {
                dim(&format!("pnpm pg:studio --port {}", STUDIO_PORT + offset))
            },
        ],
        vec![
            mark(is_storybook_up),
            "storybook".into(),
            link(Some(&storybook), &storybook),
            if is_storybook_up {
                String::new()
            } else {
                dim(&format!(
                    "cd packages/monolithium && pnpm exec storybook dev -p {}",
                    STORYBOOK_PORT + offset
                ))
            },
        ],
    ];
    let title = format!("{} {}", bold("WHERE"), dim("click a row to open it"));
    frame.boxed("WHERE", &title, |width| {
        table_rows(&rows, width)
            .into_iter()
            .flatten()
            .map(|line| line.trim_end().to_string())
            .collect()
    });
    let title = format!(
        "{} {}",
        bold("STACK"),
        dim(&if is_shared {
            format!("shared, the one of the main checkout ({project})")
        } else {
            project.clone()
        })
    );
    frame.boxed("STACK", &title, |width| {
        let mut lines = Vec::new();
        if is_shared {
            lines.push(dim(&format!(
                "its tabs are in the space {SPACE}; an own stack: pnpm wt:stack"
            )));
        } else if projects.is_none() {
            lines.push(yellow("docker does not answer"));
        } else {
            let rows: Vec<Vec<String>> = containers_of(&tree_dir)
                .into_iter()
                .map(|(service, state, health)| {
                    vec![
                        service,
                        if state == "running" {
                            green(&state)
                        } else {
                            yellow(&state)
                        },
                        if health == "healthy" {
                            green(&health)
                        } else if health.is_empty() {
                            String::new()
                        } else {
                            yellow(&health)
                        },
                    ]
                })
                .collect();
            lines.extend(
                table_rows(&rows, width)
                    .into_iter()
                    .flatten()
                    .map(|line| line.trim_end().to_string()),
            );
            if !is_up {
                lines.push(format!(
                    "{} {}",
                    button("▶ start the stack", "cyan"),
                    dim("pnpm stack:up")
                ));
            }
        }
        lines
    });
    frame.blank();
    frame.push(dim(
        " pnpm stack:up|down|reset · pnpm wt:mr --update · pnpm overview (everything)",
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_area_is_where_in_the_repo_a_file_is() {
        assert_eq!(area_of("products/vita/web/src/server/db.ts"), "vita/server");
        assert_eq!(area_of("products/vita/core/x.ts"), "vita-core");
        assert_eq!(area_of("packages/utilities/src/devtools/a.ts"), "utilities/devtools");
        assert_eq!(area_of("packages/breadboard/README.md"), "breadboard");
        assert_eq!(area_of("scripts/x.mjs"), "scripts");
        assert_eq!(area_of("AGENTS.md"), "AGENTS.md");
    }

    #[test]
    fn a_rename_counts_under_its_new_name() {
        assert_eq!(renamed("src/{a => b}/x.ts"), "src/b/x.ts");
        assert_eq!(renamed("old.md => new.md"), "new.md");
        assert_eq!(renamed("src/{ => b}/x.ts"), "src/b/x.ts");
    }

    #[test]
    fn bars_scale_to_the_largest() {
        assert_eq!(bar_of(1, 0, 1, 4), ("████".into(), String::new()));
        assert_eq!(bar_of(1, 1, 100, 4), ("█".into(), "░".into()));
        assert_eq!(bar_of(0, 0, 0, 4), (String::new(), String::new()));
    }

    #[test]
    fn a_failed_job_shows_the_end_of_its_log() {
        let trace = "2026-09-28T13:02:20.886478Z 01E+$ pnpm test\nsection_start:1:step_script\r\x1b[31mFAIL x.test.ts\x1b[0m\nok\nERROR: Job failed: exit code 1\nCleaning up project directory\n";
        assert_eq!(log_of(trace), vec!["$ pnpm test", "FAIL x.test.ts", "ok"]);
    }
}
