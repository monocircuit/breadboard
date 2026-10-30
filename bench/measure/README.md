# Measuring the agents' time (#451)

Where the wall-clock time of breadboard's headless task agents goes, and
how often one change compiles breadboard. Two tools, both read-only for the
repo: `measure.py` reads transcripts, `builds.sh` works in a scratch
worktree of its own.

## Where an agent's time goes: `measure.py`

From the Claude Code transcripts of every agent the run engine started
(`<main>/.claude/runs/status.json`), each run split into what was pending
at each moment: the model (coding), checks & builds, waiting on others,
git, GitLab, file tools, … Definitions in the script's docstring.

```sh
sh fetch_gitlab.sh /tmp/breadboard-measure/data            # pushes and MRs since 2026-10-15
python3 measure.py --out /tmp/breadboard-measure \
  --orchestrator ~/.claude/projects/<main>/<orchestrator session>.jsonl
cat /tmp/breadboard-measure/summary.txt
```

| Option | What |
| ------ | ---- |
| `--windows <file>` | the periods compared and the holds (pauses, reboots) left out of idle time; default [`windows-2026-10-19.json`](windows-2026-10-19.json) |
| `--orchestrator <file>` | the orchestrator's transcript, for the queue latency (`runs tell` → run start); without it that line is left out |
| `--main <dir>` | the main checkout whose `.claude/runs/` and transcripts are read; default this clone's |
| `--out <dir>` | where `summary.txt` and the CSVs go, and `data/` is read from; default `$TMPDIR/breadboard-measure` |

**The 2026-10-19 table** comes back as it was with the default windows
and the orchestrator session `aa4b1937-9ca7-485a-ab15-bccdaf4f1ca8` of the
platform checkout: the windows file cuts every transcript at the time of
that run (08:04:22Z) and drops GitLab's pushes and merges after its fetch
(08:00Z). A re-measure gets a windows file of its own: a `BEFORE` and an
`AFTER` period (an `AFTER` without `to` runs until now, a
`gitlab_fetched` left out takes GitLab as it is), the holds from the
orchestration's journal.

| 2026-10-19 | BEFORE (16–18 Oct) | AFTER (19 Oct, after #446/#450) |
| ---------- | ------------------ | ------------------------------- |
| agent wall in clean runs | 53.0 h | 1.2 h |
| model (coding) | 11.0 % | 35.9 % |
| checks & builds | 75.1 % | 51.6 % |
| waiting on others | 11.8 % | 5.7 % |

**The 2026-10-30 re-measure** (#459): [`windows-2026-10-30.json`](windows-2026-10-30.json),
BEFORE as above, AFTER from the merge of !439 (#451) until the run, the
orchestrator transcripts `aa4b1937-…` and `453258af-…` concatenated into one
file. Since then `bb submit` (it runs `wt:check`) and a `test-runner`
subagent (it runs `wt:check` or the gauntlet) count as checks & builds, and
`timeout … tail -f /dev/null` as waiting; BEFORE comes out as before.

| 2026-10-30 | BEFORE (16–18 Oct) | AFTER (21–30 Oct, after #451) |
| ---------- | ------------------ | ----------------------------- |
| agent wall in clean runs | 53.0 h | 36.1 h (455 runs, 89 agents, 67 issues merged, up to 8 at once) |
| model (coding) | 11.0 % | 24.6 % |
| checks & builds | 75.1 % | 41.7 % |
| waiting on others | 11.8 % | 16.4 % |
| `wt:check` in the foreground, p90 | 600 s | 104 s |

Of AFTER's checks: `bb submit` 25 % (median 43 s), the `test-runner`
subagent 21 % (median 143 s), the gauntlet run directly 14 %, `eslint` 9 %;
vita's and lab's agents are in it too.

## How often one change compiles breadboard: `builds.sh`

```sh
sh builds.sh by-hand origin/develop   # the agent before #451
sh builds.sh one-build HEAD           # after it
sh builds.sh check HEAD               # wt:check alone
```

A fresh scratch worktree of the rev (detached, in `$TMPDIR`, removed
afterwards; `pnpm install --offline` first), its binary from the store as
`wt:new` leaves it. Then one change to both crates, twice (`cold`: the
worktree's first, `warm`: the next), each step timed with the units cargo
compiled in it, release (`breadboard`, `breadboard-core`) and debug (test
and clippy units). The flows are in the script's head; `GAUNTLET_FILE`
picks the check file the gauntlet step runs (its build is what counts),
`BENCH_OUT` keeps the table.

Measured 2026-10-19/21 on 32 cores with sccache's cache warm; a release
compile is one of the `breadboard` binary (its build script is a unit of
its own), `test:rs` and `wt:check`'s clippy compile the debug units either
way.

| Step | by hand (develop, before #451) cold / warm | one-build (#451) cold / warm |
| ---- | ------------------------------------------ | ---------------------------- |
| `cargo build --release` by hand | 87 s / 74 s, release compile | not run |
| gauntlet (`batch.check.mjs`) | 8 s / 8 s | 81 s / 75 s, the release compile |
| `test:rs` | 58 s / 17 s | 52 s / 18 s |
| `wt:check` after the commit | 82 s / 103 s, **release compile again** | 32 s / 31 s, **no release compile** |
| **release compiles of breadboard per change** | **2** | **1** |
| **the iteration** | 235 s / 202 s | 165 s / 124 s |

Why `wt:check` compiled again: the launcher compiles the key of the
sources into the binary (`BREADBOARD_SOURCE_KEY`, #339), and a build of
uncommitted sources had none, so the build after the commit compiled the
crate with another one. Now the key is that of the tree as built.
What is left: `test:rs` (debug, `--test`) and clippy (check mode) compile
breadboard's crates in their own modes; cargo shares no artifacts between
them and a release build, so each change still compiles the crates three
ways. `wt:check` runs both; an agent that runs `test:rs` itself before
only finds it fresh.
