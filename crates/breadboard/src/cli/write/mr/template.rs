//! The description of a merge request before it is sent (issues.mjs
//! `unfilled`, `templateToFill`; mr-tests.mjs): what is still template, the
//! Tests boxes that say they don't apply, and the template put down with its
//! Tests section filled from the branch (#302).

use super::super::boxes::{box_of, heading_of, is_fence};
use super::super::common::Here;
use breadboard_core::git::{git, tree_of};
use breadboard_core::json;
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// A change here needs vita's integration tests (`pnpm test:int`).
static SERVER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^products/vita/(?:[^/]+/)*src/server/").unwrap());
/// A change here is seen in the browser: vita's pages and components, its screens, monolithium.
static BROWSER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:products/vita/web/src/(?:app|components)/|products/vita/ui/src/|packages/monolithium/src/)")
        .unwrap()
});
/// A template box the pre-fill decides on: the package checks, test:int, the browser.
static PREFILLED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"pnpm --filter|wt:check|test:int|browser").unwrap());
/// What a box says when it doesn't apply: `: no browser change`, `n/a`. `\b` as JavaScript's, ASCII.
static NOT_APPLYING: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r"(?i)[:—–(]\s*(?:no|not|nothing)(?-u:\b).*(?-u:\b)(?:change[sd]?|touched|apply|applies|applicable|needed)(?-u:\b)")
            .unwrap(),
        Regex::new(r"(?i)(?-u:\b)(?:n/a|doesn't apply|does not apply|not applicable)(?-u:\b)").unwrap(),
    ]
});
static TESTS_HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^##\s+Tests(?-u:\b)").unwrap());
static SECTION_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:#{1,6}\s|Refs #|Closes #)").unwrap());
static EMPTY_BOX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[-*+]\s+\[ \]").unwrap());
static TESTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^tests(?-u:\b)").unwrap());

/// Where the description of a worktree lives: under its ignored `.claude/`,
/// not in a temp folder that is cleared under the session.
pub fn description_file(top: &Path) -> PathBuf {
    top.join(".claude").join("mr-description.md")
}

/// A line without its inline code (`/`[^`]*`/g`).
fn without_code(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(open) = rest.find('`') {
        match rest[open + 1..].find('`') {
            Some(close) => {
                out += &rest[..open];
                rest = &rest[open + 1 + close + 1..];
            }
            None => break,
        }
    }
    out + rest
}

/// `  <line padStart 4>  `
fn at(index: usize) -> String {
    format!("  {:>4}  ", index + 1)
}

/// The places nobody wrote: a bullet that stayed empty, a comment of the
/// template that stayed in. Boxes belong there, and code is text the author wrote.
pub fn unfilled(text: &str) -> Vec<String> {
    let mut left = Vec::new();
    let mut is_in_comment = false;
    let mut is_in_code = false;
    for (index, raw) in text.split('\n').enumerate() {
        if is_fence(raw) {
            is_in_code = !is_in_code;
            continue;
        }
        if is_in_code {
            continue;
        }
        let line = without_code(raw);
        if is_in_comment {
            if line.contains("-->") {
                is_in_comment = false;
            }
            continue;
        }
        if line.contains("<!--") {
            left.push(format!("{}{}", at(index), raw.trim()));
            if !line.contains("-->") {
                is_in_comment = true;
            }
        } else if matches!(line.trim(), "-" | "*") {
            left.push(format!("{}{} (empty bullet)", at(index), raw.trim()));
        }
    }
    left
}

/// The boxes left unticked under Tests whose text says they don't apply.
pub fn inapplicable(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut is_in_tests = false;
    for (index, raw) in text.split('\n').enumerate() {
        if let Some(heading) = heading_of(raw) {
            is_in_tests = TESTS.is_match(heading);
        }
        if !is_in_tests {
            continue;
        }
        let Some((' ', says)) = box_of(raw) else { continue };
        let says = without_code(says);
        if NOT_APPLYING.iter().any(|pattern| pattern.is_match(&says)) {
            found.push(format!("{}{}", at(index), raw.trim()));
        }
    }
    found
}

