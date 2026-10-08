# The rewrite's decision log (#314)

Newest first, one entry per decision with the rule of
[`rewrite-rules.md`](rewrite-rules.md) it follows. The loop decides; the
user reads this instead of answering questions.

## 2026-10-08 23:01 · overview / NEXT and ACTIONS, round 5 · no digit acts on a block the user didn't pick
Chose: with nothing picked a digit does nothing, NEXT's action included;
the digits answer only the block picked with Tab (NEXT's own is the first
Tab stop). NEXT's hint says so before its keys: `tab to it, then 1-2 picks
that option: typed to its agent`, `tab to it, then 1 looks good: …`; the
block's options and buttons carry no digits until then. Against: round 4's
"nothing picked: they answer NEXT's action" (the critic, `src/na5-stray.tmp.mjs`:
`2 more tests please⏎` typed into the overview by mistake ran `herdr agent
prompt … #2 decide … → OpenTUI on Bun`, moved #2 to answered[], and left
the try's `1 looks good 2 broken` keyed for the next stray digit), and a
digit that lights an option and waits for Enter (two ways to pick a row,
rule 4). Why: rule 6 (a write into an agent only on a deliberate input),
the bar (lazygit's single keys act only on the focused item of the panel
you are in). Rule 3 has nothing to keep: Node has no keys here. New key
binding: none; the digits keep their meaning on a picked block. Suite:
`buttons.check.mjs` +1 (`2 more tests please⏎` into the overview: no
answer in actions.json, no `prompt`/`send-text` to herdr; Node passes).
Chose: a decide's options are no longer inside the block's click: hover
on its head or question lights head and text only, each option lights on
its own (it answers that option; the block opens its space). Why: rule 1
(a lit area must claim what a click does), the critic.
Chose: the try's spot after a click keeps Node's two spaces before it in
every state (`the page opened  :3000/x`, NEXT's UP NEXT row too); it wraps
whole to the next line where both don't fit. Why: rule 3 (`trySpot`
after two spaces), the critic: one space read as one sentence.
Chose: an UP NEXT row whose click did all it does (the stale-plan `order`
row: the plan written) goes from UP NEXT and NEXT at once, NEXT shows the
next row still to do, until a read without it (at most 60 s, as a clicked
answer). Against: drawn until the next read (0.6 s and more). Why: rule
3 (Node drops it on the click).
Decided against: keeping a delivered answer's block with `→ <answer> ·
typed into its agent's pane` once the read has it gone. Why: rule 3
(Node's `shownAction` draws an answer only while its action is read, so
Node removes the block at once too), rule 4 (a phantom block outside the
read); the band's log line confirms it at once, quoted.
Left to the frame (not this piece's files; no write at stake): at 80x24
after `j`×5 ACTIONS' pinned title still sits over a try's tail (`a
terminal: cargo run --release`, `looks good  broken`). A click there no
longer answers: the frame shows the whole block first and runs nothing
(view.rs `show_clicked`; probed: actions.json unchanged, the try's head
back on screen). A block's head kept under the pin is the frame's to
build. Also the frame's: the scrollbar `┃` in the island's right padding
and in the blank row after the box.
Against the release binary: buttons 27/27, content 24/24, hover 3/4 (the
MR row's, MERGE REQUESTS' piece, in progress). cargo test of actions 19/19
(new: no digit unpicked, the tab hint, the decide's body without its
options, the spot's two spaces, the UP NEXT row gone at once), also with
`env -i`; clippy -D warnings and rustfmt clean in these files.

## 2026-10-08 23:05 · CLI / setup group, round 3 · Ctrl-C stamps no failure, one pane lookup a run, the binary's push guard, a refused claim says so
Chose: (1) wt:check judges turbo's end only when it wasn't stopped: Ctrl-C
reaches turbo and wt:check together, turbo dies of the SIGINT and the
50 ms watcher lost the race, so a stop was stamped `failed` (no failure
named) and said `✗ wt:check failed`. Now a stop (the flag, or turbo ended
by SIGINT/SIGTERM) is seen after turbo's end under one lock with the
watcher: `running` cleared, nothing else written, exit 130, as spec §4
and Node. Why: rule 3 and the rules' "a wrong fact": the overview showed
every interrupted check as failed and the hook named a failure with no
test. Suite: setup `Ctrl-C during wt:check is no failure` (SIGINT to the
process group mid-turbo; passes against Node); unit `a_stopped_run_is_no_failure`.
(2) wt:new looks up the pane it runs in once a run (`setup::herdr::
pane_here`): it asked three times (the orchestration here, the worktree an
agent leaves, the tab it moves), and when herdr told no shell the
lifecycle's own memo can't hold "none", so each was a scan of every pane.
With the lifecycle's 16-at-a-time scan (its round 3): agent wt:new in the
world's herdr, first line at 0.56 s (was 7.0 s), 96 `pane process-info`
(was 190; the critic counted 215), done in 1.4 s. Why: rule 7.
(3) wt:new and wt:mr install the binary's push guard now
(`cli::write::mr::SHIM` = `herdr::hook::PRE_PUSH_SHIM`, hooks/pre-push):
the pushing worktree's own binary, the Node hook of a branch from before
the rewrite, else the main checkout's launcher, else `✗ push refused: no
push guard`. Node's shim installed by our wt:new needed node at every push
and switched the #184/#298 guard off in silence (`exit 0`) once src/ goes.
Same marker, so Node's installer and ours each replace the other's: no
effect on the live workflow until the cut-over. Against: the 22:23 entry's
"in the cut-over commit" (one more thing that commit can miss). Why: rule
8, rule 2 (#298). (4) A claim origin refused is no longer `✓ <branch>
pushed`: `✓ <branch> not pushed (git push -u origin <branch>), worktree
ready`; a human's summary says `(from origin/develop, not pushed: git push
-u origin <branch>)`. Node says `pushed` under its own `! origin refused
the branch` (a wrong fact, found in this round's probe; not kept).
(5) `changelog`: a merged MR whose diffs GitLab answered without a file is
listed under `repo`, not left out: the critic's `Nothing merged in this
window.` over 42 merges came from the fake GitLab's empty `/diffs`, not
from the window (ours filters by `merged_at` before reading any diff, the
42 were inside it). An MR whose diffs failed stays out and is read next
time, as before. Why: a wrong fact (Node the same). (6) `changelog
--watch`: a title mark that doesn't fit beside the title is left out whole
(`] [ 1–9 jump · z Z fold` goes first at 60 columns), never cut into the
last cell (#304; parity, but a cut line). (7) `changelog:release` counts in
the singular: `with 1 entry, 1 fragment deleted`.
Decided, kept: (8) the `.env.local` stack comment says `written by
breadboard (pnpm wt:new, wt:stack)`, not Node's `written by
packages/breadboard/src/worktree.mjs`: that file goes with the cut-over,
and the line is read by people only (no reader in src/, crates/, hooks;
the variables under it are unchanged, rule 5). Rule 3 deviation, nothing
lost. (9) The #230 heal writes the tab bar's status as `herdr:install`
does (the launcher, `sh <plugin>/breadboard.sh --no-wait herdr status`)
when the package has the launcher, else only the gone path is replaced
(lifecycle's `plugin::with_status`, its round 3); the user's
`~/.config/herdr/config.toml` stays read-only and is named with the
command that writes it (memory ask-before-global-config). (10) wt:check
still refuses before the plugin check when there is no turbo: `turbo_of`
takes the worktree's or the main checkout's, so this refusal only comes in
a repo with no `pnpm install` at all, where wt:new (which installs),
stack:* and wt:remove heal; a refusal costs no herdr call (01:40, rule 7).
Suite: setup +2 (Ctrl-C above; `wt:new by an agent in herdr: its own tab
moves into the new worktree's space`, spec §5.4: herdr told the pane's
shell and hands back the space, `pane move … --new-tab`, `tab rename`,
`this tab moved into the space …`; passes against Node, 4 s, and ours,
2.4 s). Release: setup 31/31, cli 19/19 (+1 todo), lifecycle 12/12;
Node: setup 22 pass, 6 skipped (rewrite only), 3 todo. cargo test of
cli::setup, write::mr, herdr::hook 64/64, in a clean env too (the crate
472/475 there: three tests of tui/overview/worktrees.rs, another piece's
work in progress). Smallest changes elsewhere: `cli/write/mr.rs` (`SHIM`),
`herdr/hook.rs` (its `dead_code` allow and doc).

## 2026-10-08 22:58 · overview / worktree view, round 4 · what a click opens scrolls into view; open hunks follow Tab file by file
Chose: a click or Enter that opens a file's hunks (DIFF, `⚠ also changed
…`, DEVELOP's `✗ a merge conflicts in …`) answers `Flow::Show`: the frame
places it as Enter's, the clicked row kept and what opened scrolled into
view, the row going up no further than the top (`Held::showing`, the
frame's held block, `place_held`). A click that closes them, or runs
anything else, still keeps its row under the pointer. Before: every click
held its row, so a file in the bottom third opened 3 lines on screen and
the rest under `↓ 33 more` (the critic: README.md in DEVELOP at 120x40;
now `here` and `on develop` whole, gx-render's 15-line hunk and its develop
side whole, long.md's three hunks whole under its row). Against: the
pointer's row kept (a second click at the same cell closes, as Node's
toggles). Now a second click there hits what moved under it, at worst a
hunk line, which opens nvim and writes nothing. Why: rule 1 (the critic:
the click looked as if it did almost nothing), the bar (lazygit shows the
picked file's diff whole). No scroll code in the view: one `Flow` variant
(rule 4).
Chose: a click is the pointer's position: the run loop takes it as a move
before the click (`hover(Some(x, y))`), and the frame keeps the block the
click ran on (`View::clicked`) until the pointer moves, so Tab without a
pick goes on from it even when what the click opened scrolled another
block under the pointer. Before: only a motion report set it (the critic:
PgDn, a click on `M long.md`, Tab picked `monolithium/components`, `9 of
42`; now `18 of 42`, long.md's first hunk, and Enter runs `drop +10`).
Why: lazygit's click selects the row j/k move from; round 3's entry said
so and missed terminals that send no move first.
Chose: hunks that show follow the Tab pick: Tab (or Shift-Tab) onto
another file's row shows that file's hunks instead, without Tab stops, so
the next Tab goes to the next file (lazygit's files panel shows the diff
of whatever row j/k land on). Enter on that row makes them stops (`Tab:
hunk by hunk`); Enter again closes them, Esc closes them and drops the
pick. Nothing follows while no hunks show, so Tab through the whole view
opens nothing on its way. Before: Tab plus Enter per file and a stop per
open hunk (long.md: 3 Tabs before the next file). Against: a key that
jumps to the next file row (one more key to learn, Tab still stops on
every hunk). Why: the bar, rule 4 (Tab stays the one way to move).
Smallest change in the frame: `App::picked` (default: nothing),
`View::hold_pick`, run.rs gives a follow's rebuild the pick's held row.
Chose: the hunks' header names the keys where they act: `Tab: hunk by
hunk`, followed hunks `Tab: next file · Enter: Tab into its hunks`. Why:
at 80 columns the band drops `g G scroll · Tab Enter pick a row` (the
band's piece; its round now drops the pick whole, never `Tab` alone), so
the header is where a reader of this view learns the keyboard path once a
click opened a file. Not done here: DIFF's title hint (`click a file: its
changes …`) still drops at 80 columns, as Node's `fitParts` cuts it (00:30
entry): the count is the fact, the hint is not.
Chose: a DIFF file row whose name runs past the counts' column has its
`+a −r` on the next line, in the columns of every other row, never after
the name. Supersedes the 00:30 entry's "they follow it while they fit
beside it". Why: the critic: `A  292-overview-every-agent-has-name.md  +1
−0` at 80 columns read as broken next to the aligned rows; rule 1 (#304,
a count reads where its column says).
Tests: the frame `a_click_that_shows_what_it_opened_scrolls_it_into_view`,
`tab_after_a_click_goes_on_from_the_clicked_block_wherever_the_pointer_is_now`;
acts `open_hunks_follow_the_pick_file_by_file_and_enter_makes_them_stops`;
hunks `hunks_that_follow_the_pick_are_lines_to_click_not_stops_of_tab`; diff
`a_listed_file_longer_than_the_column_has_its_counts_under_it_in_their_columns`.
Probed in the critic's rich world (`.claude/gauntlet/tmp/wtv4/`, shots a2–a7):
120x40 PgDn, a click on README.md in DEVELOP shows both sides whole; a
click on `M long.md` then Tab picks `18 of 42`; Tab ×4 shows notes.md's
hunks under the pick, Tab tanstack's reader.ts, Enter makes them stops,
Tab picks its hunk, Esc closes; 80 columns: every count in its column.
Suite (release): views, buttons, hover 46/46.

## 2026-10-08 22:50 · cli-lifecycle, round 3 · a wait hangs under its own step; a refusal says how to go on
Chose: (1) a call that may wait runs as a step that is said only when it
waits (`proc::as_step`): past 1.5 s its `▸` comes first, then `waiting for
…, at most N s`, so a wait never hangs under the line before it (the
critic: `waiting for herdr (workspace list)` under `▸ delete … on origin`
read as the push hanging). wt:remove: `▸ herdr: close the tabs of
<feature>` (`close the space` with `--close-space`), `▸ GitLab: is an MR
open on <branch>? (its claim on origin, when it was merged)`, `▸ GitLab:
which MR <branch> has`, `▸ git fetch --prune`, the changelog's rewrite;
the handover: `▸ herdr: is <agent> idle?`, `▸ GitLab: issue #N`, `▸ herdr:
<agent> into the space <feature>`; the plugin check. A run that doesn't
wait prints what it printed before. The `/rename` wait says `at most 20 s`
(2 × 10 s, typed twice). Against: a `▸` before every call (noise on the
fast path, lazygit says nothing while nothing waits). Why: rule 1 (the
critic), the bar. Measured (herdr hangs): the step at 1.5 s, the ✓ at
10.2 s as before.
(2) The refusal while a claude stands in the worktree names its pid and
how it ends: `some-agent (claude 4242) still stands in <feature>, handed
over nowhere: end it (/exit in its tab, or kill 4242) or let it leave the
folder, then again`; a handed-over one keeps `it enters its next worktree
…`. Why: rule 3 (Node said `claude <pid>`), the critic.
(3) A `handover --dry-run` says which brief applies under the next-issue
line (#247): `with the brief below, no brief of its own`, or `…, the brief
of <file> | the brief kept for it under ## From the orchestrator`. Why:
rule 2 (#247).
(4) The layout lock is a guard (`layout::Held`): a panic inside a layout
lets go of `$TMPDIR/platform-herdr-layout-<uid>.lock`, else every wt:new,
overview and wt:herdr waited up to 60 s. Why: the critic, rule 7.
(5) `wt:action change ""` (or blanks) says one line, `✗ what is the change?
pnpm wt:action change <what>`, not the 12-line usage (the smallest change
in `write/action.rs`, `change_text`). Against: Node's usage (rule 3
deviation, nothing lost: `pnpm wt:action --help` has it). Why: the bar.
(6) wt:herdr's `#332` pass leaves the tabs it just closed out of its
order (`keep_order_except`): no `pane move <ws>:p2 --new-tab` of a closed
tab's pane, whatever a slow herdr (or the static fake) still lists. The
fake stays as it is (it never was ours to fix there). Measured: wt:herdr
96 herdr calls (was 117), no `pane move` in calls.jsonl.
Decided, not changed: a handover whose `/rename` never takes (#313) still
writes the record's `workers[id].name` as the new feature, with `was` the
old name, `failed[]` the reason, no `recordStart`. Against: keeping the old
name until the rename took (the critic's first option): nothing reads the
session's name back into the record, so once the user types the two lines
the ✗ names, the record would say the old name for good and `--to <new>`
would find no agent. The ✗ and `failed[]` say the rename is pending; the
overview showing `was` for an agent whose last failure is its `/rename` is
the overview's to decide. Why: rule 5 (the record other sessions read
stays right once the given lines are typed), rule 4.
Also in the round: wt:remove refuses a folder its parent doesn't let this
user delete before anything goes (`failed to delete <dir>: Permission
denied`; git deleted the files and unregistered it, then failed, the
branch left behind); `herdr::current_pane` asks the panes' shells 16 at a
time and remembers the answer for the run; the plugin heal writes the tab
bar's status as `herdr:install` does.
Suite: `lifecycle.check.mjs` +1 (critic's item 3: `wt:action change` with
GitLab refusing the connection in a worktree with an open MR: `✗ … GitLab`,
not `no MR yet`, the actions untouched; knownBug "to file", Node says `✓
change requested … (no MR yet)` and exits 0). Not filed from the loop (no
GitLab writes): the lead files it and puts the number in.

## 2026-10-08 22:34 · herdr-plugin, round 5 · everything that starts the binary on its own goes through herdr/breadboard.sh, which builds it from HEAD first
Chose: (1) `herdr/breadboard.sh`, a launcher next to the manifest: every
`[[startup]]`, `[[actions]]` and `[[panes]]` entry of `herdr-plugin.rs.toml`
runs `["sh", "breadboard.sh", …, "herdr", <action>]` (a program on PATH and
a file of the plugin's directory, the shape `node run.mjs` had: no reliance
on how herdr resolves a relative program), the tab bar's status in the
configs `sh <plugin>/breadboard.sh --no-wait herdr status`, the Stop and
pre-push shims the main checkout's launcher. It runs
`target/release/breadboard` of its package, built first (`cargo build
--release`, progress on stderr: in the popup of a pane) when it is missing
or older than HEAD: `target/release/breadboard.source` holds `git ls-tree
HEAD -- crates hooks Cargo.toml Cargo.lock` of the build, so a pull by hand,
a fast-forward by any command or an upgrade from the Node plugin is seen at
the next key, never only after `--reload`. `--no-wait` (the tab bar's 3 s,
an `open` action, the Stop hook's 10 s) never waits: the binary from before
answers and one build runs behind it (a lock dir, one at a time; sources
that failed once are not built again behind, a key builds them in front and
shows why). Missing with no build possible: one line (`breadboard isn't
built: pnpm build:rs`) in the tab bar instead of a command error every
10 s, `✗ breadboard can't run until it is built …` for a key. A package
without Cargo.toml (a world, a shipped binary) runs the binary it has.
`pnpm herdr` / `--reload` build through the same launcher (`--built`), the
reload after a pull that moved develop then runs the pulled binary with
`--pulled`. Against: the manifest running `../target/release/breadboard`
directly (the critic: every action ENOENT after an upgrade, keys dead with
no word, the tab bar an error every 10 s; a stale binary after any pull not
made by `--reload`), and a build in the action/status itself (herdr's
timeouts; an action's output only reaches `plugin log`). Why: #230's
acceptance "moving the package again needs no hand work" (rule 2), rule 3
(Node always ran the pulled source), rule 5 (the manifest and the configs'
status are read by herdr: the old status forms, `node …/run.mjs status` and
`…/breadboard herdr status`, are still recognised and rewritten in place).
What stays outside Rust: this sh file (rule 8: no runtime besides sh, which
herdr and git already need).
(2) The cut-over (the lead's commit) must also: move `herdr/cutover/run.mjs`
to `herdr/run.mjs` and `herdr/cutover/herdr.mjs` to `src/herdr.mjs` when
src/ goes (thin Node shims that run the launcher: the one Node reload that
pulls the cut-over re-execs the pulled `src/herdr.mjs --reload --pulled`,
the old manifest and old tab bar lines call `node run.mjs …` until the
reload relinks and rewrites them); make the root scripts `"herdr": "sh
packages/breadboard/herdr/breadboard.sh herdr"` and `"herdr:install": "sh
packages/breadboard/herdr/breadboard.sh herdr --install"` (a fresh clone
builds before it runs); rename `herdr-plugin.rs.toml` to
`herdr-plugin.toml`. The shims go once every checkout has reloaded. Why:
rule 5 (a reader for the old form), the critic's deadlock (a) and (b).
(3) Bare `breadboard herdr` started by herdr as a plugin command (any of
`HERDR_PLUGIN_ID`, `_STATE_DIR`, `_CONTEXT_JSON` set) is refused with the
usage instead of setting a session up. Why: rule 6 (no window, no attach
without a deliberate input).
(4) `--reload`'s `is_overview` no longer splits a one-string command line
on spaces: `<path with a space>/breadboard overview` is an overview and
gets its `q`. Why: an overview left on old code with no word is a wrong
fact (rule 1 of the triage).
Tests added (unit, next to the code): the launcher (missing/older/fresh,
no cargo, a failed build, the build behind the tab bar, no sources), the
cut-over's Node shims (run when node is on PATH; a clean env has none and
says so), the hooks' shims through a launcher, `pull_develop` (up to date,
dirty, another branch, fast-forward, diverged), the reload's rerun of the
pulled build (its exit code, a failed build goes on here), ensureBinary's
sha256 check (nothing written on a mismatch), the Omarchy edge package of
a pacman db, `is_overview` with spaces, the plugin command without an
action. Suite: `plugin.check.mjs`'s manifest check reads the launcher's
shape (`sh breadboard.sh [--no-wait] herdr …`, every `open` `--no-wait`);
the world's plugin directory gets `breadboard.sh` with the manifests.
Not done here: `--here` attaching and the new window's spawn need a real
terminal and herdr (a unit test covers `terminal_window` only); restore's extra `pane move`s and `no id back` line belong to
the layout piece (open there); the commands pane's `platform (root)` to the
commands piece.

## 2026-10-08 23:00 · overview / WORKTREES, round 4 · #332's app and stack act, the merge result and the migrations stand on the node
Chose: a node's app and stack cells are clicks of their own (#332 takes the
worktree tab and with it WHERE's and STACK's rows): the app (`● :3007`,
`○ :3007`, `⇒ :3000`) opens `http://localhost:<port>` in the tab group of
the worktree's space (WHERE's `openInGroup`, else xdg-open) when it answers
at the click, else the band says `pnpm dev in products/vita/web starts the
app of <it>` (`Act::OpenApp`, asked at the click: the dot may be 2 s old);
an own stack docker told isn't all running reads `▶ stack down` / `▶ stack
exited` and a click runs `stack:up` detached in that worktree (INIT_CWD,
`$TMPDIR/overview-stack-<basename>.log`, the actor's `tries::start_stack`,
the same run the view of everything and a try use), the cell says `◐
starting …` while it runs, `stack …` once it ended until a docker answer
read after its end tells (`Run::done_at` against the node's `docker_at`:
never `▶ stack down` again for the 12 s docker is held), and a failure
turns the cell red with `✗ the stack didn't start: <why>` under the facts.
A running, shared, unknown (`…`, `?`) stack is no button: the node's click
(its space). Each a button region on its words, so hover lights it alone
inside the lit node. No `u` key here: with many nodes it would have to pick
one (rule 4, one way: the click on the node's own cell). Against: the cells
as text only (the critic: the tab #332 removes took the only way to act on
them), an armed second click (Node started on one click; a start is no
loss). Why: rule 3 (WHERE's and STACK's clicks), rule 2 (#332).
Chose: two lines the worktree view alone had stand under a node's facts,
only where there is something to do: DEVELOP's merge result `✗ a merge of
develop conflicts in a, b +N more` (red, `git merge-tree`, each name a click
into that worktree's nvim like the collision line's; read only where a file
collides with develop: a conflict needs both sides to change it), and
STACK's migrations `⚠ 2 migrations not applied: 0041_a, 0042_b +N more  ▶
pnpm pg:migrate` (the button runs it detached in its vita, `◐ migrating …`
with its log's last line, its error red after it; its end forgets what core
holds, and the line stays away until a read after it) or `⚠ the database
doesn't answer: <why>`, asked only where the database runs (its own stack
all running, or the shared one). A clean merge says it on develop's part of
the collision line: `⚠ collides with develop, merges cleanly: tree.mjs`; a
conflicting file is named once, on the red line, not on the yellow one.
All applied, or no psql or journal: nothing. Against: `✓ merges cleanly`
and `✓ all N applied` on every node (a row each on every node, the critic's
80x24 already short), the collision line alone (both sides changed a file
is not a conflict). Why: rule 3 (DEVELOP's and STACK's lines, now without a
view of their own), the bar (lazygit marks the exception, not the norm).
Chose: the migrations are asked apart from git's held facts: core's
`develop::migrations_held`, at most once in 15 s per worktree or when its
journal changed (develop.mjs's cache), on every local read of the
orchestration (`reader::read_orchestration`, beside docker's answer), psql
with a 3 s connect limit. Against: in `with_orchestration_facts` (held by
the worktree's git key: a database migrated by hand would never show).
Why: rule 3 (Node's 15 s), rule 7.
Chose: a worktree's app counts as up only where a process running in that
worktree listens on its port (core's `local::is_app_of`, Linux's /proc;
where /proc can't tell, the port alone as before). Against: anything on
localhost:<port> (the critic: `●` next to `stack down` in a world whose
ports the host's own dev servers held). Why: a wrong fact (rule 3's intent:
WHERE's dot is this checkout's app).
Chose: why there is no app and no stack (`no .env.local in any worktree: no
stack, the app can't run there`, the old STACK's words) or why stacks are
`?` stands on a line of its own under the record, not in the title's note.
Against: the note (a title wider than its line drops it whole: at 120 with
`9.9 GB free (shared stack below 1.5 GB)` it was gone). An env-less node
beside others with one keeps `—  no .env.local`, a shared one `⇒ :3000
shared`: told apart on the node.
Not here: (1) Node's #332 design is still not on develop
(`origin/feature/332-…` has no commit past it, `git log origin/develop
--grep '#332'` is empty): built from the requirement and the worktree
view's rows; re-check once it lands. (2) At 80x24 a colliding node is 6
rows (was 7): its title wraps, and #304 (rule 1) says a title is never cut.
(3) A click on a colliding file without nvim on the PATH puts `✗ nvim: no
nvim on the PATH` in the band (21:55's decision): a deliberate click's
answer goes where every act's goes; DONE keeps what it pushed aside.
Suite: new `worktrees.check.mjs` (8 checks, `knownBug("#332: …")`, todo
against Node: 8/8 todo): distance, app and stack as docker tells, the
collision both ways, the conflict, the app's and the stack's click (not the
node's), a shared and a running stack keep the node's click, the env-less
line once. Release: worktrees 8/8, content, hover, isolation 33/33. cargo
test: worktrees 18/18 (new: the cells' clicks, the stack's start, its wait
for docker and its failure, the migrations line, the merge result), core's
held migrations, /proc's listeners.
Smallest changes outside the piece: `act.rs` (`Act::OpenApp`, `Act::Migrate`
routed outside the worktree view), `actor/tries.rs` (`migrate`; a stack
started well is kept with `done_at`), `actor/spawn.rs` (`Run::done_at`),
`actor/mod.rs` (`migrations`), core's `reader.rs` (the migrations beside
docker), `local.rs`, `model/develop.rs`, `model/tree.rs`, `model/mod.rs`.

## 2026-10-07 22:30 · loop · a win is the critic's blind pick and a green suite
Chose: a piece wins when its critic picks ours blind and the suite's checks
touching it pass, as the loop's prompt says. Parity and requirement gaps no
longer veto a blind pick by themselves: the critic weighs them as a
developer would (a wrong number or a wrong write still makes the other the
better tool) and names them either way, for the next builder round. Each
piece's loop now starts with the critic, on the state as it is.
Against: the loop's first rule, "ours wins blind AND parity complete".
Why: the user's prompt (the critic says which a developer would rather
work in); under the stricter rule about half the critics picked ours blind
and still marked it lost for a missing log entry or a doubled mark, so no
piece could win short of perfect. Parity stays enforced by the suite
(rule 1) and rule 3.

## 2026-10-07 22:31 · overview / WORKTREES and AGENTS, round 3 · git's facts never wait for docker; a stack docker hasn't told is `…`, never `down`
Chose: the orchestration's local read asks docker on a thread of its own
(core's `reader::Stacks`): the nodes draw with git's facts at once, an own
stack says `stack …` (dim) until an answer is there, `stack ?` (yellow, the
title `docker does not answer: the stacks are unknown` once) when docker
didn't answer in time; `stack down` only when docker said so. An answer
another process wrote to the shared `containers.json` is taken at the next
look (≤ 2 s; the file is in the read's stamp), never after this process's
own 12 s. `--once` waits for it (`should_wait_for_docker`). The stack column
keeps the room of `stack running` from the first frame, and whether the
cells fit beside the names is chosen by the widest cells, so the answer
moves no line. Against: the read blocking on `local::containers()` (the
critic: a second overview showed `+1 down` for 28 s while the first said
`running(2)`; one overview with a 6 s docker stayed on `reading the
worktrees …` for 6.2 s), a `None` that meant both down and unknown. Why:
#331 and #332 (rule 2), the bar (lazygit fills a panel in place, never
moves rows under you). Measured (critic's probe, docker 5 s, a second
overview 300 ms later): nodes at 97 ms with `stack …`, the stacks at 5.3 s
(was `down` from 2.1 s to 30.2 s); one overview, docker 6 s: nodes at
177 ms (was 6.2 s), stacks at 7.1 s, no other frame change.
Chose: WORKTREES says a stack in words, without its `+N` (the app's port
says it): `stack running`, `stack exited`, `stack 2 running, 1 exited`,
`stack down`, `shared` (core's `tree::stack_of`). Against: everything's
`+7 down` / `exited(1)` (read as `7 down` and an exit code). Why: the bar;
rule 3 untouched (Node's tree has no stack; the view of everything keeps
`stack_cell`).
Chose: `N changed` counts files (`git status --porcelain
--untracked-files=all`): a new folder of 7 files is `7 changed`, not 1.
Against: lines of `git status` (Node's tree too). Why: rule 3 in its intent
(Node's worktree view, which #332 folds into this node, said `N
uncommitted` files; actions say `N uncommitted files` from the same count).
Chose: the facts cut at the agent end without ` ·` (`#305 · !318 draft …
pending    ● Niklas Kovač …`): the agent isn't the next fact. Why: the
critic.
Chose: a free agent named after the worktree it came from says only its MR
(`0 % · from !310 · 1 issue`); another worktree keeps `from <it> (!310)`.
Against: Node's repeat (rule 3 deviation, nothing lost: the name is on the
row's first line).
Smallest changes outside the piece: core `local.rs` (`shared_file`,
`read_containers` public), `model/mod.rs` (`Docker` on `Tree`),
`everything.rs` (`stack_cell` says `…`/`?` for an unknown stack), `git.rs`
(`changed_count`), `free_agents.rs`, `overview/mod.rs` (`--once` waits).
Suite (release): content 24/24, hover 4/4, views 11/11, buttons 25/25,
isolation 4/4. cargo test core 246/246, worktrees 14/14 (new: docker
answering late moves nothing at 66–120 columns), clippy -D warnings, rustfmt
clean. Still open: #332's Node design isn't on develop (re-check once it
lands); at 80x24 a colliding node is 7 rows.

## 2026-10-07 22:30 · overview / NEXT and ACTIONS, round 4 · a digit answers the lit block, never another
Chose: the digit keys follow the Tab pick, as lazygit's keys act only on
the focused menu and its highlighted row. Nothing picked: they do nothing
(since round 5, 2026-10-08 23:01 above; this round still had them answer NEXT's
action). An action's block picked (in ACTIONS, or NEXT's own, or
one of a decide's options): they answer that action's own answers (try
1/2, plan 1/2, decide 1..n, look 1). Anything else picked (the chat, a
worktree, an UP NEXT row, an MR): a digit does nothing. Against: digits
always on NEXT (round 3: Tab onto ACTIONS' try, then `2`, typed NEXT's
decide `→ OpenTUI on Bun` into the agent, irreversibly). Why: rule 6 (a
write to an agent the user did not aim at), the bar (lazygit); Node has no
keys here, so rule 3 has nothing to keep. Suite: `buttons.check.mjs` +1
(Tab onto ACTIONS' try, `2`: no decide in actions.json, no `#2 decide` in
calls.jsonl, the try answered `broken` where a pick lit it; Node passes,
it picks nothing).
Chose: the keys are drawn where they act: the picked block's buttons carry
their digits (`1 looks good` `2 broken`, cyan, as NEXT's) and under it a
hint line names each key and what `enter` does there (`enter runs its
try`; on a decide's option `enter picks "OpenTUI on Bun": typed to its
agent`), lazygit's `Confirm: <enter>`. NEXT, while another block is
picked, draws its options and buttons without keys and drops its hint, so
the screen never shows two blocks' digits at once. Why: the bar, rule 1
(the critic: the picked try showed no keys).
Chose (frame, the smallest change): `Input::Key` carries the block picked
with Tab; `Page::picked()` gives it to the build; a new pick builds the page
again, and a pick whose block is gone builds it once more without its
keys. Action blocks click as `Act::Action { key, go }` (the item's key), so
two actions of one agent (both `opens its agent's tab`) are two picks.
Why: rule 4 (the app knew no pick; one field, one variant).
Chose: a ✓ clicked here counts as `1 ticked off` in ACTIONS' note until the
read has it gone, not `answered, on its way`: a tick-off goes to no one.
Why: rule 1 (the critic, `src/na4-tick.tmp.mjs`), its own words (`its
agent is not told`).
Against the binary: buttons 25/25, content 24/24; cargo test of tui 208
green, clippy -D warnings, fmt clean in these files.

## 2026-10-07 22:40 · daemon, round 3 · docker's shared answer is keyed by the docker that would answer; wt:list reads it too
Chose: (1) the file docker's read is shared through is
`$XDG_RUNTIME_DIR/breadboard/containers-<key>.json`, the key a hash of the
`docker` PATH runs (resolved through its links), DOCKER_HOST,
DOCKER_CONTEXT, DOCKER_CONFIG and HOME (its `.docker` holds the current
context), as the serve socket is keyed by its environment (core
`local::containers_file`, tested next to `containers_name` and
`docker_on`). Against: `containers.json` keyed only by DOCKER_*, which
every process with the same XDG_RUNTIME_DIR shared whichever docker its
PATH ran: the critic's probe had world 2 (its docker knows no stack, 0
docker calls) print world 1's `vita-77 … docker compose -p vita-77 down -v`,
and `/run/user/1000/breadboard/containers.json` held a world's empty fake
answer twice during the loop, so the user's overviews would have shown no
stacks for up to 12 s. Probe again: world 2 now calls its own docker and
shows no vita-77. The old `containers.json` is no longer read (no other
reader of it exists). Why: the safety rule of requirements.md (a world
never reads or feeds the real docker), rule 2 (#331: one shared read per
docker, not per machine).
(2) `wt:list` counts a worktree's running containers from the same shared
read (one `docker ps` for every stack, none while another process's read
is fresh; never the machine's docker from a unit test), not `docker
compose … ps -q` per worktree with its own stack. The line is unchanged
(`own +1 (pg 5433), 2 containers up` / `stopped`); a try in a world with
`vita-1` running two containers and one exited printed exactly that with
one docker call. Smallest change outside the piece: cli/read/list.rs.
Why: rule 2 (#331: never per stack), rule 3 (the same line).
Not changed: the herdr plugin's `status` (the herdr-plugin builder moved it
onto the same file in its round 4, see 22:23); the `--herdr` overview's
per-space herdr lists (cli/lifecycle/herdr.rs, lists no other overview
asks for). docker's cadence: a local read every 2 s finds the file old
after 12 s, so the shared read runs every 12–14 s (measured 4/min).
Measured (critic2-daemon.tmp.mjs, 5 overviews, 60 s after 25 s warm-up,
3 own stacks): distinct mix 63 calls/min (herdr 43, GitLab 16, docker 4),
CPU 1.00 % own / 7.27 % with reaped children (Node: 515/min, 23.97 % /
115.10 %); live mix with the `--herdr` overview 282/min (herdr 262, GitLab
16, docker 4), 1.50 % / 22.22 % (Node: 1808/min, 22.68 % / 176.55 %).
Suite (release): budgets 11/11, content 24/24, views 9/9, cli 19/20
(+1 todo). cargo test core 246/246, daemon and list 13/13, rustfmt clean;
clippy of the binary fails only on cli/lifecycle/herdr.rs (another piece's
work in progress).

## 2026-10-07 22:23 · herdr-plugin, round 4 · the tab bar's status reads docker through the shared file; the hooks' shims of the binary
Chose: (1) `breadboard herdr status` counts the running vita stacks from
core's shared read (`local::containers()`, `containers-<key>.json` in
`$XDG_RUNTIME_DIR/breadboard`, fresh for 12 s, taken under its lock), not
its own `docker compose ls`: no docker call while another process's read
is fresh (the overview daemon's, another herdr server's), one `docker ps`
by the lock holder otherwise. A stack counts as Node's `compose ls` listed
it: a project named `vita…` with a running container; the line is
unchanged (`⬢ 2 stacks · 2 need you`). Against: today's poll, 6 docker
calls a minute per herdr server on top of the daemon's 4-5. Why: rule 2
(#331: one shared docker read every 10-15 s, never a poll per process).
(2) The suite's status checks pin "at most one docker read and no GitLab"
(was "exactly one docker call"), each run in a runtime dir of its own; the
fake docker answers `ps --format <template>` and `compose ls` from one
container model (`containers` in docker.json), so Node and ours read the
same world. New: two more runs within the 12 s make no docker call
(known bug #331 for Node, which polls). Why: rule 1 (what the check pins
changes, not how much), rule 2.
(3) The pre-push shim of the binary, `packages/breadboard/hooks/pre-push`
(core of step 5; `herdr::hook::PRE_PUSH_SHIM` includes it): the pushing
worktree's own `target/release/breadboard hook pre-push`, else the Node
hook of a branch from before the rewrite, else the main checkout's binary
(a worktree of the binary isn't built by wt:new), else `✗ push refused: no
push guard to run (#184, #298) … pnpm build:rs`, exit 1. The marker line
stays, so both installers recognise it as theirs. Against: today's shim's
`exit 0` when no hook is found (once src/ is gone, the #184 guard and the
#298 gate would switch off in silence), and the 09:20 entry's `hook
--help` probe (the setup group's `hook` refuses `--help`). Not switched:
`SHIM` of cli/write/mr.rs (also wt:new's) becomes `crate::herdr::hook::
PRE_PUSH_SHIM` in the cut-over commit. Why: rule 2 (#298: nothing reaches
GitLab unchecked), rule 5 (older branches keep their Node guard), rule 8.
(4) Claude Code's Stop hook at the cut-over runs `sh
"$CLAUDE_PROJECT_DIR/packages/breadboard/hooks/stop-answers"` (replaces
decision 7's inline line): the worktree's own binary, else the main
checkout's, else exit 0 in silence. A worktree not built yet now gets its
answers handed over too, not only "blocks no stop". settings.json is per
branch, so an older branch keeps its Node line. Why: rule 3 (answers are
handed over in every worktree, as today), rule 4 (one file the check runs
byte for byte, not a line to copy).
Suite: plugin.check.mjs 24/24 ours (+3: the #331 shared read, pre-push
through the main checkout's binary refusing an unchecked commit, the Stop
shim as settings.json runs it; the status checks rewritten as in (2));
Node 17 pass, 5 skipped (rewrite only), 2 todo (#331, #230).
cli.check.mjs 19/19 (+1 todo). Smallest change elsewhere: the fake
`src/fakes/docker.mjs` (the container model; without one its answers are
as before).

## 2026-10-07 22:07 · cli-lifecycle, round 2 · a herdr that does not answer is no refusal of the agent
Chose: (1) the handover tells "herdr did not answer" from "herdr knows no
pane with its conversation": `pane list` that hangs past 10 s, fails or
answers no JSON ends it with `✗ herdr did not answer in 10 s (herdr pane
list): can't tell whether <agent> is idle; nothing changed, run the same
command again once herdr answers`, before GitLab is read; no
`recordFailure`, no setup claim, the record byte for byte as it was, nothing
typed. After the setup (#272) the same silence removes the worktree again
(no worktree without its agent) and writes the issue's `failed: <why>`, but
nothing into the agent's `failed`. Against: today's refusal (the critic's
probe: a false `no claude runs it` kept in a rule-5 file that the overview
and later handovers read). Why: rule 3 (spec 2.5: the refusals are the
agent's states, herdr's silence is none of them), rule 6.
(2) A process that found herdr silent tells the commands it starts
(`BREADBOARD_HERDR_SILENT=1`: the handover's wt:new and its undo's
wt:remove, a removal's changelog rewrite): they don't wait 10 s for it
again (the undo after a silent setup took 31 s, now 22 s, of them 10 s in
wt:new's own layout). Why: rule 7.
(3) `wt:herdr`: the step `▸ herdr: the layout of …` comes before the plugin
check, so a wait shows under the step it belongs to; a herdr that hung
says `✗ herdr did not answer in 10 s: nothing was laid out; pnpm wt:herdr
again once it answers`, `is its server running? (herdr status)` only for
one that failed at once. Why: rule 1 (the critic), rule 3.
(4) The read of a merged branch's MRs after its worktree is gone (only the
claim on origin and the record's `mergedAt` wait for it) gets 8 s, not 15:
a silent GitLab cost a removal that was done 15 s; late, both are said and
left (`kept <branch> on origin: GitLab did not answer …; by hand: …`).
Against: after the ✓ (the overview reads a command's last line as its
outcome). Why: rule 7, rule 4.
Not changed: `wt:action change` with GitLab refusing connections names
glab's crash (`glab failed: node:internal/…`): the world's fake glab
crashes where a real one says `dial tcp … connection refused`; `change.rs`
and `write/glab.rs` belong to the write group, whose builder edits them now.
Suite: `cli.check.mjs` +1 (the handover, herdr never answers: stderr names
herdr, no `no claude runs it`, the record byte for byte, no worktree of the
issue, no `herdr agent …` call; knownBug "to file", today's waits without
end: todo against Node, 40 s). Against the binary: cli 19/19 + 1 todo,
lifecycle 10/10; Node: the new check todo. cargo test of the lifecycle
group green (`a_failed_pane_list_is_no_empty_one` new), clippy -D warnings,
rustfmt clean.

## 2026-10-07 22:40 · overview / worktree view, round 3 · DIFF and its hunks follow the disk; the open file is marked; every name reachable by key
Chose: DIFF's cache key (core `DiffCache`) is HEAD, origin/develop and
`status --porcelain -uall` plus each listed file's size and change time
(`diff::stamp_of`); the open hunks are read again when the file's counts
or its stamp change (looked at on each build, a `stat`). Before: Node's
key (status only), so a second edit of a file already ` M` left the row's
counts, the total and the hunks as they were, and a click on a hunk line
sent nvim to the line from before the edit (the critic: `drop +10` where
the text was on 15). Against: `git diff --numstat` per read as the key
(a diff to decide whether to diff). Why: rule 3 in its intent (Node ran
`git diff -U0` at click time, its target was never stale), lazygit's
diff follows the files. Probed: an edit 12 s in shows `+8 −2`, `+106`,
`@@ -1,13 +1,18 @@` within one read, and the click runs `drop +15`.
Chose: the file whose hunks show has its name on the pick's ground and
bold in its row (a file row, an area's name, a name on a line of names,
`+N more` for a hidden one). Before: only the hunks under it told, the
`▌` came with Tab alone. Why: lazygit marks the file it shows the diff of.
Chose: a line of several names (`⚠ also changed …`, `✗ a merge conflicts
in …`, `… N more:`, `… N areas`) steps: Enter on it, or a click off its
names, shows the next name's hunks (the first while none shows, closed
after the last, the hidden ones too); a click on a name still toggles that
name. The hunks' header says `Enter on its line: next name` there. The
line's action stays the same over the steps, so the frame keeps the pick
on it. Supersedes round 2's "anywhere else on the line opens the first"
(Node's `fileAt`). Against: a Tab stop per name (the frame lights whole
rows: every stop of a line would look the same). Why: rule 4, and the
keyboard reached only the first name (the critic).
Chose: a failed job's log is one stop of Tab (Enter: nvim at its first
problem), each line naming a file a button to it, as a hunk and its lines;
a click on a line naming no file runs the log's Enter. Before: a stop per
such line (22 Tabs to DIFF). Why: rule 4 (round 2's hunk decision, one
way), the critic's count; the job row still opens the job.
Chose: Tab without a pick goes on from the block under the pointer (the
one just clicked), Shift-Tab back from it; before it began at the first
block on screen. Smallest change in the frame (`view.rs` `select_by`, one
`.or(self.lit.block)`, a test). Why: lazygit's click selects the row that
j/k move from.
Chose: `stack:up`'s and `pg:migrate`'s error two columns after the hint or
button (`click or u  ✗ pnpm stack:up failed`). Why: rule 3 (actions.mjs's
two spaces).
Chose: the local read asks psql for the migrations only where STACK shows
them (a shared stack, or an own one that runs; core `reader.rs`). Before:
psql every read with the own stack down, its answer never drawn (3 calls
in 36 s idle; now 0). Why: rule 7 (#331's spirit: no read nobody sees).
Not done: a key that jumps to a section (lazygit's 1-5): the frame has no
way for a view to move the pick or the scroll; left to the frame piece.
Suite (release): views 11/11, buttons 25/25, hover 4/4, content 24/24.
cargo test: breadboard 421, core 242; clippy -D warnings, rustfmt clean.
Smallest changes elsewhere: core `model/diff.rs` (the key, `stamp_of`, a
test), core `reader.rs` (migrations only where shown), `tui/view.rs`
(`select_by`, a test). Probes: `.claude/gauntlet/tmp/wtv3/` (live.tmp.mjs,
psql.tmp.mjs; shots live/, live3/, psql/, up/).

## 2026-10-07 22:10 · herdr plugin, round 3 · nvim's `changes` tab calls the binary; a reload says when the old code runs on
Chose: `nvim/orchestration.lua` (the tab `changes` of an orchestration's
space, `:OrchestrationChanges`) reads the change set from the binary of
its own package, `<lua dir>/../target/release/breadboard wt:orchestrate
<name> --changes`, when it is built; `node <lua dir>/../src/worktree.mjs
orchestrate <name> --changes` only while the binary isn't built and src/
is still there; neither: the list says `<binary> isn't built: pnpm build:rs
in <package>` instead of a node `module not found` every 60 s. The JSON is
the same (`null` for an MR without issue, `mergedAt` of an open one and
both counts of a binary file: changes.rs `count`, as changeset.mjs; the
Lua's `counts` reads `vim.NIL` as `binary`). Against: leaving the line for
the lead's cut-over commit (it breaks silently the day src/ goes, and no
check reads nvim). Why: rule 5 (the Lua is the reader of that JSON and
outlives src/), rule 8 (no Node once built), spec §12/§18. The cut-over
needs nothing here; the commit that deletes src/ may drop the `node`
branch of `changesCommand` (dead then). Tried in a world with the real
nvim headless: the binary's list (6 MRs, `read 21:53`), the missing-binary
line; Node's branch reads GitLab from its own checkout, not the world, so
it is not run from here.
Chose: `--reload` after a pull whose build fails goes on in this process
(no second run of the same binary), and its last line is `! <session>
runs the code from before the pull until pnpm build:rs in the main
checkout and a reload (prefix+shift+l); …`, the layout step `the overviews
run the binary from before the pull`. Before: `✓ … runs the current
workflow code`, false (the critic, round 2: no cargo on PATH, develop one
commit ahead). Why: rule 3 in its intent (Node's ✓ is true because it
re-execs the pulled .mjs; ours may only print it when it is), and a
success line that lies is worse than none.
Chose: the plugin's panes run the binary as it was started (`argv[0]`
against the start cwd: the manifest's `../target/release/breadboard`), not
`current_exe()`. A rebuild while a pane runs replaces the file and
`/proc/self/exe` then names `… (deleted)`: `✗ pnpm commands didn't run: No
such file or directory` (the critic, once, in a worktree while others
built). A bare `breadboard` from PATH takes `current_exe()` with that
suffix cut. Same for `pnpm herdr`'s window command. Why: rule 3 (Node
runs `node <file>`, a path, never an inode) and rule 4 (one function,
`herdr::started_binary`, unit-tested).
Chose: herdr/config.toml's header says pnpm herdr `fills in the tab bar's
status command` instead of `{{RUN}} becomes the path of run.mjs` (filled,
that comment named run.mjs in .herdr/config.toml even where the command is
the binary). Node writes the same new words. Why: rule 4.
Left to their pieces: the commands pane in a worktree lists only its own
package (commands piece, logged there); `restore` issuing `pane move` for
panes of tabs it just closed (layout piece: the fake herdr still lists
them, real herdr doesn't).
Suite: plugin 21/21 against the binary (Node 17 + 1 todo, the rest
binary-only), cli 19/20 + 1 todo. cargo test herdr 42, clippy -D warnings,
rustfmt clean. Smallest changes elsewhere: `nvim/orchestration.lua`,
`herdr/config.toml` (comment).

## 2026-10-07 22:00 · overview / NEXT and ACTIONS, round 3 · #301: a `?` in code or a URL asks nothing; an automatic row says what a click does
Chose: `question_of` (core `conversations.rs`, behind `waits · auto` and the
plugin's `asks you`) takes a line of the last paragraph only when a `?`
ends a sentence there (only closing marks such as `**`, `)`, `"` between it
and a space or the line's end) once its inline code, URLs (`x://…`,
`<…>`) and link targets are out; lines of a fenced block (```` ``` ````,
`~~~`) never count, and the paragraphs are found after the fences, so a
blank line inside a block doesn't split it. The row's text has no `**`,
`__` or backticks, a link reads as its text: `**Shall I merge it?**` →
`Shall I merge it?`; the #270 line (`` `/?timeline=skia` ``) makes no row,
so ACTIONS says `1 for you`. Against: Node's `questionOf` (any `?`, the
markdown kept), which ours copied. Why: rule 2, #301 is open in
`breadboard · Epic 15` and its acceptance criteria win (spec
overview-orchestration §ACTIONS `CHANGE (#301)`); the test
`takes_only_a_real_question` pins it next to the code. A bare `/?x=1`
outside code needs no rule of its own: its `?` ends no sentence.
Chose: NEXT draws a hint under an automatic item too: `a click on it opens
its agent's tab: answer it there` for a `waits` (it has no buttons, so `on
it`, not `on the rest`); ACTIONS' pointer up says the same (`↑ in NEXT · a
click here opens its agent's tab: answer it there`). Against: no line (the
critic: the NEXT row didn't say what a click does, ACTIONS did). Why: the
bar, lazygit's options bar names what the key at hand does; rule 3 loses
nothing (Node had no hint at all), one dim row.
Not changed: the two texts of a blocked pane (`its agent is at a question:
it goes in once that is answered` on an answer, `the agent is at a
question: the draft follows once it's answered` on a draft) stay Node's:
the answer's text is stored in `actions.json` as `waits` (rule 5, byte
parity). Digits for an ACTIONS block picked with Tab need the frame's pick
in the overview's key path (`run.rs`, `Ctx`): left for the frame piece, the
look's buttons below NEXT are clicks only for now. NEXT's hover ground is
the theme's.
Suite (release): buttons 24/24, content 24/24. cargo test (core
conversations 5/5, overview::actions 12/12), clippy -D warnings, fmt clean
in these files. Probed in a world (`src/na-crit3c.tmp.mjs`): NEXT `waits ·
auto 302-… / Shall I merge it? / a click on it opens its agent's tab:
answer it there`; ACTIONS `1 for you`, no row for 307 nor for 305's
`https://…/merge_requests?state=opened`.

## 2026-10-07 21:59 · overview / MERGE REQUESTS, round 2 · a click never scrolls, the row's merge button goes, the merged newest first
Chose: a mouse click holds the row it ran on at its screen row and scrolls
nothing to show what it opened (`view.rs`: `Held::is_pointer`, `place_held`
returns before its reveal); Enter on a Tab pick still shows what opened
under it, as far as the block's top stays on screen. Against: round 1's
frame (not logged before), which scrolled after every click until the
opened rows showed: unfolding `▾ 42 merged` at 120x40 moved the fold 17
rows up (22 at 120x120), so a third click at the same cell landed on a
merged row and opened !276 in the browser instead of folding. Why: rule 3
(Node keeps the fold on its row through fold, refresh and unfold), the bar
(lazygit's toggles never move what is under the cursor), rule 6's spirit
(a click acts on what the reader sees under the pointer). Measured: at
120x120 the fold stays on row 70 through fold, unfold, fold; the frame's
test now clicks three times at one cell (80x24, 120x40). A fold on the
first scrolling row may still move down by the title its section pins
once it opened (the frame's pin).
Chose: an MR row has no merge button; `ready to merge` is its status
alone, the row's click opens the MR, its one button resolves conflicts
(§11.3). The two-click merge is the posted merge action's (ACTIONS, NEXT's
`two clicks merge`), as in Node; while the overview merges an MR its row
says `merging …`, then `merged` (read only, no button to arm). Against:
round 1's green ` merge ` beside `ready to merge` (two clicks, a `PUT
merge_requests/N/merge`), which the 20:50 entry listed as Node's: Node's
row has none. Why: rule 6 (GitLab writes exactly where today's breadboard
writes them: a merge only from the merge an agent posted, never a second
place); the title gets the button's 8 columns back.
Chose: the merged rows newest first (by merge time, the closed and the
untimed after them). Against: Node's oldest first (merge-order.mjs `merged`
by `mergedAt` ascending, §11.1). Why: the bar (lazygit's commits and git
log put the newest on top): what just went in stands right under
`▾ 42 merged`, where the eye is after a merge (in the world !308 merged on
two clicks shows on the fold's next row; Node puts it 42 rows down, under
the rest). Nothing is dropped (rule 3): the same rows, in another order.
Kept, checked again: the bar under every open MR with boxes, a full one
too (#294's acceptance, rule 2); `checking for conflicts …` on each row
GitLab hasn't settled (in the world the MR shas never match the pushed
ones, so it never settles there; Node shows the same).
Suite: `buttons.check.mjs` +1 ("three clicks at one cell fold, unfold and
fold it again; nothing opens": 120x40, scrolled to the fold with the
wheel, the fold's row and the frame compared, `xdg-open` read from
calls.jsonl, after 3 s of a still screen so Node's late reflow (seen
moving its fold 6 rows in 2 of 12 runs with 1-2 s) is over): Node 6/6,
ours 6/6, ours with round 1's reveal fails (`the fold left row 39 after a
click that unfolds it`). Against the binary: content 24/24,
hover 4/4, buttons 24/24, views 9/9. cargo test 407 + 242, clippy -D
warnings, rustfmt clean. Smallest change elsewhere: `tui/view.rs` (the
frame's held scroll and its two tests).

## 2026-10-07 21:56 · overview actor, round 2 · an agent has one name in UP NEXT and the log; a half set up handover says so
Chose: (1) every UP NEXT `hand over` row (announced: `after !a`, `now`,
`once it is idle`, `waits: …`, `failed: …; tries again once <name> is
idle`; suggested `(free, …)`; `handing over …`; `its handover to <name>
failed`) and its `who` name the agent by its display name (#292),
`names_of(issues, known).shown(...)` in core's `worker_actions_of`, as
Node's `shown`; the row's key and `Go::HandOver.to` keep the record name
(`wt:orchestrate --to` needs it). Before: `#301 → 306-overview-merge-requests-crash-row
after !318 (announced)` next to `Emil Keller` in AGENTS. Why: rule 3
(workers.mjs:186, 371-439). (2) The log says the same name: the actor
keeps the orchestration's `Names` from each read (`Actor::names`), and
`handing/handed #n to <name>`, `told <name> …`, `<name> didn't take the
line`, `<name> is free in …` go through it; `waits for <worktree>'s agent`
becomes `waits for <name>` when the agent has one. Why: rule 3 (one agent,
one name on one screen), rule 4 (one place, not per caller). (3) A child's
last line loses its own `✓ `/`✗ ` before it follows ours (`spawn::said`):
`✓ ended: ⇶ … ended`, `✓ handed #301 to Emil Keller: #301 went to …`.
Why: rule 4. (4) A handover whose wt:new failed after the setup (the CLI
exits 0, orchestrate.mjs:306) is yellow `◐ handed #301 to Emil Keller,
half set up: wt:new failed after the setup`, its command and log path
under it, instead of a green entry with the failure at its end. Why:
rule 1 (#306: errors that say what to do); Node says nothing there.
Not changed here: wt:remove's exit code when `git worktree remove` fails
(lifecycle piece); `--herdr`'s GitLab-dependent states arriving later
(core's everything read).
Suite against the binary: buttons 24/24, isolation 4/4, cli 19/19 + 1
todo. cargo test: core 237/237 (+1: names in announced rows), binary all
but `tui::view::a_click_keeps_its_row…` (the frame piece's file, in
progress). Probe critic-actor-announce: `NEXT hand over Emil Keller #301
→ Emil Keller after !318 (announced)`, one line at 120 columns. Smallest
change elsewhere: `crates/core/src/model/workers.rs` (names in rows).

## 2026-10-07 21:55 · herdr layout · #332: an orchestration's worktree space has no overview tab
Chose: a worktree whose `.claude/origin` names an orchestration with a
record (core's `record::is_orchestrated`) gets no `overview` tab in its
space, from `wt:new --origin`/`wt:orchestrate` (setup) and from every
layout (`wt:herdr`, a reload, the overview's own): one from before closes
unless something else runs in it (a second pane, an agent, this command;
`is_busy`), the order is `root · <feature> · nvim`. A worktree of no
orchestration, or of one whose record is gone, keeps `overview --worktree`.
Against: the origin alone (an ended orchestration's leftover would lose
its only view). Node's design of #332 is still not on develop
(`feature/332-…` has no commit), so built from the requirement. Suite:
two known-bug checks (`#332: …`), passing against ours, todo against Node:
lifecycle's `wt:herdr` closes 305's recorded overview tab, keeps 296's once
its origin is gone, creates none; setup's `wt:new --origin breadboard`
with herdr handing back the space and its tabs creates root, the feature
and nvim, no overview. Why: rule 2 (#332); rule 6 (the closes are part of
the layout the user asked for). The smallest changes outside the piece:
`lifecycle/layout/spaces.rs`, `setup/herdr.rs` (`tab_order`), core's
`record.rs`.

## 2026-10-07 21:55 · overview / WORKTREES · round 2: the agent always under the state, a node 3 rows at 120 with work in it
Chose: the agent stands at the right of the first facts line in every
node; the facts that don't fit beside it flow under it, never the agent
onto a line of its own. Without a column beside the name (80), the app
and stack stand at the right of the next facts line, padded so the apps
line up down the edge (on the agent's line when everything fits there).
Measured in the critic's probe world (.env.local, own and shared stacks,
collisions): 120 columns, every node 3 rows, 4 with a collision (was 5 on
nodes with `N changed`, the agent switching between two places); 80
columns 5 + collision + the title's wrap (was 6–8, a lone `untested: 1
commit` row). Against: round 1's agent line before the facts when they
didn't fit (the agent jumped as you scan down). Why: the bar (lazygit's
branches keep each fact in one place per row); rule 7.
Chose: an issue's state says only the exception: `#305`, `#303 closed`,
`#305 ?`/`…` while GitLab doesn't tell; the agent's counts `· 2 issues ·
1 merged`. Against: Node's `#305 open` and `· 2 issues · 1 MR merged`
(spec overview-orchestration.md §657). Why: rule 3 deviation, nothing is
lost (an open issue is what a live node has; unknown has its own mark),
8 columns that decide whether the agent fits beside the facts at 120.
Chose: a colliding file is named by the shortest end of its path that no
file of this worktree's work nor of the worktrees it collides with shares
(core's `Node::near`): 305's line reads `#304: tree.mjs, web/src/index.ts`
because 304 changed `core/src/index.ts` too. Against: shortening against
the node's own files only (`index.ts`, which one?). Why: the critic.
Chose: each file name of the collision line is a click that opens the
file in that worktree's nvim at its first change, `+N more` the first not
named (DIFF's click, Node's `openFile`); its hover lights the name on its
own inside the lit node; why not (`no nvim on the PATH`) goes to the
band. The line wraps at file names now, never inside one. Smallest
changes outside the piece: `act.rs` routes `OpenFile` outside the
worktree view, `worktree_view.rs` makes `editor` visible to it. Why: rule
3 (the worktree view's click, which #332 moves here); the log's "Not here"
of 21:10 had no rule.
Not here: the sticky WORKTREES header hiding the name line of the node
under it, and right-aligned text touching the scrollbar (every section's,
MERGE REQUESTS' `acceptance 2/2┃` too): the frame's; the blind mask of
`snap.mjs` (the suite's tool).
Suite (release): content 24/24, hover 4/4, views 9/9, buttons 24/24,
isolation 4/4, cli 19/20 (+1 todo), lifecycle 10/10, setup 27/27; Node:
lifecycle and setup green with the two #332 checks todo. cargo test (core
241/241; breadboard's worktrees 13/13; its 3 failures are other pieces'
work in progress), clippy -D warnings, rustfmt clean.

## 2026-10-07 21:58 · daemon, round 2 · a daemon that hangs is given up, reads before a write skip it, docker's read has a time limit
Chose: (1) the daemon says on every GitLab feed that it is there (an empty
line every 5 s, `ALIVE_EVERY`); a feed silent for 15 s (`client::SILENT`)
is given up at once, not reconnected (a stopped daemon still accepts
connections and holds its pid file, so no other one can start): the
overview says `GitLab didn't answer <t> (breadboard serve stopped
answering: this overview reads GitLab by itself now)` and reads GitLab
itself on the same cadence (`daemon::read_gitlab`); its herdr reads stop
asking the daemon too. A herdr read the daemon leaves unanswered for 3 s
makes the reads skip it for 30 s (lifecycle's SILENCE idea), not 3 s on
every read. Probed (two overviews, daemon SIGSTOPped, `r` pressed): first
GitLab request 11 s after the stop (before: none in 46 s), herdr 12.7
calls / 10 s from both (before: 25 in 46 s, every look 3 s late). Against:
counting a silence as one of MAX_LOSSES (four silences, 60 s, before the
first read); a time limit on `r` alone (a hang without `r` stayed unseen).
Why: parity (overview spec: a failed read keeps the last good data and
says so), rule 7.
(2) The reads a write follows are fresh, never the daemon's 1 s answer:
`agent_pane_of` (whether its agent is at a question before `agent
prompt`) reads `pane list` here (`herdr::panes_of_now`), `idle_pane_of`
(actor, worktree view's editor) asks `pane process-info` here
(`herdr::is_idle_now`) before `pane run`. As Node. Against: the shared
answer (a line could go into an agent that went to a question less than
1 s before). Why: rule 3, rule 6.
(3) docker: one `docker ps` may take 10 s (`DOCKER_WAIT`), then it is
killed and counts as not answering, for everyone, until the next read 12 s
later; the lock of a reader that died is taken over after 15 s (was 30).
With no answer on file yet and another process reading (a cold start of N
overviews), a process waits up to 2 s for that answer, else has none yet:
docker is asked by the lock holder alone, never N times. Against: Node's
no limit (#331's dockerd under load held every lock holder, one after
another). Why: #331 (rule 2), rule 7.
(4) The runtime folder (socket, `containers.json`) is used only when it is
this user's alone: a real directory, owned by the user, mode 0700
(`local::own_dir`); else no daemon is started or asked and docker is read
by the process itself. Against: trusting `/tmp/breadboard-<uid>` made by
anyone (its socket answers herdr reads the actor acts on).
(5) Another docker than the default (`DOCKER_HOST`, `DOCKER_CONTEXT`,
`DOCKER_CONFIG` set) shares its read through `containers-<key>.json`;
the default keeps `containers.json` (no other reader of it exists: no Node
design of #331). Why: rule 5 (the file's format unchanged).
(6) GitLab stays one read per view, not one for all views: it is one
GraphQL request per view per 30 s (probed: 32 requests/min, all `POST
graphql`, for 10 distinct views; ~45/min for #331's 14 overviews, 2 % of
GitLab's 2000/min). One read of the union of all asks would merge the
views' queries in core's GitLab readers (another piece's) for that. The
entry of 21:15 said "10 overviews: 59/min (flat)": that probe repeated
5 views twice; 10 distinct views make 79/min (GitLab 32), see below.
Against: the union now. Why: rule 4, rule 7. Not done: the herdr plugin's
tab bar status still runs `docker compose ls` every 10 s per herdr server
(the plugin piece's; `plugin.check.mjs` pins it through docker's fake).
Measured (release, breadboard world, 3 own stacks, a minute after 25 s):
5 distinct views 62 calls/min (herdr 42, GitLab 16, docker 4), CPU 1.33 %
own, 10.35 % with reaped children; 10 distinct views 79/min (herdr 43,
GitLab 32, docker 4), 2.53 % / 16.10 %. Node (critic, same probe): 518/min
and 13.95 % / 69.6 %; 743/min and 33.7 % / 164.8 %. Suite: budgets 11/11
(first frame 244 ms, GitLab 901 ms, 3 calls, idle 0.20 %, 22 MiB), views
9/9, content 23/24 (the 80-column MR title, another piece's layout).
Smallest changes elsewhere: core `herdr.rs` (`panes_of_now`,
`is_idle_now`), `tui/overview/actor/herdr.rs`, `worktree_view/editor.rs`.

## 2026-10-07 21:50 · overview / everything, round 2 · GitLab's answer reaches the sidebar after a busy tick, an opened list's fold stays put, the print without boxes
Chose: (1) `--herdr`: a read (local or GitLab) that comes in while the
sync's tick is busy marks one more tick due; the busy tick's effect runs
it at once with the newest local and GitLab read (`actor/sync.rs`
`is_due`), as Node's `r` during a read queues one more. The busy flag is
cleared by a drop guard, so a tick that panics no longer stops every next
one. Measured in breadboard worlds (fake herdr, 120x40, the laptop loaded
by the other builders): the first `wt_state=… · !iid` after 7, 7, 7, 11 s
(then, under heavier load, 12, 12, 13, 14, 24 s) against 7–23 s and one
run without it before, Node 22 s and once not in 30 s. What is left is the
first tick's layout (every space made, 7–10 s of fake `herdr` processes;
the real herdr answers in milliseconds), which runs only when the set of
worktrees changes. Against: the layout on a thread of its own beside the
reports (a pane `move` renames the id a report targets; two writers of
herdr in one process), a shared slot of the newest read polled inside the
job. Why: rule 3 (Node's queue of one more), rule 4.
(2) An opened MR or issue list keeps its fold line where `… N more` was,
under the rows the pane showed by itself, saying `N more below · click
to show fewer`; the rows it opened come under it. The frame holds the
clicked block and shows what opened under it, so at 80x24 the click on
`… 13 more` puts `13 more below` on top with `!317` right under it
(before: the fold line moved to the list's end and the frame followed it
to `↑ 29`, `!317` 26 rows up). A second click gives back the frame before.
Against: the fold at the list's end (the click's row moves away), keeping
the scroll (the new rows start off screen under the pointer's row only
when the list was cut at the screen's bottom). Why: rule 1 (the critic:
what opened shows where the click was), rule 4 (the frame's held block,
no scroll code in the view).
(3) Not carried: `overview --once` piped prints no `╭─ TITLE ╮ │ │ ╰╯`
boxes and no `[ label ]` / `( label )` around buttons (spec §1.1/§1.4).
The print is the TUI's sheet as text: section names start their lines,
a button's label reads as its words (`▶ start the default stack  pnpm
stack:up`); nothing in a print is clicked, and the boxes cost 4 columns
of every line and a row per section. Why: rule 4 (one way to draw the
overview, `view::once`), the 20:10 entry's list completed.
Left: at 80x24 the first screen holds 1 of 14 MRs and 1 of 64 issues, the
issue's labels one row under the screen (the band, 11 worktree rows and
both titles leave 5 rows; each list keeps at least one). The keys line at
80 columns drops `g G scroll · Tab Enter pick a row`: the band's (another
piece's), logged 2026-10-07 under the pick.
Suite: views 9/9, content 24/24, buttons 24/24, hover 4/4.

## 2026-10-07 21:50 · CLI / setup group, round 2 · one implementation with the lifecycle group: the issue, the layout, the claims
Chose: the setup group keeps only what no other group does (wt:new's
steps, the stacks, wt:check's run, the changelogs, `commands`, `browser`,
the moves of the tab that runs it) and calls the rest where it already
is, with `repo::lifecycle()` (the cwd's main checkout as the lifecycle
group's `Repo`). Gone from `cli/setup/`: `nvim.rs`, the herdr layout of
`herdr.rs` (spaces, tabs, order, duplicates, editor, worktree space, stack
tabs, slots, marks: 1051 → 217 lines), `orchestrator_space` and the
checkout of `orchestrate/space.rs`, `slug_of`, `read_issue`'s body,
`make_worktree`, `undo_start`, the record entries, `workers_of`,
`refusal_of`, the worker history, `after_args`/`parse_after`, the
conversations' copies, the push guard, the check stamp and failure
parsing, the env helpers. Where they now are: `lifecycle/handover.rs`
(the issue of an orchestration, its worktree, the undo, the claim on
origin, `--after`), `lifecycle/layout/**` (herdr's layout), core
(`check`, `conversations`, `local`, `model::workers`, `gitlab::paths_of`),
`write/mr.rs` (the push guard). Where the copies gave different results,
the one kept is: (1) the feature name of an issue: Node's NFKD-then-strip
(`Cæsar’s résumé of the plugin` → `230-c-sar-s-resume-plugin` for a fresh
start and a handover; it was `230-caesar-s-re-sume-plugin` fresh): every
mark `\p{M}` dropped, not only U+0300–036F, the compatibility forms
titles meet (`ﬁ`, `ĳ`, fullwidth, `ș`) folded; `æ ø ß ł` split a word as
there. (2) A claim on origin is `feature/<n>-…` or `fix/<n>-…` only, for
wt:new and wt:orchestrate (the handover's `*<n>-*` took `feature/112-x`
for #12), within 30 s. (3) An undone start replaces the issue's entry
with its why (the handover's `write_record` kept the entry without why
before it, today's `undoStart` too: the record said the worktree went,
not why). (4) A start's `wt:new` exit code: a worktree there with a
non-zero exit is `(wt:new failed after the setup, see above)` in both
(#130); a `wt:new` that didn't start is `✗ wt:new did not start: …`. (5)
A stack tab herdr creates without a pane is `<tabs> opened empty: herdr
gave no pane, nothing runs there` in `stack:*`, `wt:new` and now also
`wt:herdr` and the overview's layout (log 00:45; they counted it as
`+label`); a stack that is down reads the panes only when a tab of it is
there to close. (6) The pane this runs in is the lifecycle group's:
none without `HERDR_PANE_ID` (a stack:up/down of the overview's start
asked 105 herdr calls in the world, now 9). (7) The orchestration's
`changes` and `nvim` tabs take the Lua beside this binary, else the
cwd's checkout's, else the one it was built in (`repo::package_dir`, one
for both groups). (8) wt:check's failures: core's parser with today's
`id.split("#")` (the task between the first two `#`), 5 tests named in
wt:check's line, 1 in the pre-push hook's, 2 in the overview.
Also: the footer of a start whose issue went to an idle agent says
`SendMessage(to: "<feature>")`, the agent's name after the handover's
`/rename`, as the handover's own line does (it named the old name).
Against: moving all of it into core this round (the lifecycle and
overview builders change those files at the same time; the group
modules are the smaller step and the next move to core is one place, not
two). Why: rule 4 (one way to do a thing; 2.0k lines less in setup),
rule 5 (one branch and worktree name per issue, whichever command
starts it), rule 3 (Node's slug), rule 7 (the calls).
Deviation, kept: an issue GitLab doesn't have is `✗ #308 is no issue of
this project (glab issue list)`, one GitLab doesn't answer `✗ issue #308
could not be read: GitLab did not answer; again once it does (glab issue
view 308 says why)` (Node: `issue #308 could not be read` for both): the
handover's words, rule 1. `--queue --after 13` takes 13 as `!13`, as
`--to … --after 13` does (Node: refused as no MR).
Corrected (entry 00:40): the hand-read argv does not keep "a flag before
the feature is a usage error": `wt:new --fix 12-x` makes `fix/12-x` as
`wt:new 12-x --fix` does (Node prints the usage); `--origin=Bad_x` is
refused with the name rule like `--origin Bad_x` (Node takes it and
writes an origin no orchestration has). Kept both: a flag's place means
nothing to a person typing it, a bad origin would mark a worktree for an
orchestration that can't exist (rule 6: no push on a typo).
Not this round: the setup group's GitLab reads through core's client
(one `glab api` process a call, 98 for `--changelog`): it needs the
suite's "diffs GitLab didn't answer" check to fail the read in the fake
GitLab, not in a glab wrapper, and core's error words (`HTTP 502
(<what>)`) to carry the status text; the next round of this piece.
Suite (release binary): setup 27/27, cli 19/19 (+1 todo), lifecycle
10/10, plugin 21/21. 61 cases of `compare-setup.tmp.mjs` rerun: output
as before but for the deviations above, herdr's reads fewer (wt:new 27 →
23, `--queue` 32 → 15, stack:up/down 105 → 9), the writes the same.
cargo test: cli 179/179, core 241/241 (one TUI frame test of another
piece fails at this time), clippy -D warnings, rustfmt clean. Smallest
changes elsewhere: `lifecycle.rs` (`claude`, `herdr` public),
`lifecycle/handover.rs` (the above, `description` on `Issue`),
`lifecycle/layout.rs` and `layout/tabs.rs` (`open_tab`, the empty tabs),
`layout/orchestrations.rs` (`chat_of_pane`, `record_chat` public),
`lifecycle/repo.rs` (`package_dir`), `write.rs` (`mr` visible in `cli`),
core `check.rs` (`failures_line`, the task split).

## 2026-10-07 21:40 · overview / worktree view, round 2 · a file click shows its hunks, a conflict both sides, a hunk is one Tab stop
Chose: a click (or Enter) on a file of DIFF (an area's row, a file's row,
`… N more:`, the `… N areas` row's names), of `⚠ also changed …` and of
DEVELOP's `✗ a merge conflicts in …` shows that file's hunks under the row;
a line of them opens nvim at that line, a hunk's `@@` row (Enter on the
hunk) at its first change. Supersedes the 00:30 entry's "a click opens nvim
at that file's first change" and the 21:00 entry's "every … click of
render-worktree.mjs" kept: Node's click went to nvim at once (spec §2.8,
§2.9). Against: Node's direct nvim (a developer sees nothing of the change
before leaving the view). Why: the bar is lazygit, whose files panel shows
the diff of the file picked and opens the editor from a line (`e` in the
staging view); rule 4: one way to nvim, from the line meant, one click
further than Node. The `… N areas` row had no click in Node; its names now
show hunks like every other name (rule 3 adds, loses nothing).
Chose: a file develop changed too (DEVELOP's conflicts, `also changed on
develop`) shows both sides under dim headings: `here` (`git diff
<merge-base> -- f`, its lines open nvim) and `on develop` (`git diff
<merge-base> origin/develop -- f`, its lines open nothing: they aren't in
this checkout). Against: this branch's side alone, which hid the reason the
file conflicts (the critic: `# platform (develop)` appeared nowhere). Why:
lazygit's merge view shows both sides of a conflict; rule 4 (two `git
diff`s, no merge-tree parse).
Chose: each hunk is one stop of Tab, its lines are buttons (no stop; hover
lights the line in cyan inside its lit hunk, the click log says
`button <file>:<line>`); the file's dim line above the hunks is plain, so
Enter on a row keeps the pick on it (before: the line had the row's action,
Tab stopped on it, and Enter there closed the hunks and dropped the pick).
Against: a stop per line (a 60-line hunk took 60 Tabs). Why: lazygit's
files panel moves file to file, its staging view line by line: here Tab
moves row to row and hunk to hunk, the mouse line by line. No change
outside this piece: the frame already skips buttons for Tab.
Chose: a click whose command can't start says why: `no pnpm on the PATH`
(`Started::start`, for `pg:migrate`, `stack:up`, `wt:remove`), not
`pg:migrate ended with -1`. Why: #306 (errors say what to do), rule 1.
Chose: AGENT's cells start 2 after its widest label (`context  ███`),
`migrations  ⚠ …` 2 after its label, as Node's `table`. Why: rule 3.
Not changed: STACK's containers come in core's order (alphabetical, the
daemon's shared `docker ps` read, #331); docker's own order is the daemon
piece's to keep.
Suite (release): views 9/9, buttons 24/24, hover 4/4, content 24/24.
cargo test (worktree_view) 22/22, clippy -D warnings, rustfmt clean.
Probed in a world (`.claude/gauntlet/tmp/wtv/plan30-32`): a click on
`README.md` in `✗ a merge conflicts in README.md, …` shows `here`
(`− # platform (gauntlet world)`, `1 + # platform (this branch)`, `2 + more`)
and `on develop` (`1 + # platform (develop)`); a click on `2 + more` runs
`nvim --server .claude/nvim.sock … drop +2 …/README.md`; Tab after Enter on
`render.mjs` keeps `▌` on its row (11 of 21), its 41-line hunk is one stop.

## 2026-10-07 21:21 · herdr plugin, round 2 · your tab bar follows the binary, the status counts from any cwd, a missing binary is built or said
Chose: (1) `herdr:install`, and `--reload` for your own herdr, rewrite the
tab bar's status in `~/.config/herdr/config.toml` in place: every
`command = "node <…>/run.mjs status"` and every `<…>/breadboard herdr
status` (a moved checkout's, a bare one) becomes this checkout's binary,
the rest of the file byte for byte, one step line (`the tab bar's status
in ~/… runs <binary>`); the "add it" hint only when no status of any form
is there (`setup.rs` `with_current_status`/`has_status`, shared with
`.herdr/config.toml`'s `with_status_entry`). Supersedes (3)'s "prints the
line". Against: printing the line every run (today's hand work, the #230
bug again once the Node code goes). Why: rule 2 (#230: "herdr:install also
rewrites the tab bar's status command", "moving the package again needs no
hand work"), rule 5 (a reader for the old form).
(2) `herdr status` finds its repo as Node does from any cwd: the cwd's
checkout when it is one of this repo (a main with
`packages/breadboard/herdr/herdr-plugin.toml`), else the checkout of the
binary as started (`argv[0]`, a link not resolved), else of the file
(`current_exe`). herdr runs `tab_bar_right` commands in its server's cwd
(`$HOME` for the user's own herdr): from there ours counted only the main
space's agents (`· 1 needs you` against Node's `· 2 need you`). Against:
`--repo <main>` in the written command (a format change in two configs,
and the suite's ` status", interval_seconds` stays the line's end). Why:
rule 3 (Node: the script's own checkout), rule 4.
(3) The binary's path in the configs is `<package>/target/release/breadboard`,
no `herdr/../`. `pnpm herdr`, `herdr:install` and `--reload` check it
exists: missing, they build it (`cargo build --release` in the package,
progress shown; `--reload`'s rebuild after a pull shares the function and
now shows its progress too), else say `! <binary> is missing: the
plugin's keys and the tab bar's status fail until it is built (pnpm
build:rs in <package>)` and go on (the session runs without it). Against:
failing (a session without the binary still works before the cut-over),
silence (every key and the tab bar fail every 10 s). Why: rule 1 (#306:
errors that say what to do), rule 8.
Not changed: the commands pane lists the package the key was pressed in
(spec §4, `cwd = from`); which `package.json` that is belongs to the
commands piece.
Suite: `plugin.check.mjs` +3: the session config's tab bar command run
through `sh -c` from `$HOME` and `/` (Node and ours `⬡ no stack · 2 need
you`); `herdr:install` over your own status of a moved package (known bug
#230: todo against Node, passes in ours); rewrite-only: every command of
`herdr-plugin.rs.toml` resolves from the plugin's directory, each `open`
opens its own pane, each pane runs `herdr pane <its id>`. The binary
subject links itself at the world's `packages/breadboard/target/release/`
(ignored by the world's git), where the manifest and configs look.
Against the binary: plugin 21/21, cli 18/18 (+1 todo); Node: plugin 17 +
3 rewrite-only skips + 1 todo. cargo test 404/404, clippy -D warnings,
rustfmt clean. Smallest changes elsewhere: `breadboard-gauntlet/src/subject.mjs`
(`builtIn`), its README's plugin row.

## 2026-10-07 21:40 · overview / NEXT, ACTIONS · a merge's words follow what its click does
Chose: what a click on a merge does is said from its offer (merge.rs
`click`): blocked, `opens its MR in GitLab`; clear, `merges on a second
click` (the first arms it: `click again to merge !N`); gone from the view,
nothing. NEXT's hint under a blocked merge: `a click on the rest opens !318
in GitLab: two clicks merge it once its pipeline passes and GitLab has
checked it` (each blocker by next.rs' `once_text`, `GitLab is checking` as
`GitLab has checked it`, one without words as `the rest above clears`;
merged or closed: the first half alone). ACTIONS' pointer to the action in
NEXT says `↑ in NEXT: answer it there` only where it has answers; else
`↑ in NEXT · a click here <does>` (`: answer it there` for a question
without options, which its agent's tab answers). The heading's `buttons and
options answer its agent` only when one does (a ✓ alone tells no one), and
a try that can't be tried yet counts as `1 waiting`, not `for you`. Against:
`merges` for every merge and `answer it there` for every action in NEXT
(the critic, round 1: both name an action the click won't do; Node says
only `the rest of a block does it`, which is true but says nothing).
Why: rule 3 deviation in words only (the clicks are Node's), logged; the
bar: lazygit's menus never name an action a key won't do. Suite: buttons
23/23, content 24/24, hover 4/4, views 9/9.

## 2026-10-07 21:15 · daemon · what `breadboard serve` serves: herdr's reads and GitLab per view
Chose: one daemon per user (`daemon/`), started by the first live overview,
serving (1) herdr's reads (`pane list`, `tab list`, `workspace list`,
`pane process-info`: core's `herdr::herdr` asks it first, `share_reads`),
one call for every overview within 1 s, and (2) GitLab: one feed per view
(`orchestration <name>`, `worktree <dir>`, `everything`) on the overview's
cadence (`daemon::read_gitlab`, moved out of the overview unchanged), sent
to every overview of that view, `r` in one reads for all, the last answer
first to an overview opened later. The local read (git, the records) stays
in each overview. Against: the plan's daemon that serves whole views
(`Local` over the socket): serde on ~30 types of other pieces' files for a
gain only where the same view is open twice; the live setup's overviews are
mostly different views, what they repeat is herdr, docker and GitLab.
Measured (breadboard world, 5 overviews: 2 × the orchestration, everything,
2 worktrees, 3 own stacks; a minute after start): calls to the fakes
654/min Node, 182/min ours before, **58/min** now (herdr 34, GitLab 12,
docker 4, herdr tab/space lists 4 + 4); 10 overviews of the same 5 views:
59/min; 10 distinct views: 79/min (GitLab grows by one read per view: see
"daemon, round 2").
CPU of all breadboard processes together: Node 18.75 %, ours 0.90 % before,
0.97 % now with the daemon (0.15 % of it); the calls saved are processes
herdr, docker and glab no longer start. Suite: budgets 11/11 (first frame
145 ms, GitLab 754 ms, 3 calls, second refresh 2, idle 0.10 %, 22 MiB),
content 24/24, views 9/9. Why: rule 7; rule 4 (fewer files touched, one
cadence for both paths).

## 2026-10-07 21:15 · daemon · #331: docker read once in 12 s by all processes, through a file
Chose: core's `local::containers()`: one `docker ps --all --filter
label=com.docker.compose.project` (project, service, one-off, state,
status a line) at most once in 12 s for every process of the user,
shared through `$XDG_RUNTIME_DIR/breadboard/containers.json`
(`{"at": <ms>, "containers": [{project, service, state, health}] | null}`;
without XDG_RUNTIME_DIR `$TMPDIR/breadboard-<uid>/`); the reader that
finds it old takes `containers.json.lock` (a directory, as core's locks)
and writes it, the others take what it holds. `compose_projects()` (the
stacks with compose's `exited(1), running(2)`) and `containers_of()` (one
stack's service, state, health; one-off containers left out as `compose ps`
does) come from it: no `compose ls` / `compose ps` per process or stack
any more, with or without the daemon (a print, an overview that reads by
itself, Node's fix of #331 can read the same file). Against: docker in the
daemon only (the overviews that can't reach it, and every other process,
would poll again). The Node design of #331 isn't in develop
(fix/331-overviews-poll-docker-every-2 has no commit): the file is the one
the issue proposes. Measured: 20 docker calls/min → 4 (5 overviews), 5 with
10. Tests never ask the machine's docker (`cfg!(test)`). Why: #331 (rule 2);
rule 4.

## 2026-10-07 21:15 · daemon · the socket carries a key of the binary and the environment
Chose: `serve-<key>.sock` in `$XDG_RUNTIME_DIR/breadboard/` (else
`$TMPDIR/breadboard-<uid>/`, `/tmp/breadboard-<uid>/` where that path is
too long for a socket); the key hashes the binary (path, size, time) and
PATH, HOME, XDG_CONFIG_HOME, HERDR_ENV, HERDR_SOCKET_PATH,
BREADBOARD_GITLAB_URL, GITLAB_TOKEN. Against: the plan's one
`breadboard.sock`. Why: a gauntlet world (its PATH of fakes, its herdr)
never reaches the user's daemon or the other way round (loop safety); a
rebuilt binary never talks to an old daemon in another protocol. No other
program reads the socket (rule 5 not touched).

## 2026-10-07 21:15 · daemon · its life: started by the first overview, gone 30 s after the last
Chose: the first live overview runs `breadboard serve`, which forks into a
session of its own (so neither the overview's end nor its terminal's, nor
the suite's `stop` of the overview's process tree, takes it along) and tells
its starter over a pipe once it listens, or that it can't (the overview
then reads by itself at once: measured 776 ms to GitLab's data without a
daemon). One per socket (core's `PidFile`). It ends 30 s after its last
connection closed (an overview reloaded finds it warm) or once its socket
isn't its own (a world removed). A feed ends with its last follower, its
last answer kept 30 s. A daemon killed under two overviews: both follow a
new one at the next line (probed: `r` read for both, no error line); three
losses in a row without an answer and the overview reads by itself. It
only reads (rule 6): herdr writes are refused by it whatever is asked.
`--once` never starts one. Why: rule 4 (connections counted, no presence
protocol); rule 6.

## 2026-10-07 21:15 · overview · the local looks fall on the clock's even seconds
Chose: each overview's 2 s look waits for the next even second instead of
2 s after its last: every overview asks herdr at the same moment and the
daemon's one call (fresh for 1 s) answers all. Measured: herdr `pane list`
132/min → 34/min for 5 overviews. Against: a daemon that polls herdr and
pushes (a second channel per overview for the same effect). Why: rule 7;
rule 4.

## 2026-10-07 21:15 · daemon · a view's ask changes in place, never a new feed
Chose: the overview sends its new ask on the open feed (`Said::Ask`); the
feed's next read takes it, as the overview's own reader did. Against: a feed
per ask: an orchestration's ask changes ~6 times in its first second (its
local reads settle), each a new feed with a full read (measured 17 GitLab
calls/min instead of 12). Why: rule 7.

## 2026-10-07 21:10 · CLI / wt:remove · the worktree meant is named, never taken
Chose: `wt:remove 296` (no worktree `platform-296`) is refused with the one
meant: `did you mean 296-overview-hover-…? (pnpm wt:remove 296-…)`, by the
issue number or a typo (`cli/typo.rs`); with none near, the worktrees as
before. Against: removing `296-…` from `296` (a destructive command goes
only by the name it was given). Why: rule 3 (today's refusal kept, its
words exact); lazygit's bar (the next step said, not looked up).

## 2026-10-07 21:10 · CLI / lifecycle + wt:action change · nothing waits without end or without a word
Chose: every call of the lifecycle group to another machine has a limit
and says what it waits for after 1.5 s (`  waiting for GitLab (the MRs of
fix/303-x), at most 15 s`): glab 15 s (core's client gives a call as
long; 5 s for a read that only names an MR in a refusal), `git fetch` and
`git push --delete` 60 s, the claim check's `ls-remote` 30 s, the
changelog rewrite after the ✓ 90 s, every herdr call 10 s, the layout
lock's wait (it was silent up to 60 s). Past a limit the call counts as
unanswered, as a refusing GitLab already did (#306): the merged worktree
still goes, its claim and changelog stay with the way to retry, a fetch
goes on with the last one's refs (they only keep more), herdr's tabs stay
(`pnpm wt:herdr tidies up once it answers`). One silent GitLab is asked
no more in that run (a prune of five merged worktrees waited 15 s each),
one silent herdr not for 60 s (the overview, which calls the layout in
its process, asks again after). The waits are said only by a command run
from the command line (`proc::say_waits`), never on the overview's
screen. A child's end is its output's end, no polling (a layout makes ~80
herdr calls; wt:herdr in the world 2.4–2.7 s as before). `wt:action
change` (the write group's, the smallest change: `glab_within` in
`write/glab.rs`, used by `change.rs` only) gets the same 15 s: refused
before it writes anything, `✗ glab failed: GitLab did not answer in 15 s`
without the login hint. Against: today's (no limit: a GitLab or herdr that
takes the call and never answers held wt:remove, wt:action change, the
handover and wt:herdr without end, measured: killed after 100 s).
Why: rule 1 (the new check below); rule 2 (#306: a GitLab failure never
holds what is the point); rule 7 (measured: 15.0 s to the ✗ or ✓ with a
silent GitLab, 10.1 s with a silent herdr, against no end).
Suite: `lifecycle.check.mjs` "GitLab takes the call and never answers"
(known bug against Node, `to file:`; Node held 40 s to the test's limit);
cli + lifecycle against the release binary: 27 pass, 0 fail, 1 todo
(wt:orchestrate needs claude) of 28.

## 2026-10-07 21:05 · CLI / setup group · loop 2, round 1: the cwd's repo only, no herdr write on a dry run, a third of stack:up's wait
Measured first: 61 cases side by side with Node, each in a fresh world
(wt:new human/agent/--shared/--fix/--origin, every wt:orchestrate form
incl. a fresh start in and outside herdr, `--now`, `--queue [--after]`,
`--changes` (the JSON equal but for the world's shas), `--changelog`,
`--end`, wt:stack, stack:*, wt:check, changelog and changelog:*,
commands, browser). Output equal to Node's but for the deliberate
deviations already logged (refusals that say what is wrong, #247's brief,
wt:stack's agent line, `--end` of nothing, exit 1 when a start failed).
Chose: (1) the repo is the cwd's only: `INIT_CWD` is followed only when
it lies in the cwd's repo (for `commands`' package and an agent's
worktree), and the main checkout never comes from it; before, a cwd
outside any repo with an `INIT_CWD` of another (a pnpm script of one
checkout, a world under a checkout) acted on that one. (2) #230's
fallback, the plugin beside the binary, only when that binary lies in a
checkout of the cwd's repo (`lifecycle/plugin.rs`, the smallest change
there): a binary of another checkout linked herdr to that checkout's
plugin, whose actions then act there, and is gone again with it. (3) A
dry run (`--dry-run`, `--title … --dry-run`) asks herdr nothing about the
plugin, so links nothing; the plugin check of wt:orchestrate and wt:check
runs after their refusals (`--after` without `--queue`, no turbo, no
merge base), as every other command's. (4) Finding the pane this runs in
asks herdr's `process-info` 16 panes at a time: a stack:up/down outside
every pane (the overview's start, a plain terminal) asked 94 panes one
after another: 3.4 s in the world, now 0.64 s; the same pane wins (herdr's
order). Suite: two checks in setup.check.mjs, passing against Node too:
wt:orchestrate outside herdr (`--queue` waits in the record and makes
nothing; a fresh start leaves the worktree on `fix/230-…`, the pushed
claim, `.claude/origin`, the brief, the record's branch, the start
command); a rewrite also: an `INIT_CWD` of another repo is not followed.
Seen, left to the lifecycle piece: the undo of a failed `--now` start
(`wt:remove` of an empty branch) waits for a changelog rewrite although
the branch has no merged MR to list (7.0 s against Node's 4.2 s).
Why: the task's safety rule and rule 6 ((1)–(3)), rule 7 ((4)), rule 1.
Suite: setup 26/26, cli 18/18 (+1 todo), lifecycle 8/8.

## 2026-10-07 21:00 · overview / NEXT, ACTIONS · the hint as lazygit's options bar, answers on their way counted apart
Chose: NEXT's hint under its block is parts, each key cyan and bold as on
its button, each part whole on a line: `1-2 picks that option: typed to
its agent`, `1 looks good: typed to its agent · 2 broken: a draft in its
agent's input for you to finish`. Against: round 2's dim `press 1 looks
good: … · or click` (110 columns, two lines at 100). Why: the bar
(lazygit's options bar names each key in its colour and what it does);
a button and an option light on hover, so `or click` said nothing. No new
key binding: the digits are the ones logged on 2026-10-02.
Chose: under a ✓ in NEXT (a kind without answers, no key), the hint says
what a click on the rest of the block does, a question without options
`a click on the rest opens its space: answer it there`; none under a merge,
whose text says what its two clicks do. Against: Node, nothing. Why: the
critic's question "what can I do now": the ✓ only ticks off, the way to
answer the agent was nowhere on screen.
Chose: ACTIONS' note counts the answers on their way apart: `2 for you ·
1 answered, on its way`, `nothing waits for you · 3 answered, on their
way` (the click's answer counts at once). Against: Node's `3 for you` for
three answers that wait for their agent. Why: rule 3 deviation: they
wait for the agent, not the user; NEXT already says `nothing for you`.
Chose: an answer on its way reads `→ Rust + ratatui · its agent is at a
question: …`, the waits after a dim `·` glued to its first word. Against:
Node's `→ Rust + ratatui its agent is …`, one run without colour. The
suite's `→ looks good` still reads the same.
Kept: an action's text whole, no 12-line cap with `…` (Node's
`MAX_LINES`). Why: #304 and #273 (no cut text), and Node's own wrap breaks
a multi-line plan mid-line and keeps all of it; NEXT's sticky part is cut
to half the pane by the frame, its rest scrolls with the view.
Suite: buttons 23/23, content 24/24, hover 4/4, views 9/9.

## 2026-10-07 21:10 · overview / WORKTREES · #332: distance, collisions, app and stack on each node
Chose: what only the worktree view had stands on each node of WORKTREES,
since a worktree space of an orchestration gets no overview tab of its own
(#332; its Node design isn't on develop yet, so built from the requirement
and DIFF's, WHERE's and STACK's words): (1) how far it is from develop in
the work fact as lazygit's branches say it, `↑1 ↓13` (↓ yellow, none once
merged), where Node said `1 ahead`; (2) where its open work collides, a
yellow `⚠ collides with develop: tree.mjs · #304: render.mjs, index.ts ·
#302, #292: .env.local` line under the facts, only on a node that
collides: develop first, the other worktrees by their issue, the ones
sharing the same files in one part, 3 files a part then `+N more`, each
file by the shortest end of its path no other file of the node shares;
(3) its app and stack (`● :3002  +2 running(5)`, `⇒ :3000  shared`,
everything.rs's cells, `app_cell` now core's beside `stack_cell`) in a
column after the widest name, chosen once for all nodes when every state
still fits beside it; where it doesn't (80 columns) at the right of the
agent's own line, padded so the apps line up down the edge, else ending
the facts; with no .env.local in any worktree, no column and the title
says `no .env.local in any` once. Against: DIFF's sentence per overlap
with full paths and feature names (a node with six neighbours took 6
lines at 120 columns), `N ahead · N behind` in words (the agent fell off
the facts line on every node: 4 rows a node instead of 3), the app and
stack as facts (no column to scan). Why: rule 2 (#332); rule 7 and the
bar: lazygit's branches show `↑1↓2` and a column per fact; at 120 columns
a node stays 3 rows, one more only where it collides.
Chose: the collisions come from the local read: `git::on_develop` (the
work's files `git diff --name-only <merge-base> origin/develop` also has),
read with the other orchestration facts and cached with them by the
worktree's refs; the other worktrees from the `files` the merge order
reads already. Only the orchestration's own worktrees count (its read
lists no others). Why: rule 4, no new read; rule 7.
Chose: the orchestration's local read asks docker for the stacks only
when a worktree has a stack of its own, at most once in 12 s
(`DOCKER_EVERY_MS`); a shared or env-less worktree needs none. Against:
`compose_projects()` per local read, as the worktree view and the view of
everything still do. Why: #331 (one docker read every 10–15 s); the
daemon builder's shared read replaces this cache when it lands.
Not here: docker not answering reads as `+2 down` (Node's stackCell too);
the file names have no click (the node's click goes to its space).
Suite: content 24/24, hover 4/4 (28/28 against the release binary);
cargo test, clippy -D warnings, rustfmt clean.

## 2026-10-07 21:05 · herdr plugin, session, Stop hook · `breadboard herdr`, `herdr:install`, `hook stop-answers`
Chose: (1) one word for both halves of herdr.mjs and run.mjs: `breadboard
herdr` takes flags (`--here`, `--no-attach`, `--reload`, `--stop`,
`--install`) as `pnpm herdr`, and the plugin's actions as words
(`restore [--startup]`, `open <pane>`, `pane <pane>`, `status`);
`breadboard herdr:install` is `pnpm herdr:install`. The code is
`crates/breadboard/src/herdr/` (plugin, session, setup, own, reload,
sidebar, hook), `main.rs` dispatches it. Against: `breadboard plugin
<action>` (spec §18): a second name for one thing. Why: rules 4 and 5
(the pnpm names stay, the words never collide with a flag).
(2) The manifest of the rewrite is `herdr/herdr-plugin.rs.toml`, every
`command` `["../target/release/breadboard", "herdr", …]`: herdr runs plugin
commands in the plugin's directory, so the binary of the checkout herdr
linked runs, never one on PATH. The plugin's repo is the checkout of that
directory (`git rev-parse --show-toplevel` of the cwd), never the one
beside the binary: a binary of one checkout run in a world acts on the
world. The cut-over renames it over `herdr-plugin.toml`; `--reload` then
relinks (its sha256 changes). Why: rules 5, 8 and the loop's safety rule.
(3) The tab bar's status is `<plugin dir>/../target/release/breadboard herdr
status` in `.herdr/config.toml` (`node {{RUN}} status` of herdr/config.toml
is filled with it). `with_status_entry` turns an old `node …/run.mjs status`
(a config written before, a line you added, a moved package's) into the
binary's, so no tab bar gets two entries; `herdr:install` prints the line
for your own `[ui]` (a block can't add to it). Node's four
`withStatusEntry` cases hold byte for byte. Why: rule 5 with a reader for
the old form, rule 2 (#230: a move needs no hand work).
(4) `--reload` finds overviews by `overview.mjs` (Node's) or a `breadboard
overview` command line; after a pull that moved develop it builds the main
checkout's binary (`cargo build --release --quiet`, incremental) and runs
the rest as `<main>/…/breadboard herdr --reload --pulled`; without cargo or
a build it says so and goes on with itself. Against: re-exec of the stale
binary (Node re-execs its pulled .mjs; a binary has to be built first).
Why: rules 3 and 8.
(5) Downloads (the pinned herdr, Omarchy's database) through core's
reqwest; sha256 through `aws-lc-rs`, which rustls builds already (no new
crate in the build); the Omarchy database decoded by `zstd -dc` (pacman
needs zstd, so it is there where the probe matters; without it the hint
Node gives when its fetch fails). Against: `sha2`/`ruzstd` (a crate more),
curl. Why: rules 4 and 8.
(6) Small fixes on purpose: an unknown pane or action says the usage
instead of a stack trace; an unknown flag of `pnpm herdr` is refused
(Node ignored `--relaod` and started a whole session); `status` outside a
checkout counts by label instead of crashing, and asks no pane list
without a space of ours; the stack pane's hint names the action
(`pnpm stack:up --port <p>`, Node printed `stack:<action>` literally); the
commands pane prints the list when `less` is missing; `herdr:install` prints
the tab bar's line. Why: rule 1 (#306), the issue's "errors that say what
to do".
(7) `breadboard hook stop-answers` is core's `take_answers` (the lock, the
drafts, the claims, `answered` kept at 50); a missing actions file is no
answer (Node made `.claude/` for its lock), whatever fails exits 0 (#306).
For the cut-over, `settings.json`'s second Stop hook becomes `b="$CLAUDE_PROJECT_DIR/packages/breadboard/target/release/breadboard";
[ -x "$b" ] && exec "$b" hook stop-answers; exit 0`: a worktree not built
yet blocks no stop. Why: rules 8 (no Node), 3 (never fails, as today).
Not switched (the lead's, after every piece won): `package.json`'s `herdr`
and `herdr:install`, `herdr-plugin.toml`, `settings.json`. Smallest
changes elsewhere: `main.rs` (dispatch), `crates/breadboard/Cargo.toml`
(`reqwest`, `aws-lc-rs`), `cli/mod.rs` (`herdr`, `herdr:install` in the
usage and the typo hints).
Suite: new `plugin.check.mjs` (subject gets `plugin` and `hook`, the fake
docker answers from `docker.json`, `herdr` is a subject command): Node 16 +
2 rewrite-only skips, the binary 18/18; cli 18/18 (+1 todo).

## 2026-10-07 21:00 · overview / worktree view · Esc, a whole diff, one file's letter
Chose: a lone Escape that nothing follows within 40 ms is the Escape key
(`tui/input.rs` `Rejoin::take_escape`, `tui/run.rs` wakes for it).
Against: holding it until the next input, as before: the worktree view's
`Esc closes` did nothing, and the click after it ran against the hunks it
should have closed (measured in a world: Esc, 1 s, a click on a DIFF row
opened nvim on a hunk line). A cut mouse report's rest comes within a
millisecond, so #303's rejoin is kept. Why: rule 1 (the view's own hint),
rule 3 (a key the view offers works).
Chose: `… N more lines, click: all of them` shows the file's whole diff
inline (`Act::Hunks { is_whole }`), instead of opening nvim at the next
line. Why: the bar is lazygit's diff view, which shows the whole diff;
each line still opens nvim at itself (rule 4: one way to reach nvim).
Chose: the hunks of a file deleted in the worktree say `deleted here` and
their lines open nothing. Against: a click that answered `nvim: <file> is
no file of this checkout`. Why: a click that only fails is a dead button
(#303).
Chose: an area of one file names it with git's letter in its colour and
`uncommitted` when it is (`1 file · M tty.mjs  uncommitted`), as a
listed file's row does; where that doesn't fit after the bar the file goes
under its area as a file row, and the bars take back the room it would
have had. Against: Node's `1 file · tty.mjs`, which loses whether the file
was added, deleted or committed. Why: rule 3 keeps Node's facts and adds
none-lost ones (lazygit's files panel shows the letter of every file);
rule 4 (one look for a file, alone or listed).
Kept, checked against the spec (§2): every section, row and click of
render-worktree.mjs in its order; #332 changes nothing here (it removes
the tab for orchestrated worktrees, the view stays for the others).
Suite (release): views 9/9, hover 4/4, buttons 23/23.

## 2026-10-07 20:56 · TUI frame · the wheel while GitLab is read (#315), Tab and Enter after a scroll
Chose: a suite check for #315 (`budgets.check.mjs`, "every notch of the
wheel scrolls while GitLab is read"): GitLab at 8 s a request, from the
first frame a notch every 400 ms for 4 s, each must move a line inside the
pane up within 300 ms while MERGE REQUESTS hasn't come. Marked
`knownBug("#315: …")`; today's Node passes it here (the bug was seen on a
loaded CI runner), the rewrite's slowest notch moves in 16 ms. The fake
GitLab drops the log line of a request answered after its world was
disposed (it threw ENOENT into the next test). Why: rule 1 (#315's
acceptance criterion: the wheel scrolls while the header says `GitLab n/5`).
Chose: Tab with nothing picked, scrolled down, picks the first block in
the rows the reader scrolled to, not a sticky one (the chat row) that was
there all along; at the top as before. Why: the bar (lazygit's selection
starts where you look); rule 3 untouched (Tab is ours).
Chose: Enter on a pick the wheel took off screen scrolls it back into view
instead of running it; a second Enter runs it. Against: lazygit, which acts
on an unseen selection. Why: rule 6's spirit (a merge or an answer only on
what the reader sees), the click resolves against the frame on screen
(#303), so Enter does too.
Kept, checked again: the band without a wall clock (the 08:55 entry), the
sticky top over a load and a scroll (a 1.5 s GitLab at 120x40 and 80x24:
the band and the chat/NEXT rows never move, the block on top stays on top
when GitLab's rows come in), a resize 120x40 → 80x24 → 120x40 gives back
the same frame, hover follows the wheel, j, space, g in a pty.
Suite (release): budgets: first frame 176 ms, GitLab's data 755 ms, idle
0.20 % CPU, 21 MiB, scrolling 1.6 % CPU settled 7 ms, #315 slowest notch
16 ms; hover 4/4. Red, not this piece: "at most 10 GitLab calls" (13: the
daemon's GraphQL reads; 3 before it).

## 2026-10-07 20:55 · overview / everything · the first screen holds every list
Chose: live, the open MRs and the issues share the rows the pane has left
under WORKTREES: all of both when they fit; else each gets half, a list
that needs less leaves the rest to the other, at least one item each, and
a capped list ends in `… N more · click to show them · glab mr list` (or
`glab issue list`): a click shows that list whole (`Fold::Mrs`,
`Fold::Issues`), `click to show fewer` folds it again. `--once` prints all.
Against: Node's (all MRs, then `max(3, rows − used − 7)` issues), which at
80x24 shows neither list on the first screen: the 9 worktrees fill it.
Measured in the breadboard world: at 80x24 WORKTREES, `!318` and `#307`
on the first screen; at 120x40 6 of 14 MRs and 5 of 64 issues, the footer
one scroll away (it is no longer reserved: data first).
Why: rule 2 (the piece's requirement: MRs and issues within the first
screen at 80x24), rule 3 (Node's cut of the issues, extended to the MRs;
its `… N more` line kept and made a click), rule 4 (one budget for both).
Chose: a worktree's work is said whole on one line where the column holds
it (`untested: 1 commit · 1 ahead`), else in short as lazygit counts a
branch (`untested ↑1`, `✓ tested ↑3`, `2 untested ↑5` when fewer than
the ahead are untested, then `N changed`); only what the short form can't
say goes on a line under the name: a failure's `✗ tests failed: …`, or
everything when not even the short form fits. Against: round 1's second
line under every row at 80 (`untested` / `untested: 1 commit · 1 ahead`),
the same fact twice and 8 extra rows. Why: rule 4 and #304 (nothing lost:
`↑N` is the ahead, the check's word its state); the 80x24 requirement.
Chose: an MR whose branch is a worktree here names only its owner (`!308 …
lukasdiegelmann1`): WORKTREES has that branch with the same `!iid` above;
others' MRs keep `<branch> · <owner>`. An issue claimed by a branch with
an MR says `← !316` and not the branch (the MR names it); a branch without
an MR (`← claimed`) still stands after the title. Against: Node, which
repeats both. Why: rule 4 (one place for a fact), rule 3 (the branch is
still on screen, one section up); 8 of the world's 14 MRs went from 2-4
rows to 1-2 at 120 columns.
Kept: `keepOverall` (`overall.rs`) as built: after a full GitLab read of
the live view, when origin's develop head moved since
`.claude/changelog.json` or `.claude/changelog.md` is missing, `breadboard
changelog` runs on a thread, its head recorded once it wrote, a failure
tried again by the next read; a print never runs it (Node's `--once`
exits before its in-process write ends). Measured in the world: the
changelog written within the first read, `develop` = `git ls-remote`'s.
Also: the MR and issue tables are laid out under the section guard
(`guarded`), so a panic there fails those two sections, not the view (#306).
Elsewhere (smallest change): `act.rs` `Fold::{Mrs, Issues}` and their
toggles, `mod.rs` `Folds::{is_every_mr_shown, is_every_issue_shown}`.
Suite: views 9/9.

## 2026-10-07 20:50 · overview / MERGE REQUESTS · round 1 of the rebuild loop: the merged rows' age alone, closed apart, `→ you` bold
Checked against the spec (§11) first: merge order with lanes and joins
(core's `graph_of`, Node's glyphs, probed with `needs`/`after` edges:
`├─┐`, `├─┘`, lanes through bars and reasons), pipeline marks that open
their pipeline, whole titles (#304), the bar (#294), reasons with `→ who`,
the conflict cell and its button, the two-click merge (probed in a world:
`click again to merge` in red in the button's own cells, the second click
a single `PUT merge_requests/308/merge`, then `[overview] !308 merged` to
its agent), the merged fold kept over redraws, all of it in `--once`.
Nothing of Node's is missing; three things change:
Chose: a merged row's status is its age alone (`6 days ago`), dim, under
the fold's `▾ 42 merged`. Against: Node's `merged 6 days ago` on each of
42 rows. Why: rule 4 (the fold and the title say `merged` already; git
log and lazygit show a date column, not a word), and #304: the 7 columns
go to the title: in the breadboard world at 120 columns 2 of the 42
merged rows take a second line, was 9, most for one orphan word (`back`,
`agent`, `size`); at 80 every one still takes two (no cut, #304).
Chose: an MR closed without a merge says `closed` (core's
`merge_requests_of`, a 3-line change there) and the fold counts it apart:
`▾ 41 merged · 1 closed`. Against: Node, whose rows and fold said
`merged` for a closed MR too. Why: rule 3 deviation, logged: a closed MR
read as merged is a wrong fact, not a wording; the title's `N merged`
already counted only the merged.
Chose: in what an MR lacks, `→ you` is bold, every other who stays dim.
Against: Node's dim for all. Why: the bar (lazygit's status colours say
at a glance which rows are yours); the words are Node's.
Kept: the bar under every open MR with boxes, a full one too (#294's
acceptance, rule 2); `checking for conflicts …` on each unsettled row
(rule 1, the `…` after a space).
Suite: content 24/24, hover 4/4, buttons 23/23 (before and after).

## 2026-10-07 21:05 · overview / actor · round 3: measured against Node in a world, the log says what and how
Measured: four probes in gauntlet worlds, Node and ours side by side
(`*.tmp.mjs`, not committed). Idle 25 s in `withActions`: the same five
`[overview] waited …` lines typed, the same automatic `wt:remove` of
303, no GitLab write, byte for byte. A decide answer: the same
`herdr agent prompt … → Rust + ratatui`, the action moved to `answered`
with `deliveredAt`. Two clicks on a posted merge: the same reads, the same
`PUT …/merge` (`should_remove_source_branch`, `squash`, `sha`), `[overview]
!316 merged` to its agent, the diffs read, the action done. A draft whose
detailed status stays `draft_status`: both stop after one read with `marked
ready, can't merge: draft`. `--once`: no write from either. Node's and
our overview started 0.3 s apart on one orchestration: 5 lines typed, not
10, one pid file, either way round (rule 5).
Fixed: the main space's `--herdr` sync reported an agent's sidebar line
without the commit its pipeline ran for and without the handover announced
for it, so a pushed agent read `waits for you: merge` / `checks open` where
Node says `waits for the pipeline` (agent-state.mjs `isBehind`,
`handoverOf`); now the same seven lines as Node in the world, and the
tokens in Node's order. Why: rule 3; rule 6 (herdr writes exactly Node's).
Fixed: a click on `open orchestrator chat` whose space is gone makes it
(`orchestrator_space`, the layout's), resumes with `--ide` where the nvim
integration is on and keeps the space's tab order after a new tab
(`restoreChat`, `chatCommand`, `keepOrder`); it said `pnpm wt:herdr brings it
back`. Why: rule 3. The layout's `nvim` module is public for its `ide_flag`.
Chose: the log (DONE, the band) is lazygit's command log: what was done in
words (`told 305-… that its wait ended`, `removed 303-…`, `merged !316`),
the line typed or the command run word for word dim under it (`[overview]
waited 8 h …`, `pnpm wt:remove … --close-space`, `glab api --method PUT
…/merge --field should_remove_source_branch=true --field squash=false
--field sha=…`); an act's start and end are one entry (`◐ removing` becomes
`✓ removed`, the newest on top), an answer that waited for its agent is
replaced when it goes in. Against: round 2's one line `told <feature>:
<the whole line>` and a second entry for each end (8 entries for 5 acts).
Why: the bar (lazygit names the action and shows its command under it);
rule 4 (one entry per act); the band now says `told #307 that !316 is
merged` instead of `told #307`.
Not ours, seen: the view of everything got GitLab after 20-40 s in the
world where Node's took under 20 s (core's read); `glab config get token`
5 times in 25 s with a failed pipeline (Node 2): core's GitLab client.
Suite: buttons 23/23, isolation 4/4, cli 18/18 + 1 todo.

## 2026-10-07 20:40 · suite · the CLI clone's checks reviewed
Kept: `lifecycle.check.mjs`, `setup.check.mjs`, the CLI checks (blocked-by
links with and without GraphQL, GitLab down, a note on a stale issue, the
handover's refusal), the fake GitLab's `blockedByIssues` as a connection,
`isGraphqlDown`, `POST merge_requests`, a note counted on its issue, glab's
`api graphql` and typed `--field`, herdr's `plugin list` as a read,
`GIT_CEILING_DIRECTORIES` in a world (a world under a checkout reaches no
repo around it). Against today's Node every new or changed check passes or
is a known bug: cli 17 + 2 todo, lifecycle 4 + 2 todo + 2 rewrite-only,
setup 18 + 5 rewrite-only. Fixed: the three known bugs without an issue
number say `to file:` (the numbers come when the lead files them), lint
(prettier, an unused constant). Kept as they are: the `rewriteOnly` skips
(an unknown flag Node ignores would act on the shared world; Node's
internal names and a binary copied out of the repo the harness can't run).
Why: the suite's rules (README, "Adding to it"); rule 1.

## 2026-10-07 20:30 · CLI / setup, lifecycle · core's setups and local time, one copy
Chose: the setup claims of wt:orchestrate and the handover are core's
(`breadboard_core::setups`; its `Claim` got `none`, `file` and `release`
from the lifecycle copy), the setup group keeps only the hold for its
`fail` path; the changelog's local day and time are core's `time::day` and
`time::hm`. Gone: `cli/lifecycle/setups.rs`, the setup group's copy,
`cli/setup/localtime.rs`. Not yet: the groups' copies of core's
conversations (`lifecycle/claude.rs`, `setup/conversations.rs`), wait
(`write/wait.rs`), check (`setup/check.rs`), herdr (`lifecycle/herdr.rs`,
`setup/herdr.rs`), the repo (`*/repo.rs`) and the glab readers of each
group: each has its own tests and critic rounds; the next CLI round moves
them onto core one group at a time, the suite judging each move.
Why: rule 4 (one way to do a thing); rule 5 (the files stay as written).

## 2026-10-07 20:20 · CLI + overview · wt:herdr and wt:remove once
Chose: the lifecycle group's `wt:herdr` and `wt:remove` (critic rounds
against Node, `lifecycle.check.mjs`) are the only ones; the main space's
overview (`--herdr`) calls its layout in-process: `layout`,
`sync_stack_tabs`, `prune_spaces`, `sort_spaces`, under its `exclusive`
lock (the lock Node's layout takes too). Against: the overview piece's own
`cli/layout.rs`, `cli/layout/order.rs` and `cli/remove.rs` (2.1k lines,
built while the CLI was in its clone), now deleted. Why: rule 4; rule 6
(the herdr writes are the lifecycle group's, measured against Node there).

## 2026-10-07 20:10 · overview --once · the TUI's print, what the CLI's did better carried over
Chose: `overview` is the TUI's, live and printed once (`--once`, or stdout
no terminal); `main.rs` asks the read group only whether the command line
is refused. The read group's printer (`cli/read/overview/**`, 9k lines)
and its copy of core's GitLab cache are deleted. Carried over from it:
(1) the command line: `--help`, an unknown flag, `--orchestration` with
`--worktree`, a `--worktree` directory in no worktree are refused with
the usage, exit 1; (2) while GitLab hasn't answered a worktree says
`MR …`, as its issue's `#303 …`, never `no MR yet` (the suite's GitLab-down
check); (3) NEXT says `what is for you can't be told: GitLab didn't answer`
where a failed first read left `GitLab …`; (4) a print's error line ends
after its reason (`tried again` only where it is); (5) a GitLab that
answers REST and not GraphQL: core's orchestration read takes Node's REST
route (`gitlab/rest.rs`: the open MRs, the last 100 pipelines, the issues,
the lately updated MRs only for an unsettled branch, each open issue's
links, a failed pipeline's jobs, the awaited singles), remembered until
the next full read; a merged MR's deployment stays GraphQL's. Not carried:
`agent: 2nd issue (#297 → #305 at 13%)` (the TUI's #292 line names the
agent, its issues and merged MRs); colours with `FORCE_COLOR` in a print
(nothing pipes the overview: the plugin's overview pane runs live); the
merged worktree's own row before its removal (the TUI folds a due removal
into "done and removed", rule 1). Measured in the breadboard world at 120
columns: GitLab calls of a print 2 (TUI) against 5 (the read group's)
and 5 (Node); with GraphQL down 5 s of GraphQL's retries, then REST.
Why: rule 4 (one way to draw the overview), rule 1 (the cli checks), rule
3 (Node read REST), rule 7 (#305: the fewer calls).
Suite: cli 18/18 (+1 todo: wt:orchestrate needs claude).

## 2026-10-07 20:00 · merge · the CLI from its clone onto the branch's core
Chose: the branch's `breadboard-core` as it is: the CLI clone's changes to
actions, git, json and lock were on the branch already, record and time
are the branch's (#292's `display_name`, `hm`, `clock`), the rest the
clone's formatting at another width; the CLI built against it unchanged.
`cli/mod.rs` the clone's (a group per module, typo hints), `main.rs` and
the Cargo files the union. Both halves' log entries below, by time.
Why: rule 4 (one core), rule 5 (the formats both read and write).

## 2026-10-02 09:20 · overview / everything · herdr's layout in Rust, the work cell, the MRs at 80
Chose: herdr's layout in Rust (`cli/layout.rs`, `cli/layout/order.rs`):
`wt:herdr` and the main overview's `--herdr` build the main space, a space
per worktree (`worktree open`, else `workspace create`), the
orchestrations' spaces, stack tabs, the marks (`markOrigins`), prune the
spaces of removed worktrees (only when every pane stands in the deleted
directory, a tab an agent works in stays: wt:remove's `releaseSpace`) and
sort the group over herdr's socket (`workspace.move_block`, std's
UnixStream). The overview calls it in-process (Node's `keepInStep`): prune
and sort every tick, the layout when the set of worktrees changes, a
stack's tabs when it goes up or down, without resuming conversations.
Against: spawning `wt:herdr` (it resumes conversations and focuses the
main space, Node's loop does neither; it answered `not built yet`, so the
band showed a red `herdr not in step` all the time). Why: rules 3, 6, 8.
Measured in the breadboard world: `wt:herdr` writes exactly Node's list
(8 `tab close` of 299's space, kept the tab its agent works in; `worktree
open` and `workspace create` for 303; `report-metadata w4P --clear-token
orch1`; `workspace focus wD`), with the same 237 reads; the `--herdr` loop
over 30 s the same writes as Node's apart from how often it ticks (entry
below) and the order of `pane report-metadata`'s flags.
Chose: a new tab runs this binary under the package.json names: `'<exe>'
overview --herdr|--worktree <dir>|--orchestration <name>`, `'<exe>'
changelog --watch`, `'<exe>' wt:orchestrate <name> --changelog --watch`.
Against: Node's `node …/overview.mjs`. Why: rules 5 and 8; the changelog
commands are the CLI piece's to build.
Chose: the nvim tab's command (and the claudecode.nvim clone of ide.mjs
with it) is made only when the tab is made or its shell idles. Against:
Node, which clones on every layout while the plugin is missing. Why: rule
7; the herdr writes are the same.
Chose: a worktree's work names its check first (`untested: 1 commit · 1
changed · 1 ahead`) and is never shortened: where each fact fits the work
column it wraps in the column; where the column is narrower than a fact
(80 columns), the column holds the check in a word and the whole work
stands on the row's next line, under the name. Against: round 1, which
dropped `N ahead` and the commit count unmarked; Node, which cuts with `…`
and names the check last. Why: #304 and the critic; check first, so one
column of checks runs down the rows.
Chose: where an MR's cells go under its title (80 columns), its state and
pipeline always start the next line, padded to columns, branch and owner
after them where they fit whole. Against: after the title's last word.
Why: the critic: the pipeline landed somewhere else on every row.
Chose: the orchestration's heading `⇶ <name>` is a row: hover, and a click
brings its space `⇶ <title>` to the front (`FocusSpace` of its checkout;
`actor/herdr::space_label` names an orchestration checkout's space).
Against: Node's `⇶ <name>` after every worktree's name. Why: one line
instead of the same words on each row; the critic: it had no click.
Logged now, from round 1 (rule 3): without any .env.local the app and stack
columns go and the title says `no .env.local in any` once (Node: `—  no
.env.local` on every row); a worktree's MR and pipeline stand in a column
before the work, the work wraps last; ISSUES has the claim (`← !316`) as a
column and the claiming branch after the title (Node: right after the
`←`), so the titles line up; MR pipelines come from GraphQL's head
pipeline of each MR, so !102 and !184–!188 show `✓ passed` where Node says
`no pipeline` (its REST read pages only 100 pipelines and misses
`refs/merge-requests/102/head`, rule 1 over 3); the `wt_state` of a pane
names its open tries (`checks open: try #1 broken, … · !303`) where Node's
view of everything reads no actions (`waits for the pipeline · !303`), so
the main and the orchestration's overview no longer write two lines for
one pane (rule 4).
Suite: views 9/9, content 24/24, buttons 23/23, hover 4/4, isolation 4/4,
budgets 9/9; cli: wt:herdr green (the other wt:* are the CLI piece's).

## 2026-10-02 09:20 · CLI / setup group · round 2: wt:stack idempotent, exact claims, the Rust pre-push hook
Chose: (1) `wt:stack` in a worktree that has its own stack already
(COMPOSE_PROJECT_NAME and POSTGRES_PORT in its .env.local) sets it up
again instead of refusing: `▸ own stack on offset N already: app …,
Postgres …, Keycloak …, SeaweedFS …, set up again`, `pnpm dev:setup`, the
stack-tab sync, the summary, exit 0. Only an .env.local that differs from
the main one and is no stack's is still refused, now saying so. Node
wrote .env.local before dev:setup, so a failed dev:setup could never be
finished; the failure's hint now says `run pnpm wt:stack in <dir>, it
sets the stack up again`, and every other failing step of wt:new names
the commands that finish the worktree from there (`pnpm install
--frozen-lockfile && … && pnpm wt:stack`, `cp <main .env.local> …` with
--shared) instead of "run the step again". An agent's wt:stack summary is
`✓ own stack +N ready: app …, Postgres …, herdr: stack tabs …`, not wt:new's
"pushed, EnterWorktree" (nothing was pushed, the agent stands there).
(2) `wt:orchestrate <name> --end` of an orchestration nothing is left of
is ✓ and exit 0 as in Node (an end run twice), the line says what was
looked for: `✓ ⇶ tiny ended already: no herdr space, record, setups or
checkout of tiny is left`. Round one's `✗ no orchestration …
(pnpm wt:list …)` named a command that lists no orchestrations.
(3) The claim check of wt:new and wt:orchestrate matches only
`feature/<n>-…` and `fix/<n>-…`: Node's `*<n>-*` refused issue 5 for
`fix/305-…` and 23 for `feature/123-…`. (4) The plugin check (#230) runs
once nothing refused: a usage error asks herdr nothing (2–5 ms instead of
30–40), `--changes` (nvim polls it) never, and a child of this binary
(`wt:new --origin`, `wt:remove`, `--end` in the background, `--to`) gets
`BREADBOARD_PLUGIN_CHECKED=1` and leaves it to its parent. (5) Errors say
what is wrong: `the feature Bad_Name is not kebab-case (…): bad-name?`,
`the orchestration is missing`, `Foo is no issue: its number (12) or
<n>-<feature>`, `which issues? …`, `--cap is gone` before anything else,
`v1.5.0 is the tag, the version is 1.5.0: pnpm changelog:release 1.5.0`,
`the version is missing: …`, `the sentence is missing: …`. (6) wt:new's
stale-issue sweep reads the notes only of issues people commented on
(`user_notes_count`), as wt:issues does: one request per commented issue
instead of one per open one. The sweep stays its own reader until the
read group's `stale_issues` is reachable from here (its module is
private) or moves to core. (7) The brief of a fresh start is the
lifecycle group's `handover::brief_of`, one text for both (round one's
entry 00:55 (1) waited for core; the copies had drifted). (8) `breadboard
hook pre-push <remote> <url>`: Node's hook in Rust (the #184 MR guard
and the #298 gate, same texts, the docs-only stamp), passing the suite's
three #298 checks with a shim that calls it. The installed shim stays
Node's for now: wt:mr's installer (write group) writes the same file,
and plan step 5 switches both in one commit; the shim to switch to tries
`<top>/packages/breadboard/target/release/breadboard hook pre-push` when
that binary answers `hook --help`, else falls back to the Node hooks of
older branches, the marker kept.
Why: rule 1 (setup.check.mjs:236, the #298 checks), rule 3 ((2) Node's
exit code), rule 2 and the issue's "clear errors that say what to do"
((1), (5)), rule 7 and #305 ((4), (6)), rule 4 ((7), one brief), rule 8
((8)), rule 5 ((8): the shim's marker and fallback for older branches).
Suite: setup.check.mjs 23/23, cli.check.mjs 18/18 (+1 todo).

## 2026-10-02 09:10 · overview / actor · retried answers, the band's log, the --herdr look
Chose: the local read's 2 s look also asks herdr (one `pane list`, hashed)
and reads again when an agent's state or a pane changed, so an answer held
back by a blocked agent goes in within one look of it being free. Against:
reading only on a file change or after 15 s (14 s late). Why: rule 1 (the
buttons check "reaches it within 3 s"), rule 3 (Node reads every 2 s).
Idle CPU stays 0.2 % (rule 7): herdr is asked, nothing is read again unless
it changed.
Chose: an answer that gets in on a retry logs a green `decide #2 of #307 →
answer · typed into its agent's pane`, which replaces the yellow waiting
entry in the band and DONE. Why: the critic: the UI never said it arrived.
Chose: the band's short entry glues `of #307` and `→ answer` with no-break
spaces, so a narrow band shows the answer whole or leaves it out (DONE has
it). Against: `→ Rust +`. Why: #304 (no silent cut).
Chose: `· N more` counts every other thing done in the band's minute, not
at most 2. Why: it said `2 more` beside a DONE of 8.
Chose: with `--herdr` the view of everything watches herdr's states too, so
the sidebar's line of an agent follows within a look and pruning/sorting
runs on every herdr change. Against: Node's sync after every 2 s read,
which in the world closes the same tabs every tick (40 in 20 s, ours 8).
Why: rule 7 (idle near 0), rule 6 (no repeated writes herdr took already).
Chose: a try whose terminal didn't open names the step (`no space …`,
`herdr shows no tab try …`, `herdr didn't bring its tab try to the front`).
Against: Node's `no space of its worktree` for each. Why: the critic: it
blamed a space that exists.
Suite: buttons 23/23, isolation 4/4, budgets 9/9; cli: overview --once,
wt:remove ×2, wt:herdr green, the other wt:* are the CLI piece's.

## 2026-10-02 09:10 · overview / NEXT, ACTIONS · keys only for answers its agent gets, ✓ says what it does
Chose: in NEXT the digit keys give only answers that go to the agent (a
decide's options, a try's or plan's buttons, `done`), and since
2026-10-08 only once its block is picked with Tab (a stray digit does
nothing; see the round-5 entry); the ✓ of a kind
without answers (merge, change, a decide without options) has no key and
reads `✓ tick off  takes it off the list · its agent is not told`. Against:
round 1's `1 ✓` with `press 1 to answer`. Why: rule 3: Node has no keys
here at all, and its ✓ only ticks the item off (answers.mjs `answerWith`:
`doneAction`, nothing typed); the critic: one keystroke removed an agent's
merge ask under a hint that said it answered. A key in a lazygit menu says
what its row does; a ✓ that tells no one gets no single-key shortcut.
Chose: Enter is not an answer key: it runs only the block picked with Tab
(the frame), which is lit with its bar, as lazygit's menu runs its
highlighted row. Against: round 1's Enter = the recommended option with
nothing on screen selected. Why: rule 4, one way to pick a row; the
critic. The hint under NEXT names each key and where its answer goes:
`press 1-2 or click an option: typed to its agent`, `press 1 looks good:
typed to its agent · 2 broken: written into its agent's input, for you to
finish · or click`.
Chose: ACTIONS' note says what a click on the rest of a block does, from
the blocks shown: `a click on the rest opens its space, merges or runs its
try`. Against: Node's and round 1's `the rest of a block does it`. Why:
the critic (lazygit names what each row does); the note drops it whole
where it doesn't fit (80 columns), where each block's own text still says
`opens a terminal: …` / `two clicks merge`.
Chose: ACTIONS draws the action NEXT shows only by its head and `↑ in NEXT:
answer it there` (a click on it does the action). Against: Node, which
draws the full block in both places. Why: rule 3 deviation, logged: NEXT
is sticky, so the block is always on screen above; the same options twice
made two places to answer one question and cost ACTIONS 6-8 rows at 80x24.
Chose: an answer not yet delivered whose agent's pane was `blocked` at the
last try says `its agent is at a question: it goes in once that is
answered`. Against: Node's `at its next stop` for it. Why: the critic: the
agent is not going to stop; the words say what holds it up (a draft's own
`waits` is kept, as Node).
Chose: the answer in the log is quoted (`decide #2 of 307-x → “OpenTUI on
Bun” · typed into its agent's pane`). Why: the critic: a cut answer must
never read as a shorter one; the band (actor's `short_of`) shows it whole
or not, DONE in full.
Deferred: #308's auto-merge line under NEXT and its toggle. Node does not
have it yet; it comes with #308 (core's record, the band's toggle, the
actor's merge), not as a NEXT line alone. Rule 3 then makes it parity.
Suite: buttons 23/23, content 24/24.

## 2026-10-02 09:10 · CLI / wt:remove, wt:prune, wt:orchestrate --to, wt:herdr · lifecycle, round 2
Chose: (1) A removal whose MR read GitLab didn't answer starts no
changelog rewrite: it says `▸ the changelog of <name> is not rewritten:
GitLab did not answer; pnpm wt:orchestrate <name> --changelog tries
again`, still ✓ and exit 0; wt:prune leaves an orchestration's changelog
as it is when one of its removals got no answer. A rewrite that loses a
merge the changelog listed (a merge never becomes unmerged) is put back
the same way. Node blanked it to "Nothing merged yet."; the suite checks
it byte-identical now. (2) The /rename failure of a handover prints the
two lines to type on stdout above, the ✗ reason stays one line: the
overview shows a background command's last line. (3) An announced
handover keeps its --peer next to its --brief (`peers` of the record,
read by the later handover before the orchestration's peer); its dry run
and ✓ say which brief and peer apply. (4) ✓ green, ✗ red, ▸ cyan, and
wt:prune's verdict (kept yellow, would be cyan, done green), on a
terminal without NO_COLOR only. (5) Wording: a release names the feature
once; the leftIn refusal says "it enters its next worktree …" only for
an agent bound to another worktree; an issue that can't be read says
whether GitLab didn't answer or it doesn't exist, and what to do.
(6) Kept: current_pane is None without HERDR_PANE_ID, also for wt:herdr
from the plugin's restore: herdr answers with the focused pane then,
which doesn't run the command (Node's releaseSpace guards the same way).
Why: rule 2 (#306: a GitLab failure never blanks what is the point;
#247 brief and peer; #313 the overview's error line), rule 3 for the
rest of the wording (same facts, said once), rule 5 (`peers` is a new
field beside `briefs`, the old records read as before). Suite: the
lifecycle checks 8/8 and the CLI checks of these commands 6/6.

## 2026-10-02 09:04 · overview / WORKTREES · a node in 3 rows, a long state at the right
Chose: who works in a worktree (#292's `● name · N issues · N MRs merged`)
at the right of its facts line, under the state, when both fit; else on a
line of its own before the facts, as Node. Against: Node's agent line
always of its own. Why: the critic (lazygit wins on density, 4 rows a
node): at 120 columns a node is 3 rows (name/state, facts/agent, title),
7 nodes in 21 rows instead of 28; nothing is dropped (rule 3), the state
and the agent's dot share a column in the colour of whom it waits for.
Chose: a state wider than the line goes on lines of its own, each at the
right edge (tree.mjs). Against: round 1's words at the facts indent. Why:
rule 3, an undocumented deviation the critic found on queued nodes at 80.
Not here: OSC 8 links on `#N`, `!iid` and the pipeline mark need the
frame to carry a span's url to the terminal (core's `Span::url` has
them); the queued nodes' `handover failed: wt:orchestrate: not built yet`
is the CLI's stub. The settled-then-shift of WORKTREES is gone with the
frame's wake of the local reader (08:55): `removed #303` is in the first
settled frame.
Suite: content 24/24, hover 4/4, buttons 23/23, views 9/9, isolation 4/4.

## 2026-10-02 08:55 · TUI frame · the overview's own acts show at once, pages land on a head, the band
Chose: what the overview did by itself (a `wt:remove`, a delivery) wakes
the local reader when its effect comes back, so the result shows at once;
the wake only looks at the reader's stamp, the 2 s poll stays. Against:
waiting for the next poll. Why: rule 1 (#296): the reflow came up to 2 s
late, after the first screen that held still, so hover.check measured a
layout that then moved 3 rows; hover 2/4 at 00:50 was this, not the frame.
Chose: PgDn, space and PgUp land where the top row starts a block or a
section shows its title or pin, up to half a page less far, never further;
the end and the top as they are. Against: Node's fixed `rows − sticky − 1`.
Why: the critic: a page landed on an AGENTS line without its agent or
title; lazygit's pages start at an item; rule 3's reason (the last row
seen stays on screen) still holds.
Chose: a rebuild places the scroll by the anchor it was last placed by
while the reader hasn't scrolled since. Why: the critic: 120x40 → 80x24 →
120x40 moved the view a row (a block's rows in proportion rounded twice);
now the round trip gives back the same frame.
Chose: the scroll thumb runs beside the rows under a pinned title, not on
it. Why: the critic: `WORKTREES … removed ┃` read as part of the title.
Chose: below 30 rows (`band::TIGHT_BELOW`) the band leaves out its blank
first and last line and the sticky part the blank after the chat row and
after NEXT. Against: Node's 5-row band and blanks. Why: rule 7, the bar:
at 80x24 the list gets 18 rows instead of 14 (lazygit's commits: 21); the
band's ground and the islands keep the parts apart.
Chose: no wall clock in the band. Against: Node's `5:40:23 PM` on line 2.
Why: Node's was fixed at render time, so it showed the last redraw, not
now; a true one costs a frame a second (rule 7, idle CPU), the pane's
host shows the time, lazygit has none; `GitLab 14:57` says how old the
data is, the thing the clock stood in for.
Chose: the keys line is Node's, then ` · Tab Enter pick a row`; Tab,
Shift-Tab, Enter, Ctrl-D, Ctrl-U added. Why: rule 3 keeps every Node key
and its words in front (a narrow band drops the new ones first); the bar
is lazygit, which works without a mouse.
Suite: hover 4/4 (twice), budgets 9/9 (first frame 160 ms, idle 0.10 %,
24 MiB, 40 wheel reports/s at 1.6-1.9 % CPU settled 7 ms), views 9/9,
content 24/24, isolation 4/4; buttons 22/23 (the new answers-retry test of
the actor piece).

## 2026-10-02 08:40 · CLI / overview --once, wt:plan · what can't be read is said, not guessed
Chose: (1) the "blocked by" links of the open issues come from one
GraphQL query; when GitLab doesn't answer it (an older GitLab, a proxy
that passes REST only) each issue's REST `issues/<n>/links` is read side by
side, as Node did, and what neither answers is a ⚠ line under the title
band ("the merge order may miss one"); the suite checks the order with
GraphQL down. (2) An orchestration record that doesn't parse or lists no
`issues` is named with what to do (overview and wt:plan), where both said
"no orchestration recorded". (3) A worktree's `.claude/actions.json` that
can't be read is a ⚠ line in ACTIONS and its count in the heading, where
the view said "nothing waits for you". (4) An MR row's reason that names
the MR acting already (`waits for !311`) has no `→ !311` after it. (5)
wt:plan says when another change holds the record's lock and how long it
waits, instead of hanging silently. (6) While GitLab doesn't answer, a
worktree's facts say `MR …` (unknown), as its issue's `#303 …`, not
`no MR yet`: Node's guess is a known bug of the suite now.
Why: rule 3 (no silent loss: Node's REST links, every row stays), rule 7
(1 call where GraphQL answers, 5 for the whole view), rule 4 (4: the
arrow only repeated the MR), and errors that say what to do. Suite: the
CLI checks 21/21 against the binary (1 todo: wt:orchestrate needs claude);
Node fails the new GitLab-down check as its known bug.

## 2026-10-02 01:50 · CLI / wt:tick, wt:note, wt:mr --update, wt:action change · what the suite can't reach
Chose: the verdicts the world can't provoke are functions with tests of
their own: `is_ticked_back` (wt:tick: as many boxes as before, the chosen
ones ticked; a body that became a path has none), `is_note_back` (wt:note:
one of the newest notes is the text; an error object or no JSON is `⚠`),
`is_description_back` (wt:mr --update), and change's `report`: a draft
that failed keeps the change written and said, drops `is a draft now`, and
ends with `✗ glab could not make !N a draft (…): do it by hand …`, exit 1.
The 30-minute conflicts verdict was tested already (`verdict_of`). The
pre-push shim (19:45) is now step 5 of rewrite-plan.md, so the cut-over
can't miss it. Against: failure switches in the gauntlet's fake GitLab,
which three builders change at once this round.
Why: rule 1 (each `⚠` and `✗` path held by a check), rule 4 (pure
functions, no fake-GitLab knobs for four lines). Re-checked in a world:
`wt:mr --dryrun`, `wt:mr --help`, `wt:tick 307 3 --help`, `wt:action look
x --bogus` make 0 glab calls; a closed !316 gets a new draft (!319);
`wt:wait !308 --hours 0.0001` ends after 71 ms, `!99999 !99998` at once.

## 2026-10-02 01:40 · CLI / wt:remove, wt:prune · a changelog that lists the merge is not rewritten (#305)
Chose: a removal of merged work reads its MRs once anyway (the claim on
origin, `mergedAt`); when its orchestration's changelog lists the last
merged one (`[!<iid>](`) already, as the overview or an earlier removal
wrote it, nothing is rewritten: 22 calls, 1 of them glab, where it made
218 (99 glab), and in a world with a 200 ms GitLab it exits at 0.9 s
(Node 2.4 s, round four 3.7 s). Otherwise `▸ rewrite the changelog of
<name> (!<iid> is not in it yet)` says it, the rewrite runs beside
herdr's calls and the ✓ comes before the wait, as remove.mjs did; a
failure is said after the ✓ (exit 0: the removal is done). wt:prune
starts its rewrites at once, beside the plugin's check, and waits after
its report. When GitLab doesn't answer or there is no merged MR, the
changelog is rewritten as today's always did. Open: the rewrite itself
(setup group, `wt:orchestrate --changelog`) still asks GitLab per issue,
12 workers with two calls one after the other each: 2.8 s at 200 ms
where Node's takes ~1 s; a removal of a merge the changelog misses exits
at 3.1 s against Node's 2.4 s until it runs both reads of an issue at
once or adds the one merge (#305).
Why: rule 7 (measured above), rule 3 (the ✓ before the wait, as Node),
rule 4 (the check reads the file it would write, no new state).

## 2026-10-02 01:40 · CLI / lifecycle group · flags, the plugin check, wt:herdr's silent space, prune's columns
Chose: an unknown flag of wt:remove, wt:prune, wt:herdr and the
handover says the one meant (`unknown flag --dryrun: did you mean
--dry-run?`, cli/typo.rs, the same as the setup and write groups), on
the same two lines as theirs (`✗ …`, the usage). The plugin check
(#230, one `herdr plugin list`) runs once nothing refused: after a
removal's own steps, after the handover's agent, context and issue
checks; a refusal costs no extra process (round four checked on every
refusal). wt:herdr says `space <feature> not set up: herdr gave no id
back for it (worktree open, workspace create)` where both tools said
nothing. wt:prune pads the names to 32 as today's, or to the longest
when one is longer, so the column stays one: nothing reads these lines
(the overview runs wt:remove, not wt:prune; checked in src/).
Kept: wt:remove and wt:prune refuse outside a checkout (01:15).
Why: rule 3 (exact errors, no silent outcome), rule 7 (no process a
refusal doesn't need), rule 2 for the columns (the wording is today's).
Suite: `lifecycle.check.mjs` keeps --keep-branch, the uncommitted
refusal, #305's calls, wt:prune of a gone branch and leftover folders,
wt:herdr outside herdr and the flag hints green (Node: 4 pass, #305 a
known bug, the hints rewrite-only).

## 2026-10-02 01:20 · CLI / every command · an unknown command, `--help`, a bare `breadboard`
Chose: a name no group takes says `✗ unknown command <name>: did you
mean <nearest>?` and the usage with every command, a group per line, on
stderr, exit 1 (`wt:actoin` → wt:action, `action` → wt:action, `up` →
stack:up, `overvieww` → overview). `breadboard --help` (`-h`, `help`)
prints the same usage on stdout, exit 0; a bare `breadboard` on stderr,
exit 1. Against: round four's leftover `<name>: not built yet`, exit 2,
which named nothing; and Node's one line
`usage: node …/worktree.mjs new|orchestrate|…|remove …`, which lists its
internal names, not the ones people type (`pnpm wt:*`, `stack:*`).
Why: rule 3 (stderr, exit 1, `✗` as `fail`), rule 5 (the names typed
stay, the usage lists exactly those), rule 4 (one list in cli/mod.rs,
`cli/typo.rs` the one place that guesses, for commands and the write
group's flags). Suite: cli.check.mjs 14/14, 1 todo (wt:orchestrate).

## 2026-10-02 01:20 · CLI / wt:action, wt:tick, wt:note, wt:mr, wt:wait · a typo says what was meant
Chose: an unknown flag says the one it likely is, as setup's wt:new does:
`✗ unknown flag --dryrun: did you mean --dry-run?` (a flag two edits
away, or the only one it begins: `--message` → `--message-file`), then
the usage while it has at most 3 lines, else
`  pnpm wt:action --help shows every form` (one wrong flag was 11 lines
of usage). wt:wait says it the same way (`--hour`, `--cancle`; was
`--hour is no MR (!213) and no flag`), a word that is no MR
`x2 is no MR (!213 or 213)`. `wt:action lok x` says
`lok is no kind of action: did you mean look? (…)`. `wt:tick x` says
`x is no issue number (34 or #34)` as wt:mr does (Node's "This branch
carries no issue number" was untrue: the word was wrong, the branch had
one); a box that is no number `a is no box number: pnpm wt:tick 307 lists
them, numbered` (was the usage). `wt:mr --description-file <f>` with no
such file is `✗ <f> does not exist — write the description there first`
before GitLab is asked; it put the template down at
.claude/mr-description.md and asked to fill that instead.
`wt:action --orchestration ../../x` says wt:plan's
`../../x is no orchestration's name: names are kebab-case (…)`, not
`no orchestration … is recorded`. `wt:action --list` right-aligns the ids
to the widest one, so `  3  plan` and ` 155  look` start their kinds in
one column (spec §11 names Node's shift a bug).
Kept: `wt:mr --dry-run` prints `body <TMPDIR>/wt-mr-…/description.md`
as Node does, and that file stays (checked in a world: it exists after
the run). Still open: the pre-push shim runs `node …/pre-push.mjs` until
the hook is a breadboard command (wt:new installs the same shim; the
cut-over changes both at once).
Why: rule 3 + the bar (exact errors that say what to do), rule 4 (one
`nearest`, the same answer from every group of one binary), rule 6 (a
missing description file sends nothing and writes nothing).
Suite: wt:action, wt:tick, wt:note, wt:mr, wt:wait 5/5.

## 2026-10-02 01:10 · CLI / wt:issues · 2 calls, and an error that says what to do
Chose: an issue GitLab counts no comment of (`user_notes_count`, the
notes people wrote, in the issues list already read) is stale without
reading its notes; only an issue with comments has its notes read, to
see whether one came after the merge. In the breadboard world: 2 calls
where it made 13 (the 11 candidates have none), ~230 ms against Node's
~450 ms, the output byte-identical. Against: one GraphQL query for MRs,
issues and notes (spec §3's "simpler form"): the fake GitLab's schema has
no notes and the fake glab no `api graphql`, so it could not be checked,
and the count already leaves nothing to batch here. A GitLab error now
says what went wrong in words, glab's own reason after it, and what to
try: `✗ GitLab could not be read: GitLab can't be reached (Error: bad
port)` and `  glab api <call> failed: check the login (glab auth status)
and the network, then pnpm wt:issues again.` (a 401 reads `glab is not
logged in, or its login expired`, a missing glab `install glab: …`).
Node printed `GitLab: Node.js v25.8.0 (…)` there. For the gauntlet: the
fake GitLab doesn't raise `user_notes_count` when a note is posted, as
GitLab does; a world that notes an issue nobody had commented on and
then runs wt:issues would see it stale.
Why: rule 7 (measured: 2 calls, nothing lost), rule 3 with the bar for
the error (an exact error that says what to do), rule 4 (one REST path).

## 2026-10-02 01:05 · CLI / overview --once · a button never splits, narrow widths read as rows
Chose: wrapping keeps a button or chip whole (`[ ✕ dismiss ]` moves to
the next line, with colour too, where it is the coloured ground that
holds it); a word longer than its line breaks after its last `-` or `/`
that fits (`306-overview-merge-requests-` / `crash-row`, a URL after a
`/`), a hard cut only without either. A table row whose columns are wider
than the line is stacked: its first two cells on the first line, the rest
below a step further in (WORKTREES at 60 columns no longer wraps back to
the marker column); a last cell with too little room beside a wide first
column goes a step in from the edge instead of under it (AGENTS at 60: 11
columns of one-word lines before). An MR's status (`merged 14 h ago`)
goes on the line below its title where both don't fit, instead of past
the edge. The title band wraps a name too wide for it over its lines
(`--worktree` at 60 cut the branch to `feature/307-bre`, #304); the
worktree's path wraps under itself. STACK's migrations line is built
(develop.mjs `migrationsOf`, actions.mjs `pushMigrations`: `psql` with
`MIGRATION_DATABASE_URL`, the journal's `when` against drizzle's newest,
`✓ all N applied` / `⚠ N migrations not applied: …` and `[ ▶ pnpm
pg:migrate ]` / `the database doesn't answer: …`), with
`PGCONNECT_TIMEOUT=3` so a gone database costs seconds, not minutes.
Kept: ACTIONS' empty line ` none: …` and the `GitLab HH:MM` line have no
extra indent with FORCE_COLOR. Node's coloured print shows them two
columns further in only because its `dim()` puts an escape before the
box's two-space prefix, which its plain print strips; every other line
of the box agrees, so ours aligns them in both modes. Measured against
Node with FORCE_COLOR at 120, escapes stripped: the remaining
differences are these two lines, the dismiss buttons, and the titles
Node cuts with `…` (#304). At 120 and 200 columns plain, every view is
byte-identical to before but for the dismiss buttons.
Why: rule 3 + the bar (lazygit never splits a button), rule 2 (#304,
nothing cut), rule 3 for the migrations line (a parity loss until now).

## 2026-10-02 01:15 · CLI / lifecycle group · herdr's reads remembered only inside a layout
Chose: `cli/lifecycle/herdr.rs` remembers a read only inside
`herdr::remembering` (wt:herdr under its layout lock, wt:remove's release
of its tab and space), there until the next write, per thread; everywhere
else every read asks herdr. Against: round three's process-wide cache,
cleared only by this process's own writes: the handover's #272 re-check
after wt:new got the agent's state from before the setup, typed `/rename`
and the next issue into a working agent and said ✓ (Node refuses and
removes the worktree again). Now as Node: refused, worktree and claim gone.
Why: rule 3 (#272's refusal), rule 6 (no prompt into a working agent),
rule 7 (wt:herdr keeps its ~70 reads against Node's ~240: the repeats are
all inside the layout). Suite: `the handover` check (a fake pnpm sets the
agent working during wt:new) fails on round three's binary, passes on
Node and ours.

## 2026-10-02 01:15 · CLI / wt:remove, wt:prune · the changelog is waited for, once per prune
Chose: a removal of merged work rewrites its orchestration's changelog
(`wt:orchestrate <name> --changelog`) before its ✓ and waits for it, quiet
unless it fails (`▸ the changelog of <name> is not rewritten: <reason>;
pnpm wt:orchestrate <name> --changelog tries again`); wt:prune rewrites
each orchestration's once after all its removals. Against: round three's
detached process per removal, which outlived the command, dropped its
errors, and in a prune of N merged worktrees ran N rewrites of the same
file at once (~200 GitLab calls each).
Why: rule 3 (Node's wt:remove awaited the rewrite before exiting), rule 4
(one rewrite, no orphan process), rule 7 (a prune asks GitLab once per
orchestration, not per worktree). The same changelog as Node's in a world.

## 2026-10-02 01:15 · CLI / wt:remove, handover · small deviations of round four
Chose: wt:remove takes the feature in any place (`--keep-branch 303-x`,
its flags all stand alone; both tools refused it with the usage). The
word right behind `--after` is its value even without `!`: `--after x308`
says `✗ x308 is no MR (!232)` (today's took it for a second issue).
A handover whose session can't be read (no running claude shows it)
types the next-issue line at once, as Node did, instead of waiting a
second (handover in a world: 0.8 s, Node 1.3 s). Kept: the commands run
from a checkout of the repo (`✗ <dir> is in no checkout …: run it from
the main checkout or one of its worktrees`); Node found the repo from its
script's path, the binary's place (a target dir, later CI's prebuilt) says
nothing about which checkout is meant, and pnpm, the overview and the
plugin all start it in one.
Why: rule 3 + the bar (flags in any order, the exact error), rule 4.

## 2026-10-02 01:05 · CLI / wt:tick, wt:mr, wt:note · every text to GitLab through a file
Chose: descriptions and comments reach glab as `--field <key>=@<file>`
of `glab api` (a temp file, removed after): `PUT issues/<n>` (wt:tick),
`PUT merge_requests/<iid>` (wt:mr --update), `POST merge_requests`
(wt:mr: the branches, `Draft: <title>`, the labels, `assignee_id` from
`glab api user`, the issue's `milestone_id`, `remove_source_branch`) and
`POST issues/<n>/notes` (wt:note, after a `glab issue view` that gives the
comment's URL and the same `✗ GitLab has no issue #N` as wt:tick). The
value is always a path, so a text starting with `@` is never read as one.
Against: the text as one argument (`--raw-field description=<text>`,
`mr create --description`, `issue note --message`), which Linux caps at
128 KiB: a 180 KB description failed with E2BIG before it reached GitLab,
where Node ticked it; `--description-file`, missing in glab 1.114 (#157).
Supersedes 19:55.
Why: rule 3 (Node ticks a long description), rule 2 (#157), rule 6 (the
routes `glab issue|mr update`, `mr create` and `issue note` take; one read
more each, `user` or the issue). In a world: a 190 KB issue description
ticked, a 186 KB MR description sent, a 200 KB comment posted. Gauntlet:
the fake glab reads `--field k=@file` and typed values as glab does, the
fake GitLab answers `GET user` and `POST merge_requests`.

## 2026-10-02 01:05 · CLI / write group · glab's failures said exactly
Chose: a refusal is said with its status, `(glab auth status)` only where
the login can be the cause: `GitLab answered 404 Not found`, `GitLab
refused it (403 Forbidden): the account glab uses may not do this here
(glab auth status)`, `GitLab does not take glab's login (401 …): glab auth
login`, `GitLab failed (503 …): try again in a minute`; GitLab's JSON
message as words (`title is too long`); glab not starting (`glab could
not be started: …`) without the suffix. A call that got no answer keeps
`<glab's line> (glab auth status)`. A create GitLab refuses as invalid
adds `Is everything pushed? git push`. Against: the suffix after every
failure, raw JSON included (`404 {"message":…} (glab auth status)`).
Why: rule 3, an error says what to do; nothing another program reads
changes (rule 5).

## 2026-10-02 01:05 · CLI / wt:action · a broken actions.json is kept
Chose: an `actions.json` that is there but no JSON object (cut off
mid-write) is refused by every form of wt:action (`change` before it
drafts the MR): `✗ .claude/actions.json is no JSON object (cut off
mid-write?): its actions can't be read, so it is kept as it is.` and the
`mv … .broken` that starts empty. Against: Node and #306's parity, which
read it as empty, so the next post overwrote it and every open action
was gone without a word. The overview still reads it as empty (#306): a
view must draw, a write must not destroy.
Why: rule 3, #306 names refusing as better; rule 5, the file stays as
the other sessions wrote it.

## 2026-10-02 01:05 · CLI / wt:action · a lock left behind
Chose: a `<file>.lock` older than 5 s is taken over at once and said so:
`  takes over <lock>, left behind by a writer that ended`; only a younger
one prints `waits for …, held by another writer (taken over after 5 s)`.
Supersedes that line of 19:50 for a stale lock.
Why: rule 3: the old line promised a wait that did not happen.

## 2026-10-02 01:05 · CLI / wt:mr --update · refused without an open MR
Chose: `wt:mr --update` without an open MR of the branch (none, or a
closed one) is `✗ no open MR of <branch> to update: pnpm wt:mr opens it`,
before the push guard and any write. Against: Node, which opened a new
draft MR.
Why: rule 6 over rule 3: `--update` asks to send a description again;
an MR opened instead is a GitLab write nobody asked for (as 20:10 says
of typos).

## 2026-10-02 01:05 · gauntlet · a world reaches no repo around it
Chose: a world sets `GIT_CEILING_DIRECTORIES` to the folder that holds
it. Built under a checkout's `.claude/gauntlet/tmp`, a cwd outside the
world's repos (its home) found that checkout, and wt:action wrote into
its real `.claude/actions.json`.
Why: the loop never acts on the live workflow (rewrite-rules.md, Working).

## 2026-10-02 01:00 · CLI / overview --once · GitLab's call budget (#305) and the disk cache
Chose: the printed orchestration reads GitLab in at most three rounds of
calls side by side: (1) the open MRs, the last 100 pipelines, the issues
not known as closed; (2) the MRs updated lately, only for a branch neither
open nor known as final, the blocked-by links of all open issues in one
GraphQL query (`glab api graphql`, `blockedByIssues { nodes { iid } }` as
real GitLab has it), the jobs of a failed open pipeline; (3) single MRs,
only for what the lists don't hold. What can't change any more comes from
`.claude/gitlab-cache.json` and goes into it: Node's file, kinds and shapes
(`mrs` + `branches`, `issues`, `states`; `jobs` only read: the print reads
no job logs, and Node caches a failed pipeline's jobs only with them).
`--once` writes that cache: it holds only facts GitLab can't change any
more, Node's `--once` writes it too, and spec §9's "changes nothing" is
about the workflow (actions, deliveries, herdr, GitLab), which it still
doesn't touch. Measured in the breadboard world at COLUMNS=120: 5 calls
cold (was 11), 4 on the second run (was the same 11), whatever the number
of open issues (was 4 + one per open issue); the print is byte-identical
to before, and Node's print on the cache ours wrote is byte-identical to
its cold one. In the world the GraphQL call is a 404: the fake glab sends
`api graphql` to its REST routes, and the fake's `blockedByIssues` is a
list where GitLab has a connection (both for the gauntlet to fix). The
links are then left out, as Node's failed `issues/<n>/links` read did;
the scenario has none. Supersedes "GitLab's call budget waits for core's
client". Against: waiting longer for core's client (the critic's top gap
two rounds running; its cache is this file and format, so the move is a
swap); a second HTTP client with token, retries and rate limit in the read
group (core's job, rule 4).
Why: rule 1 and rule 7 (#305: at most 10 calls, a second run asks nothing
final), rule 5 (the cache file Node reads), rule 4 (glab, no new crate).

## 2026-10-02 00:55 · CLI / setup group · what waits for the halves to meet
Chose: three things of round two's critic stay open in the setup group,
on purpose. (1) The brief text exists twice, `cli/setup/orchestrate/issue.rs`
(a fresh start) and `cli/lifecycle/handover/brief.rs` (a handover): agents
read it (rule 5), so one copy goes into breadboard-core once the halves
meet, not into one group's module the other imports. (2) #230: the
lifecycle group builds `plugin::heal` (the plugin relinked, the repo's
config repaired, once per run inside herdr); `wt:new`, `wt:orchestrate`
and `wt:stack` call it from there instead of a second copy. (3) An unknown
command, `--help` and `help` belong to `cli/mod.rs`, which no group owns:
today's `usage: … new|orchestrate|…` with exit 1 goes there.
Why: rule 4, one way to do a thing; a copy now would be the thing the
critic flagged next round.

## 2026-10-02 00:50 · overview / MERGE REQUESTS · merged rows, reasons, the `…` mark
Chose: a merged row is Node's again: `!iid`, its pipeline mark (a click
opens the pipeline) and the dim `merged <ago>` at the right of its first
line, the title between. Against: round 2's age in the pipeline's column.
Why: rule 3: GitLab merges on a red pipeline too, and #308's `merged
automatically HH:MM` needs the status's own column.
Chose: a title's words that don't fit beside the status wrap under the
title column across the whole row, not only the title column. Why: #304
takes the `…`, so a long merged title at 80 columns needs two lines; this
keeps it at two instead of three (lazygit's subjects use the whole width).
Chose: a reason no wider than half the line is glued into one word, so
`try #2 broken → you` wraps whole; a longer one wraps at its words, its
`→ who` always glued. Against: Node, which breaks inside a reason. Why: the
critic (round 2); half the line, so a long reason never leaves the line
before it half empty.
Chose: `checking for conflicts …` with a space, as every `…` mark after a
word (`look::uncut`). Against: Node's `conflicts…`. Why: rule 1: the
suite's CUT check (`/[\w)(#]…/`, #304) reads `s…` as a cut.
Suite: content 24/24, buttons 22/22; hover 2/4 (one row too many lit in
both MERGE REQUESTS and WORKTREES, from the frame's change at 00:44).

## 2026-10-02 00:50 · CLI / pnpm commands · its lines from commands.mjs
Chose: `pnpm commands` reads the `DESCRIPTIONS` object of
`packages/breadboard/src/commands.mjs` as data (a small reader of its
`key: "…" + '…'` literals, no Node) instead of a Rust copy of it.
Against: the copy of round one, which drifts the first time someone adds
a script: AGENTS.md sends the new line to commands.mjs. Byte-identical
to Node's output for `commands` and `commands --all` in this repo; a
world without commands.mjs shows the scripts' commands, as Node does for
a script without a line. When commands.mjs goes with the cut-over, its
object moves to a data file both read, and AGENTS.md says so.
Why: rule 4 (one source of truth), rule 8 (no Node at run time).

## 2026-10-02 00:45 · CLI / stack:up · a herdr tab without a pane
Chose: a tab herdr creates without a `root_pane` (the command could not
be started in it) no longer counts as running: `▸ herdr: postgres
keycloak seaweedfs studio opened empty: herdr gave no pane, nothing runs
there` instead of `+postgres +keycloak …`. Node dies there with a
TypeError. Why: rule 3 says decide it; a tool that reports logs running
in an empty tab is wrong, and the line says what to look at.

## 2026-10-02 00:40 · CLI / setup group · the command line
Chose: the command line read by hand (`proc::Args`, ~60 lines) instead
of clap; each command names the flags it takes, and any other is refused
before anything happens: `✗ unknown flag --shraed: did you mean
--shared?` and the usage, exit 1 (a near flag within two edits is
named). Today's internal names run the same commands: `control
up|down|reset|ps`, `platform-changelog add|…`, `new`, `orchestrate`,
`stack`, `check` (the herdr plugin, tabs and the Node overview spawn
them). Against: clap, which would need a custom setting for each of
today's quirks (`--x=y` counted only so spelled, a value starting with
`--` counted as missing, `--after !1 !2` taking several, a flag before
the feature being a usage error); Node's silence (`wt:new 12-x --shraed`
made the worktree on its own stack and pushed the claim).
Why: rule 4 (fewer dependencies, the quirks in one place), rule 5 (the
names and flags stay), rule 6 (a typo is no push and no herdr write).

## 2026-10-02 00:35 · CLI / pnpm changelog · a failed GitLab read is no "changed nothing"
Chose: every GitLab read of the setup group is tried again on a 429, a
5xx, a timeout or the network, after 1 s and 4 s as lib/glab.mjs does,
and a `glab api` is killed after 15 s. A merged MR whose diffs still
fail stays out of `.claude/changelog.json`, so the next run reads it
again, and the run says so last: `! GitLab: HTTP 502 Bad Gateway (!310's
diffs): read again next time` (exit 0, the changelog is written). The
release tags keep the pages read and say what is missing. The kept MRs
are written by iid ascending, as Node's `Object.fromEntries` does.
Against: round one's `unwrap_or_default`, which stored `"310":
{"paths":[],"changesets":[]}` after one 502 and never read it again (the
MR fell into no group, gone from the changelog until changelog.json was
deleted); Node's way of dying with a stack trace and writing nothing.
Why: rule 5 (changelog.json is read by the overview: same shape, same
order); rule 3 says decide it, and a tool that heals itself on the next
run beats one that refuses the whole rewrite.

## 2026-10-02 00:32 · CLI / wt:orchestrate · the brief of an issue (#247)
Chose: an issue gets the `--brief` of the very run that starts it, else
the one an earlier run kept for that issue (`record.briefs[<n>]`), else
none; the record's single `brief` is still written for the Node
overview's queue starts while both run, but no longer read for issues of
another run. The dry run says per issue which it gets: `#285 gets the
brief of --brief <file>`, `… the brief an earlier run gave it` or `… no
brief of the orchestrator`.
Against: Node's `--brief ?? record.brief`, which handed one issue's
brief to every later issue of the orchestration. The wait for `/rename`
(#313) went with the handover to the lifecycle group (its log entry).
Why: rule 2, #247's criteria over today's behaviour; rule 5 for the
`brief` field other readers still use.

## 2026-10-02 00:30 · overview / worktree view · DIFF's files, its columns, DEVELOP's rows
Chose: DIFF lists the files of an area with more than one under it (up to
5, then `… N more:` by name), each with git's letter in lazygit's colours
(A/? green, M yellow, D red, R cyan), its path below the area's folder, its
`+a −r`, and a dim `uncommitted` when it is not committed yet; a click
opens nvim at that file's first change. An area of one file keeps Node's
row, `area +a −r bar 1 file · <name>`. Against: Node's one row per area
(spec §2.8). Why: the bar is lazygit's files panel (loop prompt): the file
a developer wants is one click away instead of behind its area's top file;
rule 3 keeps every Node row and click, this only adds rows.
Chose: the name column fits the area names, and a file's path only while
the bars keep their full 24 cells beside it; a longer path runs into the
counts (they follow it, or go on the next line in their columns) and wraps
after a `/`. The last cell gets what its widest wants up to 24 columns,
at least 16 (`MIN_WRAP`); the bars narrow for it (to 6) before it wraps,
and it wraps between `N files ·` and the name. Why: #304 and the previous
critic: one long path made the column 68 wide and split `uncommitted`
two letters a line at 80 columns.
Chose: DIFF's title parts its hint with ` · `, not Node's 2 spaces, so at
80 columns the hint drops alone. Against: Node, whose `fitParts` drops
`N uncommitted` together with the hint there. Why: rule 1 (#304: nothing
lost by a cut that a reader needs), the count is the fact, the hint is not.
Chose: a DEVELOP merge's title and branch wrap under the title column
(tableRows' hanging indent), not at column 1; the ISSUE title is Node's
`#N  d/t done` again, without the extra ` ✓`. Why: rule 3.
Suite: views 9/9, content 24/24, buttons 22/22, hover 4/4.

## 2026-10-02 · TUI frame · pinned titles, the sticky cut, the page, the band's cuts
Chose: a section's title stays pinned on top of the scrolling rows while a
line of that section with text on it shows under the pin, and only in a
pane of at least 4 title heights. Once only the section's trailing blank is
left, or it is gone, there is no pin and the row goes back to the content.
Against: Node, which has no pin. Why: lazygit's panels never lose their
title, and in a long WORKTREES or MERGE REQUESTS list the reader still
knows which section the rows belong to. Rule 1 for the fix: a pin over
the section's last line (`built yet` at 80x24, offset 6) hid a row that
the `↑ N` mark did not count.
Chose: a page (PgDn, space, PgUp) is `room − 1` rows, as Node's
`max(1, rows − sticky − 1)`, less the pin's rows when a pin covers the top
of where it lands. So 28 rows at 120x40 under a pin, 29 without one.
Why: the last row seen stays on screen as the first, and is not hidden
under the pin (Node's own reason for the `− 1`).
Chose: the sticky part (chat row, NEXT) is cut to half the pane, and its
rows past the cut scroll with the rest. Against: the spec's cut to the
whole pane. Why: rule 1: when the sticky part is cut to the pane, a NEXT
taller than the pane hides every other section and they cannot be
reached. The cut depends only on the pane size, never on the scroll.
Chose: the band's log of the last thing done is cut at a word, with no
`…`, as Node cuts the band's right side. The keys at 80 columns drop
`g G scroll` the same way, as Node does. Why: #304 (no `…`), rule 3.
Chose: an atom wider than a line (a branch, a long button) breaks after
its last `-`, `/`, `_`, `:`, `.` or space that fits, and only otherwise
in the middle of a word (`303-overview-every-drawn-` / `button-works`).
Why: #304, it still reads as the name.
Suite: budgets, hover, content, views, buttons, isolation green. Release, 120x40: first frame 115 ms
(the suite's clock, with six builds running on the machine), idle CPU
0.10 %, 23 MiB peak, 40 wheel reports/s at 1.3 % CPU, settled 7 ms after
the last report.

## 2026-10-02 00:30 · CLI / setup group · GitLab through glab until core's client
Chose: the setup group keeps one `glab api` per call (up to 12 side by
side, a pool instead of round one's chunks) until breadboard-core's
kept-alive HTTP client (`gitlab/client.rs` of wave 1, with the token,
Retry-After and the rate limit) is in this crate's core; then every read
of the group moves to it. Against: a second HTTP client in
`cli/setup/gitlab.rs` now. GitLab's rate-limit headers stay unread until
then (glab hides them). Also kept, from round one: the pre-push hook
stays Node's behind the shim wt:new and wt:mr install; wt:check writes
the stamp in Node's format, which that hook reads (#298, checked by the
suite's `setup.check.mjs`). `wt:orchestrate --end` says `✓ ⇶ <title>
ended` where Node says the name: the title is what the space shows.
Why: rule 4 (one client, not one per group); rule 5 (the stamp's
format, the hook's shim).

## 2026-10-02 00:20 · CLI / wt:remove, wt:prune, wt:herdr, handover · the plugin after a move (#230)
Chose: once per run inside herdr, `wt:remove`, `wt:prune`, `wt:herdr` and
the handover ask `herdr plugin list`; when `monocircuit.platform` is linked
from a path that is gone, they `plugin unlink` + `plugin link` the
package's current `herdr/` (the main checkout's, else the one next to the
binary, as session.mjs `pluginDir`) and say `▸ plugin monocircuit.platform
linked again from <dir>: <old> is gone`. The repo's `.herdr/config.toml`
gets every gone `…/herdr` (the listed one, and any `<dir>/run.mjs` whose
dir is gone) replaced, `server reload-config` only when this herdr runs
with that file (the session `platform`). `~/.config/herdr/config.toml` is
only read: a gone path there gets one line naming it and what to do
(`pnpm herdr:install writes the plugin's block again; a line of your own
there needs <dir>`). wt:remove checks after the removal (the plugin may
have been linked from the tree that just went), wt:prune after all its
removals; `wt:prune --dry-run` and a handover `--dry-run` ask herdr nothing.
herdr's `plugin list` format is read loosely (JSON: the entry naming the
plugin; text: its line and the indented ones under it); a listing that
names no path changes nothing.
Against: editing `~/.config` (the user's memory ask-before-global-config:
a change under ~ gets a question first, and the loop asks none; the plugin
piece's `herdr:install` owns that block); relinking a path that exists
(a plugin linked from a worktree on purpose stays).
Why: rule 2 (#230's criteria beat parity); rule 6 (herdr writes only when
the link is broken: one read a run otherwise). Note for the suite: the fake
herdr counts `plugin list` as a write (it is not in its READS); it is a read.

## 2026-10-02 00:15 · CLI / internal names · remove, prune, orchestrate
Chose: `breadboard remove|prune <…>` and `breadboard orchestrate <name>
<issue> --to <agent>` are `wt:remove`, `wt:prune` and the handover, as
today's `worktree.mjs remove|prune|orchestrate` (spec §0.1, the overview's
✕ and the tabs spawn them). `breadboard herdr` is not taken for wt:herdr:
it is the name `pnpm herdr` (setup/plugin piece) wants, and the plugin's
`restore` calls `breadboard wt:herdr --no-focus` at the cut-over.
Why: rule 5 (today's names keep working), rule 4 (one name per command
where two pieces want the same word).

## 2026-10-02 00:10 · overview / WORKTREES, AGENTS · clock form, armed dismiss
Chose: times in WORKTREES (`merged 16:01`, `removed 1 Oct 15:30`) in 24 h
`HH:MM`, the form of the band. Against: Node's locale form (`04:01 PM`
under en-US, 24 h under de-DE). Why: rule 4: one form on the whole screen,
no locale tables (`localtime_r` only).
Chose: the armed dismiss names the agent by its display name
(free-agents.mjs `dismissOf`, #292); the AGENTS layout is chosen once for
all rows against the widest label the button can show, armed included, so
a first click never reflows a row; where it does not fit beside the name,
the name goes on its own line under the display name instead of being cut.
Why: rule 2 (#292), rule 3, #304 (nothing cut with `…`).

## 2026-10-02 00:10 · CLI / wt:herdr · whose conversations are merged
Chose: the layout merges only the claude projects of worktree roots
(`<main>-<feature>`, there or gone) into the main checkout's project, told
by the `cwd` its transcripts started in; a project of a session started in
a subdirectory (`platform/packages/breadboard`, `platform-314-x/.claude/…`)
stays its own. Against: today's prefix match on the project's name, which
merged and symlinked those too (seen on the live machine).
Why: rule 3 says decide it; the name can't tell `/` from `-`, the
transcript can, and a merge is not undone (rule 5's spirit: no lost work).

## 2026-10-02 00:05 · CLI / wt:remove, wt:herdr, handover · what they say
Chose: `wt:remove` refused for uncommitted changes lists them first
(`▸ uncommitted in <feature>:`, the porcelain lines, at most 10), the ✗
line stays today's and last; refused for commits develop lacks it says
how many and its MR (`…has 2 commits that are not in develop, its MR !308
is open. Merge its MR first, or keep the branch with --keep-branch`;
merged without them → `An MR of their own (pnpm wt:mr)`; none → `pnpm
wt:mr`; GitLab silent → today's sentence), one glab read on that path.
`wt:herdr` says `▸ herdr: the layout of Monocircuit/Platform and its <n>
worktrees` before its seconds of work. A handover whose `/rename` doesn't
show at once says `▸ /rename <feature> typed, waiting for its session to
show the name` (and again when typed a second time); the record's
`renamed` is set only once the rename took (#313). A handover whose
wt:new failed after the worktree existed ends `✓ … told in its prompt (wt:new
failed after the setup, see above)`, as fresh starts do.
Why: rule 3 + the bar (lazygit names the files; a silent 20 s wait reads as
hung); the ✗ line other processes parse is unchanged.

## 2026-10-01 23:55 · CLI / overview --once · GitLab's call budget waits for core's client
Chose: the printed orchestration keeps its 11 GitLab calls (4 lists, then
the links of each open issue) and reads no disk cache, until the core
snapshot carries core's GitLab client (`crates/core/src/gitlab` of the
branch: one GraphQL query for the links, final things in
`.claude/gitlab-cache.json`); the print then moves onto it. `wt:issues`
keeps its notes per issue the same way; its error names GitLab once
(`✗ GitLab could not be read: Error: bad port (projects/…)`).
Against: a second HTTP and GraphQL client in the read group.
Why: rule 4, one GitLab client; `glab api graphql` is no way around it
in the world (the fake glab sends it to the REST routes), and the fake's
`blockedByIssues` is a list where real GitLab has a connection
(`{ nodes { iid } }`), which the suite should match.

## 2026-10-01 23:50 · CLI / overview --once · headings, refusals, what a print says
Chose: a box heading too long for its line drops its ` · ` parts from
the end, its counts first stay (boxes.mjs `fitParts`, no `…`): at 80
columns `WORKTREES 7 wait for GitLab · 1 will be removed · 41 removed`,
`AGENTS 3 free`, `MERGE REQUESTS 7 open · 42 merged`; only a first part
too wide for the line wraps. Round one wrapped the tail into the box,
where it read as the box's first line. `overview --worktree <dir>` of a
directory in no worktree is `✗ <dir> is not a worktree of the repo: pnpm
wt:list shows them` on stderr, exit 1 (Node printed `null` or the name
and went on, exit 0); `--orchestration` with `--worktree` is `✗
--orchestration and --worktree exclude each other: one view at a time`
and the usage, exit 1 (Node silently took the orchestration, round one
the worktree). GitLab's error line ends after its reason: a print asks
once, `tried again` was not true.
Why: rule 3 with rule 2 (#304: no `…`); rule 5 keeps the flags, the
input that can't be shown is refused instead of drawn.

## 2026-10-01 23:45 · CLI / overview --once · agents and tries as the live view knows them
Chose: the printed orchestration reads herdr once (`herdr pane list`,
inside herdr only, read only) and the conversation bound to each worktree
(`.claude/herdr-session`, its transcript's tail): WORKTREES says `at work`,
`asks you`, `→ next #N after !M`, `being set up for … (wt:new) …, leave
it`, `waits: its agent is working` / `<claude> still stands in it` for a
merged one, and per node `agent <name>`, `was <name>`, its context (`30%
of 200k · 5 min ago`); ACTIONS gets the `waits` of an agent's question or
permission prompt, `clean up` of a merged issue's open boxes, a failed
pipeline's `click to tell its agent`; a try shows its state in its head
(`ready`, `not pushed`, `waits for merge !N`, `deploying (publish, 3
min)`, `deploy failed (job)`, the deploy read only for an open try on the
test instance) and one not ready is grey, without buttons, skipped by
NEXT; UP NEXT the handovers (announced, or to the free agent that knows
the issue's files, else the one with most room) and fresh starts
(memory permitting); AGENTS each free agent's state and context; NEXT
`· 2 agents at work`; the resolve button only when a click reaches the
agent; `--worktree` the AGENT section. In a world with sessions and
transcripts the output equals Node's but for the heading (above) and
the uncut titles (#304); 1 herdr call where Node makes 4 (it reads the
orchestrator's tabs and panes for a chat button `--once` doesn't draw).
A print starts nothing: a due handover or fresh start is named, not run.
Not yet: STACK's migrations line (psql).
Against: waiting for core: its `conversations.rs` is in the snapshot but
not compiled; `cli/read/overview/agents.rs` is a trimmed copy of it and
goes once core exports it.
Why: rule 3, the critic's biggest gap (the 22:50 entry's lost lines);
rule 6, herdr is only read.

## 2026-10-01 23:40 · CLI / wt:orchestrate --to · one handover
Chose: `wt:orchestrate <name> <issue> --to <agent> [--after !<mr>…]`, the
call people and the overview make, runs the lifecycle group's handover
(`cli/lifecycle/handover.rs`): `cli/lifecycle.rs` takes `wt:orchestrate`
when `--to` is given and none of `--end`, `--changelog`, `--changes`;
every other `wt:orchestrate` goes on to the setup group. Its command line
is checked first: `--dryrun` is `✗ unknown flag --dryrun` and the usage,
exit 1, nothing read or written (it handed over for real before: worktree
made, branch pushed, herdr `pane move` and both prompts); `--title`,
`--queue`, `--now`, `--cap`, `--watch` with `--to` say they don't go with
it; `--help` prints what a handover does. The names `wt:handover` and
`wt:change` are gone: they were in no package.json. A change for an MR is
`wt:action change` alone (the write group's, which tells "no MR" from
"glab failed" and takes the actions lock); lifecycle's copy is deleted.
Against: two implementations each, the weaker one the one that ran.
Why: rule 5 (the names stay exactly, no new ones), rule 4 (one way to do
a thing), rule 6 (a typo is no herdr write). The setup group's
`hand_over`/`announce` in `cli/setup/orchestrate/mod.rs` are unreachable
now and go in its next commit.

## 2026-10-01 23:35 · CLI / wt:remove, wt:prune, wt:herdr, handover · the command line
Chose: the read and write groups' check and wording: `--help` / `-h`
prints what the command does and its usage on stdout, exit 0; an unknown
flag (`✗ unknown flag --keep-brnach`), a valued flag without its value, a
switch given one, a word too many (`✗ x is one argument too many`) is
refused with the usage, exit 1, before anything is read. `wt:remove`
takes the worktree's folder too (`../platform-303-x`, `platform-303-x`),
and a name that is none lists the worktrees after today's
`✗ <dir> is not a worktree (pnpm wt:list)`. Against: Node's way of
running on (`wt:remove 303-… --keep-brnach` removed the worktree and
deleted its branch, exit 0).
Why: rule 6 over rule 3: a typo of a flag that keeps something must not
destroy it; rule 4: one wording across the CLI (the three checkers belong
in one `cli/args.rs` once the groups meet).

## 2026-10-01 23:30 · CLI / wt:prune · a failed remove, the columns
Chose: a worktree whose removal fails gets `kept, wt:remove failed:
<reason>` on its line and the prune goes on with the others; at the end
`✗ <n> worktree(s) could not be removed: the reason is on each line
above`, exit 1. The names are padded to 32 as today, or to the longest
name when one is longer, so the second column stays one column.
Against: Node, whose in-process `fail` exited in the middle of the prune
(the branches and folders after it untouched, no report); the frozen
32-column padding that pushed long names' lines out of line.
Why: rule 3 says decide it; the entries are independent, and a prune
that stops halfway without its report is the worse tool (rule 4: the
same loop, no early exit). The padding changes nothing for names under
32 characters.

## 2026-10-01 23:25 · CLI / wt:remove · GitLab reads, the changelog, what it says
Chose: one `merge_requests?state=all&source_branch=` read answers both
questions of a removal (an open MR on its claim, when it was merged):
1 GitLab request where Node made 2. The orchestration's changelog rewrite
after a merged removal runs as a detached `wt:orchestrate <name>
--changelog` process (its own process group, output to nowhere), so the
removal neither waits for it nor cuts it off. `--keep-branch` ends
`✓ <feature> removed, <branch> kept` merged or not; when GitLab doesn't
answer whether an open MR builds on the claim, the claim stays and a step
says so with the command to delete it.
Against: Node's `writeChangelog` promise kept alive in the same process;
its silent claim kept on a GitLab error.
Why: rule 7 / #305: the rewrite itself still asks GitLab about 96 times
(one MR list per record branch, one issue per record issue) in the
world; it is the setup group's `changelog.rs` and moves to core's
batched/GraphQL client. Rule 3: the background rewrite is today's, only
the mechanism differs.

## 2026-10-01 23:20 · CLI / overview --worktree --once · the worktree's view
Chose: the printed view of one worktree ports render-worktree.mjs: WORK
(origin, issue, MR with its URL, `tests untested: 1 commit pnpm wt:check
before the push`, the delete button or why not yet), WAITING FOR, ISSUE
with its boxes per heading, PIPELINE with every job and a failed one's
log, DIFF against develop by area with its overlaps, DEVELOP since the
branch started with `✓ merges cleanly` (`git merge-tree`), WHERE and STACK
with containers. In the scenario world it equals Node's but for the
PIPELINE line, which wraps instead of cutting the last job (#304). 9
GitLab calls (Node 11 requests, 17 glab processes), 230 ms (Node 600 ms).
Not yet: AGENT (the conversation, core's transcript reader), the try
state of a merged MR's deploy, and STACK's migrations line (psql).
`--worktree` takes the worktree the directory lies in, not only its root;
one that isn't prints the directory (Node printed `null`).
Why: rule 3, the critic's "a developer would rather use Node's here";
rule 2 (#304) for the wrap.

## 2026-10-01 23:15 · CLI / overview --once · the view of everything
Chose: render.mjs ported: `platform  develop ◐ running` in the band,
WORKTREES with app, stack (`no .env.local`), work and `untested: 1
commit`, the MR and its pipeline; STACKS WITHOUT A WORKTREE; MERGE
REQUESTS with branch and assignee; ISSUES claimed first, claimed meaning
a branch on origin named for it (Node's rule; round one also counted an
open MR's branch, which said claimed where Node said unclaimed). Every
cell whole: an issue's title comes before its claim and labels and wraps
under itself (one line for most of the 64 issues, Node cut them with
`…`); a cell with too little room left goes on its own line under the
name (`!303 draft ✓ passed` under its worktree). 83 ms, 3 GitLab calls
and one `git ls-remote` (Node 365 ms).
Why: rule 3; rule 2 (#304) over Node's cuts; rule 4: one table helper
that wraps for all three views.

## 2026-10-01 23:10 · CLI / wt:plan, wt:list, wt:issues · small deviations of round one
Chose: outside a git checkout every read command says `✗ not in a git
checkout of the platform repo: run it in the main checkout or one of its
worktrees` (Node ran in its own source's repo); `wt:plan` refuses a name
outside kebab-case (`../x` read a file outside the records) and writes
the plan under `<record>.lock`, the lock `wt:action --orchestration`
takes, so the two never lose each other's write (Node wrote unlocked).
Why: rule 3 with the spec's bug list (§11): the same files and formats
(rule 5), the lock directory is Node's own, so Node and Rust keep working
side by side.

## 2026-10-01 23:05 · CLI / wt:list, wt:plan, wt:issues, overview --once · the command line
Chose: the read commands check their command line as the write ones do:
`--help` / `-h` prints what the command does and its usage (exit 0, no
GitLab read); an unknown flag, a valued flag without its value
(`overview --orchestration --once`), a word too many (`wt:list x`) is
`✗ <problem>` and the usage, exit 1. `wt:plan x --work=3` is refused with
`✗ --work takes its issues as separate arguments: --work 3`, `--work abc`
with `✗ abc is no issue number`. `overview` checks only when it prints
(the live view's flags are the TUI's); `--worktree` alone stays the cwd.
Against: Node's silence (`wt:plan x --work=3` wrote nothing, exit 0;
`wt:issues --help` read GitLab).
Why: rule 3 over the letter of rule 5: the names and flags stay, a typo
no longer passes as success. Hand parsing, one 60-line checker shared by
the group (rule 4); the write group has the same one, they belong in one
`cli/args.rs` once the groups meet.

## 2026-10-01 23:00 · TUI frame · redraw on change, atoms that never split
Chose: a generic frame (`tui/`): sections render into a Scene of atoms that
wrap whole, hit regions from the code that places the text, blocks and
buttons, each section in `catch_unwind`; redraw only on a change, at most
once per 16 ms; the clock ticks only for a section that asks
(`redraw_in`); the terminal size polled every 250 ms (a hidden herdr pane
may get no SIGWINCH); a lone Escape held until the next input so a split
mouse report still clicks; a scroll thumb in the last column; no crate but
`libc` (local time).
Why: rules 1 and 7 (#296, #303, #304, #306; idle CPU, wheel bursts);
rule 4 for the crates. Demo, release: first frame 16 ms, idle 0 %, 0.8 %
CPU at 40 wheel reports/s, 6.6 MiB (Node: 1.7 s, 10 %, 12 %, 146 MiB).

## 2026-10-01 23:00 · core · one GraphQL request a refresh, final things from disk
Chose: the orchestration's GitLab read is one GraphQL request (open MRs,
pipelines, jobs, issues) plus the shared disk cache `.claude/gitlab-cache.json`
in Node's shapes for merged and closed things; blocked-by links in a second
request in real GitLab's shape on full reads only; failed-job logs over
REST once per job; deployments only for branches with an open try on the
test instance; pipelines filtered on the client so the query is valid for
real GitLab and the fake.
Why: rule 7 (#305): the breadboard world at 250 ms a request, first
refresh 2 calls in 0.6 s, the next 1 call (Node: 154 calls, 7.2 s);
rule 5: the cache file is Node's, so both run side by side.

## 2026-10-01 23:00 · core · models are data, never actions
Chose: `orchestration_of` / `worktree_of` / `everything_of` return plain
structs, each section derived in its own `catch_unwind`; what a click or the
overview itself would do comes back as values (`Go`, `orders`, `tells`),
executed by the TUI; change detection by stat stamps plus a 15 s window,
no watcher thread; time via `localtime_r`, no chrono.
Why: rule 6 (writes only where the Node overview writes, in one place),
#306, rules 4 and 7.

## 2026-10-01 23:00 · CLI / overview --orchestration --once · the frame the live view draws
Chose: the printed view ports render-orchestration.mjs and what it draws
from (merge-order, mr-blockers, tree, numbering, history, next,
action-block, the auto actions, boxes and the title band) into
`cli/read/overview/orchestration/`, painted as Node paints (`FORCE_COLOR`:
islands, truecolor band, herdr's tones and pink, OSC 8 links). Every open
MR has its second line (`checking for conflicts… · pipeline pending →
GitLab · draft: mark ready → agent`), merged rows their pipeline, worktree
nodes their issue state, merge time, `untested: 1 commit`, `agent: 2nd
issue (#297 → #305 at 13%)` and wait, the state in the colour of whom it
waits for; WORKTREES counts as Node (`7 wait for GitLab · 1 will be
removed · 41 removed · 9.3 GB free …`, `agents 45 · issues 49 · reused 4,
started at 13% · handovers failed 1`; #242, no worktree and no removedAt,
counts as removed again). In the scenario world the output equals Node's
but for the free memory and the two differences below. Against: the
TUI's snapshot and section renderers, which aren't in this clone yet:
when core's model lands the printing moves onto it, the layout code
here is the reference for it.
Why: rule 3 (spec §9: `--once` prints the frame the TUI draws). Two
deliberate differences: nothing is cut with `…` (#304, rule 2): a long MR
title wraps under itself (`!318 … (#305)`, the merged titles); and a
wrapped line uses the box's full width (Node lost two columns to its
indent strip: AGENTS' `dismiss ]`, the reasons of !316 and !318). Rule 7:
171 ms and 11 GitLab calls against Node's 1.6 s and 157.

## 2026-10-01 22:55 · CLI / overview --once · GitLab, read only, no cache
Chose: an orchestration's GitLab read in lists side by side: open MRs,
the MRs updated lately, the last 100 pipelines (head pipeline by commit),
the issues by iid; then the links of the open issues, the jobs of failed
pipelines only, single reads only for what the lists don't hold (an MR's
pipeline, an awaited MR's state). No `.claude/gitlab-cache.json` is read
or written. Against: Node's per-branch reads (157 calls cold) and its
cache writes from `--once` (#305).
Why: spec §9 says `--once` changes nothing; a print that writes a cache
other overviews merge into is a write the user didn't ask for (rule 6 in
spirit), and the cache belongs to core's GitLab client (plan step 1),
which every view will share. Rule 7: 11 calls in the world, the jobs of a
running pipeline aren't needed for any line it prints.

## 2026-10-01 22:50 · CLI / overview --once · what the print can't know yet
Chose: until core reads herdr and the agents' transcripts (its
`conversations`, `check` and `wait` modules are in the snapshot but not
compiled), the printed orchestration knows no agent's herdr state: a
worktree whose agent has nothing to wait for says `no agent`, a free agent
shows `0 %` of its context, there is no `at work` / `asks you`, no try
state in an ACTIONS head, no `waits` action from an agent's question, no
handover or start in UP NEXT (workers.mjs), no `being set up` (setups) and
no clean-up held by an agent; GitLab's minute shows without the request
rate. Each comes with core's reader; none is dropped. With GitLab down the
error line under the band names the reason (`GitLab didn't answer 22:55
(Error: connect ECONNREFUSED …), tried again`, the first meaningful line
of glab's stderr, not its last `Node.js v25` line) and NEXT says `what is
for you can't be told: GitLab didn't answer` instead of Node's progress
counter.
Why: rule 3: logged, not lost; rule 4: no second herdr and transcript
reader next to core's.

## 2026-10-01 22:25 · CLI · built in a private clone, judged against Node
Chose: the four command groups (read, write, lifecycle, setup) built in
parallel in a local clone from a snapshot of core, each looped builder →
fresh critic. lazygit has no CLI, so the CLI critic compares ours blind
with today's Node output, with lazygit's polish as the bar, plus parity
and the suite.
Why: the loop's prompt (a critic per piece) where the bar has no
counterpart; a clone instead of a registered worktree so the live overview
never sees it ("never against the live workflow").

## 2026-10-01 22:15 · loop · durable scratch after a reboot
Chose: specs, results and scratch under `.claude/gauntlet/` of this
worktree (git-ignored), not `/tmp`.
Why: a reboot wiped `/tmp` with the six parity specs; they were recovered
from the agents' transcripts.

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
