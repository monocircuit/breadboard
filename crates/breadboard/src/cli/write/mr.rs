//! `pnpm wt:mr [<issue>] [--update] [--dry-run] [--description-file <f>]`
//! (commands/mr.mjs): the draft merge request of this worktree, opened with
//! the first commits so a colleague who takes over sees what exists. Title
//! from the last commit, the description written for this branch: without
//! one the template is put down (its Tests section filled from the branch,
//! #302) and nothing is sent. `--update` sends the description again, the
//! same body, so a later one can't drop the `Closes #<n>` of the work item.

mod template;

use super::boxes::box_report;
use super::common::{Argv, Here, Outcome, Takes, help, number_of, plural, say, temp_dir};
use super::glab;
use breadboard_core::git::{git, issue_of_branch};
use regex::Regex;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::sync::LazyLock;

const USAGE: &str = "usage: pnpm wt:mr [<issue>] [--update] [--dry-run] [--description-file <f>]";
const ABOUT: &str = "pnpm wt:mr opens the draft merge request of this worktree: the title from the last commit,
the description from .claude/mr-description.md (put down from the template when there is none),
Closes #<issue>, the issue's labels and milestone. It installs the push guard (#184) too.

  <issue>                   the work item, when the branch name carries none (34-…)
  --update                  send the description of the open MR again
  --dry-run                 show what would be sent, send nothing
  --description-file <f>    the description from <f> instead";
const TAKES: Takes = Takes {
    valued: &["--description-file"],
    switches: &["--update", "--dry-run"],
    words: Some(1),
    usage: USAGE,
};

/// The label of an MR that needs no changelog entry.
const NO_CHANGELOG: &str = "no-changelog";

static CLOSES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Closes #(\d+)").unwrap());
static HAS_CLOSES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Closes #\d").unwrap());
static REFS_AT_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Refs #\d*$").unwrap());

/// The push guard (#184, push-guard.mjs `SHIM`), byte for byte: the hook
/// lives in the repo, so every worktree runs the guard of its own branch.
pub const SHIM: &str = r#"#!/bin/sh
# monocircuit push guard (#184), installed by pnpm wt:new and wt:mr.
top="$(git rev-parse --show-toplevel)"
for package in breadboard workflow; do
  hook="$top/packages/$package/src/git-hooks/pre-push.mjs"
  [ -f "$hook" ] && exec node "$hook" "$@"
done
exit 0
"#;
const MARKER: &str = "monocircuit push guard (#184)";

