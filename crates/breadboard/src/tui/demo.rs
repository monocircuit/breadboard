//! `breadboard overview --demo`: the frame with made-up sections, so the
//! frame is judged (wrapping, buttons, hover, scrolling, a broken section)
//! before any real section exists. A fake reader answers `r` like GitLab.

use super::theme;
use super::{App, Flow, Header, Input, Page, Scene, Section, Tone, clock};
use ratatui::style::Style;
use ratatui::text::Span;
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc::UnboundedSender;

/// What a click in the demo does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// A button: its label and whose it is.
    Press(&'static str, String),
    /// A block: which one.
    Open(String),
    /// Folds or unfolds the done worktrees.
    Fold,
    /// Breaks the BROKEN section, or mends it.
    Break,
    End,
}

/// What the fake reader sends.
pub struct Model {
    read_at: SystemTime,
    reads: u32,
}

pub struct Demo {
    sender: Option<UnboundedSender<Model>>,
    read_at: Option<SystemTime>,
    reads: u32,
    is_reading: bool,
    last: Option<Action>,
    is_folded: bool,
    is_broken: bool,
}

const ISSUES: [(&str, &str, &str); 6] = [
    (
        "305",
        "overview-gitlab-refresh-makes-312",
        "a GitLab refresh makes 312 calls and takes 29 s; read final things once, batch the rest",
    ),
    (
        "302",
        "wt-mr-mr-s-tests",
        "wt:mr: an MR's Tests list only checks that apply, no empty boxes with 'no browser change'",
    ),
    (
        "292",
        "overview-every-agent-has-name",
        "every agent has a name, shown under its worktree with its issues and MRs",
    ),
    (
        "294",
        "overview-open-mr-shows-progress",
        "an open MR shows a progress bar of its ticked boxes under its title",
    ),
    (
        "296",
        "overview-hover-highlights-whole-block",
        "hover highlights the whole block (worktree, MR, action), not one line",
    ),
    (
        "304",
        "overview-pipeline-cuts-its-jobs",
        "PIPELINE cuts its jobs off at the edge instead of wrapping",
    ),
];

const JOBS: [(&str, Tone); 9] = [
    ("✓ install", Tone::Green),
    ("✓ check:lint", Tone::Green),
    ("✓ check:typecheck", Tone::Green),
    ("◐ check:test", Tone::Yellow),
    ("✓ check:agents", Tone::Green),
    ("✗ check:breadboard-rs", Tone::Red),
    ("… check:breadboard-gauntlet-rs", Tone::Yellow),
    ("… integration:vita", Tone::Yellow),
    ("▸ deploy:test", Tone::Gray),
];

impl Demo {
    pub fn new(sender: Option<UnboundedSender<Model>>) -> Self {
        let mut demo = Self {
            sender,
            read_at: None,
            reads: 0,
            is_reading: false,
            last: None,
            is_folded: true,
            is_broken: false,
        };
        demo.read();
        demo
    }

    /// A read like GitLab's: an answer after a moment, on the channel.
    fn read(&mut self) {
        let Some(sender) = self.sender.clone() else {
            self.update(Model {
                read_at: SystemTime::now(),
                reads: self.reads + 1,
            });
            return;
        };
        self.is_reading = true;
        let reads = self.reads + 1;
        let answer = move || {
            let _ = sender.send(Model {
                read_at: SystemTime::now(),
                reads,
            });
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(async move {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    answer();
                });
            }
            Err(_) => answer(),
        }
    }

    fn said(&self) -> String {
        match &self.last {
            None => "nothing clicked yet".into(),
            Some(Action::Press(label, whose)) => format!("clicked the button {label} of {whose}"),
            Some(Action::Open(what)) => format!("clicked the block {what}"),
            Some(Action::Fold) => "clicked the fold".into(),
            Some(Action::Break) => "clicked break".into(),
            Some(Action::End) => "ending".into(),
        }
    }
}

impl App for Demo {
    type Action = Action;
    type Model = Model;

    fn header(&self) -> Header {
        let quiet = Style::new().fg(theme::BAND_QUIET);
        let status = match (self.read_at, self.is_reading) {
            (_, true) => vec![Span::styled("GitLab reading…", quiet)],
            (Some(at), false) => vec![Span::styled(format!("GitLab {}", clock(at)), quiet)],
            (None, false) => vec![],
        };
        Header::new("⇶ breadboard demo").status(status)
    }

