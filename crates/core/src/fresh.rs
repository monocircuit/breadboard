//! When an agent starts fresh (#445): a resume reads its whole history
//! again, and once it idled past the prompt cache's hour that history is
//! written to the cache anew before it does anything. So an agent past
//! `[agents] fresh_above` tokens of context, or past `idle_fresh_above`
//! and idle longer than `idle_minutes`, takes no next issue: it retires at
//! its merge (or its handover is refused) and a fresh agent with its
//! handoff takes the issue. Replaces the share of the window of #381.

use crate::config::Agents;
use crate::time::span;

/// The rule, its numbers in tokens and ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fresh {
    /// Above this much context an agent always starts fresh.
    pub above: u64,
    /// Above this much, it starts fresh once idle `idle_ms`.
    pub idle_above: u64,
    /// How long idle its cache is gone: a little under its hour.
    pub idle_ms: i64,
}

impl Default for Fresh {
    fn default() -> Self {
        Fresh::of(&Agents::default())
    }
}

/// What a conversation holds: the context of its last answer and when it came.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Context {
    pub used: u64,
    /// When its last answer was written (ms), None before one.
    pub at: Option<i64>,
}

/// `162 k`: tokens in thousands.
pub fn k(tokens: u64) -> String {
    format!("{} k", (tokens + 500) / 1000)
}

impl Fresh {
    /// The rule of breadboard.toml's `[agents]`.
    pub fn of(agents: &Agents) -> Fresh {
        Fresh {
            above: agents.fresh_above,
            idle_above: agents.idle_fresh_above,
            idle_ms: i64::from(agents.idle_minutes) * 60_000,
        }
    }

    /// Why an agent holding `context` starts fresh at `now`, None: it may
    /// take its next issue. `its context holds 162 k tokens, above 150 k` /
    /// `idle 2 h with 120 k tokens of context: its cache is gone`.
    pub fn why(&self, context: &Context, now: i64) -> Option<String> {
        if context.used > self.above {
            return Some(format!(
                "its context holds {} tokens, above {}",
                k(context.used),
                k(self.above)
            ));
        }
        let idle = context.at.map(|at| now - at)?;
        (context.used > self.idle_above && idle > self.idle_ms).then(|| {
            format!(
                "idle {} with {} tokens of context, above {}: its cache is gone, a resume writes it all again",
                span(idle),
                k(context.used),
                k(self.idle_above)
            )
        })
    }

    /// `150 k, or 100 k idle 55 min`: the rule in a few words.
    pub fn text(&self) -> String {
        format!(
            "{}, or {} idle {}",
            k(self.above),
            k(self.idle_above),
            span(self.idle_ms)
        )
    }
}

/// Where an agent writes its handoff at the end of its last real run (#445):
/// when it posts its merge and may retire at it, so retiring needs no
/// wake-up of its own. Beside `.claude/handoff.md`, the one it got.
pub const HANDOFF_NEXT: &str = ".claude/handoff-next.md";

/// The handoff the agent of `dir` wrote at its last run, kept next to the
/// record of `name` (`handoff_file`): its path there relative to `main`,
/// None when it wrote none.
pub fn keep_handoff(main: &std::path::Path, name: &str, id: &str, dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(HANDOFF_NEXT))
        .ok()
        .filter(|text| !text.trim().is_empty())?;
    let kept = crate::record::handoff_file(main, name, id);
    std::fs::create_dir_all(kept.parent()?).ok()?;
    std::fs::write(&kept, text).ok()?;
    Some(kept.strip_prefix(main).unwrap_or(&kept).display().to_string())
}

/// What `wt:action merge` tells an agent that may retire at its merge
/// (#445): its context past `idle_above`, it is fresh past `above` or once
/// idle. The handoff is written now, in this run, while its cache is warm;
/// None for one that will take its next issue itself.
pub fn handoff_request(fresh: &Fresh, used: u64) -> Option<String> {
    (used > fresh.idle_above).then(|| {
        format!(
            "  Your context holds {} tokens: past {} a fresh agent takes your next issue, past {} once you idle {}. So write your handoff now, in this run, into {HANDOFF_NEXT}: at most one page on what you learned about the files you changed: where things live, the rules and pitfalls you found, what is unfinished or fragile. Facts a newcomer needs, not the story of your issue. Should you retire, breadboard keeps it; no run wakes you for it later.",
            k(used),
            k(fresh.above),
            k(fresh.idle_above),
            span(fresh.idle_ms)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn context(used: u64, at: Option<i64>) -> Context {
        Context { used, at }
    }

    #[test]
    fn a_large_context_starts_fresh_at_once_a_middling_one_only_once_its_cache_is_gone() {
        let fresh = Fresh::default();
        assert_eq!(
            (fresh.above, fresh.idle_above, fresh.idle_ms),
            (150_000, 100_000, 55 * MIN)
        );
        let now = 1000 * MIN;
        // Small: never, however long idle.
        assert_eq!(fresh.why(&context(90_000, Some(0)), now), None);
        // Middling: fresh only past the idle time.
        assert_eq!(fresh.why(&context(120_000, Some(now - 10 * MIN)), now), None);
        assert_eq!(
            fresh.why(&context(120_000, Some(now - 125 * MIN)), now).as_deref(),
            Some("idle 2 h with 120 k tokens of context, above 100 k: its cache is gone, a resume writes it all again")
        );
        // No answer yet: it can't have idled.
        assert_eq!(fresh.why(&context(120_000, None), now), None);
        // Large: fresh at once.
        assert_eq!(
            fresh.why(&context(162_400, Some(now)), now).as_deref(),
            Some("its context holds 162 k tokens, above 150 k")
        );
        assert_eq!(fresh.text(), "150 k, or 100 k idle 55 min");
    }

    #[test]
    fn asks_for_the_handoff_at_the_merge_only_of_one_that_may_retire() {
        let fresh = Fresh::default();
        assert_eq!(handoff_request(&fresh, 80_000), None);
        let asked = handoff_request(&fresh, 120_000).unwrap();
        assert!(asked.starts_with("  Your context holds 120 k tokens: past 150 k a fresh agent"));
        assert!(asked.contains("now, in this run, into .claude/handoff-next.md"));
    }

    #[test]
    fn keeps_the_handoff_it_wrote_next_to_the_record() {
        let scratch = crate::lock::scratch::Scratch::new();
        let main = scratch.join("platform");
        let dir = scratch.join("platform-7-x");
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        assert_eq!(keep_handoff(&main, "o", "s1", &dir), None);
        std::fs::write(dir.join(HANDOFF_NEXT), "# notes\n").unwrap();
        let kept = keep_handoff(&main, "o", "s1", &dir).unwrap();
        assert_eq!(kept, ".claude/orchestrations/o.handoffs/s1.md");
        assert_eq!(std::fs::read_to_string(main.join(kept)).unwrap(), "# notes\n");
    }

    #[test]
    fn its_numbers_come_from_the_config() {
        let agents = Agents {
            fresh_above: 300_000,
            idle_fresh_above: 50_000,
            idle_minutes: 30,
            ..Agents::default()
        };
        let fresh = Fresh::of(&agents);
        assert_eq!(fresh.why(&context(200_000, Some(0)), 10 * MIN), None);
        assert!(fresh.why(&context(60_000, Some(0)), 31 * MIN).is_some());
    }
}
