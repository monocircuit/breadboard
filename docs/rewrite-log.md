# The rewrite's decision log (#314)

Newest first, one entry per decision with the rule of
[`rewrite-rules.md`](rewrite-rules.md) it follows. The loop decides; the
user reads this instead of answering questions.

## 2026-10-01 20:10 · CLI / wt:action, wt:tick, wt:note, wt:mr, wt:wait · the command line
Chose: each command checks its whole command line before it reads or
writes anything: an unknown flag (`✗ unknown flag --dryrun`), a valued flag
without its value, a switch given one (`--update=yes`) or a word too many
is refused with the usage, exit 1. `--help` / `-h` print what the command
does and its usage on stdout, exit 0. `wt:mr <x>` refuses an `<x>` that is
no issue number at once. Parsed by hand, no clap. Against: Node's way of
reading only the flags it knows and running on.
Why: rule 6 over rule 3: in Node `wt:mr --dryrun` (or `--help`) installed
the push guard and ran `glab mr create --draft`, `wt:tick 307 3 --help`
ticked box 3 on GitLab, `wt:action look x --bogus` posted and dropped the
flag; a typo must never be a GitLab write nobody asked for. `wt:wait`
already refused unknown flags in Node and keeps its wording. Hand parsing:
rule 5 keeps `flagValue`'s exact reading (first occurrence wins, a next
`--x` is no value, `--flag=value`), which clap would change; rule 4: one
50-line checker, no dependency, start stays ~2 ms.

## 2026-10-01 20:05 · CLI / wt:mr · closed, merged, missing template, URL
Chose: a merged MR of the branch is refused (`!N of <branch> is merged
already: new work goes on a new branch …`); a closed one is history, the
branch gets a new draft MR (`--dry-run` previews it), and the output says
`!N … is closed: a new merge request was opened, that one stays closed`.
A missing `.gitlab/merge_request_templates/Default.md` is a `✗` line that
says what to write. When glab prints no URL after `mr create`, it is read
back with `glab mr view <branch>`; failing that the line says how to find
it. Against: Node's "open already" for any MR glab returns, its ENOENT
stack trace, its empty URL line.
Why: rule 3, the documented intent (parity spec §7, §11): "open already"
for a merged or closed MR is wrong, and a reopen had no way around but a
hand-written `glab mr create`; rule 6: the create is the same write Node
makes, only no longer blocked.

## 2026-10-01 20:00 · CLI / wt:wait · ends when it should
Chose: an MR GitLab answers 404 for ends the wait at once (`✗ GitLab has
no MR !99999 and !99998`, the wait cleared). Between asks it sleeps at most
until `--hours` runs out (`--hours 0.0001` ends after ~0.4 s, Node 60 s)
and looks every second whether `.claude/wait.json` is still its own: after
`--cancel` or a newer wt:wait it exits within a second (Node polled GitLab
up to another minute).
Why: rule 3, Node's intent: its 404 test read a field its errors don't
have, so a typo waited 8 h; rule 7: measured 92 ms for the 404, 433 ms for
the limit, 564 ms after a cancel. One ask a minute stays (rule 6).

## 2026-10-01 19:55 · CLI / wt:tick, wt:mr --update · the description written as text (#157)
Chose: `glab api --method PUT projects/:fullpath/issues/<n>` (and
`merge_requests/<iid>`) `--raw-field description=<text>`. Against:
`glab issue|mr update --description-file`, which glab 1.114 doesn't have.
Why: rule 2, #157's acceptance; the route GitLab sees is the same PUT
(rule 6). `--raw-field`, not `--field`: a body starting with `@` would be
read as a file.

## 2026-10-01 19:50 · CLI / wt:note, wt:action · small deviations
Chose: wt:note's read-back that is no JSON is `⚠ … GitLab does not show
it back` (Node crashed). `wt:action --orchestration` refuses a name outside
`NAME` (`../orchestrations/breadboard` was a path out of the records).
`wt:action --done=5` works like `--done 5` (Node printed the usage).
`--list` leaves out the time of an answered action without `answeredAt`
(Node printed `(undefined)`). A write that finds `<file>.lock` held says
on stderr what it waits for (`waits for …/actions.json.lock, held by
another writer (taken over after 5 s)`; Node was silent for 5 s).
Why: rule 3 says no deviation is silent; each is a crash, a path traversal
or a wrong line in Node, and the formats read by others stay (rule 5).

## 2026-10-01 19:45 · CLI / wt:mr · the push guard shim stays Node's for now
Chose: the pre-push shim wt:mr installs stays byte for byte today's
(`exec node "$top/packages/{breadboard,workflow}/src/git-hooks/pre-push.mjs"`).
Open: at the cut-over (plan step 6) the shim must call the binary, and
wt:new installs the same shim; both change in that one commit.
Why: rule 5: every worktree shares the hooks folder, and a shim calling a
binary that Node worktrees don't have would block their pushes.

## 2026-10-01 17:40 · CI · the suite against the binary
Chose: `check:breadboard-rs` (fmt, clippy `-D warnings`, `cargo test`, the
release binary) and `check:breadboard-gauntlet-rs` (the suite with
`BREADBOARD_SUBJECT` = that binary), the latter `allow_failure` until the
loop has built every piece. `check:breadboard` keeps the Node half (ESLint,
`node --test`) without cargo.
Why: rule 1, the suite is the spec, and it must run on every push; a job
that is red by design while the pieces are missing would only hide a new
failure, so it reports without blocking until the cut-over, then blocks.

## 2026-10-01 17:35 · workspace · two crates, one binary
Chose: a cargo workspace in `packages/breadboard/` with `crates/core`
(`breadboard-core`: records, actions, origin, git, herdr, GitLab, the
snapshot the views draw) and `crates/breadboard` (the binary: `tui/`,
`cli/`, the herdr plugin, the daemon). Against: the plan's five crates.
Why: rule 4: the TUI, CLI, daemon and plugin are one binary and share all
their data code; splitting them into crates adds manifests and no boundary
anyone reads. `core` stays apart so the readers are tested without a
terminal and compile once. The package's `lint`/`test` run cargo too, so
`pnpm wt:check` gates the Rust code like the Node code.

## 2026-10-01 17:30 · tools · `snap`, a screenshot for builders and critics
Chose: `pnpm --filter @monocircuit/breadboard-gauntlet snap`: any TUI
(Node, Rust, lazygit) in the gauntlet's pty and world, a script of keys and
mouse played in, the screen as text and as a PNG drawn by headless
Chromium, `--blind` masks the names.
Why: the critic compares screenshots and interaction blind (the loop's
prompt); one tool for both subjects keeps the comparison fair (same
terminal, size, font, palette).
