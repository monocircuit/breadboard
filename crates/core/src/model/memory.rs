//! The machine is the only limit on an orchestration's agents (#229,
//! worktree/memory.mjs): below the threshold a fresh agent starts on the
//! shared stack; the overview names the threshold.

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
/// What a worktree's own dev stack takes, and what a claude does (a
/// headless agent only while its run is in flight, `runs::RUN`: idle it is
/// no process and costs nothing, #358).
pub const STACK: u64 = 1 << 30;
pub const AGENT: u64 = 1 << 29;
/// Below this much free memory a fresh agent gets the shared stack.
pub const THRESHOLD: u64 = STACK + AGENT;

/// `3.2 GB`.
pub fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / GIB)
}

/// `MemAvailable` of a `/proc/meminfo` text, in bytes.
pub fn available_in(meminfo: &str) -> Option<u64> {
    meminfo.lines().find_map(|line| {
        let rest = line.strip_prefix("MemAvailable:")?.trim();
        rest.strip_suffix("kB")?.trim().parse::<u64>().ok().map(|kb| kb * 1024)
    })
}

/// The memory free for a new process, in bytes: Linux's `MemAvailable` (it
/// counts the caches that give way); None elsewhere.
pub fn free_memory() -> Option<u64> {
    available_in(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_linux_says_is_available() {
        assert_eq!(
            available_in("MemTotal:       32000000 kB\nMemAvailable:    2048 kB\n"),
            Some(2048 * 1024)
        );
        assert_eq!(available_in("MemFree: 1 kB"), None);
        assert_eq!(gb(THRESHOLD), "1.5 GB");
        assert_eq!(gb(3_435_973_837), "3.2 GB");
    }
}
