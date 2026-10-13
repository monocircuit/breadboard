//! An issue's boxes as the overview's fold lists them (#346): by section in
//! the issue's order, each box with its state, its whole text and how it is
//! verified, #309's mark at its end (`(test: …)`, `(agent: …)`, `(try #N)`,
//! `(after deploy, try #N)`), a try's with its answer.

use crate::actions::Action;
use crate::gitlab::IssueBox;
use regex::Regex;
use std::sync::LazyLock;

/// #309's mark at the end of a box: what verifies it.
static MARK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*\(((?:test|agent):[^()]*|try(?: #\d+)?|after deploy(?:, try #\d+)?)\)\s*$").unwrap()
});
/// The try a mark names.
static TRY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"try #(\d+)").unwrap());

/// A section of the boxes: its heading (empty for the boxes before any) and
/// its boxes in the issue's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxSection {
    pub heading: String,
    pub boxes: Vec<BoxLine>,
}

/// A box: ticked or not, its text without the mark, how it is verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxLine {
    pub is_ticked: bool,
    pub text: String,
    pub check: Option<Check>,
}

/// How a box is verified: the mark's words (`test: a.rs › it works`,
/// `try #12`); a try's answer once posted (`open` while it waits for one,
/// `looks good`, `broken`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub text: String,
    pub answer: Option<String>,
}

/// The sections of `boxes`, the tries' answers from the worktree's actions
/// (`actions` open, `answered` delivered); empty for an issue without boxes.
pub fn sections_of(boxes: &[IssueBox], actions: &[Action], answered: &[Action]) -> Vec<BoxSection> {
    let mut sections: Vec<BoxSection> = Vec::new();
    for found in boxes {
        let heading = found.heading.as_deref().map(str::trim).unwrap_or("").to_string();
        let line = line_of(found, actions, answered);
        match sections.iter_mut().find(|section| section.heading == heading) {
            Some(section) => section.boxes.push(line),
            None => sections.push(BoxSection {
                heading,
                boxes: vec![line],
            }),
        }
    }
    sections
}

/// The sections of an issue's boxes, then the MR's own (its description's
/// Tests, #346) apart, each of those headed `!iid · Tests`: both lists as
/// GitLab shows them, neither mixed into the other.
pub fn all_sections_of(
    issue: &[IssueBox],
    mr: Option<(i64, &[IssueBox])>,
    actions: &[Action],
    answered: &[Action],
) -> Vec<BoxSection> {
    let mut sections = sections_of(issue, actions, answered);
    if let Some((iid, boxes)) = mr {
        sections.extend(sections_of(boxes, actions, answered).into_iter().map(|section| {
            let heading = if section.heading.is_empty() {
                format!("!{iid}")
            } else {
                format!("!{iid} · {}", section.heading)
            };
            BoxSection { heading, ..section }
        }));
    }
    sections
}

/// The boxes that still hold the merge (#354): the open Work and acceptance
/// boxes, those the deployment checks left out, as the row's reasons count
/// them; shown under the row before any click.
pub fn open_sections_of(boxes: &[IssueBox], actions: &[Action], answered: &[Action]) -> Vec<BoxSection> {
    use super::merge::{is_acceptance, is_after_deploy, is_work};
    let open: Vec<IssueBox> = boxes
        .iter()
        .filter(|found| !found.is_ticked && (is_work(found) || is_acceptance(found)) && !is_after_deploy(found))
        .cloned()
        .collect();
    sections_of(&open, actions, answered)
}

fn line_of(found: &IssueBox, actions: &[Action], answered: &[Action]) -> BoxLine {
    let (text, check) = match MARK.captures(&found.text) {
        Some(captures) => {
            let mark = captures[1].trim().to_string();
            let answer = TRY
                .captures(&mark)
                .and_then(|id| id[1].parse::<i64>().ok())
                .and_then(|id| answer_of(id, actions, answered));
            let whole = captures.get(0).expect("the whole match");
            (
                found.text[..whole.start()].to_string(),
                Some(Check { text: mark, answer }),
            )
        }
        None => (found.text.clone(), None),
    };
    BoxLine {
        is_ticked: found.is_ticked,
        text: plain(&text),
        check,
    }
}

/// A try's answer: the one given (delivered or not yet), `open` while it
/// waits for one; None for a try no longer in the actions.
fn answer_of(id: i64, actions: &[Action], answered: &[Action]) -> Option<String> {
    let found: Vec<&Action> = actions
        .iter()
        .chain(answered)
        .filter(|action| action.id == id && action.is_try())
        .collect();
    found
        .iter()
        .find_map(|action| action.answer.clone())
        .or_else(|| (!found.is_empty()).then(|| "open".to_string()))
}

