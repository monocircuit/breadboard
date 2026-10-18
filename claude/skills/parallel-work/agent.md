# An orchestrated agent

You were started by `{{run}} wt:orchestrate` with a brief (`.claude/brief.md`):
your issue, your worktree and branch, your base. This is what that brief
leaves out. The orchestrator's side of the same is [orchestrate.md](orchestrate.md).

## Your run

You run headless (`claude -p`, #358): breadboard resumes your conversation
whenever a line comes for you (an answer, a merge, a failed pipeline, the end
of a wait), and nothing runs in between. When you stop, you are done until
the next line.

- **Long commands in the foreground**: `{{run}} wt:check`, the gauntlet,
  builds. A background command or Monitor is stopped when your run ends, and
  its notification never reaches you (#417).
- **Wait only with `{{run}} wt:wait !N … --then "<what next>"`**, then stop:
  it records the wait and returns at once; its line starts your next run
  once the MRs are merged, or says why the wait ended. Never end a run with
  "I'll wait for …". An issue that builds on an open MR starts at once all
  the same.
- `AskUserQuestion` is off (#360): a question for the user is a `decide`
  action. A tool call auto mode would ask about is denied, and the overview
  asks the user for you (a `permission` action): say in your note what you
  need it for and stop. The answer comes as a line, `[overview] the user
  allowed <rule> …` or `… denied <rule> …`. A call a deny rule blocks never
  reaches the user: go on without it.
- The user may open your conversation in a tab and type there: their word
  wins over the orchestrator's.
- Your base moving is no reason to merge it: only on a conflict of your MR
  or when told, and only with `{{run}} wt:merge-base` (#396).
- A handover to your next issue comes as a line,
  `[overview] next issue #<n>, your name is <feature> now: …`: do what it
  says, the new worktree's `.claude/brief.md` is your task from then on
  ([herdr.md](herdr.md), "Next issue").

## The work

Per step: commit by path, then `bb submit [<box>…]` (`wt:check` unless HEAD
passed it, push, `wt:tick` of the boxes given, and what the MR still lacks).
At once: what a run leaves unpushed, breadboard checks and pushes once it
ended, and a red check comes back to you (#450). The draft MR comes with the first commit (`{{run}} wt:mr`, a
description you write). `bb status` says where you are and the next step.

- A test that fails on `{{branch}}` too is fixed once, in its own fix MR;
  the others wait for it with `wt:wait`.
- **Your pushes skip CI** (#357): breadboard starts your MR's pipeline once
  every box is ticked (after-deploy ones excepted) and `wt:check` passed at
  HEAD. Don't start one yourself, and don't mark the MR ready: breadboard
  does that once every box is ticked, and puts it back to draft when one is
  unticked.
- A failed pipeline: breadboard first retries a job the runner failed and
  one that failed only once (#371). A job that failed twice comes as the line
  `[overview] the pipeline of !N failed: job …, <url>: fix it, … (CI fix 1/3)`:
  read the job's log (`glab ci trace <job id>`), fix what is your MR's doing,
  `wt:check` and push in that run. Each push is one of a few fix runs; past
  them the user looks. When the line says `{{branch}}` fails the same job,
  say so in the MR (`glab mr note`) and go on.
- **A change requested for your MR**: `{{run}} wt:action change "<what>"`
  first (the MR goes to draft, your merge action is withdrawn), then build
  it, and post the merge again once it is pushed and green. Merged
  meanwhile: the command says so, and the change gets its own issue, branch
  and draft MR at once ([orchestrate.md](orchestrate.md), "Change requests").

## Telling

The overview shows your work: state, commits, MR, pipeline, boxes, what you
wait for. No status messages.

- At every stage `{{run}} wt:action note "<one line>"`: what you are on, in
  plain words for the user (`cause found: …`, `waits for !N`).
- `SendMessage` to the orchestrator only for a plan that differs from the
  issue, a decision that is the user's (with its `decide` posted), or what
  concerns another agent's work. Reply to the `from` of its messages.

## Actions: the user's next step

Whenever you stop and the next step is the user's, leave an action with
`{{run}} wt:action`; while you work, leave none. The overview's NEXT and
ACTIONS come from them: the user never has to ask you what is next.

- `merge "<what is in it>"` once the work is done: the pipeline passed,
  every box ticked, every try answered `looks good`, its turn in the merge
  order (`wt:plan`). Never merge yourself.
- `try "<what to check>" --run "<command>"` or `--url <page>` for what only
  a person can judge; the box it leaves open names it (`… (try #<id>)`) and
  is ticked on `looks good`. A check that needs the deployment is
  `--on test --url <path>`, posted with the merge, its box
  `(after deploy, try #<id>)`. A `broken` puts the MR back to draft until a
  fix is pushed, green and tried again.
- `decide "<what is decided>? <why the recommendation>" --option "<label>"
  --why "<what happens>" … --recommend <label>`: short labels that say it,
  not `A`/`B` ([orchestrate.md](orchestrate.md), "Answers").
- `plan "<the plan in one line>"` before larger work; `look "<what>"` for
  anything else the user has to do.

The answer comes as one line of your next run, `[overview] #<id> <kind>
"<text>" → <answer>`; a `broken` or `different` carries the user's words
after it: work from those. An action that no longer applies:
`{{run}} wt:action --done <id>` (`--list` shows them).
