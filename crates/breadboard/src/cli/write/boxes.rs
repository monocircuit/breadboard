//! The checkboxes of an issue description (issues.mjs `boxesOf`, `tally`,
//! `printBoxes`, `boxReport`): numbered in order, a box in fenced code is an
//! example and gets none. The whole text after the box stays, `(test: …)`
//! of #309 included.

use super::glab;
use std::path::Path;

/// A box: its line in the description, whether it is ticked, its text and the heading above it.
#[derive(Debug, Clone, PartialEq)]
pub struct Box {
    pub line: usize,
    pub is_ticked: bool,
    pub text: String,
    pub heading: Option<String>,
}

/// Whether a line opens or closes fenced code (`^\s*```).
pub fn is_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

/// The text of a heading line (`^#{1,6}\s+(.*)$`), trimmed.
pub fn heading_of(line: &str) -> Option<&str> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    let rest = &line[hashes..];
    ((1..=6).contains(&hashes) && rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

/// `(mark, text)` of a box line (`^\s*[-*+]\s+\[([ xX])\]\s+(.*)$`), the text trimmed.
pub fn box_of(line: &str) -> Option<(char, &str)> {
    let rest = line.trim_start().strip_prefix(['-', '*', '+'])?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start().strip_prefix('[')?;
    let mark = rest.chars().next().filter(|mark| matches!(mark, ' ' | 'x' | 'X'))?;
    let rest = rest[1..].strip_prefix(']')?;
    rest.starts_with(char::is_whitespace).then(|| (mark, rest.trim()))
}

/// A line without the `\r` of a CRLF description: Node's `$` didn't match
/// before it, so such a description had no boxes at all.
fn bare(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

pub fn boxes_of(text: &str) -> Vec<Box> {
    let mut boxes = Vec::new();
    let mut heading = None;
    let mut is_in_code = false;
    for (line, raw) in text.split('\n').enumerate() {
        let raw = bare(raw);
        if is_fence(raw) {
            is_in_code = !is_in_code;
            continue;
        }
        if is_in_code {
            continue;
        }
        if let Some(title) = heading_of(raw) {
            heading = Some(title.to_string());
        }
        if let Some((mark, text)) = box_of(raw) {
            boxes.push(Box {
                line,
                is_ticked: mark != ' ',
                text: text.to_string(),
                heading: heading.clone(),
            });
        }
    }
    boxes
}

/// `3/9`
pub fn tally(boxes: &[Box]) -> String {
    format!("{}/{}", boxes.iter().filter(|b| b.is_ticked).count(), boxes.len())
}

/// The first `count` UTF-16 units of `text` (JavaScript's `slice`), never half a character.
fn js_slice(text: &str, count: usize) -> &str {
    let mut units = 0;
    for (at, c) in text.char_indices() {
        units += c.len_utf16();
        if units > count {
            return &text[..at];
        }
    }
    text
}

fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The boxes numbered under their headings, the way wt:tick shows them;
/// `marked`: the indexes just ticked. Lines, each as console.log prints one.
pub fn print_boxes(boxes: &[Box], marked: &[usize]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut heading: Option<Option<&str>> = None;
    for (index, each) in boxes.iter().enumerate() {
        if heading != Some(each.heading.as_deref()) {
            heading = Some(each.heading.as_deref());
            lines.push(format!("\n  {}", each.heading.as_deref().unwrap_or("(no heading)")));
        }
        let text = if js_len(&each.text) > 100 {
            format!("{}…", js_slice(&each.text, 99))
        } else {
            each.text.clone()
        };
        lines.push(format!(
            "  {:>3} {} {text}{}",
            index + 1,
            if each.is_ticked { "[x]" } else { "[ ]" },
            if marked.contains(&index) {
                "   ← ticked now"
            } else {
                ""
            }
        ));
    }
    lines
}

/// The end of wt:mr's answer: how far the issue's boxes are ticked, and the
/// command that ticks them. Without `text` the issue is read first.
pub fn box_report(top: &Path, number: &str, text: Option<&str>) -> String {
    let read;
    let text = match text {
        Some(text) => text,
        None => {
            let Some(issue) = glab::view(top, "issue", number) else {
                return String::new();
            };
            read = glab::text_of(&issue, "description").to_string();
            &read
        }
    };
    let boxes = boxes_of(text);
    if boxes.is_empty() {
        return String::new();
    }
    if boxes.iter().any(|b| !b.is_ticked) {
        format!(
            "\n\n  #{number}: {} boxes ticked. Done one? pnpm wt:tick {number} lists them, pnpm wt:tick {number} <box>… ticks",
            tally(&boxes)
        )
    } else {
        format!("\n\n  #{number}: all {} boxes ticked", boxes.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISSUE: &str = "Intro\n- [ ] before any heading\n## Scope\n- [x] done (test: a.test.mjs › it works)\n  * [ ]  indented  \n```\n- [ ] an example\n```\n### Later\n+ [X] capital\n-[ ] no space\n- [ ]\n- [ ] \n####### seven\n- [ ] under seven";

    #[test]
    fn numbers_the_boxes_outside_code_under_their_heading() {
        let boxes = boxes_of(ISSUE);
        let texts: Vec<&str> = boxes.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "before any heading",
                "done (test: a.test.mjs › it works)",
                "indented",
                "capital",
                "",
                "under seven"
            ]
        );
        assert_eq!(boxes[0].heading, None);
        assert_eq!(boxes[2].heading.as_deref(), Some("Scope"));
        assert_eq!(boxes[5].heading.as_deref(), Some("Later"));
        assert_eq!(
            boxes.iter().map(|b| b.is_ticked).collect::<Vec<_>>(),
            [false, true, false, true, false, false]
        );
        assert_eq!(boxes[1].line, 3);
        assert_eq!(tally(&boxes), "2/6");
    }

    #[test]
    fn reads_a_crlf_description() {
        let boxes = boxes_of("## A\r\n- [ ] one\r\n- [x] two\r\n");
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0].heading.as_deref(), Some("A"));
        assert_eq!(boxes[0].text, "one");
    }

    #[test]
    fn prints_them_numbered_and_cut_at_a_hundred() {
        let long = "x".repeat(101);
        let boxes = boxes_of(&format!("- [ ] first\n## H\n- [x] {long}"));
        let lines = print_boxes(&boxes, &[1]);
        assert_eq!(
            lines,
            [
                "\n  (no heading)".to_string(),
                "    1 [ ] first".to_string(),
                "\n  H".to_string(),
                format!("    2 [x] {}…   ← ticked now", "x".repeat(99)),
            ]
        );
        let hundred = "y".repeat(100);
        assert!(print_boxes(&boxes_of(&format!("- [ ] {hundred}")), &[])[1].ends_with(&hundred));
    }

    #[test]
    fn reports_how_far_the_boxes_are() {
        let top = Path::new("/");
        assert_eq!(
            box_report(top, "7", Some("- [x] a\n- [ ] b")),
            "\n\n  #7: 1/2 boxes ticked. Done one? pnpm wt:tick 7 lists them, pnpm wt:tick 7 <box>… ticks"
        );
        assert_eq!(box_report(top, "7", Some("- [x] a")), "\n\n  #7: all 1 boxes ticked");
        assert_eq!(box_report(top, "7", Some("nothing")), "");
    }
}
