//! The view of one worktree (`--worktree <dir>`, render-worktree.mjs,
//! actions.mjs): where it is, WORK (its origin, issue, MR, tests, the delete
//! button), WAITING FOR, AGENT, ISSUE, PIPELINE (every job whole, #304),
//! DIFF, DEVELOP, then WHERE and STACK when it has an .env.local. Each
//! section on its own (#306); a click on a row opens what it names.

mod acts;
mod editor;
mod started;

pub use acts::State;

use super::{Act, Ctx, Overview, look};
use crate::tui::scene::{Region, Target};
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
use breadboard_core::model::diff::{Area, Change, Diff, bar_of};
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
/// How many areas DIFF names, the rest summed up.
const AREAS: usize = 6;
/// How many merges DEVELOP names.
const MERGES: usize = 6;
/// The bars of DIFF, at their widest.
const BAR: u64 = 24;
/// How many files of an area DIFF lists under it.
const FILES: usize = 5;
/// Where a file's path starts in DIFF: after its letter.
const FILE_AT: u16 = 5;

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
        _ => Flow::Same,
    }
}

/// A key the frame has no use for: `u` starts the stack whose button shows.
pub fn key(app: &mut Overview, key: KeyEvent) -> Flow {
    let Some(Model::Worktree(model)) = &app.model else {
        return Flow::Same;
    };
    match (key.code, acts::up_target(model)) {
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
        page.section("DIFF", |scene| diff_of(scene, diff, &local.dir));
    }
    if let Some(develop) = local
        .develop
        .as_ref()
        .filter(|develop| !develop.merges.is_empty() || develop.direct > 0)
    {
        page.section("DEVELOP", |scene| develop_of(scene, develop, model, &local.dir));
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
    scene.atom(vec![
        look::span(format!("!{}", mr.iid), Style::new()),
        look::span(state, style),
    ]);
    scene.atom(look::spans(&[pipeline_mark(mr.pipeline.as_ref())]));
    scene.indent(scene.x() + 1);
    look::say(scene, &mr.title, Style::new());
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
                look::say(scene, &error, theme::tone(Tone::Red));
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
    let all = if boxes.done == boxes.total { " ✓" } else { "" };
    scene.note(format!("#{} · {}/{} done{all}", boxes.iid, boxes.done, boxes.total));
    for group in &boxes.groups {
        scene.atom(vec![look::bold(group.heading.as_deref().unwrap_or("—"))]);
        let count = format!("{}/{}", group.done, group.total);
        if group.done == group.total {
            scene.atom_gap(vec![look::span(format!("{count} ✓"), theme::tone(Tone::Green))], 2);
        } else {
            scene.atom_gap(vec![look::span(count, Style::new())], 2);
        }
        scene.end_line();
        for open in &group.open {
            scene.pad_to(3);
            scene.atom(vec![look::dim("○")]);
            scene.indent(5);
            look::say(scene, open, Style::new());
            scene.indent(0);
            scene.end_line();
        }
    }
    if boxes.more_open > 0 {
        scene.pad_to(3);
        scene.text(&format!("… {} more open", boxes.more_open), theme::dim());
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
            let draw = |scene: &mut Scene<Act>| {
                scene.pad_to(6);
                scene.indent(6);
                look::say(scene, line, if target.is_some() { Style::new() } else { theme::dim() });
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

/// `█` added green, `░` removed red, scaled so `max` fills `BAR`.
fn bar(added: u64, removed: u64, max: u64, cells: u64) -> Vec<Span<'static>> {
    let (plus, minus) = bar_of(added, removed, max, cells);
    vec![
        look::span("█".repeat(plus), theme::tone(Tone::Green)),
        look::span("░".repeat(minus), theme::tone(Tone::Red)),
    ]
}

fn diff_of(scene: &mut Scene<Act>, diff: &Diff, dir: &Path) {
    scene.note("against develop · click a file: nvim at the change");
    let mut facts = vec![
        vec![look::span(format!("{} ahead", diff.ahead), Style::new())],
        vec![look::span(
            format!("{} behind", diff.behind),
            if diff.behind > 0 {
                theme::tone(Tone::Yellow)
            } else {
                theme::dim()
            },
        )],
    ];
    if diff.uncommitted > 0 {
        facts.push(vec![look::span(
            format!("{} uncommitted", diff.uncommitted),
            theme::tone(Tone::Yellow),
        )]);
    }
    look::parted(scene, &facts);
    scene.end_line();
    if diff.files == 0 {
        scene.text("nothing changed yet", theme::dim());
        return;
    }
    scene.atom(vec![added_span(diff.added), Span::raw(" "), removed_span(diff.removed)]);
    scene.text(&format!("in {}", counted(diff.files as u64, "file")), Style::new());
    scene.atom_gap(bar(diff.added, diff.removed, diff.added + diff.removed, BAR), 2);
    scene.blank();
    let shown = &diff.areas[..diff.areas.len().min(AREAS)];
    // A table: an area's row, its files under it by their path below the area's folder.
    let files_of = |area: &'_ Area| -> Vec<(String, Change)> {
        let folder = common_folder(area.changes.iter().map(|change| change.file.as_str()));
        let mut files: Vec<(String, Change)> = area
            .changes
            .iter()
            .take(FILES)
            .map(|change| (change.file[folder.len()..].to_string(), change.clone()))
            .collect();
        // A file at the top is its own area: its row says it all.
        files.retain(|(path, _)| *path != area.name);
        files
    };
    let tables: Vec<(&Area, Vec<(String, Change)>)> = shown.iter().map(|area| (area, files_of(area))).collect();
    let mut name = 0u16;
    for (area, files) in &tables {
        name = name.max(width(&area.name));
        for (path, _) in files {
            name = name.max(FILE_AT + width(path));
        }
    }
    // Paths too long for a column: they wrap, the counts follow them.
    let name = name.min(scene.width().saturating_sub(40).max(16));
    let number = |text: String| width(&text);
    let plus = tables
        .iter()
        .flat_map(|(area, files)| std::iter::once(area.added).chain(files.iter().map(|(_, change)| change.added)))
        .map(|added| number(format!("+{added}")))
        .max()
        .unwrap_or(0);
    let minus = tables
        .iter()
        .flat_map(|(area, files)| std::iter::once(area.removed).chain(files.iter().map(|(_, change)| change.removed)))
        .map(|removed| number(format!("−{removed}")))
        .max()
        .unwrap_or(0);
    let columns = Columns {
        plus: name + 2,
        minus: name + 2 + plus + 2,
        rest: name + 2 + plus + 2 + minus + 2,
    };
    let biggest = shown.iter().map(|area| area.added + area.removed).max().unwrap_or(0);
    // The bars narrower in a narrow pane, so `N files` stays on their line.
    let cells = (scene.width().saturating_sub(columns.rest + 11) as u64).clamp(6, BAR);
    for (area, files) in &tables {
        let open = Act::OpenFile {
            dir: dir.to_path_buf(),
            file: area.top.clone(),
            line: None,
        };
        scene.block(open, |scene| {
            scene.atom_gap(vec![look::bold(&area.name)], 0);
            counts(scene, &columns, area.added, area.removed);
            scene.atom_gap(bar(area.added, area.removed, biggest, cells), 0);
            let mut last = counted(area.files as u64, "file");
            let is_uncommitted = area.changes.iter().any(|change| change.is_uncommitted);
            if files.is_empty() && area.changes.len() == 1 && is_uncommitted {
                last.push_str(" · uncommitted");
            }
            scene.atom_gap(vec![look::dim(last)], 2);
            scene.indent(0);
        });
        for (path, change) in files {
            file_row(scene, path, change, &columns, dir);
        }
        if area.changes.len() > FILES {
            scene.pad_to(FILE_AT);
            scene.text(
                &format!("… {} more in {}", area.changes.len() - FILES, area.name),
                theme::dim(),
            );
            scene.end_line();
        }
    }
    if diff.areas.len() > AREAS {
        let rest = &diff.areas[AREAS..];
        let added: u64 = rest.iter().map(|area| area.added).sum();
        let removed: u64 = rest.iter().map(|area| area.removed).sum();
        let files: usize = rest.iter().map(|area| area.files).sum();
        scene.text(
            &format!(
                "… {} +{added} −{removed} · {}",
                counted(rest.len() as u64, "area"),
                counted(files as u64, "file")
            ),
            theme::dim(),
        );
        scene.end_line();
    }
    if !diff.with_develop.is_empty() {
        collision(
            scene,
            "also changed on develop:",
            &diff.with_develop,
            Some("merge develop in while it is small"),
            dir,
        );
    }
    for other in &diff.with_worktrees {
        scene.atom(vec![look::span("⚠", theme::tone(Tone::Yellow))]);
        scene.atom(vec![look::span("also changed in", Style::new())]);
        scene.atom(vec![look::bold(format!("{}:", other.name))]);
        files(scene, &other.files, None, dir);
    }
}

/// git's letter of a file in its colour, as lazygit's files panel: new
/// green, changed yellow, gone red, moved cyan.
fn status_mark(status: char) -> Span<'static> {
    let tone = match status {
        'A' | '?' => Tone::Green,
        'D' => Tone::Red,
        'R' | 'C' => Tone::Cyan,
        _ => Tone::Yellow,
    };
    look::span(status.to_string(), theme::tone(tone))
}

/// `+12` green, `+0` quiet.
fn added_span(added: u64) -> Span<'static> {
    let style = if added > 0 {
        theme::tone(Tone::Green)
    } else {
        theme::dim()
    };
    look::span(format!("+{added}"), style)
}

