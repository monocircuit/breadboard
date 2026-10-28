//! `pnpm wt:mr [<issue>] [--update] [--dry-run] [--description-file <f>]`
//! (commands/mr.mjs): the draft merge request of this worktree, opened with
//! the first commits so a colleague who takes over sees what exists. Title
//! from the last commit, the description written for this branch: without
//! one the template is put down (its Tests section filled from the branch,
//! #302) and nothing is sent. `--update` sends the description again, the
//! same body, so a later one can't drop the `Closes #<n>` of the work item.
//! One MR handles exactly its task (#349): the issue of its branch, and the
//! others of a bundle (#321, `issues` of `.claude/origin`), each with its
//! `Closes`; a description that closes another is refused. It merges into
//! the worktree's base: develop, or the epic of its orchestration.
//! A fix that passed wt:check without a test (`--no-test "<reason>"`, #316)
//! gets its reason as a line of the description.

mod template;

use super::boxes::box_report;
use super::common::{Argv, Here, Outcome, Takes, help, number_of, plural, say, temp_dir};
use super::glab;
use breadboard_core::git::{base_of, git, issue_of_branch};
use breadboard_core::gitlab::issues_text;
use breadboard_core::record::{bundle_of, issues_in};
use regex::Regex;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::sync::LazyLock;

const USAGE: &str = "usage: pnpm wt:mr [<issue>] [--update] [--dry-run] [--description-file <f>]";
const ABOUT: &str = "pnpm wt:mr opens the draft merge request of this worktree: the title from the last commit,
the description from .claude/mr-description.md (put down from the template when there is none),
Closes #<issue> for every issue of the task, their labels and milestone, into the worktree's base
(develop or its epic). One merge request handles exactly its task: the issue of its branch, and
those wt:new --also bundled with it. It installs the push guard (#184) too.

  <issue>                   the work item, when the branch name carries none (34-…)
  --update                  send the description of the open MR again
  --dry-run                 show what would be sent, send nothing
  --description-file <f>    the description from <f> instead";
const TAKES: Takes = Takes {
    valued: &["--description-file"],
    switches: &["--update", "--dry-run"],
    words: Some(1),
    usage: USAGE,
    command: "wt:mr",
};

/// The label of an MR that needs no changelog entry.
const NO_CHANGELOG: &str = "no-changelog";

static CLOSES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Closes #(\d+)").unwrap());
static REFS_AT_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Refs #\d*$").unwrap());
/// GitLab's closing pattern: a keyword, then the issues it closes (`#1, #2 and #3`).
static CLOSING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:clos(?:e|es|ed|ing)|fix(?:|es|ed|ing)|resolv(?:e|es|ed|ing)|implement(?:s|ed|ing)?)\b:?\s+((?:[\w./-]*#\d+(?:\s*,\s*|\s+and\s+|,?\s+and\s+)?)+)").unwrap()
});
static ISSUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"#(\d+)").unwrap());

/// The issues a description closes when its MR is merged, as GitLab reads it.
pub(crate) fn closed_by(body: &str) -> Vec<u64> {
    let mut numbers: Vec<u64> = CLOSING
        .captures_iter(body)
        .flat_map(|found| {
            ISSUE
                .captures_iter(&found[1])
                .filter_map(|issue| issue[1].parse().ok())
                .collect::<Vec<_>>()
        })
        .collect();
    numbers.sort_unstable();
    numbers.dedup();
    numbers
}

/// Why `body` is no MR of the task of `numbers` (#349, #321), None when it is.
fn second_issue(body: &str, numbers: &[i64]) -> Option<String> {
    let others: Vec<i64> = closed_by(body)
        .into_iter()
        .map(|closed| closed as i64)
        .filter(|closed| !numbers.contains(closed))
        .collect();
    (!others.is_empty()).then(|| {
        format!(
            "the description closes {} besides {}: one merge request handles exactly its task. Another issue gets its own branch and MR ({run} wt:new <n>-<feature>, --also <m> for one that belongs with it); to mention one, write Refs #<n>",
            issues_text(&others), issues_text(numbers), run = breadboard_core::config::run_word())
    })
}

