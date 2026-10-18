//! The rows of herdr's sidebar the workflow adds to a config (sidebar.mjs
//! `sidebarConfig`): the spaces show an orchestration's mark and the branch,
//! an agent its space, its repo and what it waits for. Written into
//! `.herdr/config.toml` and into the marked block of your own config; the
//! tokens are reported by the overview of the main space (`--herdr`).

use breadboard_core::herdr::colour_in;

/// The colour of an orchestration's slot: `$orch1`..`$orch3` (origins.mjs `ORIGIN_SLOTS`).
const ORIGIN_SLOTS: [(&str, &str); 3] = [("blue", "#89B4FA"), ("teal", "#94E2D5"), ("mauve", "#CBA6F7")];
const BRANCH: (&str, &str) = ("pink", "#FF79C6");
/// Whether the TOML text has the table `[name]`.
fn has_table(text: &str, name: &str) -> bool {
    let head = format!("[{name}]");
    text.split('\n').any(|line| line.starts_with(&head))
}

/// The sidebar rows for a config `base`, None when it has all of them.
/// Colours are the names of its `[theme.custom]`, else the fallbacks.
/// - spaces: herdr's rows with the orchestration tokens before the branch,
///   unless the base has `[ui.sidebar.spaces]` (TOML takes a table once).
/// - agents (claude): the space, the repo by the agent (what a worktree
///   waits for is in the overview, #409);
///   as `[ui.sidebar.agents] rows` without that table, else as
///   `rows_by_agent.claude`; not when the base has `rows_by_agent`.
pub fn sidebar_config(base: &str) -> Option<String> {
    let colour = |(name, fallback): (&str, &str)| colour_in(base, name, fallback);
    let mut parts: Vec<String> = Vec::new();
    if !has_table(base, "ui.sidebar.spaces") {
        let mut tokens: Vec<String> = ORIGIN_SLOTS
            .iter()
            .enumerate()
            .map(|(i, slot)| {
                format!(
                    "{{ token = \"$orch{}\", fg = \"{}\", bold = true }}",
                    i + 1,
                    colour(*slot)
                )
            })
            .collect();
        let pink = colour(BRANCH);
        tokens.push(format!("{{ token = \"branch\", fg = \"{pink}\" }}"));
        tokens.push(format!("{{ token = \"$wt_branch\", fg = \"{pink}\" }}"));
        parts.push(
            "# The spaces of an orchestration (pnpm wt:orchestrate) show `⇶ <title>` (its name without one) in its colour, written by pnpm herdr."
                .into(),
        );
        parts.push("[ui.sidebar.spaces]".into());
        parts.push(
            "# A worktree's branch is $wt_branch: herdr's branch stays empty for a linked worktree. Both in pink."
                .into(),
        );
        parts.push(format!(
            "rows = [[\"state_icon\", \"workspace\"], [{}, \"git_status\"]]",
            tokens.join(", ")
        ));
    }
    if !has_table(base, "ui.sidebar.agents.rows_by_agent") {
        let rows = "[[\"state_icon\", \"workspace\"], [{ token = \"$wt_repo\" }, \"agent\"]]";
        if !parts.is_empty() {
            parts.push(String::new());
        }
        parts.push(
            "# A claude agent: its space, then its repo (a worktree's) and the agent; what a worktree waits for is in the overview (#409)."
                .into(),
        );
        if has_table(base, "ui.sidebar.agents") {
            parts.push("[ui.sidebar.agents.rows_by_agent]".into());
            parts.push(format!("claude = {rows}"));
        } else {
            parts.push("[ui.sidebar.agents]".into());
            parts.push(format!("rows = {rows}"));
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT_ROWS: &str = "[[\"state_icon\", \"workspace\"], [{ token = \"$wt_repo\" }, \"agent\"]]";

    #[test]
    fn gives_a_config_without_sidebar_rows_both_tables() {
        let rows = sidebar_config("").unwrap();
        assert!(rows.lines().any(|line| line == "[ui.sidebar.spaces]"));
        assert!(rows.lines().any(|line| line == "[ui.sidebar.agents]"));
        assert!(rows.contains(&format!("\nrows = {AGENT_ROWS}")));
        let spaces = rows.lines().find(|line| line.contains("$orch1")).unwrap();
        assert_eq!(
            spaces,
            "rows = [[\"state_icon\", \"workspace\"], [{ token = \"$orch1\", fg = \"#89B4FA\", bold = true }, { token = \"$orch2\", fg = \"#94E2D5\", bold = true }, { token = \"$orch3\", fg = \"#CBA6F7\", bold = true }, { token = \"branch\", fg = \"#FF79C6\" }, { token = \"$wt_branch\", fg = \"#FF79C6\" }, \"git_status\"]]",
            "{spaces}"
        );
        assert!(!rows.contains("$wt_state"), "{rows}");
    }

    #[test]
    fn names_no_space_twice_on_an_agent_card() {
        let rows = sidebar_config("").unwrap();
        let agent = rows.lines().find(|line| line.contains("$wt_repo")).unwrap();
        assert_eq!(agent.matches("\"workspace\"").count(), 1);
        assert!(!agent.contains("\"tab\"") && !agent.contains("$wt_head") && !agent.contains("$wt_tree"));
    }

    #[test]
    fn takes_the_colours_of_theme_custom_else_its_own() {
        let base = "[theme.custom]\nblue = \"#112233\"\nred = \"#AA0000\"\n\n[ui]\npink = \"#000000\"";
        let rows = sidebar_config(base).unwrap();
        assert!(rows.contains("{ token = \"$orch1\", fg = \"#112233\", bold = true }"));
        assert!(rows.contains("{ token = \"$wt_branch\", fg = \"#FF79C6\" }"));
    }

    #[test]
    fn leaves_spaces_rows_of_the_config_alone() {
        let rows = sidebar_config("[ui.sidebar.spaces]\nrows = [[\"workspace\"]]\n").unwrap();
        assert!(!rows.contains("[ui.sidebar.spaces]"));
        assert!(rows.contains("[ui.sidebar.agents]"));
    }

    #[test]
    fn keeps_the_agent_rows_of_the_config_for_other_agents() {
        let base = "[ui.sidebar.agents]\nrows = [[\"state_icon\", \"tab\"], [\"workspace\", \"agent\"]]\n\n[ui.sidebar.spaces]\nrows = []";
        let rows = sidebar_config(base).unwrap();
        assert!(!rows.lines().any(|line| line == "[ui.sidebar.agents]"));
        assert!(rows.lines().any(|line| line == "[ui.sidebar.agents.rows_by_agent]"));
        assert!(rows.contains(&format!("\nclaude = {AGENT_ROWS}")));
    }

    #[test]
    fn is_none_when_the_config_has_all_of_it() {
        let base = "[ui.sidebar.agents]\nrows = []\n[ui.sidebar.agents.rows_by_agent]\nclaude = []\n[ui.sidebar.spaces]\nrows = []";
        assert_eq!(sidebar_config(base), None);
    }
}
