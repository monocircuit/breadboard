//! The title band on top of every view (overview/boxes.mjs): the logo at the
//! right end, the view's name on the left, its status (`GitLab 14:57`) and
//! what the scroll cuts off on the right of the first line, the last thing
//! done (`log`) on the right of the name, the keys on the right of the last.
//! A line above and below, all on the grey of the section heads. Its height
//! never changes with the log or the scroll: the rows under it stay put.
//! No wall clock: the status says how old the data is, and only that ticks.

use super::scene::{Scene, width_of};
use super::theme::{self, LOGO};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The keys of the views, as the Node overview lists them.
pub const KEYS: &str = "q quit · r refresh GitLab · click a row · wheel j k ↑↓ PgUp PgDn space g G scroll";

/// What the band says.
#[derive(Clone, Debug, Default)]
pub struct Header {
    /// The view's name: `⇶ breadboard`.
    pub title: String,
    /// The state of its data: `GitLab 14:57`, or why a read failed.
    pub status: Vec<Span<'static>>,
    /// What was done last, for a while (`14:02:11 ✓ typed into …`): one
    /// row at most, cut to the room right of the name.
    pub log: Vec<Span<'static>>,
    /// How many more things were done in that while: ` · 2 more`, never cut.
    pub log_more: usize,
    pub keys: String,
    /// When the band shows something else by itself (the log runs out).
    pub redraw_in: Option<Duration>,
}

impl Header {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            status: vec![],
            log: vec![],
            log_more: 0,
            keys: KEYS.into(),
            redraw_in: None,
        }
    }

    pub fn status(mut self, status: Vec<Span<'static>>) -> Self {
        self.status = status;
        self
    }

    /// The last thing done and how many more, shown until `until` has passed.
    pub fn log(mut self, log: Vec<Span<'static>>, more: usize, until: Duration) -> Self {
        self.log = log;
        self.log_more = more;
        self.redraw_in = Some(self.redraw_in.map_or(until, |was| was.min(until)));
        self
    }
}

/// `14:57`: a time in the band and the sections, in the local zone, as
/// `GitLab 14:57` says when GitLab last answered.
pub fn clock(at: SystemTime) -> String {
    let seconds = at.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs()) as libc::time_t;
    // SAFETY: localtime_r writes only into `tm`, which outlives the call.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&seconds, &mut tm) }.is_null() {
        return "--:--".into();
    }
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

/// The logo and the room around it: two columns before it, one after.
const LOGO_ROOM: u16 = 5 + 3;
/// Below this many columns the band drops the logo.
const LOGO_FROM: u16 = 40;

