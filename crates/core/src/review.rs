//! The cheap review of an MR before its merge is offered (#374): once its
//! bar is full and its pipeline green, the live overview that acts for its
//! orchestration starts one `claude -p --model <[review] model>` in its
//! worktree. It reads only the diff against the MR's target (given in the
//! prompt, lockfiles left out) and the issue, with read-only tools, and
//! answers with its findings: `serious: …` sends the MR back to its agent
//! (the line, and its bar holds the merge), `small: …` is listed for the
//! user. Both go into an MR comment. One review per head; a tiny, docs-only
//! or huge diff is skipped, and none starts while the daily limit holds.
//!
//! The reviews of a worktree are in its `.claude/reviews.json`; each one
//! done is appended to `<main>/.claude/reviews.jsonl` too, which outlives
//! the worktree: the cost per review is measured from it.

use crate::config::Review as Config;
use crate::json;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where a review stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Running,
    Done,
    /// Not worth a review (tiny, docs only) or too large for a cheap one: `skipped` says why.
    Skipped,
}

/// How much a finding weighs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weight {
    /// A bug, a broken rule of AGENTS.md, a part of the issue not done: back to its agent.
    Serious,
    /// Worth a look, not worth holding the merge: the user's.
    Small,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub weight: Weight,
    pub text: String,
}

/// One review of an MR at a head.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub iid: i64,
    /// The commit it read: a new push is reviewed again.
    pub head: String,
    pub model: String,
    pub state: State,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    /// The diff's changed lines, lockfiles aside.
    #[serde(default)]
    pub lines: u64,
    /// Its tokens as #372 counts them: input, cache writes, output.
    #[serde(default)]
    pub tokens: u64,
    /// What claude said it cost.
    #[serde(default)]
    pub dollars: f64,
    /// When it started, ISO.
    pub at: String,
    /// The pid of a running review: one whose process is gone failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Its conversation: the daily cost counts its transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Why it ended without findings it could read (didn't start, no answer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
}

impl Review {
    pub fn serious(&self) -> usize {
        self.findings
            .iter()
            .filter(|found| found.weight == Weight::Serious)
            .count()
    }

    /// Whether it read the commit `sha` (GitLab's head, maybe short).
    pub fn is_at(&self, sha: &str) -> bool {
        !sha.is_empty() && (self.head.starts_with(sha) || sha.starts_with(&self.head))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Reviews {
    reviews: Vec<Review>,
}

/// How many reviews a worktree keeps, the newest.
const KEPT: usize = 20;

pub fn reviews_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("reviews.json")
}

/// The repo's log of every review done or skipped, one JSON line each.
pub fn log_file(main: &Path) -> PathBuf {
    main.join(".claude").join("reviews.jsonl")
}

/// Where a review's stream and answer go: `.claude/reviews/<head>/`.
pub fn evidence_dir(dir: &Path, head: &str) -> PathBuf {
    let short: String = head.chars().take(8).collect();
    dir.join(".claude").join("reviews").join(short)
}

/// The reviews of the worktree `dir`, oldest first.
pub fn reviews_of(dir: &Path) -> Vec<Review> {
    json::read(&reviews_file(dir))
        .and_then(|value| serde_json::from_value::<Reviews>(value).ok())
        .map(|reviews| reviews.reviews)
        .unwrap_or_default()
}

/// `review` written into `dir`, in place of the one of its head.
pub fn write_review(dir: &Path, review: &Review) -> std::io::Result<()> {
    let mut reviews: Vec<Review> = reviews_of(dir)
        .into_iter()
        .filter(|known| known.head != review.head)
        .collect();
    reviews.push(review.clone());
    let skip = reviews.len().saturating_sub(KEPT);
    let reviews = Reviews {
        reviews: reviews.into_iter().skip(skip).collect(),
    };
    json::write_pretty(&reviews_file(dir), &serde_json::to_value(reviews).unwrap_or_default())
}

/// A review that ended appended to the repo's log, with its issue.
pub fn log(main: &Path, issue: Option<i64>, review: &Review) -> std::io::Result<()> {
    use std::io::Write;
    let mut line = serde_json::to_value(review).unwrap_or_default();
    if let Some(object) = line.as_object_mut() {
        object.remove("pid");
        object.insert("issue".into(), issue.into());
    }
    let file = log_file(main);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = std::fs::OpenOptions::new().create(true).append(true).open(file)?;
    writeln!(out, "{line}")
}

/// The review of the commit `sha`, the latest.
pub fn review_at<'a>(reviews: &'a [Review], sha: &str) -> Option<&'a Review> {
    reviews.iter().rev().find(|review| review.is_at(sha))
}

