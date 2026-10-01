//! AGENTS (#289, free-agents.mjs): the free agents of an orchestration, their
//! worktree gone, waiting in their tab for the next issue. A row is a table
//! line: its state, its name (#292) and the name it goes by, aligned over
//! the rows, the dismiss button at the right edge; its context and where it
//! came from under the name. Two clicks anywhere on a row dismiss it. The
//! button stays whole on one line, the text wraps before it.

use super::{Act, Ctx, look};
use crate::tui::theme::{self, Tone};
use crate::tui::{Page, Scene};
use breadboard_core::model::free_agents::{FreeRow, free_head};
use breadboard_core::model::orchestration::OrchestrationModel;
use ratatui::text::Span;

/// The dismiss button's label (`✕ click again to dismiss <name>` once armed: the actor's).
const DISMISS: &str = "✕ dismiss";

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

/// The widths of the state and name columns over all rows.
struct Columns {
    state: u16,
    display: u16,
}

impl Columns {
    fn of(rows: &[FreeRow]) -> Self {
        let widest = |of: &dyn Fn(&FreeRow) -> &str| {
            rows.iter()
                .map(|row| Span::raw(of(row)).width() as u16)
                .max()
                .unwrap_or(0)
        };
        Columns {
            state: widest(&|row| row.state.text.as_str()),
            display: widest(&|row| row.display.as_str()),
        }
    }
}

/// `free  Nadia Moreau  298-breadboard-no-push     ✕ dismiss`, then dim
/// `0 % · from 298-breadboard-no-push (!310) · 1 issue` under the name.
/// Too narrow for the columns: the words as they come, the button after them.
fn row_lines(scene: &mut Scene<Act>, row: &FreeRow, columns: &Columns, label: &str) {
    let dismiss = Act::Dismiss(row.id.clone());
    let button = Span::raw(format!(" {label} ")).width() as u16;
    scene.block(dismiss.clone(), |scene| {
        let state = look::span(&row.state.text, look::wait(row.state.tone));
        let display = look::bold(&row.display);
        let name = look::span(&row.name, look::pink());
        let under = columns.state + 2 + columns.display + 2;
        let is_table = under + name.width() as u16 + 1 + button <= scene.width();
        if !is_table {
            scene.atom(vec![state]);
            scene.atom(vec![display]);
            scene.atom(vec![name]);
            look::say(scene, &row.text, theme::dim());
            scene.button(label, Tone::Red, dismiss);
            return;
        }
        scene.atom(vec![
            pad(state, columns.state + 2),
            pad(display, columns.display + 2),
            name,
        ]);
        scene.pad_to(scene.width() - button - 1);
        scene.button(label, Tone::Red, dismiss);
        scene.end_line();
        scene.indent(under);
        look::say(scene, &row.text, theme::dim());
        scene.indent(0);
    });
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
        let scene = draw(&rows, 64);
        assert_eq!(
            text(&scene),
            [
                "free  Nadia Moreau  298-no-push                       ✕ dismiss",
                "                    0 % · from 298-no-push (!310) · 1 issue",
                "free  Emil Keller   306-crash-row                     ✕ dismiss",
                "                    0 % · from 306-crash-row (!310) · 1 issue",
            ]
        );
        let buttons: Vec<_> = scene.regions.iter().filter(|region| region.is_button()).collect();
        assert_eq!(buttons[0].area, ratatui::layout::Rect::new(53, 0, 11, 1));
        // The whole row is the block a click dismisses.
        let blocks: Vec<_> = scene.regions.iter().filter(|region| !region.is_button()).collect();
        assert_eq!((blocks[0].area.y, blocks[0].area.height), (0, 2));
        assert_eq!(blocks[0].action(), &Act::Dismiss("a".into()));
    }

    #[test]
    fn a_narrow_pane_keeps_the_button_whole_on_one_line() {
        let scene = draw(
            &[row("a", "Nadia Moreau", "298-breadboard-no-push-without-passing")],
            30,
        );
        let lines = text(&scene);
        assert!(lines.iter().any(|line| line.trim() == "✕ dismiss"), "{lines:?}");
        for line in &scene.lines {
            assert!(line.width() <= 30);
        }
    }
}