/// A box's markdown as it reads: no backticks, no bold.
fn plain(text: &str) -> String {
    text.replace("**", "").replace('`', "").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(heading: &str, text: &str, is_ticked: bool) -> IssueBox {
        IssueBox {
            heading: Some(heading.into()),
            text: text.into(),
            is_ticked,
            line: 0,
        }
    }

    fn tried(id: i64, answer: Option<&str>) -> Action {
        Action::from_value(&serde_json::json!({
            "id": id,
            "kind": "try",
            "text": "check it",
            "at": "2026-10-11T10:00:00Z",
            "answer": answer,
        }))
        .unwrap()
    }

    #[test]
    fn the_sections_keep_the_issues_order_and_every_box_whole() {
        let boxes = [
            found("Scope", "A fold control on the `MR` row, a **long** text", true),
            found("Acceptance criteria", "A click shows every box", false),
            found("Scope", "Folded by default", false),
        ];
        let sections = sections_of(&boxes, &[], &[]);
        assert_eq!(
            sections,
            [
                BoxSection {
                    heading: "Scope".into(),
                    boxes: vec![
                        BoxLine {
                            is_ticked: true,
                            text: "A fold control on the MR row, a long text".into(),
                            check: None,
                        },
                        BoxLine {
                            is_ticked: false,
                            text: "Folded by default".into(),
                            check: None,
                        },
                    ],
                },
                BoxSection {
                    heading: "Acceptance criteria".into(),
                    boxes: vec![BoxLine {
                        is_ticked: false,
                        text: "A click shows every box".into(),
                        check: None,
                    }],
                },
            ]
        );
    }

    #[test]
    fn a_mark_says_how_its_box_is_verified_a_try_with_its_answer() {
        let boxes = [
            found("Acceptance", "Ticks itself (test: a.test.mjs › sync › ticks)", true),
            found("Acceptance", "Looks right (try #12)", false),
            found("Acceptance", "Feels right (try #13)", false),
            found("Acceptance", "Broke (try #14)", false),
            found("Acceptance", "Shows (agent: browser-check)", false),
            found("Acceptance", "Works over HTTPS (after deploy, try #15)", false),
            found("Acceptance", "Not tried yet (try)", false),
            found("Acceptance", "Says (why) in the middle", false),
        ];
        let actions = [tried(12, None), tried(13, Some("looks good"))];
        let answered = [tried(14, Some("broken"))];
        // Each box as `text | check | answer`.
        let said: Vec<String> = sections_of(&boxes, &actions, &answered)[0]
            .boxes
            .iter()
            .map(|line| match &line.check {
                Some(check) => format!(
                    "{} | {} | {}",
                    line.text,
                    check.text,
                    check.answer.as_deref().unwrap_or("-")
                ),
                None => line.text.clone(),
            })
            .collect();
        assert_eq!(
            said,
            [
                "Ticks itself | test: a.test.mjs › sync › ticks | -",
                "Looks right | try #12 | open",
                "Feels right | try #13 | looks good",
                "Broke | try #14 | broken",
                "Shows | agent: browser-check | -",
                "Works over HTTPS | after deploy, try #15 | -",
                "Not tried yet | try | -",
                "Says (why) in the middle",
            ]
        );
    }

    #[test]
    fn the_mrs_own_boxes_follow_the_issues_under_its_iid() {
        let issue = [found("Scope", "the fold", false)];
        let mr = [found("Tests", "pnpm wt:check passed", true)];
        let headings: Vec<String> = all_sections_of(&issue, Some((334, &mr)), &[], &[])
            .into_iter()
            .map(|section| section.heading)
            .collect();
        assert_eq!(headings, ["Scope", "!334 · Tests"]);
        let loose = [IssueBox {
            heading: None,
            ..found("", "a box", true)
        }];
        assert_eq!(all_sections_of(&[], Some((334, &loose)), &[], &[])[0].heading, "!334");
    }

    #[test]
    fn the_open_boxes_are_the_work_and_acceptance_ones_that_hold_the_merge() {
        let boxes = [
            found("Work", "done already", true),
            found("Work", "the open boxes under the row", false),
            found("Scope", "a scope box holds no merge", false),
            found("Acceptance criteria", "every open box in full", false),
            found(
                "Acceptance criteria",
                "the test instance shows it (after deploy, try #4)",
                false,
            ),
        ];
        let open: Vec<(String, Vec<String>)> = open_sections_of(&boxes, &[], &[])
            .into_iter()
            .map(|section| {
                (
                    section.heading,
                    section.boxes.into_iter().map(|line| line.text).collect(),
                )
            })
            .collect();
        assert_eq!(
            open,
            [
                ("Work".to_string(), vec!["the open boxes under the row".to_string()]),
                (
                    "Acceptance criteria".to_string(),
                    vec!["every open box in full".to_string()]
                ),
            ]
        );
    }

    #[test]
    fn boxes_before_any_heading_have_a_section_without_one() {
        let boxes = [IssueBox {
            heading: None,
            text: "loose".into(),
            is_ticked: false,
            line: 0,
        }];
        assert_eq!(sections_of(&boxes, &[], &[])[0].heading, "");
    }
}
