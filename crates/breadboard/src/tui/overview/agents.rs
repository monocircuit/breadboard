//! AGENTS (#289, free-agents.mjs): the free agents of an orchestration, their
//! worktree gone, waiting in their tab for the next issue. A row is a table
//! line: its state, its name (#292) and the name it goes by, aligned over
//! the rows, the dismiss button at the right edge; its context and where it
//! came from under the name. Two clicks anywhere on a row dismiss it. The
//! button stays whole on one line, the text wraps before it.

use super::{Act, Ctx, look};
use crate::tui::theme::Tone;
use crate::tui::{Page, Scene};
use breadboard_core::model::free_agents::{FreeRow, free_head};
use breadboard_core::model::orchestration::OrchestrationModel;
use ratatui::text::Span;

/// The dismiss button's label (`✕ click again to dismiss <name>` once armed: the actor's).
const DISMISS: &str = "✕ dismiss";

/// The label of a row's armed dismiss (free-agents.mjs `dismissOf`): by its display name (#292).
fn armed(row: &FreeRow) -> String {
    format!("✕ click again to dismiss {}", row.display)
}

pub fn build(page: &mut Page<Act>, model: &OrchestrationModel, ctx: &Ctx) {
    let rows = match &model.free_agents {
        Ok(rows) if rows.is_empty() => return,
        Ok(rows) => rows,
        Err(why) => return page.failed("AGENTS", why),
    };
    page.section("AGENTS", |scene| {
        scene.note(free_head(rows.len()).text);
        let columns = Columns::of(rows);
        for row in rows {
            let label = super::actor::view::dismiss_label(ctx, &row.id, DISMISS);
            row_lines(scene, row, &columns, &label);
        }
    });
}

/// The widths of the columns over all rows, the button's at its widest
/// (armed): the layout is chosen once for all rows, so they line up and a
/// first click never reflows one.
struct Columns {
    state: u16,
    display: u16,
    name: u16,
    button: u16,
}

impl Columns {
    fn of(rows: &[FreeRow]) -> Self {
        let widest = |of: &dyn Fn(&FreeRow) -> String| {
            rows.iter()
                .map(|row| Span::raw(of(row)).width() as u16)
                .max()
                .unwrap_or(0)
        };
        Columns {
            state: widest(&|row| row.state.text.clone()),
            display: widest(&|row| row.display.clone()),
            name: widest(&|row| row.name.clone()),
            button: widest(&|row| format!(" {} ", armed(row))).max(button_width(DISMISS)),
        }
    }

    /// Where the name and the text under it start.
    fn under(&self) -> u16 {
        self.state + 2 + self.display + 2
    }
}

/// The columns of a button with `label`.
fn button_width(label: &str) -> u16 {
    Span::raw(format!(" {label} ")).width() as u16
}

/// `free  Nadia Moreau  298-breadboard-no-push     ✕ dismiss`, then dim
/// `0 % · from !310 · 1 issue` under the name (core's text).
/// The widest label not beside the widest name: the name and the text go
/// under the display name, the button stays at the right edge of the first
/// line. Not even beside the display name: the words as they
/// come, the button after them. Nothing is cut (#304).
fn row_lines(scene: &mut Scene<Act>, row: &FreeRow, columns: &Columns, label: &str) {
    let dismiss = Act::Dismiss(row.id.clone());
    let button = button_width(label);
    let widest = columns.button.max(button);
    let under = columns.under();
    scene.block(dismiss.clone(), |scene| {
        let state = look::span(&row.state.text, look::wait(row.state.tone));
        let display = look::bold(&row.display);
        let name = look::span(&row.name, look::pink());
        let is_wide = under + columns.name + 1 + widest <= scene.width();
        let is_stacked = under + 1 + widest <= scene.width();
        if !is_wide && !is_stacked {
            scene.atom(vec![state]);
            scene.atom(vec![display]);
            scene.atom(vec![name]);
            let room = scene.width();
            text(scene, row, room);
            scene.button(label, Tone::Red, dismiss);
            return;
        }
        let mut first = vec![pad(state, columns.state + 2), pad(display, columns.display + 2)];
        if is_wide {
            first.push(name.clone());
        }
        scene.atom(first);
        scene.pad_to(scene.width() - button - 1);
        scene.button(label, Tone::Red, dismiss);
        scene.end_line();
        // Beside the name: under it. Stacked: under the display name, where the most room is.
        let indent = if is_wide { under } else { columns.state + 2 };
        scene.indent(indent);
        if !is_wide {
            scene.atom(vec![name]);
            scene.end_line();
        }
        text(scene, row, scene.width().saturating_sub(indent));
        scene.indent(0);
    });
}