/// The template with its Tests section replaced by `lines` under a comment
/// that says what to do with them; its other boxes (the changelog) stay.
/// Nothing applies: an empty bullet, which wt:mr refuses until someone says why.
pub fn with_tests(text: &str, lines: &[String]) -> String {
    let all: Vec<&str> = text.split('\n').collect();
    let Some(from) = all.iter().position(|line| TESTS_HEADING.is_match(line)) else {
        return text.to_string();
    };
    let to = (from + 1..all.len())
        .find(|&at| SECTION_END.is_match(all[at]))
        .unwrap_or(all.len());
    let kept = all[from + 1..to]
        .iter()
        .filter(|line| EMPTY_BOX.is_match(line) && !PREFILLED.is_match(line));
    let mut boxes: Vec<&str> = lines.iter().map(String::as_str).chain(kept.copied()).collect();
    if boxes.is_empty() {
        boxes.push("-");
    }
    let mut section = vec![
        all[from],
        "",
        "<!-- Pre-filled from the branch's changes: only what applies. Tick what ran,",
        "     with its result; delete a line that doesn't apply. Something that",
        "     applies but didn't run goes into a sentence below, not an empty box. -->",
    ];
    section.extend(boxes);
    section.push("");
    all[..from]
        .iter()
        .copied()
        .chain(section)
        .chain(all[to..].iter().copied())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The files the branch changes against develop, committed or not, untracked included.
fn changed_files(top: &Path) -> Vec<String> {
    let base = git(top, &["merge-base", "origin/develop", "HEAD"]);
    let mut files = BTreeSet::new();
    if base.is_ok && !base.out().is_empty() {
        files.extend(git(top, &["diff", "--name-only", base.out()]).lines().map(String::from));
    }
    files.extend(
        git(top, &["ls-files", "--others", "--exclude-standard"])
            .lines()
            .map(String::from),
    );
    files.into_iter().filter(|file| !file.is_empty()).collect()
}

/// The packages of `files`: the name in the nearest `package.json` above each, the root's left out.
fn packages_of(files: &[String], name_at: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let mut names = BTreeSet::new();
    for file in files {
        let mut dir = file.rsplit_once('/').map(|(dir, _)| dir);
        while let Some(at) = dir.filter(|dir| !dir.is_empty()) {
            if let Some(name) = name_at(at) {
                names.insert(name);
                break;
            }
            dir = at.rsplit_once('/').map(|(dir, _)| dir);
        }
    }
    names.into_iter().collect()
}

/// The wt:check line (#298): ticked with its result when the last check that
/// passed tested HEAD's tree, else the check still to run on the changed packages.
fn check_line(passed: Option<(&Value, &str)>, packages: &[String]) -> Option<String> {
    if let Some((check, sha)) = passed {
        let names = json::strs_of(check, "packages");
        let what = match names.len() {
            0 => "no package changed".to_string(),
            count => format!(
                "lint / typecheck / test of {count} package{} ({})",
                if count == 1 { "" } else { "s" },
                names.join(", ")
            ),
        };
        let duration = check
            .get("duration")
            .filter(|d| json::is_truthy(Some(d)))
            .map(|d| match d {
                Value::String(text) => format!(", {text} s"),
                other => format!(", {other} s"),
            });
        let sha: String = sha.chars().take(8).collect();
        return Some(format!(
            "- [x] `pnpm wt:check` passed on {sha}: {what}{}",
            duration.unwrap_or_default()
        ));
    }
    (!packages.is_empty()).then(|| {
        format!(
            "- [ ] `pnpm wt:check`: lint / typecheck / test of {} and what depends on them",
            packages.join(", ")
        )
    })
}

/// The box lines of the Tests section for the branch in `top`.
fn test_lines_of(top: &Path) -> Vec<String> {
    let files = changed_files(top);
    let name_at = |dir: &str| {
        json::read(&top.join(dir).join("package.json"))
            .and_then(|package| json::str_of(&package, "name").map(String::from))
    };
    let packages = packages_of(&files, name_at);
    let record = json::read(&top.join(".claude").join("checked.json")).unwrap_or(Value::Null);
    let passed = record.get("passed").filter(|passed| {
        json::str_of(passed, "tree").is_some_and(|tree| Some(tree.to_string()) == tree_of(top, "HEAD"))
    });
    let sha = git(top, &["rev-parse", "HEAD"]).out().to_string();
    let mut lines: Vec<String> = check_line(passed.map(|passed| (passed, sha.as_str())), &packages)
        .into_iter()
        .collect();
    if files.iter().any(|file| SERVER.is_match(file)) {
        lines.push("- [ ] `pnpm test:int` (`src/server` changed)".into());
    }
    if files.iter().any(|file| BROWSER.is_match(file)) {
        lines.push("- [ ] clicked through in the browser".into());
    }
    lines
}

/// No description written yet: the template goes into the worktree, so the
/// answer to the failure is "fill this in", not "find the template".
/// Returns the failure's message.
pub fn to_fill(here: &Here, top: &Path, number: &str) -> String {
    let file = description_file(top);
    let shown = here.shown(&file.to_string_lossy());
    if !file.exists() {
        let source = top.join(".gitlab").join("merge_request_templates").join("Default.md");
        let Ok(template) = fs::read_to_string(&source) else {
            return format!(
                "{shown} doesn't exist, and neither does the template it starts from (.gitlab/merge_request_templates/Default.md): write the description into {shown}, then run pnpm wt:mr again."
            );
        };
        let filled = with_tests(&template, &test_lines_of(top));
        let filled = filled.trim_end();
        let filled = filled.strip_suffix("Refs #").unwrap_or(filled);
        let written = fs::create_dir_all(file.parent().unwrap_or(top))
            .and_then(|()| fs::write(&file, format!("{filled}Closes #{number}\n")));
        if let Err(error) = written {
            return format!("{shown} could not be written: {error}");
        }
    }
    format!(
        "a merge request needs a description written for this branch — the template is waiting in\n\n  {shown}\n\n  Fill every section (no empty bullets, no <!-- comments -->), then run pnpm wt:mr again."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_what_is_still_template() {
        let text = "# T\n<!-- say what -->\n-\n- [ ] a box\n`<!-- code -->` is fine\n```\n<!-- in code -->\n-\n```\n<!-- two\nlines -->\n  * \nwritten";
        assert_eq!(
            unfilled(text),
            [
                "     2  <!-- say what -->",
                "     3  - (empty bullet)",
                "    10  <!-- two",
                "    12  * (empty bullet)"
            ]
        );
        assert!(unfilled("# Done\n\nAll written.\n- [x] `pnpm wt:check` passed").is_empty());
    }

    #[test]
    fn finds_tests_boxes_that_say_they_do_not_apply() {
        let text = "## Changes\n- [ ] no browser change: not needed\n## Tests\n- [ ] clicked through in the browser: no browser change\n- [ ] `pnpm test:int` (n/a)\n- [x] wt:check: nothing changed\n- [ ] `no: change` in code only\n- [ ] the import works\n## Notes";
        assert_eq!(
            inapplicable(text),
            [
                "     4  - [ ] clicked through in the browser: no browser change",
                "     5  - [ ] `pnpm test:int` (n/a)"
            ]
        );
    }

    #[test]
    fn fills_the_tests_section_from_the_branch() {
        let template = "## Changes\n-\n\n## Tests\n\n<!-- what can apply -->\n- [ ] `pnpm wt:check` passed on <sha>\n- [ ] `pnpm test:int` (`src/server` changed)\n- [ ] clicked through in the browser\n- [ ] changelog entry in `.changelog/`\n\nRefs #";
        let filled = with_tests(
            template,
            &["- [ ] `pnpm wt:check`: lint / typecheck / test of a and what depends on them".into()],
        );
        assert_eq!(
            filled,
            "## Changes\n-\n\n## Tests\n\n<!-- Pre-filled from the branch's changes: only what applies. Tick what ran,\n     with its result; delete a line that doesn't apply. Something that\n     applies but didn't run goes into a sentence below, not an empty box. -->\n- [ ] `pnpm wt:check`: lint / typecheck / test of a and what depends on them\n- [ ] changelog entry in `.changelog/`\n\nRefs #"
        );
        assert!(with_tests("## Tests\n- [ ] browser\n", &[]).contains("box. -->\n-\n"));
        assert_eq!(with_tests("no tests", &[]), "no tests");
    }

    #[test]
    fn names_the_packages_of_the_changed_files() {
        let files = vec![
            "packages/a/src/x.ts".to_string(),
            "packages/a/y.ts".into(),
            "README.md".into(),
            "tools/b/c/d.ts".into(),
        ];
        let name_at = |dir: &str| match dir {
            "packages/a" => Some("@m/a".to_string()),
            "tools/b" => Some("b".to_string()),
            _ => None,
        };
        assert_eq!(packages_of(&files, name_at), ["@m/a", "b"]);
    }

    #[test]
    fn says_which_check_ran_or_is_to_run() {
        let passed = serde_json::json!({ "packages": ["a", "b"], "duration": 41, "tree": "t" });
        assert_eq!(
            check_line(Some((&passed, "0123456789abcdef")), &[]).unwrap(),
            "- [x] `pnpm wt:check` passed on 01234567: lint / typecheck / test of 2 packages (a, b), 41 s"
        );
        assert_eq!(
            check_line(Some((&serde_json::json!({}), "abc")), &[]).unwrap(),
            "- [x] `pnpm wt:check` passed on abc: no package changed"
        );
        assert_eq!(check_line(None, &[]), None);
        assert_eq!(
            check_line(None, &["a".into()]).unwrap(),
            "- [ ] `pnpm wt:check`: lint / typecheck / test of a and what depends on them"
        );
    }

    #[test]
    fn strips_inline_code() {
        assert_eq!(without_code("a `b` c `d"), "a  c `d");
    }
}
