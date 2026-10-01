//! Local time, as `new Date(ms).getHours()` and friends see it: the day and
//! time of the changelogs (`## 2026-09-30`, `· 15:27`) are the reader's.
//! libc's `localtime_r` knows the zone and its daylight saving.

/// `(year, month, day, hour, minute)` of `ms` since the epoch, local time.
pub fn local(ms: i64) -> (i64, u32, u32, u32, u32) {
    let seconds = ms.div_euclid(1000) as libc::time_t;
    // SAFETY: localtime_r writes into the tm we own; a zeroed tm is valid.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let done = unsafe { libc::localtime_r(&seconds, &mut tm) };
    if done.is_null() {
        let days = ms.div_euclid(86_400_000);
        let (year, month, day) = civil(days);
        let minutes = ms.rem_euclid(86_400_000) / 60_000;
        return (year, month, day, (minutes / 60) as u32, (minutes % 60) as u32);
    }
    (
        i64::from(tm.tm_year) + 1900,
        (tm.tm_mon + 1) as u32,
        tm.tm_mday as u32,
        tm.tm_hour as u32,
        tm.tm_min as u32,
    )
}

/// The civil date of a day since 1970-01-01 (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// `2026-09-30`, local.
pub fn day_of(ms: i64) -> String {
    let (year, month, day, _, _) = local(ms);
    format!("{year}-{month:02}-{day:02}")
}

/// `15:27`, local.
pub fn time_of(ms: i64) -> String {
    let (_, _, _, hour, minute) = local(ms);
    format!("{hour:02}:{minute:02}")
}

/// Today, local.
pub fn today() -> String {
    day_of(breadboard_core::time::now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_the_civil_date() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_727), (2026, 10, 1));
        assert_eq!(day_of(0).len(), 10);
        assert_eq!(time_of(0).len(), 5);
    }
}
