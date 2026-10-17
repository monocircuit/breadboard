//! What a start of an orchestration sets right first (#380): its checkout
//! moved up to its base, the briefs of its worktrees naming the base each
//! really has. A brief written before #380 could name `main`: the base was
//! read from the config of a worktree not made yet.

use crate::cli::lifecycle::layout::orchestrations::follow_base;
use breadboard_core::git::{base_of, worktrees};
use breadboard_core::record::{self, brief_file, origin_of};
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

/// The line of a brief that names the base (handover/brief.rs).
static BASE_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Its base is `([^`]+)`").unwrap());

/// The checkout of `name` up to its base: a line when it moved, one with
/// why when it can't (it stays where it is), none when it is up to date.
pub fn follow_checkout(main: &Path, name: &str) -> Option<String> {
    let dir = breadboard_core::git::orchestrations_dir(main).join(name);
    if !dir.join(".git").exists() {
        return None;
    }
    let label = record::orchestrator_label(main, name);
    match follow_base(main, &dir) {
        Ok(true) => Some(format!("  checkout of {label} moved up to its base")),
        Ok(false) => None,
        Err(why) => {
            let at = breadboard_core::git::git(&dir, &["rev-parse", "--short", "HEAD"]);
            Some(format!("  ! the checkout of {label} stays at {}: {why}", at.out()))
        }
    }
}

/// `brief` with its base line naming `base`, None when it does already (or has none).
pub fn with_base(brief: &str, base: &str) -> Option<String> {
    let named = BASE_LINE.captures(brief)?.get(1)?.as_str();
    (named != base).then(|| BASE_LINE.replace(brief, format!("Its base is `{base}`")).into_owned())
}

/// The briefs of the worktrees of `name` set right where they name another
/// base than their worktree has; the features whose brief changed.
pub fn heal_briefs(main: &Path, name: &str) -> Vec<String> {
    let mut healed = Vec::new();
    for tree in worktrees(main).into_iter().skip(1) {
        if origin_of(&tree.dir).is_none_or(|origin| origin.name != name) {
            continue;
        }
        let file = brief_file(&tree.dir);
        let Ok(brief) = std::fs::read_to_string(&file) else {
            continue;
        };
        if let Some(fixed) = with_base(&brief, &base_of(&tree.dir).local)
            && std::fs::write(&file, fixed).is_ok()
        {
            healed.push(breadboard_core::git::feature_of(main, &tree.dir));
        }
    }
    healed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_the_base_line_right() {
        let brief = "# #1 x\n\nIts base is `main`: merge it in, its MR goes into it.\n";
        assert_eq!(
            with_base(brief, "develop").as_deref(),
            Some("# #1 x\n\nIts base is `develop`: merge it in, its MR goes into it.\n")
        );
        assert_eq!(with_base(brief, "main"), None);
        assert_eq!(with_base("# no base line\n", "develop"), None);
    }
}
