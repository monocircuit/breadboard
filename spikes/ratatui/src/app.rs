//! The state of the view, its sections and how it draws and takes input.
//! `draw` and `click` are all a test needs: no terminal, no GitLab.

use crate::gitlab::{Job, Mr};
use crate::scene::{Action, Region, Scene, Target};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

/// The lines above the scrolling body.
pub const HEADER: u16 = 2;
const HOVER: Color = Color::Rgb(40, 44, 52);
const HOVER_BUTTON: Color = Color::Rgb(70, 78, 96);

#[derive(Default)]
pub struct App {
    pub mrs: Option<Vec<Mr>>,
    pub error: Option<String>,
    pub read_at: Option<Instant>,
    pub read_took: Option<Duration>,
    pub is_reading: bool,
    pub folded: HashSet<String>,
    pub selected: Option<String>,
    /// The mouse, in screen coordinates.
    pub mouse: Option<(u16, u16)>,
    pub scroll: u16,
    /// The last scene drawn: what the mouse hits.
    pub scene: Option<Scene>,
    pub body: Rect,
    /// Every action a click ran, for the tests and the status line.
    pub ran: Vec<Action>,
}

impl App {
    /// The scene at `width`, each section on its own: one that panics draws
    /// its error in its place (#306), the others draw as usual.
    pub fn build(&self, width: u16) -> Scene {
        let mut scene = Scene::new(width);
        let sections: [(&str, fn(&App, &mut Scene)); 2] =
            [("MERGE REQUESTS", merge_requests), ("BROKEN", broken)];
        for (name, section) in sections {
            let mut part = Scene::new(width);
            let drawn = catch_unwind(AssertUnwindSafe(|| {
                section(self, &mut part);
                part
            }));
            scene.append(match drawn {
                Ok(part) => part.finish(),
                Err(panic) => failed(name, &panic_text(&panic), width),
            });
        }
        scene.finish()
    }

    fn max_scroll(&self) -> u16 {
        let lines = self.scene.as_ref().map_or(0, |scene| scene.lines.len() as u16);
        lines.saturating_sub(self.body.height)
    }

    pub fn scroll_by(&mut self, delta: i32) {
        let to = (self.scroll as i32 + delta).clamp(0, self.max_scroll() as i32);
        self.scroll = to as u16;
    }

    /// The region under a screen position.
    pub fn region_at(&self, x: u16, y: u16) -> Option<&Region> {
        let body = self.body;
        if y < body.y || y >= body.y + body.height || x >= body.width {
            return None;
        }
        self.scene.as_ref()?.hit(x, y - body.y + self.scroll)
    }

    /// A click at a screen position: what it ran.
    pub fn click(&mut self, x: u16, y: u16) -> Option<Action> {
        let action = match &self.region_at(x, y)?.target {
            Target::Button { action, .. } | Target::Block(action) => action.clone(),
        };
        match &action {
            Action::ToggleJobs(iid) => {
                if !self.folded.remove(iid) {
                    self.folded.insert(iid.clone());
                }
            }
            Action::Select(iid) => self.selected = Some(iid.clone()),
            Action::Open(_) | Action::Pipeline(_) => {}
        }
        self.ran.push(action.clone());
        Some(action)
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.body = Rect::new(0, HEADER, area.width, area.height.saturating_sub(HEADER));
        let scene = self.build(area.width);
        self.scene = Some(scene);
        self.scroll = self.scroll.min(self.max_scroll());
        let scene = self.scene.as_ref().unwrap();

        frame.render_widget(Paragraph::new(vec![self.status_line(), Line::default()]), area);
        let start = self.scroll as usize;
        let shown: Vec<Line> = scene.lines.iter().skip(start).take(self.body.height as usize).cloned().collect();
        frame.render_widget(Paragraph::new(shown), self.body);

        // Hover: the whole block, and the button under the mouse on top.
        let Some((x, y)) = self.mouse else { return };
        let Some(scene_y) = (y >= self.body.y).then(|| y - self.body.y + self.scroll) else { return };
        let on_screen = |area: Rect| -> Option<Rect> {
            let top = area.y.max(self.scroll);
            let bottom = (area.y + area.height).min(self.scroll + self.body.height);
            (top < bottom).then(|| Rect::new(area.x, top - self.scroll + self.body.y, area.width, bottom - top))
        };
        let buffer = frame.buffer_mut();
        let block = scene
            .regions
            .iter()
            .find(|r| matches!(r.target, Target::Block(_)) && contains(r.area, x, scene_y));
        if let Some(rect) = block.and_then(|r| on_screen(r.area)) {
            buffer.set_style(rect.intersection(buffer.area), Style::new().bg(HOVER));
        }
        if let Some(region) = scene.hit(x, scene_y).filter(|r| matches!(r.target, Target::Button { .. })) {
            if let Some(rect) = on_screen(region.area) {
                buffer.set_style(rect.intersection(buffer.area), Style::new().bg(HOVER_BUTTON).add_modifier(Modifier::BOLD));
            }
        }
    }

