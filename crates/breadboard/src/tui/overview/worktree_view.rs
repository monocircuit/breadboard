//! The view of one worktree (`--worktree <dir>`, render-worktree.mjs,
//! actions.mjs): where it is, WORK (its origin, issue, MR, tests, the delete
//! button), WAITING FOR, AGENT, ISSUE, PIPELINE (every job whole, #304),
//! DIFF, DEVELOP, then WHERE and STACK when it has an .env.local. Each
//! section on its own (#306); a click on a row opens what it names.

mod acts;
mod diff;
mod editor;
mod hunks;
mod started;

pub use acts::State;

use super::{Act, Ctx, Overview, look};
use crate::tui::theme::{self, Tone};
use crate::tui::{Flow, KeyEvent, Page, Scene, Section};
use breadboard_core::check::CheckState;
use breadboard_core::conversations::Claude;
use breadboard_core::gitlab::Job;
use breadboard_core::gitlab::read::OpenMr;
use breadboard_core::herdr;
use breadboard_core::model::Tree;
use breadboard_core::model::counted;
use breadboard_core::model::develop::{Develop, Migrations};
use breadboard_core::model::tree::{check_fact, mr_state, pipeline_mark, tokens};
use breadboard_core::model::worktree::{IssueBoxes, PipelineSection, WhereRow, WorktreeModel};
use breadboard_core::reader::Model;
use breadboard_core::time::ago;
use crossterm::event::KeyCode;
use ratatui::style::Style;
use ratatui::text::Span;
use std::path::Path;

/// The labels' column of the tables (WORK, AGENT, migrations).
const LABEL: u16 = 8;
/// How many merges DEVELOP names.
const MERGES: usize = 6;

/// A click in the view: what it starts is kept in the app's state of the view.
pub fn act(app: &mut Overview, act: Act) -> Flow {
    let Some(Model::Worktree(model)) = &app.model else {
        return Flow::Same;
    };
    let state = &mut app.worktree;
    state.saw(model);
    // The overview runs in the main checkout (mod.rs `main`).
    let main = std::env::current_dir().unwrap_or_default();
    match act {
        Act::Delete => acts::delete(state, model, &main),
        Act::StartStack(dir) => acts::start_stack(state, &dir),
        Act::Migrate(dir) => acts::migrate(state, &dir),
        Act::Storybook(_) => acts::start_storybook(state, model, &main),
        Act::OpenFile { file, line, .. } => acts::open_file(state, model, &main, &file, line),
        Act::Hunks { file, at } => acts::toggle_hunks(state, file, at),
        _ => Flow::Same,
    }
}

/// A key the frame has no use for: `u` starts the stack whose button shows,
/// Esc closes the hunks that show.
pub fn key(app: &mut Overview, key: KeyEvent) -> Flow {
    let Some(Model::Worktree(model)) = &app.model else {
        return Flow::Same;
    };
    match (key.code, acts::up_target(model)) {
        (KeyCode::Esc, _) => acts::close_hunks(&mut app.worktree),
        (KeyCode::Char('u'), Some(dir)) => acts::start_stack(&mut app.worktree, &dir),
        _ => Flow::Same,
    }
}