/// The push guard (#184, #298) wt:new and wt:mr put into the shared hooks
/// folder: the binary's shim (`packages/breadboard/hooks/pre-push`), which
/// runs the guard of the pushing branch: its own binary, the Node hook of a
/// branch from before the rewrite, else the main checkout's breadboard, and
/// refuses when there is none. Node's shim (`exec node …pre-push.mjs`,
/// `exit 0` without one) is replaced where found: same marker (rule 8).
pub const SHIM: &str = crate::herdr::hook::PRE_PUSH_SHIM;
/// What every version of the shim says of itself (before #342 it named the repo: `<repo> push guard (#184)`).
const MARKER: &str = "push guard (#184)";

/// Puts the hook into the hooks folder all worktrees share; one that isn't
/// ours stays. The answer says what happened (push-guard.mjs `installPushGuard`).
pub fn install_push_guard(dir: &Path) -> String {
    let hooks = git(dir, &["rev-parse", "--path-format=absolute", "--git-path", "hooks"]);
    if !hooks.is_ok || hooks.out().is_empty() {
        return "no git repository".into();
    }
    install_into(Path::new(hooks.out()))
}

/// The guard put into the hooks folder `hooks`.
fn install_into(hooks: &Path) -> String {
    let file = hooks.join("pre-push");
    let current = fs::read_to_string(&file).ok();
    if current.as_deref() == Some(SHIM) {
        return "installed".into();
    }
    if current.as_deref().is_some_and(|current| !current.contains(MARKER)) {
        return format!(
            "{} is someone else's hook, the push guard is not installed",
            file.display()
        );
    }
    let written = fs::create_dir_all(hooks)
        .and_then(|()| fs::write(&file, SHIM))
        .and_then(|()| {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&file, fs::Permissions::from_mode(0o755))
        });
    match (written, current) {
        (Err(error), _) => format!("{} could not be written: {error}", file.display()),
        (Ok(()), None) => "installed".into(),
        (Ok(()), Some(_)) => "updated".into(),
    }
}

/// The body sent: the text as written, with `Closes #<n>` for every issue
/// of the task it doesn't close yet.
fn body_of(written: &str, numbers: &[i64]) -> String {
    let closed = closed_by(written);
    let missing: Vec<String> = numbers
        .iter()
        .filter(|number| !closed.contains(&(**number as u64)))
        .map(|number| format!("Closes #{number}"))
        .collect();
    if missing.is_empty() {
        return written.trim_end().to_string();
    }
    let text = REFS_AT_END.replace(written.trim_end(), "");
    format!("{}\n\n{}", text.trim_end(), missing.join("\n"))
}

/// Whether the MR GitLab reads back holds the body sent (`⚠` otherwise: a
/// read that failed, or a description that differs, e.g. a path for the text).
fn is_description_back(back: Option<&Value>, body: &str) -> bool {
    back.is_some_and(|back| glab::text_of(back, "description").trim() == body.trim())
}

