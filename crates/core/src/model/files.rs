//! Files in the view of a worktree that a click opens in its nvim tab (#257,
//! files.mjs): the files of DIFF, the conflicts of DEVELOP, a `file:line` in
//! the log of a failed job. At the change: the first line the worktree's work
//! changes in the file, or the line the log names.

use crate::git::git;
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

/// Where a file's name stands in a line, columns from 0, `to` past the end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSpan {
    pub from: usize,
    pub to: usize,
    pub file: String,
}

/// `prefix` and a few files by name, the rest as `+N more` (which stands for
/// the first one it hides), with where each name stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLine {
    pub prefix: String,
    /// The names shown, and the `+N more` (dim) when some are hidden.
    pub names: Vec<String>,
    pub more: Option<String>,
    pub suffix: String,
    pub spans: Vec<FileSpan>,
}

impl FileLine {
    /// The line as a person reads it.
    pub fn text(&self) -> String {
        let mut text = format!("{}{}", self.prefix, self.names.join(", "));
        if let Some(more) = &self.more {
            text.push(' ');
            text.push_str(more);
        }
        text.push_str(&self.suffix);
        text
    }
}

fn width(text: &str) -> usize {
    text.chars().count()
}

pub fn file_line(prefix: &str, files: &[String], count: usize, suffix: &str) -> FileLine {
    let mut at = width(prefix);
    let mut spans = Vec::new();
    let names: Vec<String> = files.iter().take(count).cloned().collect();
    for (i, file) in names.iter().enumerate() {
        if i > 0 {
            at += 2;
        }
        spans.push(FileSpan {
            from: at,
            to: at + width(file),
            file: file.clone(),
        });
        at += width(file);
    }
    let more = (files.len() > count).then(|| {
        let more = format!("+{} more", files.len() - count);
        at += 1;
        spans.push(FileSpan {
            from: at,
            to: at + width(&more),
            file: files[count].clone(),
        });
        more
    });
    FileLine {
        prefix: prefix.to_string(),
        names,
        more,
        suffix: suffix.to_string(),
        spans,
    }
}

/// The file of `spans` under the column of the line, the nearest one before it, else the first.
pub fn file_at(spans: &[FileSpan], at: usize) -> Option<&str> {
    spans
        .iter()
        .rev()
        .find(|span| span.from <= at)
        .or(spans.first())
        .map(|span| span.file.as_str())
}

static HUNK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@").unwrap());

/// The first line of a hunk in `git diff -U0`: where the change is. A hunk
/// that only removes lines names the line before the gap; 1 without a hunk.
pub fn first_change_in(diff: &str) -> u64 {
    let Some(hunk) = HUNK.captures(diff) else { return 1 };
    let line: u64 = hunk[1].parse().unwrap_or(1);
    if hunk.get(2).map(|count| count.as_str()) == Some("0") {
        line.max(1)
    } else {
        line
    }
}

/// The first line of `file` the work of `dir` changes against its base, 1 for a new file.
pub fn first_change(dir: &Path, file: &str) -> u64 {
    let Some(base) = crate::git::merge_base(dir) else {
        return 1;
    };
    first_change_in(&git(dir, &["diff", "-U0", &base, "--", file]).stdout)
}

/// A file and line a click on a line of a log opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogTarget {
    pub file: String,
    pub line: Option<u64>,
}

static LOG_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:^|[\s'"(])((?:\.{0,2}/)?[A-Za-z0-9_@.+-]+(?:/[A-Za-z0-9_@.+-]+)*\.[A-Za-z]{1,5})(?::(\d+)(?::\d+)?|\((\d+),\d+\))?"#).unwrap()
});
static ESLINT_PROBLEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s+(\d+):\d+\s+(?:error|warning)\b").unwrap());

static TURBO_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:@[\w.-]+/)?[\w.-]+(?::[A-Za-z][\w-]*)+:(?: |$)").unwrap());

/// The `<package>:<task>: ` turbo writes before each line of a task's
/// output (`@acme/web:lint: `), or nothing.
pub fn turbo_prefix(text: &str) -> &str {
    TURBO_PREFIX.find(text).map_or("", |found| found.as_str())
}

/// What a click on each line of a failed job's log opens: a path with a line,
/// or one with a folder; a line of ESLint's problems takes the file of the
/// line above that names one alone. Turbo's prefix doesn't count, so the
/// lines of `pnpm wt:check` read as ESLint wrote them.
pub fn log_targets(log: &[String]) -> Vec<Option<LogTarget>> {
    let mut eslint_file: Option<String> = None;
    log.iter()
        .map(|text| {
            let text = &text[turbo_prefix(text).len()..];
            if let Some(problem) = ESLINT_PROBLEM.captures(text)
                && let Some(file) = &eslint_file
            {
                return Some(LogTarget {
                    file: file.clone(),
                    line: problem[1].parse().ok(),
                });
            }
            for found in LOG_PATH.captures_iter(text) {
                let file = found[1].to_string();
                let line = found
                    .get(2)
                    .or(found.get(3))
                    .and_then(|line| line.as_str().parse::<u64>().ok())
                    .filter(|line| *line > 0);
                if line.is_none() && !file.contains('/') {
                    continue;
                }
                eslint_file = (line.is_none() && text.trim() == file).then(|| file.clone());
                return Some(LogTarget { file, line });
            }
            eslint_file = None;
            None
        })
        .collect()
}