pub fn build(page: &mut Page<Act>, model: &WorktreeModel, ctx: &Ctx, state: &State) {
    let local = &model.local;
    let Some(tree) = &local.tree else {
        page.section(Section::new("GONE").plain(), |scene| {
            let name = local
                .dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            scene.atom(vec![look::bold(name)]);
            scene.atom_gap(
                vec![look::span("is not a worktree of the repo any more", Style::new())],
                2,
            );
        });
        return;
    };
    page.section(Section::new("DIR").plain(), |scene| {
        look::say(scene, &local.dir.display().to_string(), theme::dim());
        if let Some(why) = state.nvim.as_ref().filter(|_| !ctx.is_once) {
            scene.end_line();
            look::say(scene, &format!("nvim: {why}"), theme::tone(Tone::Yellow));
        }
    });
    page.section("WORK", |scene| work(scene, model, tree, ctx, state));
    if !model.waiting.is_empty() {
        page.section("WAITING FOR", |scene| {
            scene.note(format!(
                "merge in this order; next: !{}, then this branch goes on",
                model.waiting[0].mr.iid
            ));
            for (i, waiting) in model.waiting.iter().enumerate() {
                scene.block(Act::Open(waiting.mr.url.clone()), |scene| {
                    scene.atom(vec![look::dim(format!("{}.", i + 1))]);
                    open_mr(scene, &waiting.mr);
                    scene.end_line();
                });
            }
        });
    }
    if let Some(claude) = &local.claude {
        page.section("AGENT", |scene| {
            agent(scene, claude, local.agent_status.as_deref(), ctx)
        });
    }
    match &model.issue {
        Ok(Some(boxes)) => page.section("ISSUE", |scene| issue(scene, boxes)),
        Ok(None) => {}
        Err(why) => page.failed("ISSUE", why),
    }
    match &model.pipeline {
        Ok(Some(pipeline)) => {
            let mark = model
                .remote
                .as_ref()
                .and_then(|remote| remote.mr.as_ref())
                .map(|mr| pipeline_mark(mr.pipeline.as_ref()).text);
            page.section("PIPELINE", |scene| {
                if let Some(mark) = mark {
                    scene.note(mark);
                }
                jobs(scene, pipeline, &local.dir);
            });
        }
        Ok(None) => {}
        Err(why) => page.failed("PIPELINE", why),
    }
    if let Some(diff) = &local.diff {
        page.section("DIFF", |scene| {
            scene.note(diff::note(diff));
            diff::diff_of(scene, diff, &local.dir, state.hunks.as_ref());
        });
    }
    if let Some(develop) = local
        .develop
        .as_ref()
        .filter(|develop| !develop.merges.is_empty() || develop.direct > 0)
    {
        let place = diff::Files {
            dir: &local.dir,
            diff: local.diff.as_ref(),
            shown: state.hunks.as_ref(),
        };
        page.section("DEVELOP", |scene| {
            develop_of(scene, develop, model.project_url.as_deref(), &place)
        });
    }
    if !tree.has_env {
        page.section("STACK", |scene| {
            scene.text(
                "no .env.local: no stack, the app can't run here",
                theme::tone(Tone::Yellow),
            );
        });
        return;
    }
    match &model.where_rows {
        Ok(rows) => page.section("WHERE", |scene| where_rows(scene, rows, &local.dir, state)),
        Err(why) => page.failed("WHERE", why),
    }
    page.section("STACK", |scene| stack(scene, model, tree, ctx, state));
    page.section(Section::new("FOOTER").plain(), |scene| {
        scene.text(
            "pnpm stack:up|down|reset · pnpm wt:mr --update · pnpm overview (everything)",
            theme::dim(),
        );
    });
}

/// A row of a table: its label dim, the cell after it wrapping under itself.
fn row(scene: &mut Scene<Act>, label: &str, act: Option<Act>, cell: impl FnOnce(&mut Scene<Act>)) {
    let draw = |scene: &mut Scene<Act>| {
        scene.indent(0);
        scene.atom(vec![look::dim(label)]);
        scene.pad_to(LABEL);
        scene.indent(LABEL);
        cell(scene);
        scene.indent(0);
        scene.end_line();
    };
    match act {
        Some(act) => scene.block(act, draw),
        None => draw(scene),
    }
}

/// An open MR in a row: `!iid draft  … pending  title`.
fn open_mr(scene: &mut Scene<Act>, mr: &OpenMr) {
    let (state, style) = match mr.state.as_str() {
        "merged" => (" merged", theme::tone(Tone::Cyan)),
        "closed" => (" closed", theme::dim()),
        _ if mr.is_draft => (" draft", theme::dim()),
        _ => (" ready", theme::tone(Tone::Green)),
    };
    scene.atom_gap(
        vec![
            look::span(format!("!{}", mr.iid), Style::new()),
            look::span(state, style),
        ],
        2,
    );
    scene.atom_gap(look::spans(&[pipeline_mark(mr.pipeline.as_ref())]), 2);
    scene.indent(scene.x() + 2);
    say_gap(scene, &mr.title, Style::new(), 2);
    scene.indent(0);
}