/// `↑ 3 ↓ 12 more`, or nothing when the whole view fits.
pub fn more_mark(above: u16, below: u16) -> String {
    let mut parts = vec![];
    if above > 0 {
        parts.push(format!("↑ {above}"));
    }
    if below > 0 {
        parts.push(format!("↓ {below}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{} more", parts.join(" "))
    }
}

/// The band's lines at `width`: at least the logo's height and one line
/// above and below; a name or status too wide wraps and the band grows.
pub fn lines(header: &Header, mark: &str, width: u16) -> Vec<Line<'static>> {
    let has_logo = width >= LOGO_FROM;
    let inner = width.saturating_sub(2 + if has_logo { LOGO_ROOM } else { 0 }).max(1);
    let quiet = Style::new().fg(theme::BAND_QUIET);

    // The status, the mark before it while both fit; else the status alone,
    // wrapped. So the mark never changes the band's height.
    let mut status: Vec<Span<'static>> = header.status.clone();
    let with_mark = width_of(&status) + Span::raw(mark).width() as u16 + 2;
    if !mark.is_empty() && with_mark <= inner {
        status.insert(0, Span::styled(format!("{mark}  "), quiet));
    }
    let status = if width_of(&status) <= inner {
        vec![Line::from(status)]
    } else {
        wrapped(&status, inner)
    };
    let title = wrapped(
        &[Span::styled(
            header.title.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        )],
        inner,
    );
    let keys = keys_line(&header.keys, inner, quiet);

    // The log on the right of the name's first line, cut to the room it
    // leaves: it comes and goes without moving a row.
    let mut rows: Vec<(Line<'static>, Line<'static>)> = vec![(Line::default(), Line::default())];
    rows.extend(status.into_iter().map(|line| (Line::default(), line)));
    for (i, line) in title.into_iter().enumerate() {
        let room = inner.saturating_sub(line.width() as u16 + 3);
        let log = if i == 0 && !header.log.is_empty() {
            let more = Span::styled(format!(" · {} more", header.log_more), quiet);
            let more = if header.log_more > 0 { vec![more] } else { vec![] };
            let mut log = cut(&header.log, room.saturating_sub(width_of(&more)));
            if !log.is_empty() {
                log.extend(more);
            }
            log
        } else {
            vec![]
        };
        rows.push((line, Line::from(log)));
    }
    rows.push((Line::default(), keys));
    rows.push((Line::default(), Line::default()));

    rows.into_iter()
        .enumerate()
        .map(|(i, (left, right))| {
            let gap = inner.saturating_sub((left.width() + right.width()) as u16) as usize;
            let mut spans = vec![Span::raw(" ")];
            spans.extend(left.spans);
            spans.push(Span::raw(" ".repeat(gap)));
            spans.extend(right.spans);
            if has_logo {
                let art = i.checked_sub(1).and_then(|at| LOGO.get(at)).copied().unwrap_or("     ");
                spans.push(Span::raw(format!("  {art} ")));
            }
            spans.push(Span::raw(" "));
            Line::from(spans)
        })
        .collect()
}