/// The file of the checkout a log's path means, among `files` (git ls-files):
/// itself, one ending in it, or the end of it; the shortest of them.
pub fn match_file(files: &[String], logged: &str) -> Option<String> {
    let plain = logged
        .strip_prefix("./")
        .or_else(|| logged.strip_prefix('/'))
        .unwrap_or(logged);
    if files.iter().any(|file| file == plain) {
        return Some(plain.to_string());
    }
    files
        .iter()
        .filter(|file| file.ends_with(&format!("/{plain}")) || plain.ends_with(&format!("/{file}")))
        .min_by_key(|file| file.len())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn knows_where_each_file_of_a_line_stands() {
        let line = file_line("  ⚠ in: ", &strings(&["a.mjs", "b/c.mjs", "d", "e"]), 3, "");
        assert_eq!(line.text(), "  ⚠ in: a.mjs, b/c.mjs, d +1 more");
        let spans: Vec<(usize, usize, &str)> = line
            .spans
            .iter()
            .map(|span| (span.from, span.to, span.file.as_str()))
            .collect();
        assert_eq!(
            spans,
            [(8, 13, "a.mjs"), (15, 22, "b/c.mjs"), (24, 25, "d"), (26, 33, "e")]
        );
        assert_eq!(file_at(&line.spans, 9), Some("a.mjs"));
        assert_eq!(file_at(&line.spans, 13), Some("a.mjs"));
        assert_eq!(file_at(&line.spans, 16), Some("b/c.mjs"));
        assert_eq!(file_at(&line.spans, 30), Some("e"));
        assert_eq!(file_at(&line.spans, 0), Some("a.mjs"));
        assert_eq!(file_at(&[], 3), None);
    }

    #[test]
    fn finds_where_a_files_change_is() {
        assert_eq!(
            first_change_in("diff --git a/x b/x\n@@ -10,2 +12,3 @@ ctx\n+a\n@@ -40 +44 @@\n"),
            12
        );
        assert_eq!(first_change_in("@@ -5,2 +4,0 @@\n-a\n-b\n"), 4);
        assert_eq!(first_change_in("@@ -1,2 +0,0 @@\n"), 1);
        assert_eq!(first_change_in(""), 1);
    }

    #[test]
    fn finds_file_and_line_in_the_log_of_a_failed_job() {
        let target = |file: &str, line: Option<u64>| {
            Some(LogTarget {
                file: file.into(),
                line,
            })
        };
        assert_eq!(
            log_targets(&strings(&[" ❯ src/overview/files.test.mjs:12:5", "plain words"])),
            [target("src/overview/files.test.mjs", Some(12)), None]
        );
        assert_eq!(
            log_targets(&strings(&["src/app/page.tsx(31,7): error TS2322: Type …"])),
            [target("src/app/page.tsx", Some(31))]
        );
        let file = "/builds/monocircuit/platform/products/vita/web/src/a.ts";
        assert_eq!(
            log_targets(&strings(&[
                file,
                "  12:5  error  'x' is defined but never used  no-unused-vars",
                "  20:1  warning  Unexpected console  no-console",
                "✖ 2 problems (1 error, 1 warning)",
            ])),
            [target(file, None), target(file, Some(12)), target(file, Some(20)), None]
        );
        // Behind turbo's prefix, as `pnpm wt:check` logs it.
        assert_eq!(
            log_targets(&strings(&[
                "@monocircuit/breadboard:lint: ",
                "@monocircuit/breadboard:lint: work/304-overview.md",
                "@monocircuit/breadboard:lint:   1:5  error  'x' is defined but never used  no-unused-vars",
                "@monocircuit/vita-web:check:leaf-parity: src/a.ts:3:1",
            ])),
            [
                None,
                target("work/304-overview.md", None),
                target("work/304-overview.md", Some(1)),
                target("src/a.ts", Some(3)),
            ]
        );
        assert_eq!(turbo_prefix("page.test.tsx:12:7 failed"), "");
        assert_eq!(turbo_prefix("a.c:12: error"), "");
        assert_eq!(turbo_prefix("AssertionError: expected 1 to be 2"), "");
        assert_eq!(
            turbo_prefix("@monocircuit/breadboard:lint:"),
            "@monocircuit/breadboard:lint:"
        );
        assert_eq!(
            log_targets(&strings(&[
                "ERR_PNPM_RECURSIVE_RUN_FIRST_FAIL package.json",
                "see https://gitlab.com/monocircuit/platform/-/jobs/1.json"
            ])),
            [None, None]
        );
    }

    #[test]
    fn matches_a_logged_path_to_a_file_of_the_checkout() {
        let files = strings(&[
            "products/vita/web/src/a.ts",
            "packages/monolithium/src/a.ts",
            "products/vita/web/src/app/page.tsx",
            "package.json",
        ]);
        assert_eq!(match_file(&files, "./package.json").as_deref(), Some("package.json"));
        assert_eq!(
            match_file(&files, "src/app/page.tsx").as_deref(),
            Some("products/vita/web/src/app/page.tsx")
        );
        assert_eq!(
            match_file(&files, "/builds/monocircuit/platform/products/vita/web/src/a.ts").as_deref(),
            Some("products/vita/web/src/a.ts")
        );
        assert_eq!(match_file(&files, "src/gone.ts"), None);
    }
}