fn work(scene: &mut Scene<Act>, model: &WorktreeModel, tree: &Tree, ctx: &Ctx, state: &State) {
    let local = &model.local;
    if local.is_main {
        row(scene, "work", None, |scene| {
            if tree.ahead == 0 && tree.changed == 0 {
                scene.text("clean, nothing ahead of develop", theme::dim());
                return;
            }
            let mut parts = vec![];
            if tree.ahead > 0 {
                parts.push(vec![look::span(
                    format!("{} ahead of develop", tree.ahead),
                    Style::new(),
                )]);
            }
            if tree.changed > 0 {
                parts.push(vec![look::span(
                    format!("{} changed", tree.changed),
                    theme::tone(Tone::Yellow),
                )]);
            }
            look::parted(scene, &parts);
        });
    }
    if let Some(error) = &ctx.gitlab.error {
        look::say(scene, error, theme::tone(Tone::Red));
        scene.end_line();
    }
    if let Some(origin) = &tree.origin {
        row(scene, "origin", None, |scene| {
            scene.atom(vec![look::span(format!("⇶ {}", origin.name), theme::tone(Tone::Cyan))]);
            look::say(
                scene,
                "the orchestrating session, its agent reports there",
                theme::dim(),
            );
        });
    }
    match &model.remote {
        None if ctx.gitlab.error.is_none() => {
            scene.pad_to(1);
            look::say(scene, &ctx.gitlab.progress, theme::dim());
            scene.end_line();
        }
        None => {}
        Some(remote) => {
            if let Some(issue) = &remote.issue {
                row(scene, "issue", Some(Act::Open(issue.url.clone())), |scene| {
                    scene.atom(vec![look::span(format!("#{}", issue.iid), Style::new())]);
                    look::say(scene, &issue.title, Style::new());
                    if issue.state == "closed" {
                        scene.text("(closed)", theme::dim());
                    }
                });
            }
            match &remote.mr {
                Some(mr) => row(scene, "mr", Some(Act::Open(mr.url.clone())), |scene| {
                    let mut head = vec![look::span(format!("!{}", mr.iid), Style::new())];
                    head.extend(look::spans(&[mr_state(mr)]));
                    scene.atom(head);
                    scene.atom(look::spans(&[pipeline_mark(mr.pipeline.as_ref())]));
                    scene.atom_gap(vec![look::dim(&mr.url)], 2);
                }),
                None => row(scene, "mr", None, |scene| {
                    let text = if local.is_main { "—" } else { "none yet: pnpm wt:mr" };
                    scene.text(text, theme::dim());
                }),
            }
        }
    }
    if let Some(fact) = check_fact(&tree.checked) {
        row(scene, "tests", None, |scene| {
            look::flow(scene, &look::spans(&[fact]));
            match &tree.checked {
                CheckState::Failed { .. } | CheckState::Untested { .. } => {
                    scene.atom_gap(vec![look::dim("pnpm wt:check before the push")], 2);
                }
                CheckState::Tested { packages, .. } => {
                    let packages = if packages.is_empty() {
                        "no package changed".to_string()
                    } else {
                        packages.join(", ")
                    };
                    look::say(scene, &format!("· {packages}"), theme::dim());
                }
                _ => {}
            }
        });
    }
    delete(scene, model, tree, ctx, state);
}

/// The delete button: red once nothing would be lost, grey with why not
/// yet; the first click arms it for 5 s, the second deletes (actions.mjs).
fn delete(scene: &mut Scene<Act>, model: &WorktreeModel, tree: &Tree, ctx: &Ctx, state: &State) {
    let local = &model.local;
    if local.is_main || tree.branch == "(detached)" {
        return;
    }
    let removal = state.removal.as_ref();
    if let Some(removal) = removal.filter(|removal| removal.is_running()) {
        scene.block(Act::Delete, |scene| {
            scene.button("◐ deleting …", Tone::Yellow, Act::Delete);
            look::say(scene, &removal.last_line(), theme::dim());
        });
        scene.redraw_in(acts::WATCH);
        return;
    }
    if local.is_done {
        scene.block(Act::Delete, |scene| {
            if let Some(left) = state.armed_for() {
                scene.button(
                    &format!("✕ click again to delete {}", tree.name),
                    Tone::Red,
                    Act::Delete,
                );
                look::say(scene, "worktree, branch and the data of its stack", theme::dim());
                scene.redraw_in(left);
                return;
            }
            scene.button("✕ delete worktree", Tone::Red, Act::Delete);
            let hint = if ctx.is_once {
                format!("pnpm wt:remove {}", tree.name)
            } else {
                "nothing left in it: no changes, all of it in develop".into()
            };
            look::say(scene, &hint, theme::dim());
            if let Some(error) = removal.and_then(|removal| removal.error("wt:remove")) {
                say_gap(scene, &error, theme::tone(Tone::Red), 2);
            }
        });
        return;
    }
    scene.inactive("✕ delete worktree");
    let is_merged = model
        .remote
        .as_ref()
        .and_then(|remote| remote.mr.as_ref())
        .is_some_and(|mr| mr.is_merged());
    let why = if is_merged && local.own_commits > 0 {
        "merged on GitLab, but develop here is older: git fetch".to_string()
    } else {
        let mut parts = vec![];
        if tree.changed > 0 {
            parts.push(counted(tree.changed as u64, "uncommitted file"));
        }
        if local.own_commits > 0 {
            parts.push(format!(
                "{} not in develop",
                counted(local.own_commits as u64, "commit")
            ));
        }
        format!("not yet: {}", parts.join(", "))
    };
    look::say(scene, &why, theme::dim());
    scene.end_line();
}

