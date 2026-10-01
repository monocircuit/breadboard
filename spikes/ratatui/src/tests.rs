//! Headless: frames on ratatui's TestBackend, every button clicked on every
//! one of its cells, hover, wrapping and the broken section.

use crate::FIXTURE;
use crate::app::App;
use crate::gitlab::parse;
use crate::scene::{Action, Target};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

const WIDTHS: [u16; 4] = [40, 80, 120, 200];

fn app() -> App {
    let mrs = parse(&std::fs::read_to_string(FIXTURE).unwrap()).unwrap();
    App { mrs: Some(mrs), ..App::default() }
}

fn frame(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal.backend().buffer().clone()
}

fn text_at(buffer: &Buffer, x: u16, y: u16, width: u16) -> String {
    (x..x + width).map(|x| buffer[(x, y)].symbol()).collect()
}

fn line(buffer: &Buffer, y: u16) -> String {
    text_at(buffer, 0, y, buffer.area.width)
}

/// Every button in every frame, scrolled through the whole view: each of its
/// cells runs its own action, and the frame shows its label there.
#[test]
fn every_button_cell_runs_its_own_action() {
    let mut clicked = 0;
    for width in WIDTHS {
        let mut app = app();
        let height = 30;
        frame(&mut app, width, height);
        loop {
            let buffer = frame(&mut app, width, height);
            let buttons: Vec<_> = app.scene.as_ref().unwrap().regions.iter().filter_map(|region| match &region.target {
                Target::Button { label, action } => Some((region.area, label.clone(), action.clone())),
                Target::Block(_) => None,
            }).collect();
            for (area, label, action) in buttons {
                // Only what is on screen can be clicked.
                if area.y < app.scroll || area.y >= app.scroll + app.body.height {
                    continue;
                }
                let y = area.y - app.scroll + app.body.y;
                assert_eq!(text_at(&buffer, area.x, y, area.width), format!("[{label}]"), "width {width}");
                for x in area.x..area.x + area.width {
                    let before = app.folded.clone();
                    assert_eq!(app.click(x, y), Some(action.clone()), "{label} at ({x}, {y}), width {width}");
                    app.folded = before;
                    clicked += 1;
                }
            }
            let last = app.scroll;
            app.scroll_by(app.body.height as i32);
            if app.scroll == last {
                break;
            }
        }
    }
    assert!(clicked > 500, "clicked {clicked} cells");
}

/// Every `[label]` drawn is a button a click reaches: none drawn without a region.
#[test]
fn every_drawn_button_is_clickable() {
    for width in WIDTHS {
        let mut app = app();
        let buffer = frame(&mut app, width, 400);
        for y in app.body.y..buffer.area.height {
            let text = line(&buffer, y);
            for label in ["[open]", "[pipeline]", "[jobs ▾]"] {
                for (at, _) in text.match_indices(label) {
                    let x = text[..at].chars().count() as u16;
                    let hit = app.region_at(x, y).map(|r| r.target.clone());
                    assert!(matches!(hit, Some(Target::Button { .. })), "{label} at ({x}, {y}) width {width}: {hit:?}");
                }
            }
        }
    }
}

/// A job is never cut or split, and no line runs past the edge (#304).
#[test]
fn jobs_wrap_whole() {
    let mrs = app().mrs.unwrap();
    for width in WIDTHS {
        let mut app = app();
        let buffer = frame(&mut app, width, 600);
        let all: String = (0..buffer.area.height).map(|y| line(&buffer, y) + "\n").collect();
        assert!(!all.contains('…'));
        for mr in &mrs {
            for job in mr.head_pipeline.iter().flat_map(|p| &p.jobs.nodes) {
                assert!(all.lines().any(|l| l.contains(&job.name)), "{} missing at width {width}", job.name);
            }
        }
    }
}

/// Hover on any line of a block lights all of its lines and no other (#296).
#[test]
fn hover_lights_the_whole_block() {
    let mut app = app();
    frame(&mut app, 80, 200);
    let blocks: Vec<_> = app.scene.as_ref().unwrap().regions.iter().filter(|r| matches!(r.target, Target::Block(_))).map(|r| r.area).collect();
    assert!(blocks.len() > 3);
    for block in blocks.iter().take(5) {
        for dy in 0..block.height {
            app.mouse = Some((block.width - 1, block.y + dy + app.body.y));
            let buffer = frame(&mut app, 80, 200);
            for y in app.body.y..buffer.area.height {
                let is_lit = buffer[(block.width - 1, y)].bg != ratatui::style::Color::Reset;
                let is_inside = y >= block.y + app.body.y && y < block.y + block.height + app.body.y;
                assert_eq!(is_lit, is_inside, "line {y}, block {block:?}");
            }
        }
    }
}

/// The broken section draws its error; MERGE REQUESTS draws all of its MRs (#306).
#[test]
fn a_broken_section_leaves_the_others() {
    let mut app = app();
    let count = app.mrs.as_ref().unwrap().len();
    let buffer = frame(&mut app, 120, 600);
    let all: String = (0..buffer.area.height).map(|y| line(&buffer, y) + "\n").collect();
    assert!(all.contains("BROKEN  failed to draw"));
    assert!(all.contains("reading 'branch'"));
    assert_eq!(app.scene.as_ref().unwrap().regions.iter().filter(|r| matches!(r.target, Target::Block(_))).count(), count);
}

/// Folding a block's jobs moves every block below it, and clicks follow.
#[test]
fn clicks_follow_a_fold() {
    let mut app = app();
    frame(&mut app, 80, 300);
    let jobs = app.scene.as_ref().unwrap().regions.iter().find(|r| matches!(&r.target, Target::Button { action: Action::ToggleJobs(_), .. })).unwrap().area;
    app.click(jobs.x, jobs.y + app.body.y);
    frame(&mut app, 80, 300);
    let last_open = app.scene.as_ref().unwrap().regions.iter().filter(|r| matches!(&r.target, Target::Button { action: Action::Open(_), .. })).last().unwrap().clone();
    assert_eq!(app.click(last_open.area.x, last_open.area.y + app.body.y), Some(match last_open.target { Target::Button { action, .. } => action, _ => unreachable!() }));
}