    fn build(&self, page: &mut Page<Action>) {
        page.section(Section::new("NEXT").sticky().plain(), |scene| next(self, scene));
        page.section("ACTIONS", |scene| actions(self, scene));
        page.section("WORKTREES", |scene| worktrees(self, scene));
        page.section("PIPELINE", pipeline);
        page.section("BROKEN", |scene| broken(self, scene));
        page.section("MERGE REQUESTS", |scene| merge_requests(self, scene));
        page.section(Section::new("END").plain(), |scene| {
            scene.text("the demo ends here", theme::dim());
            scene.pad_to(scene.width().saturating_sub(10).max(scene.x() + 1));
            scene.button("end demo", Tone::Red, Action::End);
        });
    }

    fn update(&mut self, model: Model) {
        self.read_at = Some(model.read_at);
        self.reads = model.reads;
        self.is_reading = false;
    }

    fn act(&mut self, input: Input<Action>) -> Flow {
        match input {
            Input::Refresh => {
                self.read();
                Flow::Redraw
            }
            Input::Click(action) => {
                match action {
                    Action::End => return Flow::Quit,
                    Action::Fold => self.is_folded = !self.is_folded,
                    Action::Break => self.is_broken = !self.is_broken,
                    _ => {}
                }
                self.last = Some(action);
                Flow::Redraw
            }
            Input::Key(_) => Flow::Same,
        }
    }
}

fn next(demo: &Demo, scene: &mut Scene<Action>) {
    scene.atom(vec![Span::styled("NEXT", theme::bold())]);
    scene.pad_to(6);
    scene.indent(6);
    scene.chip("decide", Tone::Yellow);
    scene.atom_gap(
        vec![Span::styled("307-breadboard-best-stack-breadboard-rust", theme::bold())],
        2,
    );
    scene.atom_gap(vec![Span::styled("just now", theme::dim())], 2);
    scene.newline();
    scene.text(&demo.said(), theme::dim());
    scene.end_line();
}

fn actions(demo: &Demo, scene: &mut Scene<Action>) {
    scene.note("3 for you · a button or an option answers, the rest of a block does it");
    let whose = "307-breadboard-best-stack-breadboard-rust";
    scene.block(Action::Open("decide".into()), |scene| {
        scene.chip("decide", Tone::Yellow);
        scene.atom_gap(vec![Span::styled(whose, theme::bold())], 2);
        scene.atom_gap(vec![Span::styled("just now", theme::dim())], 2);
        scene.newline();
        scene.text("Which stack does the rewrite start with?", theme::bold());
        scene.newline();
        scene.text(
            "The numbers favour one, the loop may still pick the other",
            theme::dim(),
        );
        scene.newline();
        scene.button(
            "1. Rust + ratatui (Recommended)",
            Tone::Cyan,
            Action::Press("Rust + ratatui", whose.into()),
        );
        scene.button("2. OpenTUI on Bun", Tone::Cyan, Action::Press("OpenTUI", whose.into()));
    });
    scene.rule();
    scene.block(Action::Open("try".into()), |scene| {
        scene.chip("try", Tone::Cyan);
        scene.atom_gap(vec![Span::styled(whose, theme::bold())], 2);
        scene.atom_gap(vec![Span::styled("just now", theme::dim())], 2);
        scene.atom_gap(vec![Span::styled("ready", theme::tone(Tone::Green))], 2);
        scene.newline();
        scene.text(
            "the ratatui spike against live GitLab: every button works on every cell",
            Style::new(),
        );
        scene.atom_gap(
            vec![Span::styled("opens a terminal: cargo run --release", theme::dim())],
            2,
        );
        scene.newline();
        scene.button("looks good", Tone::Green, Action::Press("looks good", whose.into()));
        scene.button("broken", Tone::Red, Action::Press("broken", whose.into()));
    });
    scene.rule();
    scene.block(Action::Open("look".into()), |scene| {
        scene.chip("look", Tone::Gray);
        scene.atom_gap(vec![Span::styled(whose, theme::bold())], 2);
        scene.newline();
        scene.text("read the stack evidence before the loop starts", Style::new());
        scene.newline();
        scene.button("done", Tone::Cyan, Action::Press("done", whose.into()));
        scene.inactive("merge (waits for the pipeline)");
    });
    let _ = demo;
}

