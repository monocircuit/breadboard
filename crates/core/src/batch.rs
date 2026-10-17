//! The rolling batch branch of a product (#411): `batch/<product>-<n>` off
//! develop, the base of every task worktree of the product. Task MRs merge
//! into it by themselves, each with its own merge commit; the batch goes to
//! develop when the user ships it (#414), and the next one opens at once.
//! Replaces the milestone-bound epic branch of an orchestration (#350).
//!
//! The product's current batch is kept in its file (`.claude/products/<product>.json`,
//! `batch`): the branch, its number and the develop commit it was opened
//! from, which tells an empty batch from a shipped one.

use crate::git::git;
use crate::json::{self, str_of};
use crate::record::product_file;
use serde_json::{Value, json};
use std::path::Path;

/// What every batch branch starts with.
pub const PREFIX: &str = "batch/";

/// A product's batch as its file keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub branch: String,
    pub number: i64,
    /// The develop commit it was opened from: a batch still there holds no task.
    pub from: Option<String>,
}

/// The batch branch `number` of `product`: `batch/breadboard-1`.
pub fn branch_of(product: &str, number: i64) -> String {
    format!("{PREFIX}{product}-{number}")
}

/// The product and number of a batch branch: `batch/breadboard-1` → (breadboard, 1).
pub fn parse(branch: &str) -> Option<(&str, i64)> {
    let rest = branch.strip_prefix(PREFIX)?;
    let (product, number) = rest.rsplit_once('-')?;
    let number: i64 = number.parse().ok().filter(|number| *number > 0)?;
    (!product.is_empty() && !product.contains('/')).then_some((product, number))
}

/// Whether `branch` is a batch branch.
pub fn is_batch(branch: &str) -> bool {
    parse(branch).is_some()
}

/// Whether an MR into `target` merges by itself once its bar is full: into
/// an epic still open (#350), or into a batch while breadboard.toml's
/// `[batch] auto_merge` is on. Into develop or main it is the user's clicks.
pub fn is_auto_target(main: &Path, target: &str) -> bool {
    crate::epic::is_epic(target) || (is_batch(target) && crate::config::of(main).config.batch.auto_merge)
}

/// The batch `product`'s file keeps, None before its first.
pub fn current_of(main: &Path, product: &str) -> Option<Batch> {
    let raw = json::read(&product_file(main, product))?;
    batch_of(raw.get("batch")?)
}

fn batch_of(value: &Value) -> Option<Batch> {
    let branch = str_of(value, "branch").filter(|branch| is_batch(branch))?;
    Some(Batch {
        branch: branch.to_string(),
        number: parse(branch)?.1,
        from: str_of(value, "from").map(String::from),
    })
}

/// `batch` kept as `product`'s current one, the file's other fields as they were.
pub fn write_current(main: &Path, product: &str, batch: &Batch) -> std::io::Result<()> {
    let file = product_file(main, product);
    let mut raw = json::read_object(&file);
    raw.insert(
        "batch".into(),
        json!({ "branch": batch.branch, "number": batch.number, "from": batch.from }),
    );
    json::write_pretty(&file, &Value::Object(raw))
}

fn remote_head(main: &Path, branch: &str) -> Option<String> {
    let out = git(
        main,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/remotes/origin/{branch}"),
        ],
    );
    Some(out.out().to_string()).filter(|sha| out.is_ok && !sha.is_empty())
}

/// Whether `batch` went into develop (`develop` the default branch), as
/// far as the clone fetched origin: its head holds more than it was opened
/// from and develop has it. One gone from origin (deleted at its merge)
/// counts as shipped too.
pub fn is_shipped(main: &Path, batch: &Batch, develop: &str) -> bool {
    let Some(head) = remote_head(main, &batch.branch) else {
        return true;
    };
    batch.from.as_deref() != Some(head.as_str())
        && git(
            main,
            &[
                "merge-base",
                "--is-ancestor",
                &head,
                &format!("refs/remotes/origin/{develop}"),
            ],
        )
        .is_ok
}

/// What [`ensure_current`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// The batch was open already.
    Kept(Batch),
    /// A new batch, made from develop and pushed: the product's first, or the
    /// next after `shipped`.
    Made { batch: Batch, shipped: Option<String> },
}

impl Opened {
    pub fn batch(&self) -> &Batch {
        match self {
            Opened::Kept(batch) | Opened::Made { batch, .. } => batch,
        }
    }
}