/// `−3` red (U+2212), `−0` quiet.
fn removed_span(removed: u64) -> Span<'static> {
    let style = if removed > 0 {
        theme::tone(Tone::Red)
    } else {
        theme::dim()
    };
    look::span(format!("−{removed}"), style)
}

/// Where DIFF's columns start: added, removed, the bar (and what follows it).
struct Columns {
    plus: u16,
    minus: u16,
    rest: u16,
}

/// `+a  −r` in their columns, the line ready for what follows them.
fn counts(scene: &mut Scene<Act>, columns: &Columns, added: u64, removed: u64) {
    // At its column, or two after what ran into it.
    let at = |scene: &mut Scene<Act>, column: u16| {
        if scene.x() < column {
            scene.pad_to(column);
            0
        } else {
            2
        }
    };
    let gap = at(scene, columns.plus);
    scene.atom_gap(vec![added_span(added)], gap);
    let gap = at(scene, columns.minus);
    scene.atom_gap(vec![removed_span(removed)], gap);
    let gap = at(scene, columns.rest);
    scene.indent(scene.x().max(columns.rest));
    if gap > 0 {
        scene.pad_to(scene.x() + gap);
    }
}

/// A file of an area, as lazygit's files panel has it: its letter, its path
/// below the area's folder, its lines, whether it is committed; a click opens
/// it in nvim at its first change.
fn file_row(scene: &mut Scene<Act>, path: &str, change: &Change, columns: &Columns, dir: &Path) {
    let open = Act::OpenFile {
        dir: dir.to_path_buf(),
        file: change.file.clone(),
        line: None,
    };
    scene.block(open, |scene| {
        scene.pad_to(2);
        scene.atom_gap(vec![status_mark(change.status)], 0);
        scene.pad_to(FILE_AT);
        scene.indent(FILE_AT);
        let (folder, name) = match path.rfind('/') {
            Some(at) => path.split_at(at + 1),
            None => ("", path),
        };
        scene.atom_gap(vec![look::dim(folder), look::span(name, Style::new())], 0);
        counts(scene, columns, change.added, change.removed);
        if change.is_uncommitted {
            scene.atom_gap(vec![look::dim("uncommitted")], 0);
        }
        scene.indent(0);
    });
}