fn worktrees(demo: &Demo, scene: &mut Scene<Action>) {
    scene.note("6 wait for GitLab · 42 removed · 7.6 GB free (shared stack below 1.5 GB)");
    for (i, (issue, slug, title)) in ISSUES.iter().enumerate() {
        scene.block(Action::Open(format!("#{issue}")), |scene| {
            scene.indent(0);
            scene.atom(vec![Span::styled(format!("{:>2}. ", i + 3), theme::dim())]);
            scene.atom_gap(vec![Span::styled(format!("{issue}-{slug}"), theme::tone(Tone::Red))], 0);
            scene.right(vec![Span::styled("waits for the pipeline", theme::tone(Tone::Cyan))]);
            scene.newline();
            scene.indent(4);
            let mr = 303 + i * 3;
            let parts = [
                format!("#{issue} open"),
                format!("!{mr} draft ✓ passed"),
                "1 ahead".into(),
                "untested: 1 commit".into(),
                format!("agent: {} issue", if i == 0 { "2nd" } else { "1st" }),
                format!("waits for !{}, !{} · {} merged", mr - 3, mr - 6, i + 1),
            ];
            let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
            scene.parts(&parts, Style::new());
            scene.newline();
            scene.text(title, theme::dim());
            scene.indent(0);
        });
    }
    scene.block(Action::Fold, |scene| {
        let mark = if demo.is_folded { "▸" } else { "▾" };
        scene.text(&format!("{mark} done and removed (42) · click to show"), theme::dim());
    });
    if !demo.is_folded {
        for n in 0..42 {
            scene.indent(4);
            scene.text(&format!("{}-a-removed-worktree merged", 200 + n), theme::dim());
            scene.newline();
        }
        scene.indent(0);
    }
}

/// Jobs are atoms: a job wraps whole, never cut at the edge (#304).
fn pipeline(scene: &mut Scene<Action>) {
    scene.note("!316 · 9 jobs · a click opens the pipeline");
    scene.block(Action::Open("pipeline".into()), |scene| {
        scene.indent(2);
        for (job, tone) in JOBS {
            scene.atom_gap(vec![Span::styled(job, theme::tone(tone))], 2);
        }
        scene.indent(0);
    });
}

fn broken(demo: &Demo, scene: &mut Scene<Action>) {
    scene.note("a section that panics draws its error here, the others draw on");
    if demo.is_broken {
        let issue: Option<&str> = std::hint::black_box(None);
        issue.expect("Cannot read properties of undefined (reading 'branch')");
    }
    scene.text("click break: this section panics, nothing else changes", Style::new());
    scene.blank();
    scene.button("break", Tone::Red, Action::Break);
}

fn merge_requests(demo: &Demo, scene: &mut Scene<Action>) {
    scene.note(format!(
        "6 open · 42 merged · in merge order, the first merges first · read {}",
        demo.reads
    ));
    for (i, (issue, _, title)) in ISSUES.iter().enumerate() {
        let mr = 303 + i * 3;
        scene.block(Action::Open(format!("!{mr}")), |scene| {
            scene.atom(vec![Span::styled(format!("!{mr}"), theme::bold())]);
            scene.atom(vec![Span::styled("✓ passed", theme::tone(Tone::Green))]);
            scene.indent(4);
            scene.text(&format!("#{issue}: {title}"), Style::new());
            scene.right(vec![Span::styled("ready", theme::tone(Tone::Green))]);
            scene.newline();
            scene.text(
                "needs: the critic picks ours blind · the suite green · a review",
                theme::dim(),
            );
            scene.newline();
            scene.button("merge", Tone::Green, Action::Press("merge", format!("!{mr}")));
            scene.button(
                "resolve\u{a0}merge\u{a0}conflicts",
                Tone::Yellow,
                Action::Press("resolve", format!("!{mr}")),
            );
            scene.indent(0);
        });
    }
    if let Some(at) = demo.read_at {
        scene.indent(2);
        scene.text(&format!("GitLab {} · {}", clock(at), ago(at)), theme::dim());
        // `n min ago` moves on: drawn again at the next minute, no sooner.
        let age = at.elapsed().unwrap_or_default().as_secs();
        scene.redraw_in(Duration::from_secs(60 - age % 60));
    }
}

fn ago(at: SystemTime) -> String {
    match at.elapsed().unwrap_or_default().as_secs() / 60 {
        0 => "just now".into(),
        minutes => format!("{minutes} min ago"),
    }
}

#[cfg(test)]
pub fn demo() -> Demo {
    Demo::new(None)
}