/// The open batch of `product`: the one its file keeps while it isn't
/// shipped, else the next, made from origin's `develop` and pushed (on a
/// dry run only named). Origin as the clone fetched it last: the caller
/// fetches. A batch another clone opened on origin already is taken over.
pub fn ensure_current(main: &Path, product: &str, develop: &str, is_dry: bool) -> Result<Opened, String> {
    let kept = current_of(main, product);
    let shipped = match &kept {
        Some(batch) if !is_shipped(main, batch, develop) => return Ok(Opened::Kept(batch.clone())),
        Some(batch) => Some(batch.branch.clone()),
        None => None,
    };
    let develop_ref = format!("refs/remotes/origin/{develop}");
    let develop_head = git(main, &["rev-parse", "--verify", "--quiet", &develop_ref]);
    if !develop_head.is_ok {
        return Err(format!("origin has no {develop} to open a batch from"));
    }
    let mut number = kept.as_ref().map_or(1, |batch| batch.number + 1);
    let batch = loop {
        let branch = branch_of(product, number);
        let Some(head) = remote_head(main, &branch) else {
            break Batch {
                branch,
                number,
                from: Some(develop_head.out().to_string()),
            };
        };
        // Opened by another clone: its fork point is what it was opened from.
        let from = git(main, &["merge-base", &head, &develop_ref]).out().to_string();
        let found = Batch {
            branch,
            number,
            from: Some(from).filter(|from| !from.is_empty()),
        };
        if !is_shipped(main, &found, develop) {
            if !is_dry {
                write_current(main, product, &found).map_err(|error| format!("{product}'s batch not kept: {error}"))?;
            }
            return Ok(Opened::Made { batch: found, shipped });
        }
        number += 1;
    };
    if is_dry {
        return Ok(Opened::Made { batch, shipped });
    }
    let pushed = git(
        main,
        &[
            "push",
            "--quiet",
            "origin",
            &format!("{develop_ref}:refs/heads/{}", batch.branch),
        ],
    );
    if !pushed.is_ok {
        return Err(format!("{} not pushed: {}", batch.branch, pushed.stderr.trim()));
    }
    let _ = git(main, &["fetch", "--quiet", "origin", &batch.branch]);
    write_current(main, product, &batch).map_err(|error| format!("{product}'s batch not kept: {error}"))?;
    Ok(Opened::Made { batch, shipped })
}

/// The batch a new task of `product` branches off when batching is on
/// (breadboard.toml's `[batch] batching`), as its file keeps it; None when
/// it is off or the product has none yet.
pub fn planned_of(main: &Path, product: &str) -> Option<Batch> {
    crate::config::of(main)
        .config
        .batch
        .batching
        .then(|| current_of(main, product))
        .flatten()
}

/// The branch an orchestration's head row and GitLab read are about: its
/// epic still open (#350), else its product's batch (#411); None on develop.
pub fn branch_of_record(main: &Path, record: &crate::record::Record) -> Option<String> {
    crate::epic::epic_of(&record.raw).or_else(|| {
        let product = record.product.as_deref()?;
        planned_of(main, product).map(|batch| batch.branch)
    })
}

/// The files `batch` conflicts with origin's `develop` in, as the clone
/// fetched both (a hotfix that went past it, #411): None when develop is in
/// the batch already or the two merge cleanly, or git can't tell.
pub fn conflicts_with_develop(main: &Path, batch: &str, develop: &str) -> Option<Vec<String>> {
    let (batch, develop) = (
        format!("refs/remotes/origin/{batch}"),
        format!("refs/remotes/origin/{develop}"),
    );
    if git(main, &["merge-base", "--is-ancestor", &develop, &batch]).is_ok {
        return None;
    }
    let out = git(
        main,
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            &batch,
            &develop,
        ],
    );
    // 1: conflicts, the tree's id first and the files after it; anything else is no conflict git names.
    (out.status == 1)
        .then(|| out.lines().skip(1).map(String::from).collect::<Vec<_>>())
        .filter(|files| !files.is_empty())
}

/// What the daemon last looked at of a product's batch, kept in the store
/// (`batch-<product>.json`): one look per pair of heads.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Looked {
    pub batch: String,
    pub develop: String,
}

pub fn looked_file(store: &Path, product: &str) -> std::path::PathBuf {
    store.join(format!("batch-{product}.json"))
}