/// Puts the hook into the hooks folder all worktrees share; one that isn't
/// ours stays. The answer says what happened (push-guard.mjs `installPushGuard`).
pub fn install_push_guard(dir: &Path) -> String {
    let hooks = git(dir, &["rev-parse", "--path-format=absolute", "--git-path", "hooks"]);
    if !hooks.is_ok || hooks.out().is_empty() {
        return "no git repository".into();
    }
    let hooks = Path::new(hooks.out());
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

/// The body sent: the text as written, with `Closes #<n>` unless it has one.
fn body_of(written: &str, number: &str) -> String {
    if HAS_CLOSES.is_match(written) {
        return written.trim_end().to_string();
    }
    let text = REFS_AT_END.replace(written.trim_end(), "");
    format!("{}\n\nCloses #{number}", text.trim_end())
}

/// Whether `.changelog/` has a fragment of the issue (platform-changelog.mjs `fragmentFiles`, `issueOfName`).
fn has_fragment(top: &Path, number: &str) -> bool {
    let Ok(entries) = fs::read_dir(top.join(".changelog")) else {
        return false;
    };
    let number: Option<u64> = number.parse().ok();
    entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .any(|name| {
            let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
            name.ends_with(".md")
                && name != "README.md"
                && !digits.is_empty()
                && name[digits.len()..].starts_with('-')
                && digits.parse::<u64>().ok() == number
        })
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
    let top = here.dir();
    let branch = here.branch();
    if branch.is_empty() || branch == "develop" || branch == "main" {
        return Err(format!(
            "{} gets no merge request",
            if branch.is_empty() { "a detached HEAD" } else { &branch }
        ));
    }
    // Worktrees made before the guard existed get it here (#184).
    if !is_dry {
        let guard = install_push_guard(top);
        if guard != "installed" && guard != "updated" {
            say(&format!("! {guard}"));
        }
    }

    let is_update = argv.has("--update");
    // glab hands over the newest MR of the branch, merged or closed too (Node
    // said "open already" to both). A merged one takes nothing any more; a
    // closed one is history, the branch gets a new one.
    let newest = glab::view(top, "mr", &branch);
    let state = newest.as_ref().map(|mr| glab::text_of(mr, "state"));
    if let Some(mr) = newest.as_ref().filter(|_| state == Some("merged")) {
        return Err(format!(
            "!{} of {branch} is merged already: new work goes on a new branch, its own issue first (glab issue create), then pnpm wt:new <n>-<feature>\n  {}",
            glab::shown(mr.get("iid")),
            glab::text_of(mr, "web_url")
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
    if let Some(mr) = open
        && !is_dry
        && !is_update
    {
        let report = number
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| box_report(top, n, None))
            .unwrap_or_default();
        say(&format!(
            "\n✓ the merge request is open already\n  {}\n  {}\n\n  Its description grew out of date? pnpm wt:mr --update{report}\n",
            glab::text_of(mr, "title"),
            glab::text_of(mr, "web_url")
        ));
        return Ok(ExitCode::SUCCESS);
    }

    let Some(number) = number.filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) else {
        return Err("a merge request needs its work item: pnpm wt:mr <issue>, or a worktree named after the issue (34-…). No issue yet: glab issue create".into());
    };

    // The labels and the milestone of the work item are those of its merge
    // request. Labels fall back to the kind of the branch; the milestone does not.
    let Some(work) = glab::view(top, "issue", &number) else {
        return Err(format!(
            "issue #{number} does not exist — a merge request without a work item is none"
        ));
    };
    let mut labels = strings_of(work.get("labels"));
    let milestone = work
        .get("milestone")
        .and_then(|m| m.get("title"))
        .and_then(Value::as_str)
        .map(String::from);
    let boxes = box_report(top, &number, Some(glab::text_of(&work, "description")));
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
            "{} is still the template, {} unwritten:\n{}\n\n  Say per section what this branch changes and why, then run pnpm wt:mr again.",
            here.shown(&given),
            plural(unwritten.len(), "place"),
            unwritten.join("\n")
        ));
    }
    // A box that doesn't apply is deleted, not kept empty: an unticked box reads as open work (#302).
    let not_applying = template::inapplicable(&written);
    if !not_applying.is_empty() {
        let is_one = not_applying.len() == 1;
        return Err(format!(
            "{} keeps {} that {} apply:\n{}\n\n  Delete {}: Tests lists only what applies. Something that applies but didn't run goes into a sentence under the boxes. Then run pnpm wt:mr again.",
            here.shown(&given),
            if is_one {
                "a Tests box".to_string()
            } else {
                format!("{} Tests boxes", not_applying.len())
            },
            if is_one { "says it doesn't" } else { "say they don't" },
            not_applying.join("\n"),
            if is_one { "that line" } else { "those lines" }
        ));
    }
    let body = body_of(&written, &number);
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
            "\n▸ nothing was sent (--dry-run)\n{anew}{}  title     {title}\n  work item #{number}\n  labels    {}\n  milestone {}\n  body      {}\n\n{body}\n",
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
        if !glab::put_description(
            top,
            &format!("projects/:fullpath/merge_requests/{iid}"),
            &format!("{body}\n"),
        ) {
            return Err(format!("glab could not update the description of !{iid}"));
        }
        let back = glab::view(top, "mr", &iid);
        let is_there = back.is_some_and(|back| glab::text_of(&back, "description").trim() == body.trim());
        say(&format!(
            "\n{} description of !{iid} updated, closes #{number}{}\n  {}{boxes}\n",
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

    let description = format!("{body}\n");
    let label = labels.join(",");
    let mut args = vec![
        "mr",
        "create",
        "--draft",
        "--source-branch",
        &branch,
        "--target-branch",
        "develop",
        "--title",
        &title,
        "--description",
        &description,
        "--label",
        &label,
    ];
    if let Some(milestone) = &milestone {
        args.extend(["--milestone", milestone]);
    }
    args.extend(["--assignee", "@me", "--remove-source-branch", "--yes", "--no-editor"]);
    let created = glab::glab(top, &args);
    if !created.is_ok {
        return Err("glab could not create the merge request, is everything pushed? (git push)".into());
    }
    // glab prints the URL; one that didn't is asked for it (Node printed an empty line).
    let printed = created
        .stdout
        .split('\n')
        .map(str::trim)
        .find(|line| line.starts_with("http"))
        .map(String::from);
    let url = printed
        .or_else(|| {
            glab::view(top, "mr", &branch)
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
        "\n✓ draft merge request opened, closes #{number}, {carried}\n  {title}\n  {url}{}{anew}{boxes}\n",
        if anew.is_empty() { "" } else { "\n" }
    ));
    // check:changelog fails the MR until it has an entry: say so now, not at the pipeline.
    if !has_fragment(top, &number) {
        say(&format!(
            "  No changelog entry yet: pnpm changelog:add \"<what users notice>\" (or the label {NO_CHANGELOG})\n"
        ));
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::write::common::scratch::Scratch;

    #[test]
    fn closes_the_issue_unless_the_text_does() {
        assert_eq!(body_of("Text\n\nRefs #\n", "7"), "Text\n\nCloses #7");
        assert_eq!(body_of("Text\nRefs #12  \n", "7"), "Text\n\nCloses #7");
        assert_eq!(body_of("Text\nCloses #3\n\n", "7"), "Text\nCloses #3");
    }

    #[test]
    fn finds_the_changelog_fragment_of_the_issue() {
        let scratch = Scratch::new();
        let dir = scratch.path().join(".changelog");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("README.md"), "").unwrap();
        fs::write(dir.join("307-rust.md"), "").unwrap();
        assert!(has_fragment(scratch.path(), "307"));
        assert!(!has_fragment(scratch.path(), "30"));
    }

    #[test]
    fn installs_the_push_guard_once_and_leaves_a_foreign_hook() {
        let scratch = Scratch::new();
        let repo = scratch.path();
        assert_eq!(install_push_guard(repo), "no git repository");
        git(repo, &["init", "--quiet"]);
        assert_eq!(install_push_guard(repo), "installed");
        let hook = repo.join(".git/hooks/pre-push");
        assert_eq!(fs::read_to_string(&hook).unwrap(), SHIM);
        assert_eq!(install_push_guard(repo), "installed");
        fs::write(&hook, format!("{SHIM}# old\n")).unwrap();
        assert_eq!(install_push_guard(repo), "updated");
        fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
        assert!(install_push_guard(repo).ends_with("is someone else's hook, the push guard is not installed"));
    }
}