/// What a review holds of a merge: the row's reason and who acts on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    pub text: String,
    pub who: &'static str,
}

/// The reason a review waits for: the row's, the actor starts the run on it.
pub const DUE: &str = "review due";

/// What the review of `sha` holds while `[review]` is on (`is_enabled`):
/// due while there is none, running, serious findings back to the agent.
/// None once it passed, was skipped or failed (the user reads it then).
pub fn hold_of(reviews: &[Review], sha: Option<&str>, is_enabled: bool) -> Option<Hold> {
    let sha = sha.filter(|sha| !sha.is_empty())?;
    let Some(review) = review_at(reviews, sha) else {
        return is_enabled.then(|| Hold {
            text: DUE.into(),
            who: "breadboard",
        });
    };
    match review.state {
        State::Running => Some(Hold {
            text: format!("{} reviews it", review.model),
            who: "breadboard",
        }),
        State::Done if review.serious() > 0 => Some(Hold {
            text: serious_text(review.serious()),
            who: "agent",
        }),
        _ => None,
    }
}

/// `review: 2 serious findings`.
pub fn serious_text(serious: usize) -> String {
    format!(
        "review: {serious} serious finding{}",
        if serious == 1 { "" } else { "s" }
    )
}

/// The MR row's word of its review at the head: `reviewed ✓`, `review: 2
/// findings`, with its tokens; None without one.
pub fn row_text(review: &Review) -> String {
    let tokens = if review.tokens > 0 {
        format!(" · {} tokens", crate::spend::tokens_text(review.tokens))
    } else {
        String::new()
    };
    match review.state {
        State::Running => format!("{} reviews it …", review.model),
        State::Skipped => format!(
            "review skipped: {}",
            review.skipped.as_deref().unwrap_or("not worth one")
        ),
        State::Done if review.failed.is_some() => {
            format!("review failed: {}", review.failed.as_deref().unwrap_or_default())
        }
        State::Done if review.findings.is_empty() => format!("reviewed ✓{tokens}"),
        State::Done => {
            let count = review.findings.len();
            let serious = review.serious();
            let weight = if serious > 0 {
                format!(" ({serious} serious)")
            } else {
                String::new()
            };
            format!(
                "review: {count} finding{}{weight}{tokens}",
                if count == 1 { "" } else { "s" }
            )
        }
    }
}

/// An MR's diff as a review reads it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Diff {
    pub files: Vec<String>,
    /// Lines added and removed.
    pub lines: u64,
    pub text: String,
}

/// The pathspecs of the whole tree but `exclude`.
fn pathspecs(exclude: &[String]) -> Vec<String> {
    let mut specs = vec!["--".to_string(), ".".to_string()];
    specs.extend(exclude.iter().map(|spec| format!(":(exclude){spec}")));
    specs
}