/// herdr's state of the agent, as the AGENT title says it.
fn agent_state(status: Option<&str>) -> Option<&'static str> {
    match status? {
        "working" => Some("◐ at work"),
        "blocked" => Some("● asks you something"),
        "idle" => Some("waiting for you"),
        "done" => Some("✓ done, waiting for you"),
        _ => None,
    }
}

/// A path with the home as `~`.
fn home(path: &Path) -> String {
    let text = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && text.starts_with(&home) => format!("~{}", &text[home.len()..]),
        _ => text,
    }
}

fn agent(scene: &mut Scene<Act>, claude: &Claude, status: Option<&str>, ctx: &Ctx) {
    if let Some(state) = agent_state(status) {
        scene.note(state);
    }
    let id8: String = claude.id.chars().take(8).collect();
    row(scene, "context", None, |scene| {
        let Some(at) = claude.at else {
            scene.text(&format!("{id8} · no answer yet"), theme::dim());
            return;
        };
        let share = claude.share();
        let tone = if share >= 0.8 {
            Tone::Red
        } else if share >= 0.5 {
            Tone::Yellow
        } else {
            Tone::Green
        };
        let filled = ((share * 20.0).round() as usize).min(20);
        scene.atom(vec![
            look::span("█".repeat(filled), theme::tone(tone)),
            look::dim("░".repeat(20 - filled)),
        ]);
        scene.atom(vec![look::span(
            format!("{}%", (share * 100.0).round() as i64),
            theme::tone(tone),
        )]);
        look::say(
            scene,
            &format!("{} of {}", tokens(claude.used), tokens(claude.window)),
            Style::new(),
        );
        let model = claude.model.as_deref().unwrap_or("").trim_start_matches("claude-");
        look::say(scene, &format!("{model} · {} · {id8}", ago(at, ctx.now)), theme::dim());
        look::ticks(scene);
    });
    row(scene, "resume", None, |scene| {
        let cwd = home(&claude.cwd);
        look::say(
            scene,
            &format!("cd {cwd} && claude --resume {}", claude.id),
            theme::dim(),
        );
    });
    row(scene, "stored", None, |scene| {
        let stored = claude
            .transcript
            .canonicalize()
            .unwrap_or_else(|_| claude.transcript.clone());
        look::say(scene, &home(&stored), theme::dim());
    });
    for (label, text) in [("asked", &claude.prompt), ("said", &claude.answer)] {
        if let Some(line) = text
            .as_deref()
            .and_then(|text| text.lines().find(|line| !line.trim().is_empty()))
        {
            row(scene, label, None, |scene| look::say(scene, line, Style::new()));
        }
    }
}

fn issue(scene: &mut Scene<Act>, boxes: &IssueBoxes) {
    scene.note(format!("#{}  {}/{} done", boxes.iid, boxes.done, boxes.total));
    for group in &boxes.groups {
        scene.atom(vec![look::bold(group.heading.as_deref().unwrap_or("—"))]);
        let count = format!("{}/{}", group.done, group.total);
        if group.done == group.total {
            scene.atom_gap(vec![look::span(format!("{count} ✓"), theme::tone(Tone::Green))], 2);
        } else {
            scene.atom_gap(vec![look::span(count, Style::new())], 2);
        }
        scene.end_line();
        // `○ text` two columns in, the text wrapping under itself (actions.mjs `pushIssue`).
        for open in &group.open {
            scene.pad_to(2);
            scene.atom_gap(vec![look::dim("○")], 0);
            scene.indent(4);
            look::say(scene, open, Style::new());
            scene.indent(0);
            scene.end_line();
        }
    }
    if boxes.more_open > 0 {
        scene.pad_to(2);
        scene.atom_gap(vec![look::dim(format!("… {} more open", boxes.more_open))], 0);
        scene.end_line();
    }
}

