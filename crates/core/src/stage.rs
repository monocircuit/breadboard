//! A model per kind of work (#373, Forge's `stage_providers`): every claude
//! breadboard starts runs on the model `[agents.models]` gives its stage. A
//! task agent's stage comes from its issue's labels when it starts (a
//! `[agents] docs_labels` one is `docs`, else `build`) and is kept in its
//! worktree (`.claude/stage`); a run whose every line is a failed pipeline
//! is `ci-fix`. The try checker is `review`, the orchestrator's chat
//! `orchestrator`.

use crate::config::Config;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A kind of work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Build,
    Docs,
    Review,
    CiFix,
    Orchestrator,
}

impl Stage {
    /// Its word in `.claude/stage` and in the docs.
    pub fn word(self) -> &'static str {
        match self {
            Stage::Build => "build",
            Stage::Docs => "docs",
            Stage::Review => "review",
            Stage::CiFix => "ci-fix",
            Stage::Orchestrator => "orchestrator",
        }
    }

    pub fn parse(word: &str) -> Option<Stage> {
        [
            Stage::Build,
            Stage::Docs,
            Stage::Review,
            Stage::CiFix,
            Stage::Orchestrator,
        ]
        .into_iter()
        .find(|stage| stage.word() == word.trim())
    }
}

/// The model of `stage`: its own of `[agents.models]`, else its `default`;
/// `review` falls back to `[tries] checker` instead (a small model unless
/// set). None: claude's own default, no `--model`.
pub fn model_of(config: &Config, stage: Stage) -> Option<String> {
    let models = &config.agents.models;
    let own = match stage {
        Stage::Build => &models.build,
        Stage::Docs => &models.docs,
        Stage::Review => return Some(models.review.clone().unwrap_or_else(|| config.tries.checker.clone())),
        Stage::CiFix => &models.ci_fix,
        Stage::Orchestrator => &models.orchestrator,
    };
    own.clone().or_else(|| models.default.clone())
}

/// `--model <model>` for claude's arguments, none without one.
pub fn model_args(config: &Config, stage: Stage) -> Vec<String> {
    model_of(config, stage).map_or_else(Vec::new, |model| vec!["--model".into(), model])
}

/// ` --model '<model>'` for a shell line of `stage`, "" without one.
pub fn model_flag(config: &Config, stage: Stage) -> String {
    model_of(config, stage).map_or_else(String::new, |model| {
        format!(" --model '{}'", model.replace('\'', "'\\''"))
    })
}

/// The `model_flag` of the orchestrator's chat, by the config of `dir`'s checkout.
pub fn orchestrator_model(dir: &Path) -> String {
    model_flag(&crate::config::of(dir).config, Stage::Orchestrator)
}

/// The stage of an issue by its labels: `docs` when one is a docs label.
pub fn of_labels(config: &Config, labels: &[String]) -> Stage {
    if labels.iter().any(|label| {
        config
            .agents
            .docs_labels
            .iter()
            .any(|docs| docs.eq_ignore_ascii_case(label))
    }) {
        Stage::Docs
    } else {
        Stage::Build
    }
}

/// Where a worktree keeps its agent's stage.
pub fn stage_file(dir: &Path) -> PathBuf {
    dir.join(".claude").join("stage")
}

pub fn write(dir: &Path, stage: Stage) -> io::Result<()> {
    let file = stage_file(dir);
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(file, format!("{}\n", stage.word()))
}

/// The stage a worktree's agent started with, `build` when none is kept.
pub fn of_worktree(dir: &Path) -> Stage {
    fs::read_to_string(stage_file(dir))
        .ok()
        .and_then(|text| Stage::parse(&text))
        .unwrap_or(Stage::Build)
}

/// Whether a line for an agent is its MR's failed pipeline (`auto_actions`' `pipeline_line`).
pub fn is_ci_line(line: &str) -> bool {
    let line = line.trim_start().trim_start_matches("[overview] ");
    line.starts_with("the pipeline of !") && line.contains(" failed")
}

/// The stage of a run in `dir` with `lines`: `ci-fix` when every line is a
/// failed pipeline, else the worktree's.
pub fn of_run<'a>(dir: &Path, lines: impl IntoIterator<Item = &'a str>) -> Stage {
    let mut lines = lines.into_iter().peekable();
    if lines.peek().is_some() && lines.all(is_ci_line) {
        Stage::CiFix
    } else {
        of_worktree(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> Config {
        Config::parse(text).expect("a valid config")
    }

    #[test]
    fn a_stage_takes_its_model_else_the_default_else_none() {
        let set = config("[agents.models]\ndefault = \"opus\"\ndocs = \"sonnet\"\nci_fix = \"haiku\"\n");
        assert_eq!(model_of(&set, Stage::Docs).as_deref(), Some("sonnet"));
        assert_eq!(model_of(&set, Stage::CiFix).as_deref(), Some("haiku"));
        assert_eq!(model_of(&set, Stage::Build).as_deref(), Some("opus"));
        assert_eq!(model_of(&set, Stage::Orchestrator).as_deref(), Some("opus"));
        assert_eq!(model_flag(&set, Stage::Docs), " --model 'sonnet'");
        assert_eq!(model_args(&set, Stage::Docs), ["--model", "sonnet"]);
        let empty = config("");
        assert_eq!(model_of(&empty, Stage::Build), None);
        assert_eq!(model_flag(&empty, Stage::Build), "");
        assert!(model_args(&empty, Stage::Build).is_empty());
    }

    #[test]
    fn review_falls_back_to_the_checker_not_the_default() {
        let set = config("[agents.models]\ndefault = \"opus\"\n");
        assert_eq!(model_of(&set, Stage::Review).as_deref(), Some("haiku"));
        let own = config("[agents.models]\nreview = \"sonnet\"\n");
        assert_eq!(model_of(&own, Stage::Review).as_deref(), Some("sonnet"));
    }

    #[test]
    fn a_docs_label_makes_a_docs_issue() {
        let built_in = config("");
        assert_eq!(of_labels(&built_in, &["Documentation".into()]), Stage::Docs);
        assert_eq!(of_labels(&built_in, &["bug".into()]), Stage::Build);
        assert_eq!(of_labels(&built_in, &[]), Stage::Build);
        let own = config("[agents]\ndocs_labels = [\"docs\"]\n");
        assert_eq!(of_labels(&own, &["docs".into()]), Stage::Docs);
        assert_eq!(of_labels(&own, &["documentation".into()]), Stage::Build);
    }

    #[test]
    fn a_run_of_failed_pipelines_only_is_a_ci_fix() {
        let dir = std::env::temp_dir().join(format!("bb-stage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let failed = "[overview] the pipeline of !70 failed: job check:vita, j2: look into it";
        assert_eq!(of_run(&dir, [failed]), Stage::CiFix);
        assert_eq!(
            of_run(&dir, [failed, "[overview] #3 try \"x\" → looks good"]),
            Stage::Build
        );
        assert_eq!(of_run(&dir, []), Stage::Build);
        write(&dir, Stage::Docs).unwrap();
        assert_eq!(of_worktree(&dir), Stage::Docs);
        assert_eq!(of_run(&dir, ["Read .claude/brief.md"]), Stage::Docs);
        assert_eq!(of_run(&dir, [failed]), Stage::CiFix);
        let _ = fs::remove_dir_all(&dir);
    }
}