    fn status_line(&self) -> Line<'static> {
        let count = self.mrs.as_ref().map_or("…".into(), |mrs| mrs.len().to_string());
        let mut spans = vec![" breadboard spike · ratatui".bold(), format!("  {count} open").dim()];
        let state = match (self.is_reading, &self.error, self.read_at) {
            (true, _, _) => "GitLab: reading…".to_string(),
            (_, Some(error), _) => format!("GitLab failed: {error}"),
            (_, _, Some(at)) => format!(
                "GitLab: read in {} ms, {} s ago",
                self.read_took.unwrap_or_default().as_millis(),
                at.elapsed().as_secs()
            ),
            _ => "GitLab: not read yet".into(),
        };
        spans.push(format!("  {state}").dim());
        if let Some(action) = self.ran.last() {
            spans.push(format!("  last click: {action:?}").yellow());
        }
        Line::from(spans)
    }
}

fn contains(area: Rect, x: u16, y: u16) -> bool {
    x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height
}

fn panic_text(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panicked".into())
}

fn heading(scene: &mut Scene, name: &str, note: &str) {
    scene.end_line();
    scene.indent(0);
    scene.atom(vec![Span::raw(" "), name.to_string().bold(), format!("  {note}").dim()], 0);
    scene.newline();
}

/// A section that failed to draw: its name and why, in its own place.
fn failed(name: &str, why: &str, width: u16) -> Scene {
    let mut scene = Scene::new(width);
    heading(&mut scene, name, "failed to draw");
    scene.indent(3);
    scene.pad_to(3);
    scene.words(&format!("⚠ {why}"), Style::new().fg(Color::Red));
    scene.newline();
    scene.finish()
}

fn mark(status: &str) -> Span<'static> {
    match status {
        "SUCCESS" => "✓".green(),
        "FAILED" => "✗".red(),
        "RUNNING" => "●".yellow(),
        "PENDING" | "CREATED" | "WAITING_FOR_RESOURCE" | "PREPARING" => "○".dim(),
        _ => "·".dim(),
    }
}

/// Failed jobs first, then running and waiting ones, then the rest (#304).
fn rank(job: &Job) -> u8 {
    match job.status.as_str() {
        "FAILED" => 0,
        "RUNNING" => 1,
        "PENDING" | "CREATED" | "WAITING_FOR_RESOURCE" | "PREPARING" => 2,
        _ => 3,
    }
}

const TITLE_COLUMN: u16 = 10;

fn merge_requests(app: &App, scene: &mut Scene) {
    let Some(mrs) = &app.mrs else {
        heading(scene, "MERGE REQUESTS", "reading GitLab…");
        return;
    };
    heading(scene, "MERGE REQUESTS", &format!("{} open", mrs.len()));
    for mr in mrs {
        scene.end_line();
        let from = scene.y();
        let status = mr.head_pipeline.as_ref().map_or("NONE", |p| p.status.as_str());
        scene.indent(TITLE_COLUMN);
        scene.atom(vec![Span::raw("  "), mark(status), Span::raw(" "), format!("!{}", mr.iid).cyan()], 0);
        scene.pad_to(TITLE_COLUMN);
        let selected = app.selected.as_deref() == Some(&mr.iid);
        let title_style = if selected { Style::new().bold().underlined() } else { Style::new().bold() };
        scene.words(&mr.title, title_style);
        scene.newline();
        scene.pad_to(TITLE_COLUMN);
        let jobs = mr.head_pipeline.as_ref().map_or(&[][..], |p| &p.jobs.nodes[..]);
        let done = jobs.iter().filter(|job| job.status == "SUCCESS").count();
        let draft = if mr.draft { "draft · " } else { "" };
        scene.words(&format!("{draft}{} · {}/{} jobs · {}", status.to_lowercase(), done, jobs.len(), mr.source_branch), Style::new().dim());
        scene.button("open", Action::Open(mr.web_url.clone()), Style::new().blue());
        if let Some(pipeline) = &mr.head_pipeline {
            scene.button("pipeline", Action::Pipeline(format!("https://gitlab.com{}", pipeline.path)), Style::new().blue());
        }
        let folded = app.folded.contains(&mr.iid);
        if !jobs.is_empty() {
            scene.button(if folded { "jobs ▸" } else { "jobs ▾" }, Action::ToggleJobs(mr.iid.clone()), Style::new().blue());
        }
        if !folded && !jobs.is_empty() {
            scene.newline();
            let mut sorted: Vec<&Job> = jobs.iter().collect();
            sorted.sort_by_key(|job| rank(job));
            for job in sorted {
                scene.atom(vec![mark(&job.status), Span::raw(" "), Span::raw(job.name.clone())], 2);
            }
        }
        scene.block(from, Action::Select(mr.iid.clone()));
        scene.end_line();
    }
}

/// Throws on every draw, the way #306 did: `undefined.branch`.
fn broken(app: &App, scene: &mut Scene) {
    heading(scene, "BROKEN", "");
    let issue: Option<&str> = None;
    let _ = app;
    scene.words(issue.expect("Cannot read properties of undefined (reading 'branch')"), Style::new());
}