/// A job's mark (actions.mjs `JOB_MARK`).
fn job_mark(status: &str) -> Span<'static> {
    match status {
        "success" => look::span("✓", theme::tone(Tone::Green)),
        "failed" => look::span("✗", theme::tone(Tone::Red)),
        "running" => look::span("◐", theme::tone(Tone::Yellow)),
        "pending" | "created" => look::span("…", theme::tone(Tone::Yellow)),
        "manual" => look::dim("▸"),
        _ => look::dim("·"),
    }
}

/// Failed first, then what still runs, then the rest (#304's `JOB_RANK`).
fn job_rank(job: &Job) -> u8 {
    match job.status.as_str() {
        "failed" => 0,
        "running" | "pending" | "created" => 1,
        _ => 2,
    }
}

/// Every job whole, as many lines as they need (#304); the failed ones' logs below.
fn jobs(scene: &mut Scene<Act>, pipeline: &PipelineSection, dir: &Path) {
    let mut jobs: Vec<&Job> = pipeline.jobs.iter().collect();
    jobs.sort_by_key(|job| job_rank(job));
    for job in jobs {
        scene.atom_gap(
            vec![
                job_mark(&job.status),
                look::span(format!(" {}", job.name), Style::new()),
            ],
            3,
        );
    }
    scene.end_line();
    for log in &pipeline.logs {
        let open = log.job.url.clone().map(Act::Open);
        let head = |scene: &mut Scene<Act>| {
            scene.atom(vec![
                look::span("✗ ", theme::tone(Tone::Red)),
                look::bold(&log.job.name),
            ]);
            scene.atom_gap(vec![look::dim("click to open the job")], 2);
        };
        match open {
            Some(act) => scene.block(act, head),
            None => head(scene),
        }
        scene.end_line();
        for (line, target) in &log.lines {
            // A wrapped line goes on under its text, past its own leading spaces.
            let lead = line.len() - line.trim_start_matches(' ').len();
            let draw = |scene: &mut Scene<Act>| {
                scene.indent(6 + lead as u16);
                spaced(scene, line, if target.is_some() { Style::new() } else { theme::dim() });
                scene.indent(0);
            };
            match target {
                Some(target) => scene.block(
                    Act::OpenFile {
                        dir: dir.to_path_buf(),
                        file: target.file.clone(),
                        line: target.line,
                    },
                    draw,
                ),
                None => draw(scene),
            }
            scene.end_line();
        }
    }
}

fn develop_of(scene: &mut Scene<Act>, develop: &Develop, project: Option<&str>, place: &diff::Files) {
    scene.note(format!(
        "since the branch started  {}",
        counted(develop.merges.len() as u64, "merge request")
    ));
    match &develop.conflicts {
        None => scene.text(
            "git can't tell whether a merge would conflict (git < 2.38)",
            theme::dim(),
        ),
        Some(files) if files.is_empty() => {
            scene.atom(vec![look::span("✓", theme::tone(Tone::Green))]);
            scene.text("merges cleanly:", Style::new());
            scene.text("git fetch && git merge origin/develop", theme::dim());
        }
        Some(conflicts) => diff::conflicts(scene, conflicts, place),
    }
    scene.end_line();
    for merged in develop.merges.iter().take(MERGES) {
        let url = merged
            .mr
            .zip(project)
            .map(|(iid, project)| format!("{project}/-/merge_requests/{iid}"));
        let draw = |scene: &mut Scene<Act>| {
            if let Some(iid) = merged.mr {
                scene.atom(vec![look::span(format!("!{iid}"), Style::new())]);
            }
            // The title and the branch wrap under the title (tableRows' hanging indent).
            scene.indent(if scene.x() > 0 { scene.x() + 2 } else { 0 });
            say_gap(scene, &merged.title, Style::new(), 2);
            scene.atom_gap(vec![look::dim(&merged.branch)], 2);
            scene.indent(0);
        };
        match url {
            Some(url) => scene.block(Act::Open(url), draw),
            None => {
                draw(scene);
                scene.end_line();
            }
        }
    }
    let more = develop.merges.len().saturating_sub(MERGES);
    let mut parts = vec![];
    if more > 0 {
        parts.push(format!(
            "{} more {}",
            more,
            if more == 1 { "merge request" } else { "merge requests" }
        ));
    }
    if develop.direct > 0 {
        parts.push(format!("{} without one", counted(develop.direct as u64, "commit")));
    }
    if !parts.is_empty() {
        scene.pad_to(1);
        scene.text(&format!("… {}", parts.join(", ")), theme::dim());
        scene.end_line();
    }
}