/// The diff of the worktree `dir` against `origin/<target>` from where they
/// parted (what the MR shows), `exclude` left out. None when git can't tell.
pub fn diff_of(dir: &Path, target: &str, exclude: &[String]) -> Option<Diff> {
    let range = format!("origin/{target}...HEAD");
    let specs = pathspecs(exclude);
    let with = |head: &[&str]| -> Vec<String> {
        head.iter()
            .map(|word| word.to_string())
            .chain(specs.iter().cloned())
            .collect()
    };
    let numstat = with(&["diff", "--numstat", &range]);
    let stat = crate::git::git(dir, &numstat.iter().map(String::as_str).collect::<Vec<_>>());
    if !stat.is_ok {
        return None;
    }
    let mut diff = Diff::default();
    for line in stat.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(added), Some(removed), Some(file)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        // A binary file says `-`: it counts as a file, not as lines.
        diff.lines += added.parse::<u64>().unwrap_or(0) + removed.parse::<u64>().unwrap_or(0);
        diff.files.push(file.to_string());
    }
    let patch = with(&["diff", "--unified=3", &range]);
    let text = crate::git::git(dir, &patch.iter().map(String::as_str).collect::<Vec<_>>());
    if !text.is_ok {
        return None;
    }
    diff.text = text.stdout;
    Some(diff)
}

/// Whether a file is documentation: no code to review.
pub fn is_doc(file: &str) -> bool {
    let lower = file.to_lowercase();
    [".md", ".mdx", ".txt", ".rst"].iter().any(|ext| lower.ends_with(ext))
        || lower.starts_with(".changelog/")
        || lower.starts_with(".changeset/")
}

/// Why `diff` gets no review under `config`, None: it is reviewed.
pub fn skip_of(diff: &Diff, config: &Config) -> Option<String> {
    if diff.files.is_empty() {
        return Some("no changes".into());
    }
    if diff.files.iter().all(|file| is_doc(file)) {
        return Some("docs only".into());
    }
    if diff.lines < config.min_lines {
        return Some(format!("{} lines, under {}", diff.lines, config.min_lines));
    }
    if diff.lines > config.max_lines {
        return Some(format!(
            "{} lines, too many for a cheap review (over {})",
            diff.lines, config.max_lines
        ));
    }
    None
}

/// The AGENTS.md files whose rules cover the changed `files`, below the
/// root's (claude has that one already): each folder's on the way down,
/// those that exist in `dir`, each once.
pub fn agents_files_of(dir: &Path, files: &[String]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for file in files {
        let mut folder = Path::new(file).parent();
        while let Some(at) = folder.filter(|at| !at.as_os_str().is_empty()) {
            let agents = at.join("AGENTS.md");
            let name = agents.display().to_string();
            if !found.contains(&name) && dir.join(&agents).is_file() {
                found.push(name);
            }
            folder = at.parent();
        }
    }
    found.sort();
    found
}

/// The issue as the review is told it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Issue {
    pub number: i64,
    pub title: String,
    pub description: String,
}