/// Spans as lines of at most `width` columns, broken at words.
fn wrapped(spans: &[Span<'static>], width: u16) -> Vec<Line<'static>> {
    let mut scene = Scene::<()>::new(width);
    for span in spans {
        scene.text(&span.content, span.style);
    }
    scene.finish();
    if scene.lines.is_empty() {
        vec![Line::default()]
    } else {
        scene.lines
    }
}

/// The keys' words that fit, from the first, as Node cuts the right side
/// from its end: a narrow pane drops the last words, never a dangling `·`.
fn keys_line(keys: &str, width: u16, style: Style) -> Line<'static> {
    let mut shown = String::new();
    for word in keys.split(' ') {
        let next = if shown.is_empty() {
            word.to_string()
        } else {
            format!("{shown} {word}")
        };
        if Span::raw(next.as_str()).width() > width as usize {
            break;
        }
        shown = next;
    }
    let shown = shown.trim_end_matches(" ·").to_string();
    Line::from(Span::styled(shown, style))
}

/// `spans` in at most `width` columns: whole, or the words that fit from
/// the first, as the right side of Node's band is cut, no `…` (#304);
/// nothing when not even a few columns are left.
fn cut(spans: &[Span<'static>], width: u16) -> Vec<Span<'static>> {
    if width_of(spans) <= width {
        return spans.to_vec();
    }
    if width < 12 {
        return vec![];
    }
    let mut room = width;
    let mut out: Vec<Span<'static>> = vec![];
    'spans: for span in spans {
        let mut text = String::new();
        for word in span.content.split_inclusive(' ') {
            let w = Span::raw(word.trim_end()).width() as u16;
            if w > room {
                if !text.is_empty() {
                    out.push(Span::styled(text, span.style));
                }
                break 'spans;
            }
            room = room.saturating_sub(Span::raw(word).width() as u16);
            text.push_str(word);
        }
        out.push(Span::styled(text, span.style));
    }
    // No word or `·` left dangling at the end.
    while let Some(last) = out.last_mut() {
        let text = last
            .content
            .trim_end()
            .trim_end_matches(['·', ':', ','])
            .trim_end()
            .to_string();
        if text.is_empty() {
            out.pop();
        } else {
            last.content = text.into();
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn five_lines_with_the_logo_at_the_right() {
        let header = Header::new("⇶ breadboard").status(vec![Span::raw("GitLab 14:57")]);
        let lines = lines(&header, "↓ 84 more", 120);
        let text = text(&lines);
        assert_eq!(text.len(), 5);
        for line in &lines {
            assert_eq!(line.width(), 120);
        }
        assert!(text[1].ends_with("↓ 84 more  GitLab 14:57  ▀▀▀▀▀  "), "{}", text[1]);
        assert!(text[2].starts_with(" ⇶ breadboard "), "{}", text[2]);
        assert!(text[3].ends_with("g G scroll  ▀ 🭭 ▀  "), "{}", text[3]);
    }

    #[test]
    fn a_narrow_band_drops_the_logo_and_whole_keys_never_cuts() {
        let header = Header::new("⇶ breadboard").status(vec![Span::raw("GitLab 14:57")]);
        let lines = lines(&header, "", 30);
        let text = text(&lines);
        assert!(!text.join("").contains('▀'));
        assert_eq!(text[3].trim(), "q quit · r refresh GitLab");
        for line in &lines {
            assert_eq!(line.width(), 30);
        }
    }

    #[test]
    fn the_keys_are_cut_at_a_word_from_their_end() {
        let header = Header::new("⇶ breadboard");
        let text = text(&lines(&header, "", 80));
        assert_eq!(
            text[3].trim().trim_end_matches(['▀', '🭭', ' ']),
            "q quit · r refresh GitLab · click a row · wheel j k ↑↓ PgUp PgDn space"
        );
    }

    #[test]
    fn the_log_sits_right_of_the_name_and_never_adds_a_row() {
        let log = vec![Span::raw(
            "14:02:11 ✓ typed into the agent of 292-overview-every-agent-has-name: waited 8 h",
        )];
        for width in [200, 120, 80, 40, 20] {
            let plain = lines(&Header::new("⇶ breadboard"), "↓ 3 more", width);
            let with_log = lines(
                &Header::new("⇶ breadboard").log(log.clone(), 2, Duration::from_secs(60)),
                "↓ 3 more",
                width,
            );
            assert_eq!(plain.len(), with_log.len(), "{width}");
            for line in &with_log {
                assert_eq!(line.width(), width as usize, "{width}");
            }
            let row = &text(&with_log)[2];
            assert!(row.starts_with(" ⇶ breadboard "), "{width}: {row}");
            if width >= 120 {
                assert!(row.contains("waited 8 h · 2 more"), "{width}: {row}");
            } else if width >= 80 {
                assert!(
                    row.contains("14:02:11 ✓ typed into the agent") && row.contains(" · 2 more") && !row.contains('…'),
                    "{width}: {row}"
                );
            }
        }
    }

    #[test]
    fn a_long_status_wraps_and_the_band_grows() {
        let reason = "GitLab didn't answer 14:57 (connection refused), data from 14:50, tried again";
        let header = Header::new("x").status(vec![Span::raw(reason)]);
        let lines = lines(&header, "↓ 3 more", 60);
        assert!(lines.len() > 5);
        let all = text(&lines).join(" ");
        assert!(all.contains("tried again") && !all.contains('…'));
    }

    #[test]
    fn the_clock_is_hours_and_minutes() {
        let text = clock(SystemTime::now());
        assert_eq!(text.len(), 5, "{text}");
        assert_eq!(&text[2..3], ":");
    }

    #[test]
    fn the_mark() {
        assert_eq!(more_mark(0, 0), "");
        assert_eq!(more_mark(3, 12), "↑ 3 ↓ 12 more");
        assert_eq!(more_mark(0, 4), "↓ 4 more");
    }
}