/// Text as words that wrap whole, its first word `gap` columns after what is before it.
fn say_gap(scene: &mut Scene<Act>, text: &str, style: Style, gap: u16) {
    for (i, word) in look::words_of(&[look::span(text, style)]).into_iter().enumerate() {
        scene.atom_gap(word, if i == 0 { gap } else { 1 });
    }
}

/// A line of a log with its spaces kept: ESLint's `  12:5  error  …` keeps
/// its columns; it wraps between words, at the indent, which the caller sets
/// past the line's leading spaces.
fn spaced(scene: &mut Scene<Act>, line: &str, style: Style) {
    let mut spaces = 0u16;
    let mut is_first = true;
    for piece in line.split(' ') {
        if piece.is_empty() {
            spaces += 1;
            continue;
        }
        let word = vec![look::span(piece, style)];
        if is_first {
            scene.atom_gap(word, 0);
            is_first = false;
        } else {
            scene.atom_gap(word, spaces + 1);
        }
        spaces = 0;
    }
}

fn where_rows(scene: &mut Scene<Act>, rows: &[WhereRow], dir: &Path, state: &State) {
    scene.note("click a row to open it");
    let name = rows.iter().map(|row| row.name.len()).max().unwrap_or(0) as u16;
    // Columns as Node's table: the mark, the name, the address, the hint.
    let url_at = 1 + 2 + name + 2;
    let address = rows
        .iter()
        .filter_map(|row| row.url.as_ref().or(row.address.as_ref()))
        .map(|text| Span::raw(text.as_str()).width() as u16)
        .max()
        .unwrap_or(0);
    let hint_at = url_at + address + 2;
    // The hint in its column, or after the address where the pane has no room for one.
    let to_hint = |scene: &mut Scene<Act>| {
        let at = if scene.x() < hint_at && hint_at + 16 <= scene.width() {
            hint_at
        } else {
            scene.x() + 2
        };
        scene.pad_to(at);
        scene.indent(at);
    };
    for row in rows {
        // Outside herdr there is no tab to start Storybook in: its row shows the command.
        let start =
            (row.name == "storybook" && !row.is_on && herdr::is_in_herdr()).then(|| Act::Storybook(dir.to_path_buf()));
        let draw = |scene: &mut Scene<Act>| {
            if row.is_on {
                scene.atom(vec![look::span("●", theme::tone(Tone::Green))]);
            } else {
                scene.atom(vec![look::dim("○")]);
            }
            scene.atom_gap(vec![look::span(row.name, Style::new())], 2);
            scene.pad_to(url_at);
            scene.indent(url_at);
            if let Some(url) = row.url.as_ref().or(row.address.as_ref()) {
                scene.atom_gap(vec![look::span(url, Style::new())], 0);
            }
            if start.is_some() || row.hint.is_some() {
                to_hint(scene);
            }
            match (&start, &state.storybook) {
                (Some(start), Some(acts::Storybook::Starting)) => {
                    scene.button("◐ starting storybook …", Tone::Yellow, start.clone());
                    look::say(
                        scene,
                        "its tab storybook, the row turns ● once it answers",
                        theme::dim(),
                    );
                }
                (Some(start), Some(acts::Storybook::Failed(why))) => {
                    scene.button("▶ start storybook", Tone::Cyan, start.clone());
                    look::say(scene, why, theme::tone(Tone::Red));
                }
                (Some(start), None) => scene.button("▶ start storybook", Tone::Cyan, start.clone()),
                (None, _) => {
                    if let Some(hint) = &row.hint {
                        look::say(scene, hint, theme::dim());
                    }
                }
            }
            scene.indent(0);
        };
        match (&row.url, row.is_on, &start) {
            (Some(url), true, _) => scene.block(Act::Open(url.clone()), draw),
            (_, _, Some(start)) => scene.block(start.clone(), draw),
            _ => {
                draw(scene);
                scene.end_line();
            }
        }
    }
}

