//! Local times as the Node overview prints them (`toLocaleTimeString` of
//! en-US, `en-GB` where it asks for it): `10:33:14 PM` in the title band,
//! `04:01 PM` on a node, `22:33` for GitLab's minute. libc's `localtime_r`
//! knows the zone and its daylight saving.

/// The parts of `ms` since the epoch in local time: year, month (1–12), day, hour, minute, second.
fn local(ms: i64) -> (i64, u32, u32, u32, u32, u32) {
    let seconds = ms.div_euclid(1000) as libc::time_t;
    // SAFETY: localtime_r writes into the tm we own; a zeroed tm is valid.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let done = unsafe { libc::localtime_r(&seconds, &mut tm) };
    if done.is_null() {
        let rest = ms.rem_euclid(86_400_000) / 1000;
        return (
            1970,
            1,
            1,
            (rest / 3600) as u32,
            (rest / 60 % 60) as u32,
            (rest % 60) as u32,
        );
    }
    (
        i64::from(tm.tm_year) + 1900,
        (tm.tm_mon + 1) as u32,
        tm.tm_mday as u32,
        tm.tm_hour as u32,
        tm.tm_min as u32,
        tm.tm_sec as u32,
    )
}

/// `10` and `PM` of a 24-hour hour.
fn twelve(hour: u32) -> (u32, &'static str) {
    let half = if hour < 12 { "AM" } else { "PM" };
    let hour = match hour % 12 {
        0 => 12,
        hour => hour,
    };
    (hour, half)
}

/// `10:33:14 PM`: the band's time (`toLocaleTimeString()`).
pub fn band_time(ms: i64) -> String {
    let (_, _, _, hour, minute, second) = local(ms);
    let (hour, half) = twelve(hour);
    format!("{hour}:{minute:02}:{second:02} {half}")
}

/// `04:01 PM` today, `Sep 30 04:01 PM` before (tree.mjs `clock`).
pub fn node_time(ms: i64, now: i64) -> String {
    let (year, month, day, hour, minute, _) = local(ms);
    let (hour, half) = twelve(hour);
    let time = format!("{hour:02}:{minute:02} {half}");
    let (this_year, this_month, today, _, _, _) = local(now);
    if (year, month, day) == (this_year, this_month, today) {
        return time;
    }
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!("{} {day} {time}", MONTHS[(month as usize).saturating_sub(1) % 12])
}

/// `22:33`: a minute in 24 hours (gitlab-status.mjs `clock`).
pub fn minute(ms: i64) -> String {
    let (_, _, _, hour, minute, _) = local(ms);
    format!("{hour:02}:{minute:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_times_as_node_does() {
        assert_eq!(twelve(0), (12, "AM"));
        assert_eq!(twelve(12), (12, "PM"));
        assert_eq!(twelve(22), (10, "PM"));
        let time = band_time(0);
        assert!(time.ends_with(" AM") || time.ends_with(" PM"), "{time}");
        assert_eq!(minute(0).len(), 5);
        assert_eq!(node_time(0, 0).len(), 8);
        assert!(
            node_time(0, 40 * 86_400_000).contains(' '),
            "{}",
            node_time(0, 40 * 86_400_000)
        );
    }
}