/// Whether `.changelog/` has a fragment of one of the issues (platform-changelog.mjs `fragmentFiles`, `issueOfName`).
fn has_fragment(top: &Path, numbers: &[i64]) -> bool {
    let Ok(entries) = fs::read_dir(top.join(".changelog")) else {
        return false;
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .any(|name| {
            let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
            name.ends_with(".md")
                && name != "README.md"
                && !digits.is_empty()
                && name[digits.len()..].starts_with('-')
                && digits.parse::<i64>().is_ok_and(|digits| numbers.contains(&digits))
        })
}

/// The reason the last passed wt:check of the worktree gave for a fix without a test (#316).
fn no_test_of_stamp(top: &Path) -> Option<String> {
    let stamp = breadboard_core::check::read_check(top);
    breadboard_core::json::str_of(stamp.get("passed")?, "no_test").map(String::from)
}

fn strings_of(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

pub fn run(argv: &Argv) -> Outcome {
    // Refused before the push guard or any glab call: nothing is written on a typo.
    if argv.check(&TAKES)? {
        return help(ABOUT, USAGE);
    }
    if let Some(given) = argv.first()
        && number_of(given).is_none()
    {
        return Err(format!("{given} is no issue number (34 or #34)\n{USAGE}"));
    }
    let is_dry = argv.has("--dry-run");
    let here = Here::find();
    let Some(top) = here.top.as_deref() else {
        return Err("not in a git checkout: run it in a worktree".into());
    };
    // A file named is the description: one that isn't there is refused before
    // GitLab is asked, and no template is put down elsewhere instead.
    if let Some(given) = argv.value("--description-file") {
        let path = here.resolve(given);
        if !path.is_file() {
            return Err(format!(
                "{} {} — write the description there first",
                here.shown(given),
                if path.exists() { "is no file" } else { "does not exist" }
            ));
        }
    }
    let branch = here.branch();
    let base = base_of(top);
    if branch.is_empty() || branch == "develop" || branch == "main" || branch == base.local {
        return Err(format!(
            "{} gets no merge request",
            if branch.is_empty() { "a detached HEAD" } else { &branch }
        ));
    }
    let is_update = argv.has("--update");
    // glab hands over the newest MR of the branch, merged or closed too (Node
    // said "open already" to both). A merged one takes nothing any more; a
    // closed one is history, the branch gets a new one. glab failing is no
    // "none": that opened a second MR (rule 6).
    let newest = glab::view(top, "mr", &branch)?;

    // `--update` sends the description of the open MR again; without one it
    // opened a new MR (Node too): a GitLab write nobody asked for (rule 6).
    // A merged one is refused below, with what to do instead.
    let has_one = newest
        .as_ref()
        .is_some_and(|mr| matches!(glab::text_of(mr, "state"), "opened" | "merged"));
    if is_update && !has_one {
        return Err(format!(
            "no open MR of {branch} to update: {run} wt:mr opens it",
            run = breadboard_core::config::run_word()
        ));
    }

    // Worktrees made before the guard existed get it here (#184).
    if !is_dry {
        let guard = install_push_guard(top);
        if guard != "installed" && guard != "updated" {
            say(&format!("! {guard}"));
        }
    }
    let state = newest.as_ref().map(|mr| glab::text_of(mr, "state"));
    if let Some(mr) = newest.as_ref().filter(|_| state == Some("merged")) {
        return Err(format!(
            "!{} of {branch} is merged already: new work goes on a new branch, its own issue first (glab issue create), then {run} wt:new <n>-<feature>\n  {}",
            glab::shown(mr.get("iid")),
            glab::text_of(mr, "web_url"),
            run = breadboard_core::config::run_word()
        ));
    }
    let closed = newest.as_ref().filter(|_| state != Some("opened"));
    let open = newest.as_ref().filter(|_| state == Some("opened"));
    let closes = newest.as_ref().and_then(|mr| {
        CLOSES
            .captures(glab::text_of(mr, "description"))
            .map(|found| found[1].to_string())
    });
    let number: Option<String> = argv
        .first()
        .map(|issue| issue.strip_prefix('#').unwrap_or(issue).to_string())
        .or_else(|| issue_of_branch(&branch).map(|n| n.to_string()))
        .or(closes);
    // The others of its task (#321), as wt:new --also wrote them.
    let numbers_of = |number: &str| bundle_of(number.parse().ok(), &issues_in(top));
    if let Some(mr) = open
        && !is_dry
        && !is_update
    {
        let report: String = number
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| {
                numbers_of(n)
                    .iter()
                    .map(|number| box_report(top, &number.to_string(), None))
                    .collect()
            })
            .unwrap_or_default();
        say(&format!(
            "\n✓ the merge request is open already\n  {}\n  {}\n\n  Its description grew out of date? {run} wt:mr --update{report}\n",
            glab::text_of(mr, "title"),
            glab::text_of(mr, "web_url"),
            run = breadboard_core::config::run_word()
        ));
        return Ok(ExitCode::SUCCESS);
    }

    let Some(number) = number.filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) else {
        return Err("a merge request needs its work item: pnpm wt:mr <issue>, or a worktree named after the issue (34-…). No issue yet: glab issue create".into());
    };
    // The branch is the claim on its issue: an MR of it is of that one (#349).
    if let Some(claimed) = issue_of_branch(&branch).filter(|claimed| claimed.to_string() != number) {
        return Err(format!(
            "{branch} claims #{claimed}, not #{number}: one merge request handles exactly one issue, the one its branch claims. #{number} gets its own branch and MR ({run} wt:new {number}-<feature>)",
            run = breadboard_core::config::run_word()
        ));
    }

    let numbers = numbers_of(&number);
    let closes = issues_text(&numbers);

    // The labels and the milestone of the work items are those of its merge
    // request: every issue's labels, the first milestone one has (an MR has
    // one). Labels fall back to the kind of the branch; the milestone does not.
    let mut works: Vec<Value> = Vec::new();
    for each in &numbers {
        let Some(work) = glab::view(top, "issue", &each.to_string())? else {
            return Err(format!(
                "issue #{each} does not exist — a merge request without a work item is none"
            ));
        };
        works.push(work);
    }
    let mut labels: Vec<String> = Vec::new();
    for label in works.iter().flat_map(|work| strings_of(work.get("labels"))) {
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    let milestone_of = works
        .iter()
        .filter_map(|work| work.get("milestone"))
        .find(|milestone| milestone.get("title").and_then(Value::as_str).is_some());
    let milestone = milestone_of
        .and_then(|m| m.get("title"))
        .and_then(Value::as_str)
        .map(String::from);
    let boxes: String = numbers
        .iter()
        .zip(&works)
        .map(|(each, work)| box_report(top, &each.to_string(), Some(glab::text_of(work, "description"))))
        .collect();
    if labels.is_empty() {
        labels.push(
            if branch.starts_with("fix/") {
                "bug"
            } else {
                "enhancement"
            }
            .into(),
        );
    }

    // The description is written by whoever opens the merge request, never taken from the template.
    let default_file = template::description_file(top);
    let given = argv
        .value("--description-file")
        .map(String::from)
        .unwrap_or_else(|| default_file.to_string_lossy().into());
    let path = here.resolve(&given);
    if !path.is_file() {
        return Err(template::to_fill(&here, top, &number));
    }
    let written = String::from_utf8_lossy(
        &fs::read(&path).map_err(|error| format!("{} could not be read: {error}", here.shown(&given)))?,
    )
    .into_owned();
    let unwritten = template::unfilled(&written);
    if !unwritten.is_empty() {
        return Err(format!(
            "{} is still the template, {} unwritten:\n{}\n\n  Say per section what this branch changes and why, then run {run} wt:mr again.",
            here.shown(&given),
            plural(unwritten.len(), "place"),
            unwritten.join("\n"),
            run = breadboard_core::config::run_word()
        ));
    }
    // A box that doesn't apply is deleted, not kept empty: an unticked box reads as open work (#302).
    let not_applying = template::inapplicable(&written);
    if !not_applying.is_empty() {
        let is_one = not_applying.len() == 1;
        return Err(format!(
            "{} keeps {} that {} apply:\n{}\n\n  Delete {}: Tests lists only what applies. Something that applies but didn't run goes into a sentence under the boxes. Then run {run} wt:mr again.",
            here.shown(&given),
            if is_one {
                "a Tests box".to_string()
            } else {
                format!("{} Tests boxes", not_applying.len())
            },
            if is_one { "says it doesn't" } else { "say they don't" },
            not_applying.join("\n"),
            if is_one { "that line" } else { "those lines" },
            run = breadboard_core::config::run_word()
        ));
    }
    // A fix that passed wt:check without a test says why, for the reviewer and the MR's CI job (#316).
    let written = match no_test_of_stamp(top) {
        Some(reason) => breadboard_core::check::with_no_test(&written, &reason),
        None => written,
    };
    let body = body_of(&written, &numbers);
    if let Some(why) = second_issue(&body, &numbers) {
        return Err(format!("{}: {why}", here.shown(&given)));
    }
    let title = git(top, &["log", "-1", "--format=%s"]).out().to_string();

    // The closed one stays closed: say that this is a new one, not it reopened.
    let anew = closed
        .map(|mr| {
            format!(
                "  !{} of {branch} is {}: a new merge request {}",
                glab::shown(mr.get("iid")),
                glab::text_of(mr, "state"),
                if is_dry {
                    "would be opened"
                } else {
                    "was opened, that one stays closed"
                }
            )
        })
        .unwrap_or_default();

    if is_dry {
        let dir = temp_dir("wt-mr-").map_err(|error| format!("no temp folder for the body: {error}"))?;
        let file = dir.join("description.md");
        fs::write(&file, format!("{body}\n"))
            .map_err(|error| format!("{} could not be written: {error}", file.display()))?;
        say(&format!(
            "\n▸ nothing was sent (--dry-run)\n{anew}{}  title     {title}\n  work item {closes}\n  labels    {}\n  milestone {}\n  body      {}\n\n{body}\n",
            if anew.is_empty() { "" } else { "\n" },
            labels.join(", "),
            milestone.as_deref().unwrap_or("none — the issue has none"),
            file.display()
        ));
        return Ok(ExitCode::SUCCESS);
    }

    // An MR that is open already only gets the description, the same body.
    if let Some(mr) = open {
        let iid = glab::shown(mr.get("iid"));
        let put = glab::put_description(
            top,
            &format!("projects/:fullpath/merge_requests/{iid}"),
            &format!("{body}\n"),
        );
        if !put.is_ok {
            return Err(format!(
                "glab could not update the description of !{iid}: {}",
                put.why()
            ));
        }
        let back = glab::view(top, "mr", &iid).ok().flatten();
        let is_there = is_description_back(back.as_ref(), &body);
        say(&format!(
            "\n{} description of !{iid} updated, closes {closes}{}\n  {}{boxes}\n",
            if is_there { "✓" } else { "⚠" },
            if is_there {
                ""
            } else {
                " — what GitLab shows back differs, look at it"
            },
            glab::text_of(mr, "web_url")
        ));
        return Ok(ExitCode::SUCCESS);
    }

    // POST merge_requests, the request `glab mr create --draft … --assignee
    // @me --remove-source-branch` makes, the description through a file: an
    // argument holds at most 128 KiB (`--description <text>` did), and
    // `--description-file` is missing in glab 1.114 (#157). `@me` is the
    // login's id, the milestone the issue's.
    let me = glab::api(top, "user");
    let Some(me) = me
        .json()
        .map(|user| glab::shown(user.get("id")))
        .filter(|id| !id.is_empty())
    else {
        return Err(format!(
            "glab could not tell who you are, the MR's assignee: {}",
            me.why()
        ));
    };
    let description = format!("{body}\n");
    let draft = format!("Draft: {title}");
    let label = labels.join(",");
    let milestone_id = milestone_of
        .map(|m| glab::shown(m.get("id")))
        .filter(|id| !id.is_empty());
    let mut fields = vec![
        glab::Field::Raw("source_branch", &branch),
        glab::Field::Raw("target_branch", &base.local),
        glab::Field::Raw("title", &draft),
        glab::Field::Text("description", &description),
        glab::Field::Raw("labels", &label),
        glab::Field::Typed("assignee_id", &me),
        glab::Field::Typed("remove_source_branch", "true"),
    ];
    if let Some(id) = &milestone_id {
        fields.push(glab::Field::Typed("milestone_id", id));
    }
    let created = glab::send(top, "POST", "projects/:fullpath/merge_requests", &fields);
    if !created.is_ok {
        // GitLab refuses a branch it doesn't have as invalid (400, 422).
        let is_unpushed = matches!(created.status(), Some(400 | 404 | 422));
        return Err(format!(
            "glab could not create the merge request: {}{}",
            created.why(),
            if is_unpushed {
                "\n  Is everything pushed? git push"
            } else {
                ""
            }
        ));
    }
    // GitLab answers the MR with its URL; an answer without is asked for it (Node printed an empty line).
    let printed = created
        .json()
        .map(|mr| glab::text_of(&mr, "web_url").to_string())
        .filter(|url| !url.is_empty());
    let url = printed
        .or_else(|| {
            glab::view(top, "mr", &branch)
                .ok()
                .flatten()
                .filter(|mr| glab::text_of(mr, "state") == "opened")
                .map(|mr| glab::text_of(&mr, "web_url").to_string())
                .filter(|url| !url.is_empty())
        })
        .unwrap_or_else(|| format!("glab printed no URL: glab mr view {branch} shows it"));
    let carried = format!(
        "labels: {}{}",
        labels.join(", "),
        milestone
            .as_deref()
            .map(|m| format!(", milestone: {m}"))
            .unwrap_or_default()
    );
    say(&format!(
        "\n✓ draft merge request opened, closes {closes}, {carried}\n  {title}\n  {url}{}{anew}{boxes}\n",
        if anew.is_empty() { "" } else { "\n" }
    ));
    // check:changelog fails the MR until it has an entry: say so now, not at the pipeline.
    if !has_fragment(top, &numbers) {
        say(&format!(
            "  No changelog entry yet: {run} changelog:add \"<what users notice>\" (or the label {NO_CHANGELOG})\n",
            run = breadboard_core::config::run_word()
        ));
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::write::common::scratch::Scratch;
    use serde_json::json;

    #[test]
    fn closes_the_issue_unless_the_text_does() {
        assert_eq!(body_of("Text\n\nRefs #\n", &[7]), "Text\n\nCloses #7");
        assert_eq!(body_of("Text\nRefs #12  \n", &[7]), "Text\n\nCloses #7");
        assert_eq!(body_of("Text\nCloses #7\n\n", &[7]), "Text\nCloses #7");
        // A bundle (#321): a Closes for every issue the text doesn't close yet.
        assert_eq!(body_of("Text\n", &[7, 9]), "Text\n\nCloses #7\nCloses #9");
        assert_eq!(body_of("Text\nCloses #9\n", &[7, 9]), "Text\nCloses #9\n\nCloses #7");
    }

    #[test]
    fn refuses_a_second_issue_in_one_merge_request() {
        assert_eq!(closed_by("x\n\nCloses #349"), vec![349]);
        assert_eq!(closed_by("Fixes #1, #2 and #3. Refs #4"), vec![1, 2, 3]);
        assert_eq!(closed_by("resolves monocircuit/platform#7"), vec![7]);
        assert_eq!(second_issue("x\n\nCloses #349", &[349]), None);
        assert_eq!(second_issue("Refs #350\n\nCloses #349", &[349]), None);
        assert!(
            second_issue("Closes #349, #350", &[349])
                .unwrap()
                .starts_with("the description closes #350 besides #349")
        );
        assert!(second_issue("Fixes #12\n\nCloses #349", &[349]).is_some());
        // A bundle (#321) closes each of its issues, no other.
        assert_eq!(second_issue("Closes #349, #350", &[349, 350]), None);
        assert!(
            second_issue("Closes #349, #350, #351", &[349, 350])
                .unwrap()
                .starts_with("the description closes #351 besides #349 and #350")
        );
    }

    #[test]
    fn says_updated_only_when_gitlab_shows_the_body_back() {
        let body = "Text\n\nCloses #7";
        assert!(is_description_back(
            Some(&json!({ "description": "Text\n\nCloses #7\n" })),
            body
        ));
        assert!(!is_description_back(
            Some(&json!({ "description": "/tmp/x/description.md" })),
            body
        ));
        assert!(!is_description_back(Some(&json!({ "title": "no description" })), body));
        assert!(!is_description_back(None, body));
    }

    #[test]
    fn finds_the_changelog_fragment_of_the_issue() {
        let scratch = Scratch::new();
        let dir = scratch.path().join(".changelog");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("README.md"), "").unwrap();
        fs::write(dir.join("307-rust.md"), "").unwrap();
        assert!(has_fragment(scratch.path(), &[307]));
        assert!(has_fragment(scratch.path(), &[321, 307]));
        assert!(!has_fragment(scratch.path(), &[30]));
    }

    // Into a hooks folder of its own: install_push_guard asks git, which in a
    // scratch folder inside a real repo would find that repo's hooks.
    #[test]
    fn installs_the_push_guard_once_and_leaves_a_foreign_hook() {
        let scratch = Scratch::new();
        let hooks = scratch.path().join("hooks");
        assert_eq!(install_into(&hooks), "installed");
        let hook = hooks.join("pre-push");
        assert_eq!(fs::read_to_string(&hook).unwrap(), SHIM);
        assert_eq!(install_into(&hooks), "installed");
        fs::write(&hook, format!("{SHIM}# old\n")).unwrap();
        assert_eq!(install_into(&hooks), "updated");
        // The shim from before #342 named the repo: still ours, updated.
        fs::write(&hook, "#!/bin/sh\n# platform push guard (#184)\nexit 1\n").unwrap();
        assert_eq!(install_into(&hooks), "updated");
        fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
        assert!(install_into(&hooks).ends_with("is someone else's hook, the push guard is not installed"));
        assert_eq!(fs::read_to_string(&hook).unwrap(), "#!/bin/sh\nexit 0\n");
    }
}