/// `▶ start the stack` and what its start is at: running with its last line, or why it failed.
fn start_button(scene: &mut Scene<Act>, dir: &Path, ctx: &Ctx, state: &State) {
    let start = Act::StartStack(dir.to_path_buf());
    scene.block(start.clone(), |scene| match state.stack_shown() {
        Some(started) if started.is_running() => {
            scene.button("◐ starting the stack …", Tone::Yellow, start.clone());
            look::say(scene, &started.last_line(), theme::dim());
            scene.redraw_in(acts::WATCH);
        }
        started => {
            scene.button("▶ start the stack", Tone::Cyan, start.clone());
            let hint = if ctx.is_once { "pnpm stack:up" } else { "click or u" };
            look::say(scene, hint, theme::dim());
            if let Some(error) = started.and_then(|started| started.error("stack:up")) {
                look::say(scene, &error, theme::tone(Tone::Red));
            }
        }
    });
}

fn stack(scene: &mut Scene<Act>, model: &WorktreeModel, tree: &Tree, ctx: &Ctx, state: &State) {
    let local = &model.local;
    let is_shared = !tree.has_own_port;
    if is_shared {
        scene.note(format!("shared, the one of the main checkout ({})", tree.project));
        look::say(
            scene,
            "its tabs are in the space Monocircuit/Platform; an own stack: pnpm wt:stack",
            theme::dim(),
        );
        scene.end_line();
    } else {
        scene.note(tree.project.clone());
        if !local.has_docker {
            look::say(scene, "docker does not answer", theme::tone(Tone::Yellow));
            scene.end_line();
        } else {
            containers(scene, model);
            let is_running = tree.stack.as_deref().is_some_and(|stack| stack.starts_with("running"));
            if !is_running {
                start_button(scene, &local.dir, ctx, state);
            }
        }
    }
    let is_up = is_shared || tree.stack.as_deref().is_some_and(|stack| stack.starts_with("running"));
    if let Some(migrations) = local.migrations.as_ref().filter(|_| is_up) {
        migrations_of(scene, migrations, &local.dir, state);
    }
}

/// The containers of the worktree's own stack: service, state, health.
fn containers(scene: &mut Scene<Act>, model: &WorktreeModel) {
    let containers = &model.local.containers;
    let service = containers
        .iter()
        .map(|container| container.service.chars().count())
        .max()
        .unwrap_or(0) as u16;
    let state = containers
        .iter()
        .map(|container| container.state.chars().count())
        .max()
        .unwrap_or(0) as u16;
    let tone = |is_good: bool| theme::tone(if is_good { Tone::Green } else { Tone::Yellow });
    for container in containers {
        scene.atom(vec![look::span(&container.service, Style::new())]);
        scene.pad_to(service + 2);
        scene.atom_gap(
            vec![look::span(&container.state, tone(container.state == "running"))],
            0,
        );
        if !container.health.is_empty() {
            scene.pad_to(service + 2 + state + 2);
            scene.atom_gap(
                vec![look::span(&container.health, tone(container.health == "healthy"))],
                0,
            );
        }
        scene.end_line();
    }
}