/// The folder all `files` are in, with its `/`; empty when they share none.
fn common_folder<'a>(mut files: impl Iterator<Item = &'a str>) -> String {
    let Some(first) = files.next() else {
        return String::new();
    };
    let mut folder = &first[..first.rfind('/').map_or(0, |at| at + 1)];
    for file in files {
        while !file.starts_with(folder) {
            let trimmed = folder.trim_end_matches('/');
            folder = &folder[..trimmed.rfind('/').map_or(0, |at| at + 1)];
        }
    }
    folder.to_string()
}

fn width(text: &str) -> u16 {
    Span::raw(text).width() as u16
}

/// `⚠ also changed on develop: a, b, c +2 more  merge develop in while it is small`.
fn collision(scene: &mut Scene<Act>, what: &str, list: &[String], suffix: Option<&str>, dir: &Path) {
    scene.atom(vec![look::span("⚠", theme::tone(Tone::Yellow))]);
    look::say(scene, what, Style::new());
    files(scene, list, suffix, dir);
}

/// File names that wrap between names, never inside one (#304); a click on one opens it.
fn files(scene: &mut Scene<Act>, list: &[String], suffix: Option<&str>, dir: &Path) {
    scene.indent(4);
    let shown = list.len().min(3);
    for (i, file) in list.iter().take(shown).enumerate() {
        let text = if i + 1 < shown {
            format!("{file},")
        } else {
            file.clone()
        };
        let open = Act::OpenFile {
            dir: dir.to_path_buf(),
            file: file.clone(),
            line: None,
        };
        let style = Style::new().add_modifier(ratatui::style::Modifier::UNDERLINED);
        // A name is a target of its own: a click opens that file, the pointer lights it.
        for area in scene.atom(vec![look::span(&text, style)]) {
            let target = Target::Button {
                label: file.clone(),
                tone: Tone::Cyan,
                action: open.clone(),
            };
            scene.regions.push(Region { area, target });
        }
    }
    if list.len() > shown {
        scene.atom(vec![look::dim(format!("+{} more", list.len() - shown))]);
    }
    if let Some(suffix) = suffix {
        scene.atom_gap(vec![look::dim(suffix)], 2);
    }
    scene.indent(0);
    scene.end_line();
}

