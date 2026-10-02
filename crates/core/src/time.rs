//! Times as the Node code keeps them: ISO strings in the files
//! (`toISOString`), milliseconds since the epoch in between (`Date.now`).
//! UTC only: the local clock is the view's business.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the epoch, now.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// `Date.parse` of the forms the files and GitLab use: `2026-09-30T15:27:58.866Z`,
/// without the fraction, with an offset (`+02:00`), or a date alone (UTC). None
/// for anything else, as Node's NaN.
pub fn parse_iso(text: &str) -> Option<i64> {
    let text = text.trim();
    let bytes = text.as_bytes();
    let number = |from: usize, len: usize| -> Option<i64> {
        let part = text.get(from..from + len)?;
        part.bytes().all(|b| b.is_ascii_digit()).then(|| part.parse().ok())?
    };
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let (year, month, day) = (number(0, 4)?, number(5, 2)?, number(8, 2)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let date = days_from_civil(year, month, day) * 86_400_000;
    if bytes.len() == 10 {
        return Some(date);
    }
    if bytes[10] != b'T' && bytes[10] != b' ' {
        return None;
    }
    let (hour, minute) = (number(11, 2)?, number(14, 2)?);
    let mut at = 16;
    let mut second = 0;
    if bytes.get(at) == Some(&b':') {
        second = number(17, 2)?;
        at = 19;
    }
    let mut millis = 0;
    if bytes.get(at) == Some(&b'.') {
        let digits = text[at + 1..].bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let fraction = &text[at + 1..at + 1 + digits];
        millis = format!("{:0<3}", &fraction[..digits.min(3)]).parse().ok()?;
        at += 1 + digits;
    }
    let offset = match text.get(at..) {
        Some("Z") | Some("z") => 0,
        // Without a zone, Date.parse takes a date-time as local; the files never write one.
        Some("") => 0,
        Some(zone) if zone.len() == 6 && (zone.starts_with('+') || zone.starts_with('-')) => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let hours: i64 = zone.get(1..3)?.parse().ok()?;
            let minutes: i64 = zone.get(4..6)?.parse().ok()?;
            sign * (hours * 60 + minutes) * 60_000
        }
        _ => return None,
    };
    Some(date + ((hour * 60 + minute) * 60 + second) * 1000 + millis - offset)
}

/// `new Date(ms).toISOString()`: `2026-09-30T15:27:58.866Z`.
pub fn to_iso(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rest = ms.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second, millis) = (rest / 3_600_000, rest / 60_000 % 60, rest / 1000 % 60, rest % 1000);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// `40 s`, `3 min`, `2 h`: how long something runs (try-state.mjs `span`).
pub fn span(ms: i64) -> String {
    let seconds = (ms / 1000).max(0);
    if seconds < 60 {
        format!("{seconds} s")
    } else if seconds < 3600 {
        format!("{} min", seconds / 60)
    } else {
        format!("{} h", seconds / 3600)
    }
}

/// `just now`, `5 min ago`, `3 h ago`, `4 days ago` (render-worktree.mjs `ago`).
pub fn ago(then: i64, now: i64) -> String {
    let minutes = (now - then).div_euclid(60_000);
    if minutes < 1 {
        "just now".into()
    } else if minutes < 60 {
        format!("{minutes} min ago")
    } else if minutes < 48 * 60 {
        format!("{} h ago", minutes / 60)
    } else {
        format!("{} days ago", minutes / (24 * 60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_what_node_writes_and_writes_it_back() {
        let at = parse_iso("2026-09-30T15:27:58.866Z").unwrap();
        assert_eq!(to_iso(at), "2026-09-30T15:27:58.866Z");
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("1970-01-01"), Some(0));
        assert_eq!(parse_iso("1970-01-01T02:00:00+02:00"), Some(0));
        assert_eq!(parse_iso("2026-09-30T15:27:58.8Z").unwrap() % 1000, 800);
        assert_eq!(to_iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(to_iso(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn refuses_what_date_parse_refuses() {
        assert_eq!(parse_iso(""), None);
        assert_eq!(parse_iso("yesterday"), None);
        assert_eq!(parse_iso("2026-13-01"), None);
    }

    #[test]
    fn spans_and_ages_read_as_node_writes_them() {
        assert_eq!(span(40_000), "40 s");
        assert_eq!(span(180_000), "3 min");
        assert_eq!(span(7_200_000), "2 h");
        assert_eq!(span(-5), "0 s");
        assert_eq!(ago(0, 30_000), "just now");
        assert_eq!(ago(0, 5 * 60_000), "5 min ago");
        assert_eq!(ago(0, 3 * 3_600_000), "3 h ago");
        assert_eq!(ago(0, 72 * 3_600_000), "3 days ago");
    }
}