/// `migrations  ✓ all N applied`, or the ones missing and the button that applies them.
fn migrations_of(scene: &mut Scene<Act>, migrations: &Migrations, dir: &Path, state: &State) {
    let started = state.migration_shown();
    let pending = match migrations {
        Migrations::Error(why) => {
            row(scene, "migrations", None, |scene| {
                look::say(
                    scene,
                    &format!("the database doesn't answer: {why}"),
                    theme::tone(Tone::Yellow),
                )
            });
            return;
        }
        Migrations::Pending { pending, applied } if pending.is_empty() && started.is_none() => {
            row(scene, "migrations", None, |scene| {
                scene.atom(vec![look::span("✓", theme::tone(Tone::Green))]);
                scene.text(&format!("all {applied} applied"), theme::dim());
            });
            return;
        }
        Migrations::Pending { pending, .. } => pending,
    };
    row(scene, "migrations", None, |scene| {
        let names: Vec<&str> = pending.iter().take(3).map(String::as_str).collect();
        let more = if pending.len() > 3 {
            format!(" +{} more", pending.len() - 3)
        } else {
            String::new()
        };
        look::say(
            scene,
            &format!(
                "⚠ {} not applied: {}{more}",
                counted(pending.len() as u64, "migration"),
                names.join(", ")
            ),
            theme::tone(Tone::Yellow),
        );
    });
    let migrate = Act::Migrate(dir.to_path_buf());
    scene.block(migrate.clone(), |scene| {
        scene.pad_to(LABEL);
        scene.indent(LABEL);
        match started {
            Some(started) if started.is_running() => {
                scene.button("◐ migrating …", Tone::Yellow, migrate.clone());
                look::say(scene, &started.last_line(), theme::dim());
                scene.redraw_in(acts::WATCH);
            }
            started => {
                scene.button("▶ pnpm pg:migrate", Tone::Cyan, migrate.clone());
                if let Some(error) = started.and_then(|started| started.error("pg:migrate")) {
                    look::say(scene, &error, theme::tone(Tone::Red));
                }
            }
        }
        scene.indent(0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::scene::Target;
    use breadboard_core::model::develop::Merged;
    use breadboard_core::model::files::LogTarget;
    use breadboard_core::model::worktree::FailedLog;

    fn text(scene: &Scene<Act>) -> Vec<String> {
        scene
            .lines
            .iter()
            .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>())
            .map(|line| line.trim_end().to_string())
            .collect()
    }

    fn job(id: i64, name: &str, status: &str) -> Job {
        Job {
            id,
            name: name.into(),
            status: status.into(),
            url: Some(format!("https://gitlab/jobs/{id}")),
            ..Job::default()
        }
    }

    /// The action of the block or button at a line, off the buttons.
    fn act_at(scene: &Scene<Act>, y: u16) -> Option<&Act> {
        scene
            .regions
            .iter()
            .rev()
            .find(|region| region.contains(3, y) && matches!(region.target, Target::Block(_)))
            .map(|region| region.action())
    }

    #[test]
    fn every_job_whole_the_failed_first_and_their_logs_open_files() {
        let failed = job(3, "check:breadboard", "failed");
        let pipeline = PipelineSection {
            jobs: vec![
                job(1, "check:vita", "success"),
                job(2, "check:agents", "running"),
                failed.clone(),
            ],
            logs: vec![FailedLog {
                job: failed,
                lines: vec![
                    ("$ pnpm test".into(), None),
                    (
                        "src/a.test.mjs:12:5 error".into(),
                        Some(LogTarget {
                            file: "src/a.test.mjs".into(),
                            line: Some(12),
                        }),
                    ),
                    ("  12:5  error  'x' is not defined  no-undef".into(), None),
                ],
            }],
        };
        let mut scene = Scene::new(40);
        jobs(&mut scene, &pipeline, Path::new("/w"));
        scene.finish();
        assert_eq!(
            text(&scene),
            [
                "✗ check:breadboard   ◐ check:agents",
                "✓ check:vita",
                "✗ check:breadboard",
                "click to open the job",
                "      $ pnpm test",
                "      src/a.test.mjs:12:5 error",
                "        12:5  error  'x' is not defined",
                "        no-undef",
            ]
        );
        assert_eq!(act_at(&scene, 3), Some(&Act::Open("https://gitlab/jobs/3".into())));
        assert_eq!(act_at(&scene, 4), None);
        assert_eq!(
            act_at(&scene, 5),
            Some(&Act::OpenFile {
                dir: "/w".into(),
                file: "src/a.test.mjs".into(),
                line: Some(12),
            })
        );
    }

    #[test]
    fn a_develop_merge_wraps_under_its_title_and_opens_its_mr() {
        let merged = |n: i64| Merged {
            mr: Some(n),
            title: format!("feat(vita): develop change number {n} with a long title"),
            branch: format!("feature/{n}-develop-change"),
        };
        let develop = Develop {
            merges: vec![merged(980), merged(970)],
            direct: 1,
            conflicts: Some(vec![]),
        };
        let mut scene = Scene::new(48);
        let place = diff::Files {
            dir: Path::new("/w"),
            diff: None,
            shown: None,
        };
        develop_of(&mut scene, &develop, Some("https://gitlab/p"), &place);
        scene.finish();
        assert_eq!(
            text(&scene),
            [
                "✓ merges cleanly: git fetch && git merge",
                "origin/develop",
                "!980  feat(vita): develop change number 980 with",
                "      a long title  feature/980-develop-change",
                "!970  feat(vita): develop change number 970 with",
                "      a long title  feature/970-develop-change",
                "  … 1 commit without one",
            ]
        );
        let open = Some(Act::Open("https://gitlab/p/-/merge_requests/970".into()));
        assert_eq!(act_at(&scene, 5), open.as_ref());
    }
}