/// `0 % · from 298-x (!310) · 1 issue`, dim, each part whole on a line
/// when it fits: no line ends or starts with a dangling ` · `.
fn text(scene: &mut Scene<Act>, row: &FreeRow, room: u16) {
    let parts: Vec<Vec<Span<'static>>> = row.text.split(" · ").map(|part| vec![look::dim(part)]).collect();
    look::whole(scene, &parts, room);
}

/// `span` padded with spaces to `width` columns.
fn pad(span: Span<'static>, width: u16) -> Span<'static> {
    let short = width.saturating_sub(span.width() as u16) as usize;
    Span::styled(format!("{}{}", span.content, " ".repeat(short)), span.style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use breadboard_core::model::Tone;
    use breadboard_core::model::agent_state::AgentState;

    fn row(id: &str, display: &str, name: &str) -> FreeRow {
        FreeRow {
            id: id.into(),
            name: name.into(),
            display: display.into(),
            state: AgentState {
                key: "free",
                text: "free".into(),
                tone: Tone::Free,
            },
            text: format!("0 % · from {name} (!310) · 1 issue"),
            tab: None,
        }
    }

    fn draw(rows: &[FreeRow], width: u16) -> Scene<Act> {
        let mut scene = Scene::new(width);
        let columns = Columns::of(rows);
        for row in rows {
            row_lines(&mut scene, row, &columns, DISMISS);
        }
        scene.finish();
        scene
    }

    fn text(scene: &Scene<Act>) -> Vec<String> {
        scene
            .lines
            .iter()
            .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>())
            .map(|line| line.trim_end().to_string())
            .collect()
    }

    #[test]
    fn a_row_is_a_table_line_with_the_button_whole_at_the_right_edge() {
        let rows = [
            row("a", "Nadia Moreau", "298-no-push"),
            row("b", "Emil Keller", "306-crash-row"),
        ];
        let scene = draw(&rows, 80);
        assert_eq!(
            text(&scene),
            [
                "free  Nadia Moreau  298-no-push                                       ✕ dismiss",
                "                    0 % · from 298-no-push (!310) · 1 issue",
                "free  Emil Keller   306-crash-row                                     ✕ dismiss",
                "                    0 % · from 306-crash-row (!310) · 1 issue",
            ]
        );
        let buttons: Vec<_> = scene.regions.iter().filter(|region| region.is_button()).collect();
        assert_eq!(buttons[0].area, ratatui::layout::Rect::new(69, 0, 11, 1));
        // The whole row is the block a click dismisses.
        let blocks: Vec<_> = scene.regions.iter().filter(|region| !region.is_button()).collect();
        assert_eq!((blocks[0].area.y, blocks[0].area.height), (0, 2));
        assert_eq!(blocks[0].action(), &Act::Dismiss("a".into()));
    }

    #[test]
    fn an_armed_row_keeps_its_table_line_and_the_button_at_the_right_edge() {
        let rows = [
            row("a", "Elif Kowalczyk", "298-breadboard-no-push-without-passing"),
            row("b", "Emil Keller", "306-crash-row"),
        ];
        let label = armed(&rows[0]);
        assert_eq!(label, "✕ click again to dismiss Elif Kowalczyk");
        let mut scene = Scene::new(118);
        let columns = Columns::of(&rows);
        row_lines(&mut scene, &rows[0], &columns, &label);
        row_lines(&mut scene, &rows[1], &columns, DISMISS);
        scene.finish();
        let lines = text(&scene);
        assert!(
            lines[0].starts_with("free  Elif Kowalczyk  298-breadboard-no-push-without-passing  "),
            "{lines:?}"
        );
        assert!(
            lines[0].ends_with("✕ click again to dismiss Elif Kowalczyk"),
            "{lines:?}"
        );
        assert_eq!(
            lines[1].trim_start(),
            "0 % · from 298-breadboard-no-push-without-passing (!310) · 1 issue"
        );
        assert!(lines[2].starts_with("free  Emil Keller     306-crash-row"), "{lines:?}");
        assert!(lines[2].ends_with("✕ dismiss"), "{lines:?}");
        let buttons: Vec<_> = scene.regions.iter().filter(|region| region.is_button()).collect();
        assert_eq!(buttons[0].area.y, 0);
        assert_eq!(buttons[0].area.x + buttons[0].area.width, 118);
    }

    #[test]
    fn at_80_columns_the_name_goes_under_and_arming_moves_nothing() {
        let rows = [row("a", "Elif Kowalczyk", "298-breadboard-no-push-without-passing")];
        let unarmed = draw(&rows, 78);
        let mut armed_scene = Scene::new(78);
        row_lines(&mut armed_scene, &rows[0], &Columns::of(&rows), &armed(&rows[0]));
        armed_scene.finish();
        let (unarmed_text, armed_text) = (text(&unarmed), text(&armed_scene));
        assert_eq!(
            unarmed_text[0],
            format!("free  Elif Kowalczyk{}✕ dismiss", " ".repeat(48))
        );
        assert_eq!(
            armed_text[0],
            format!(
                "free  Elif Kowalczyk{}✕ click again to dismiss Elif Kowalczyk",
                " ".repeat(18)
            )
        );
        assert_eq!(armed_text[1], "      298-breadboard-no-push-without-passing");
        assert_eq!(armed_text[1..], unarmed_text[1..]);
        assert!(!armed_text.iter().any(|line| line.contains('…')));
    }

    #[test]
    fn a_row_too_narrow_for_its_armed_label_runs_on_before_it_is_armed() {
        // 60 columns hold `✕ dismiss` after the display name, not the armed label: no reflow on a click.
        let rows = [row("a", "Elif Kowalczyk", "298-breadboard-no-push")];
        let unarmed = draw(&rows, 60);
        let mut scene = Scene::new(60);
        row_lines(&mut scene, &rows[0], &Columns::of(&rows), &armed(&rows[0]));
        scene.finish();
        assert!(text(&unarmed)[0].starts_with("free Elif Kowalczyk 298-breadboard-no-push 0 %"));
        assert!(text(&scene)[0].starts_with("free Elif Kowalczyk 298-breadboard-no-push 0 %"));
    }

    #[test]
    fn its_text_wraps_between_parts_never_inside_one() {
        let rows = [row("a", "Emil Keller", "306-overview-merge-requests-crash-row")];
        let lines = text(&draw(&rows, 80));
        assert_eq!(
            lines[1..],
            [
                "      306-overview-merge-requests-crash-row",
                "      0 % · from 306-overview-merge-requests-crash-row (!310) · 1 issue",
            ]
        );
        let lines = text(&draw(&rows, 64));
        assert_eq!(
            lines[1..],
            [
                "      306-overview-merge-requests-crash-row",
                "      0 % · from 306-overview-merge-requests-crash-row (!310) ·",
                "      1 issue",
            ]
        );
    }

    #[test]
    fn a_narrow_pane_keeps_the_button_whole_on_one_line() {
        let scene = draw(
            &[row("a", "Nadia Moreau", "298-breadboard-no-push-without-passing")],
            30,
        );
        let lines = text(&scene);
        assert!(
            lines
                .iter()
                .any(|line| line.trim_end().ends_with("  ✕ dismiss") || line.trim() == "✕ dismiss"),
            "{lines:?}"
        );
        for line in &scene.lines {
            assert!(line.width() <= 30);
        }
    }
}