fn develop_of(scene: &mut Scene<Act>, develop: &Develop, model: &WorktreeModel, dir: &Path) {
    scene.note(format!(
        "since the branch started · {}",
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
        Some(conflicts) => {
            collision(scene, "a merge conflicts in", conflicts, None, dir);
        }
    }
    scene.end_line();
    for merged in develop.merges.iter().take(MERGES) {
        let url = merged
            .mr
            .zip(model.project_url.as_ref())
            .map(|(iid, project)| format!("{project}/-/merge_requests/{iid}"));
        let draw = |scene: &mut Scene<Act>| {
            if let Some(iid) = merged.mr {
                scene.atom(vec![look::span(format!("!{iid}"), Style::new())]);
            }
            look::say(scene, &merged.title, Style::new());
            scene.atom_gap(vec![look::dim(&merged.branch)], 2);
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

fn where_rows(scene: &mut Scene<Act>, rows: &[WhereRow], dir: &Path, state: &State) {
    scene.note("click a row to open it");
    let name = rows.iter().map(|row| row.name.len()).max().unwrap_or(0) as u16;
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
            scene.atom(vec![look::span(row.name, Style::new())]);
            scene.pad_to(2 + name + 2);
            scene.indent(2 + name + 2);
            if let Some(url) = row.url.as_ref().or(row.address.as_ref()) {
                scene.atom_gap(vec![look::span(url, Style::new())], 0);
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
                        scene.indent(scene.x() + 2);
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

    fn change(file: &str, status: char, added: u64, is_uncommitted: bool) -> Change {
        Change {
            file: file.into(),
            status,
            added,
            removed: 0,
            is_uncommitted,
        }
    }

    #[test]
    fn diff_lists_the_files_of_each_area_below_its_folder() {
        let area_of = |name: &str, changes: Vec<Change>| {
            let mut area = Area::default();
            area.name = name.into();
            area.added = changes.iter().map(|change| change.added).sum();
            area.files = changes.len();
            area.top = changes[0].file.clone();
            area.changes = changes;
            area
        };
        let area = area_of(
            "breadboard/overview",
            vec![
                change("packages/breadboard/src/overview/files.mjs", 'M', 3, false),
                change("packages/breadboard/src/overview/lib/tty.mjs", '?', 1, true),
            ],
        );
        let readme = area_of("README.md", vec![change("README.md", 'M', 1, true)]);
        let diff = Diff {
            ahead: 1,
            behind: 2,
            uncommitted: 2,
            added: 5,
            files: 3,
            areas: vec![area, readme],
            ..Diff::default()
        };
        let mut scene = Scene::new(80);
        diff_of(&mut scene, &diff, Path::new("/w"));
        scene.finish();
        assert_eq!(
            text(&scene)[3..],
            [
                "breadboard/overview  +4  −0  ████████████████████████  2 files",
                "  M  files.mjs       +3  −0",
                "  ?  lib/tty.mjs     +1  −0  uncommitted",
                "README.md            +1  −0  ██████  1 file · uncommitted",
            ]
        );
        assert_eq!(
            act_at(&scene, 5),
            Some(&Act::OpenFile {
                dir: "/w".into(),
                file: "packages/breadboard/src/overview/lib/tty.mjs".into(),
                line: None,
            })
        );
    }

    #[test]
    fn the_folder_files_share() {
        assert_eq!(common_folder(["a/b/c.ts", "a/b/d/e.ts"].into_iter()), "a/b/");
        assert_eq!(common_folder(["a/b/c.ts", "a/x.ts"].into_iter()), "a/");
        assert_eq!(common_folder(["ab/c.ts", "a/x.ts"].into_iter()), "");
        assert_eq!(common_folder(["README.md"].into_iter()), "");
        assert_eq!(common_folder(std::iter::empty()), "");
    }
}