pub fn read_looked(store: &Path, product: &str) -> Looked {
    std::fs::read_to_string(looked_file(store, product))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn write_looked(store: &Path, product: &str, looked: &Looked) {
    if let Ok(text) = serde_json::to_string(looked) {
        let _ = std::fs::write(looked_file(store, product), text + "\n");
    }
}

/// The heads of `batch` and `develop` on origin as the clone knows them.
pub fn heads_of(main: &Path, batch: &str, develop: &str) -> Looked {
    Looked {
        batch: remote_head(main, batch).unwrap_or_default(),
        develop: remote_head(main, develop).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::fixture::{repo, run};
    use crate::lock::scratch::Scratch;

    #[test]
    fn names_and_reads_a_batch_branch() {
        assert_eq!(branch_of("breadboard", 1), "batch/breadboard-1");
        assert_eq!(parse("batch/breadboard-12"), Some(("breadboard", 12)));
        assert_eq!(parse("batch/my-product-3"), Some(("my-product", 3)));
        assert_eq!(parse("batch/breadboard"), None);
        assert_eq!(parse("batch/breadboard-0"), None);
        assert_eq!(parse("batch/-1"), None);
        assert_eq!(parse("feature/batch/x-1"), None);
        assert!(is_batch("batch/vita-2") && !is_batch("epic/15-breadboard") && !is_batch("develop"));
    }

    #[test]
    fn the_first_batch_opens_from_develop_and_the_next_once_it_shipped() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        let develop = run(&main, &["rev-parse", "HEAD"]);

        let dry = ensure_current(&main, "breadboard", "develop", true).unwrap();
        assert_eq!(dry.batch().branch, "batch/breadboard-1");
        assert_eq!(current_of(&main, "breadboard"), None, "a dry run keeps nothing");

        let first = ensure_current(&main, "breadboard", "develop", false).unwrap();
        assert_eq!(
            first,
            Opened::Made {
                batch: Batch {
                    branch: "batch/breadboard-1".into(),
                    number: 1,
                    from: Some(develop.clone()),
                },
                shipped: None,
            }
        );
        assert_eq!(run(&main, &["rev-parse", "origin/batch/breadboard-1"]), develop);
        // Empty, it is no shipped batch though develop has its head.
        assert!(matches!(
            ensure_current(&main, "breadboard", "develop", false).unwrap(),
            Opened::Kept(batch) if batch.number == 1
        ));

        // A task merged into it, then the batch into develop: shipped, the next opens.
        run(
            &main,
            &["checkout", "--quiet", "-b", "work", "origin/batch/breadboard-1"],
        );
        std::fs::write(main.join("task.md"), "task\n").unwrap();
        run(&main, &["add", "task.md"]);
        run(&main, &["commit", "--quiet", "-m", "feat: a task"]);
        run(
            &main,
            &["push", "--quiet", "origin", "HEAD:refs/heads/batch/breadboard-1"],
        );
        run(&main, &["fetch", "--quiet", "origin"]);
        let batch = current_of(&main, "breadboard").unwrap();
        assert!(!is_shipped(&main, &batch, "develop"), "not in develop yet");
        run(&main, &["push", "--quiet", "origin", "HEAD:refs/heads/develop"]);
        run(&main, &["fetch", "--quiet", "origin"]);
        assert!(is_shipped(&main, &batch, "develop"));
        let next = ensure_current(&main, "breadboard", "develop", false).unwrap();
        assert_eq!(
            next,
            Opened::Made {
                batch: Batch {
                    branch: "batch/breadboard-2".into(),
                    number: 2,
                    from: Some(run(&main, &["rev-parse", "origin/develop"])),
                },
                shipped: Some("batch/breadboard-1".into()),
            }
        );
        // The product's other fields stay.
        assert_eq!(current_of(&main, "vita"), None);
    }

    #[test]
    fn a_hotfix_in_develop_conflicts_with_the_batch_or_not() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        ensure_current(&main, "vita", "develop", false).unwrap();
        let commit = |base: &str, to: &str, file: &str, text: &str| {
            run(&main, &["checkout", "--quiet", "--detach", base]);
            std::fs::write(main.join(file), text).unwrap();
            run(&main, &["add", file]);
            run(&main, &["commit", "--quiet", "-m", "feat: x"]);
            run(&main, &["push", "--quiet", "origin", &format!("HEAD:refs/heads/{to}")]);
            run(&main, &["fetch", "--quiet", "origin"]);
        };
        assert_eq!(conflicts_with_develop(&main, "batch/vita-1", "develop"), None);
        commit("origin/batch/vita-1", "batch/vita-1", "README.md", "the batch\n");
        commit("origin/develop", "develop", "other.md", "a hotfix\n");
        assert_eq!(conflicts_with_develop(&main, "batch/vita-1", "develop"), None, "clean");
        commit("origin/develop", "develop", "README.md", "a hotfix\n");
        assert_eq!(
            conflicts_with_develop(&main, "batch/vita-1", "develop"),
            Some(vec!["README.md".to_string()])
        );
    }

    #[test]
    fn a_batch_another_clone_opened_is_taken_over() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        run(&main, &["push", "--quiet", "origin", "HEAD:refs/heads/batch/vita-1"]);
        run(&main, &["fetch", "--quiet", "origin"]);
        let opened = ensure_current(&main, "vita", "develop", false).unwrap();
        assert_eq!(opened.batch().branch, "batch/vita-1");
        assert_eq!(current_of(&main, "vita").unwrap().branch, "batch/vita-1");
    }

    #[test]
    fn an_mr_into_a_batch_or_an_epic_merges_by_itself() {
        let scratch = Scratch::new();
        let main = repo(&scratch);
        assert!(is_auto_target(&main, "batch/breadboard-1"));
        assert!(is_auto_target(&main, "epic/15-breadboard"));
        assert!(!is_auto_target(&main, "develop"));
        std::fs::write(main.join("breadboard.toml"), "[batch]\nauto_merge = false\n").unwrap();
        assert!(!is_auto_target(&main, "batch/breadboard-1"), "switched off");
        assert!(planned_of(&main, "breadboard").is_none());
    }
}