/// What the reviewer is told: the issue, the rules to hold the diff
/// against, the diff, and how it answers.
pub fn prompt_of(issue: Option<&Issue>, diff: &Diff, agents: &[String]) -> String {
    let issue = match issue {
        Some(issue) => format!(
            "The issue it closes, #{} {}:\n\n<issue>\n{}\n</issue>",
            issue.number,
            issue.title,
            issue.description.trim()
        ),
        None => "Its issue is not known: judge the diff on its own.".into(),
    };
    let rules = if agents.is_empty() {
        "The repo's rules are the AGENTS.md in your context.".to_string()
    } else {
        format!(
            "The repo's rules are the AGENTS.md in your context, and for the folders it changes: {}. Read one only when a change could break a rule of it.",
            agents
                .iter()
                .map(|file| format!("`{file}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    format!(
        "You review a merge request before it is merged, cheaply: read the diff below, not the repo. \
         Open a file of the worktree only when a hunk can't be judged without it. Change nothing.\n\n\
         {issue}\n\n{rules}\n\nThe diff ({lines} changed lines in {files} files):\n\n<diff>\n{diff}\n</diff>\n\n\
         Look for bugs, a rule of AGENTS.md broken, and a part of the issue the diff doesn't do. Not for style a linter checks, nor taste.\n\
         Answer with one line per finding and nothing else:\n\
         - `serious: <file>:<line> <what is wrong, one sentence>` for a bug, a broken rule or a missing part: it goes back to the agent;\n\
         - `small: <file>:<line> <what, one sentence>` for something worth a look that need not hold the merge;\n\
         - `no findings` when there is nothing.",
        lines = diff.lines,
        files = diff.files.len(),
        diff = diff.text.trim_end(),
    )
}

/// How a review is started: `claude -p <prompt>` on its model, a
/// conversation of its own (the daily cost reads its transcript), only
/// read-only tools and no MCP servers (their tools cost tokens on every
/// answer), its cost capped, its stream to read the answer from.
pub fn args_of(model: &str, session: &str, prompt: &str, max_dollars: f64) -> Vec<String> {
    vec![
        "-p".into(),
        prompt.to_string(),
        "--session-id".into(),
        session.into(),
        "--model".into(),
        model.into(),
        "--tools".into(),
        "Read,Grep,Glob".into(),
        "--strict-mcp-config".into(),
        "--max-budget-usd".into(),
        format!("{max_dollars}"),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
    ]
}

/// What claude said a run cost: its `result` event's `total_cost_usd`.
pub fn dollars_of(lines: &[String]) -> f64 {
    lines
        .iter()
        .rev()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| event.get("type").and_then(serde_json::Value::as_str) == Some("result"))
        .find_map(|event| event.get("total_cost_usd").and_then(serde_json::Value::as_f64))
        .unwrap_or(0.0)
}

/// The findings of an answer: its lines that start with `serious:` or
/// `small:`, markdown aside. None: it named neither nor `no findings`.
pub fn findings_of(text: &str) -> Option<Vec<Finding>> {
    let mut findings = Vec::new();
    let mut is_answered = false;
    for line in text.lines() {
        let line = line
            .trim()
            .trim_start_matches(['-', '*', '•', ' '])
            .trim_matches(|ch: char| ch == '`' || ch == '*')
            .trim();
        let lower = line.to_lowercase();
        let rest = |word: &str| line[word.len()..].trim_start_matches([':', ' ']).trim().to_string();
        if lower.starts_with("serious:") {
            findings.push(Finding {
                weight: Weight::Serious,
                text: rest("serious"),
            });
        } else if lower.starts_with("small:") {
            findings.push(Finding {
                weight: Weight::Small,
                text: rest("small"),
            });
        } else if lower.starts_with("no findings") {
            is_answered = true;
        }
    }
    (is_answered || !findings.is_empty()).then_some(findings)
}

/// The MR comment of a review: its findings, serious first.
pub fn comment_of(review: &Review) -> String {
    let mut body = format!(
        "**Review** by {} at {} ({} lines, {} tokens, {})\n\n",
        review.model,
        review.head.chars().take(8).collect::<String>(),
        review.lines,
        crate::spend::tokens_text(review.tokens),
        crate::spend::dollars_text(review.dollars),
    );
    if review.findings.is_empty() {
        body.push_str("No findings.");
        return body;
    }
    for weight in [Weight::Serious, Weight::Small] {
        let found: Vec<&Finding> = review.findings.iter().filter(|found| found.weight == weight).collect();
        if found.is_empty() {
            continue;
        }
        body.push_str(match weight {
            Weight::Serious => "Serious, back to the agent:\n",
            Weight::Small => "Small, for you:\n",
        });
        for finding in found {
            body.push_str(&format!("- {}\n", finding.text));
        }
        body.push('\n');
    }
    body.trim_end().to_string()
}

/// The line that sends serious findings back to the MR's agent.
pub fn agent_line(iid: i64, review: &Review) -> String {
    let serious: Vec<String> = review
        .findings
        .iter()
        .filter(|found| found.weight == Weight::Serious)
        .map(|found| format!("- {}", found.text))
        .collect();
    format!(
        "[review] {model} reviewed !{iid} at {head}: {count} (also in an MR comment):\n{list}\n\
         Fix them and push: the new head is reviewed again, the merge waits until then. \
         A finding you hold wrong: say why under the comment (`glab mr note {iid}`) and leave it to the user with `{run} wt:action look \"<the finding and why>\"`.",
        model = review.model,
        head = review.head.chars().take(8).collect::<String>(),
        count = serious_text(serious.len()).trim_start_matches("review: "),
        list = serious.join("\n"),
        run = crate::config::run_word(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review(state: State, findings: Vec<Finding>) -> Review {
        Review {
            iid: 7,
            head: "abcdef123456".into(),
            model: "haiku".into(),
            state,
            findings,
            skipped: None,
            lines: 120,
            tokens: 18_000,
            dollars: 0.03,
            at: String::new(),
            pid: None,
            session: None,
            failed: None,
        }
    }

    fn finding(weight: Weight, text: &str) -> Finding {
        Finding {
            weight,
            text: text.into(),
        }
    }

    #[test]
    fn reads_the_findings_of_an_answer() {
        let text = "Read the diff.\n- `serious: src/a.rs:12 the lock is never released`\n* small: README.md:3 a typo\nSerious: b.rs:1 x";
        assert_eq!(
            findings_of(text),
            Some(vec![
                finding(Weight::Serious, "src/a.rs:12 the lock is never released"),
                finding(Weight::Small, "README.md:3 a typo"),
                finding(Weight::Serious, "b.rs:1 x"),
            ])
        );
        assert_eq!(findings_of("**No findings.**"), Some(vec![]));
        assert_eq!(findings_of("It looks fine to me."), None, "no answer it can read");
    }

    #[test]
    fn skips_a_diff_not_worth_a_cheap_review() {
        let config = Config::default();
        let diff = |files: &[&str], lines| Diff {
            files: files.iter().map(|file| file.to_string()).collect(),
            lines,
            text: String::new(),
        };
        assert_eq!(
            skip_of(&diff(&["README.md", "docs/a.md"], 400), &config).as_deref(),
            Some("docs only")
        );
        assert_eq!(
            skip_of(&diff(&["a.rs"], 5), &config).as_deref(),
            Some("5 lines, under 20")
        );
        assert!(
            skip_of(&diff(&["a.rs"], 9000), &config)
                .unwrap()
                .starts_with("9000 lines, too many")
        );
        assert_eq!(skip_of(&diff(&["a.rs", "README.md"], 120), &config), None);
        assert_eq!(skip_of(&diff(&[], 0), &config).as_deref(), Some("no changes"));
        assert!(is_doc(".changelog/374.md") && !is_doc("src/doc.rs"));
    }

    #[test]
    fn holds_the_merge_until_its_head_is_reviewed() {
        let sha = Some("abcdef12");
        assert_eq!(hold_of(&[], sha, false), None, "off: nothing holds");
        assert_eq!(hold_of(&[], sha, true).map(|hold| hold.text), Some(DUE.to_string()));
        assert_eq!(hold_of(&[], None, true), None, "no head");
        let running = review(State::Running, vec![]);
        assert_eq!(
            hold_of(std::slice::from_ref(&running), sha, true),
            Some(Hold {
                text: "haiku reviews it".into(),
                who: "breadboard"
            })
        );
        let serious = review(
            State::Done,
            vec![finding(Weight::Serious, "x"), finding(Weight::Small, "y")],
        );
        assert_eq!(
            hold_of(std::slice::from_ref(&serious), sha, false),
            Some(Hold {
                text: "review: 1 serious finding".into(),
                who: "agent"
            }),
            "switched off later, a finding still holds its head"
        );
        assert_eq!(
            hold_of(std::slice::from_ref(&serious), Some("0123"), true).map(|hold| hold.text),
            Some(DUE.to_string()),
            "a new push is reviewed again"
        );
        let small = review(State::Done, vec![finding(Weight::Small, "y")]);
        assert_eq!(hold_of(&[small], sha, true), None, "small ones are the user's");
    }

    #[test]
    fn says_its_review_on_the_row() {
        assert_eq!(row_text(&review(State::Done, vec![])), "reviewed ✓ · 18 k tokens");
        assert_eq!(
            row_text(&review(
                State::Done,
                vec![finding(Weight::Small, "a"), finding(Weight::Serious, "b")]
            )),
            "review: 2 findings (1 serious) · 18 k tokens"
        );
        let skipped = Review {
            skipped: Some("docs only".into()),
            ..review(State::Skipped, vec![])
        };
        assert_eq!(row_text(&skipped), "review skipped: docs only");
        assert_eq!(row_text(&review(State::Running, vec![])), "haiku reviews it …");
    }

    #[test]
    fn keeps_one_review_per_head() {
        let dir = std::env::temp_dir().join(format!("reviews-{}", std::process::id()));
        let mut one = review(State::Running, vec![]);
        write_review(&dir, &one).unwrap();
        one.state = State::Done;
        write_review(&dir, &one).unwrap();
        let reviews = reviews_of(&dir);
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].state, State::Done);
        assert!(review_at(&reviews, "abcdef").is_some() && review_at(&reviews, "fff").is_none());
        log(&dir, Some(374), &one).unwrap();
        let logged = std::fs::read_to_string(log_file(&dir)).unwrap();
        assert!(logged.contains("\"issue\":374") && logged.contains("\"tokens\":18000"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tells_the_reviewer_only_the_diff_and_the_issue() {
        let diff = Diff {
            files: vec!["packages/x/src/a.rs".into()],
            lines: 42,
            text: "+fn a() {}\n".into(),
        };
        let issue = Issue {
            number: 374,
            title: "a review".into(),
            description: "## Goal\n\nreview it".into(),
        };
        let prompt = prompt_of(Some(&issue), &diff, &["packages/x/AGENTS.md".into()]);
        assert!(prompt.contains("#374 a review:\n\n<issue>\n## Goal\n\nreview it\n</issue>"));
        assert!(prompt.contains("`packages/x/AGENTS.md`"));
        assert!(prompt.contains("(42 changed lines in 1 files):\n\n<diff>\n+fn a() {}\n</diff>"));
        let args = args_of("haiku", "s-1", "go", 0.5);
        assert_eq!(args[..6], ["-p", "go", "--session-id", "s-1", "--model", "haiku"]);
        assert!(args.windows(2).any(|pair| pair == ["--tools", "Read,Grep,Glob"]));
        assert!(args.windows(2).any(|pair| pair == ["--max-budget-usd", "0.5"]));
    }

    #[test]
    fn finds_the_agents_files_of_the_changed_folders() {
        let dir = std::env::temp_dir().join(format!("review-agents-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("packages/x/src")).unwrap();
        std::fs::write(dir.join("packages/x/AGENTS.md"), "").unwrap();
        std::fs::write(dir.join("AGENTS.md"), "").unwrap();
        let files = [
            "packages/x/src/a.rs".to_string(),
            "packages/x/b.rs".into(),
            "c.rs".into(),
        ];
        assert_eq!(agents_files_of(&dir, &files), ["packages/x/AGENTS.md"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn writes_its_comment_and_its_line_to_the_agent() {
        let done = review(
            State::Done,
            vec![
                finding(Weight::Small, "a.md:1 typo"),
                finding(Weight::Serious, "a.rs:2 bug"),
            ],
        );
        let comment = comment_of(&done);
        assert!(comment.starts_with("**Review** by haiku at abcdef12 (120 lines, 18 k tokens, $0.03)"));
        assert!(comment.contains("Serious, back to the agent:\n- a.rs:2 bug\n\nSmall, for you:\n- a.md:1 typo"));
        let line = agent_line(7, &done);
        assert!(line.starts_with(
            "[review] haiku reviewed !7 at abcdef12: 1 serious finding (also in an MR comment):\n- a.rs:2 bug\n"
        ));
        assert!(!line.contains("typo"));
    }
}
