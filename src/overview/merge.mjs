// Merging from the overview of an orchestration, by the human (#182): the
// first click on a merge action arms it, the second merges the MR as the user
// (glab, with the MR's settings, the source branch deleted). The click is the
// human's: an agent still never merges. After the merge the merge order
// carries on by itself: the agent hears that its MR is merged, NEXT moves to
// the next MR (render-orchestration.mjs), the agent of every open MR the
// merged one shares files with is told to merge develop in, and `clean up`
// offers the worktree (auto-actions.mjs).
//
// A merge is offered only when the work is done (#280): the pipeline passed,
// every box of the issue's Work section and of its acceptance criteria is
// ticked, and every `try` of its worktree was answered `looks good`. The one
// exception are the checks that need the deployment: a `try --on test` and
// an acceptance box marked `(after deploy …)` stay open across the merge.
// A merge its agent posted (`wt:action merge`) may be a draft: the click
// marks it ready, then merges. While anything blocks, the row is grey and
// says what.
import { dim, green, red } from "../lib/tty.mjs";
import {
  actionsFile,
  actionsOf,
  answeredOf,
  boxesOf,
  doneAction,
  mainDir,
} from "../worktree.mjs";
import { deliverTo } from "./answers.mjs";
import { filesOf } from "./diff.mjs";
import { exec, glab, sh, show } from "./io.mjs";
import { screen } from "./screen.mjs";

/** How long the first click keeps a merge armed. */
const ARM_MS = 5000;
/** How many shared files a line to an agent names before `+n`. */
const NAMED_FILES = 3;

/** How many words of a box name it in a row. */
const BOX_WORDS = 4;
/** How many open acceptance boxes a blocker names before `+n`. */
const NAMED_BOXES = 2;
/** What GitLab says while it works out whether an MR can be merged. */
const CHECKING = new Set([
  "checking",
  "unchecked",
  "preparing",
  "approvals_syncing",
]);
/** GitLab's `detailed_merge_status` in words; the rest with its underscores as spaces. */
const MERGE_STATUS = {
  conflict: "conflicts with develop",
  discussions_not_resolved: "open discussions",
  need_rebase: "needs a rebase onto develop",
  not_approved: "needs an approval",
  blocked_status: "blocked by another MR",
  not_open: "not open",
  ci_must_pass: "pipeline must pass",
  ci_still_running: "pipeline still running",
  draft_status: "draft",
};
/**
 * What GitLab's `detailed_merge_status` adds as a blocker, in words; null
 * for `mergeable` and for what a blocker already says (the draft, the
 * pipeline).
 */
export const mergeStatusText = (
  status,
  { isDraft = false, isPipelineBlocked = false } = {},
) => {
  if (!status || status === "mergeable") return null;
  if (CHECKING.has(status)) return "GitLab is checking";
  if (status === "draft_status" && isDraft) return null;
  if (
    ["ci_must_pass", "ci_still_running"].includes(status) &&
    isPipelineBlocked
  )
    return null;
  return MERGE_STATUS[status] ?? `GitLab: ${status.replaceAll("_", " ")}`;
};
/** The boxes of the issue's Work section, of its acceptance criteria. */
export const isWork = box => /^work\b/i.test(box.heading ?? "");
const isAcceptance = box => /acceptance/i.test(box.heading ?? "");
/** A box only the deployment can check: `… (after deploy, try #3)`. */
export const isAfterDeploy = box => /\bafter deploy/i.test(box.text);
/** The kinds of action that are a try. */
export const TRY_KINDS = new Set(["try", "browser-check"]);
/**
 * The tries of a worktree that still hold its merge, oldest first: one not
 * answered `looks good` yet (a `broken` not delivered too), and one answered
 * `broken` that no later try answered `looks good` (the fix tried again).
 * A try on the test instance (`--on test`) never holds it: it needs the
 * merge. `tree`: `{ actions, answered }`, as its actions file holds them.
 */
export const openTriesOf = tree => {
  const isTry = action => TRY_KINDS.has(action.kind) && action.on !== "test";
  const open = (tree?.actions ?? []).filter(isTry);
  const answered = (tree?.answered ?? []).filter(isTry);
  const good = [...answered, ...open].filter(
    action => action.answer === "looks good",
  );
  return [
    ...answered.filter(
      action =>
        action.answer === "broken" && !good.some(later => later.id > action.id),
    ),
    ...open.filter(action => action.answer !== "looks good"),
  ].toSorted((a, b) => a.id - b.id);
};
/**
 * What the human's checks still lack before a merge, each in a few words:
 * the open tries (`try #1 open`, `try #2 broken`), the acceptance boxes
 * not ticked but those after the deployment. None: they are all done.
 */
export const checksOf = (boxes = [], tree = null) => {
  const open = boxes.filter(
    box => isAcceptance(box) && !box.isTicked && !isAfterDeploy(box),
  );
  return [
    ...openTriesOf(tree).map(
      action =>
        `try #${action.id} ${action.answer === "broken" ? "broken" : "open"}`,
    ),
    ...(open.length
      ? [
          `${open.length} acceptance box${open.length === 1 ? "" : "es"} open: ${open
            .slice(0, NAMED_BOXES)
            .map(wordsOf)
            .join(
              ", ",
            )}${open.length > NAMED_BOXES ? ` +${open.length - NAMED_BOXES}` : ""}`,
        ]
      : []),
  ];
};
/** The first words of a box, without its markdown. */
export const wordsOf = box => {
  const words = box.text.replace(/[`*_]/g, "").split(/\s+/);
  return `${words.slice(0, BOX_WORDS).join(" ")}${words.length > BOX_WORDS ? " …" : ""}`;
};
/**
 * What keeps the MR from being merged, each in a few words; none when two
 * clicks merge it: only when GitLab says `mergeable`. `mr` as the overview
 * reads it (`isDraft`, `state`, `pipeline`, `hasConflicts`, `mergeStatus`)
 * or as GitLab answers it (`draft`, `head_pipeline`, `has_conflicts`,
 * `detailed_merge_status`); `boxes` those of its issue (boxesOf), `tree`
 * its worktree's actions (openTriesOf). Every Work box, every acceptance box
 * and every try is done first (checksOf). `canMarkReady`: its agent posted
 * the merge, so a draft is marked ready first.
 */
export const blockersOf = (
  mr,
  boxes = [],
  { canMarkReady = false, tree = null } = {},
) => {
  if (!mr) return ["no merge request"];
  if (mr.state && mr.state !== "opened") return [`it is ${mr.state}`];
  const blockers = [];
  const isDraft = mr.isDraft ?? mr.draft;
  if (isDraft && !canMarkReady)
    blockers.push("draft: its agent posts the merge");
  const open = boxes.filter(box => isWork(box) && !box.isTicked);
  if (open.length)
    blockers.push(
      `${open.length} work box${open.length === 1 ? "" : "es"} open: ${open.map(wordsOf).join(", ")}`,
    );
  blockers.push(...checksOf(boxes, tree));
  const pipeline = mr.pipeline ?? mr.head_pipeline;
  const status = pipeline?.status ?? pipeline;
  const isPipelineBlocked = status !== "success";
  if (isPipelineBlocked) blockers.push(`pipeline ${status ?? "not there yet"}`);
  const merge = mr.mergeStatus ?? mr.detailed_merge_status;
  if (mr.hasConflicts ?? mr.has_conflicts)
    blockers.push("conflicts with develop");
  else {
    const text = mergeStatusText(merge, { isDraft, isPipelineBlocked });
    if (text) blockers.push(text);
  }
  return blockers;
};
/**
 * How many checks wait for the deployment: the open acceptance boxes marked
 * after deploy, the open tries on the test instance (`tree`'s actions).
 */
export const toCheckOf = (boxes, tree = null) =>
  boxes.filter(box => isAcceptance(box) && !box.isTicked && isAfterDeploy(box))
    .length +
  (tree?.actions ?? []).filter(
    action =>
      TRY_KINDS.has(action.kind) &&
      action.on === "test" &&
      action.answer !== "looks good",
  ).length;

/** The files of `mine` the merged MR touched too, sorted. */
export const sharedFiles = (merged, mine) =>
  [...new Set(merged)].filter(file => mine.has(file)).sort();

/** `a, b, c +2`: the first few of `files`, how many more. */
const namesOf = files =>
  `${files.slice(0, NAMED_FILES).join(", ")}${files.length > NAMED_FILES ? ` +${files.length - NAMED_FILES}` : ""}`;

/**
 * The issues other than `number` whose MR is still open and whose worktree
 * is there, in the merge order (the rest by number).
 */
export const openAfter = (issues, number, order, of) => {
  const place = n => {
    const at = order?.indexOf(n) ?? -1;
    return at === -1 ? Infinity : at;
  };
  return issues
    .filter(
      issue =>
        issue.number !== number &&
        issue.tree &&
        of(issue).mr &&
        !["merged", "closed"].includes(of(issue).mr.state),
    )
    .toSorted(
      (a, b) =>
        place(a.number) - place(b.number) ||
        (a.number ?? Infinity) - (b.number ?? Infinity),
    );
};
/** The first of openAfter: the issue next in the merge order; null without one. */
export const nextAfter = (issues, number, order, of) =>
  openAfter(issues, number, order, of)[0] ?? null;

// ---------------------------------------------------------------------------
// Lines for the agents
// ---------------------------------------------------------------------------

/** Lines that didn't get into an agent's pane yet (blocked at a question): `{ dir, line }`. */
let pending = [];
/** One line to the agent of `dir`; kept for the next read when it doesn't get in. */
const tell = (dir, line) => {
  if (dir && !deliverTo(dir, line)) pending.push({ dir, line });
};
/**
 * On a read of the orchestration: the lines that didn't get in, tried again.
 * `dirs`: the worktrees still there; a line for a removed one goes.
 */
export const retryMergeLines = dirs => {
  pending = pending.filter(
    ({ dir, line }) => dirs.includes(dir) && !deliverTo(dir, line),
  );
};

// ---------------------------------------------------------------------------
// The merge
// ---------------------------------------------------------------------------

/** MR iid → `{ phase: "armed" | "running" | "merged" | "failed", error }`. */
const merges = new Map();

/** What GitLab refuses a merge with when it isn't done checking the MR (after another merge). */
const NOT_YET =
  /\b(405|406|409|422)\b|not allowed|cannot be merged|unprocessable/i;
/**
 * What went wrong, one line in words: no bare `HTTP 422`. `isNotYet`: GitLab
 * refused the merge the way it does while it still checks the MR.
 */
export const reasonOf = error => {
  const text = (error.stderr || error.message || String(error)).trim();
  const last = text.split("\n").at(-1);
  if (NOT_YET.test(text))
    return "GitLab is still checking the MR (it refused the merge for now)";
  if (/\b(401|403)\b/.test(text))
    return "GitLab refuses: no right to merge it as you";
  if (/\b404\b/.test(text)) return "GitLab doesn't find the MR";
  return last.replace(/\s*\(?HTTP \d{3}\)?/g, "").trim() || "GitLab refused";
};
/** Whether GitLab refused a merge the way it does while it still checks the MR. */
export const isNotYet = error =>
  NOT_YET.test(`${error.stderr ?? ""} ${error.message ?? ""}`);
/** How long a refused merge waits before its one retry. */
const RETRY_MS = 5000;

/**
 * After the merge of `mr` (fresh from GitLab): its agent hears it, its merge
 * action goes, and the agent of every other open MR (`others`, openAfter) is
 * told to merge develop in when its branch doesn't have the merged work yet
 * and touches a file it touched: one line each.
 */
const carryOn = async ({ issue, mr, file, actionId, others = [] }) => {
  if (file && actionId) doneAction(file, actionId);
  tell(issue.tree?.dir, `[overview] !${mr.iid} merged`);
  const dirs = others
    .map(other => other.tree?.dir)
    .filter(Boolean)
    // A branch built on the merged one has its commits already.
    .filter(
      dir =>
        sh("git", ["merge-base", "--is-ancestor", mr.sha, "HEAD"], dir)
          .status !== 0,
    );
  if (!dirs.length) return;
  const changes = await glab(
    `projects/:fullpath/merge_requests/${mr.iid}/diffs?per_page=100`,
  ).catch(() => []);
  const merged = changes.flatMap(change => [change.old_path, change.new_path]);
  for (const dir of dirs) {
    const shared = sharedFiles(merged, filesOf(dir));
    if (shared.length)
      tell(
        dir,
        `[overview] !${mr.iid} is merged and touched your files too (${namesOf(shared)}): merge develop in (git fetch origin, git merge origin/develop)`,
      );
  }
};

/** `glab api` with a write, as the user who runs the overview. */
const glabWrite = (method, endpoint, fields) =>
  exec(
    "glab",
    [
      "api",
      "--method",
      method,
      endpoint,
      ...Object.entries(fields).flatMap(([key, value]) => [
        "--field",
        `${key}=${value}`,
      ]),
    ],
    { cwd: mainDir() },
  );

/** How often, and how far apart, the MR is read again after it was marked ready. */
const SETTLE_TRIES = 10;
const SETTLE_MS = 1000;
/** The MR `iid` fresh from GitLab. */
const readMr = iid => glab(`projects/:fullpath/merge_requests/${iid}`);
/** The boxes of issue `number` fresh from GitLab; none without one. */
const readBoxes = async number =>
  number
    ? boxesOf(
        (await glab(`projects/:fullpath/issues/${number}`)).description ?? "",
      )
    : [];
/** The actions of the worktree `dir` fresh from its file, for openTriesOf; null without one. */
const readTree = dir =>
  dir
    ? {
        actions: actionsOf(actionsFile(dir)),
        answered: answeredOf(actionsFile(dir)),
      }
    : null;
/**
 * A draft marked ready (its title without `Draft:`), then read again until
 * GitLab worked out whether it can be merged.
 */
const markReady = async fresh => {
  await glabWrite("PUT", `projects/:fullpath/merge_requests/${fresh.iid}`, {
    title: fresh.title.replace(/^(\[draft\]|\(draft\)|draft:)\s*/i, ""),
  });
  let ready = await readMr(fresh.iid);
  for (
    let tries = 1;
    tries < SETTLE_TRIES &&
    (ready.draft || CHECKING.has(ready.detailed_merge_status));
    tries++
  ) {
    await new Promise(resolve => setTimeout(resolve, SETTLE_MS));
    ready = await readMr(fresh.iid);
  }
  return ready;
};

/**
 * Whether the answer `answer` to `action` puts the MR `mr` back to draft: a
 * try (not one on the test instance) found broken while the MR is ready.
 */
export const shouldRedraft = (action, answer, mr) =>
  TRY_KINDS.has(action.kind) &&
  action.on !== "test" &&
  answer === "broken" &&
  mr?.state === "opened" &&
  !mr.isDraft;
/**
 * A try answered `broken` (#280): its MR back to draft, until the fix is
 * pushed, green and tried again, and the merge its agent posted withdrawn
 * from `file`, its actions file. Nothing for any other answer.
 */
export const redraftOnBroken = async ({ action, answer, mr, file }) => {
  if (!shouldRedraft(action, answer, mr)) return;
  for (const merge of actionsOf(file).filter(other => other.kind === "merge"))
    doneAction(file, merge.id);
  try {
    await glabWrite("PUT", `projects/:fullpath/merge_requests/${mr.iid}`, {
      title: `Draft: ${mr.title.replace(/^(\[draft\]|\(draft\)|draft:)\s*/i, "")}`,
    });
  } catch {
    // Its agent puts it back to draft too (the brief): the overview only spares it the step.
  }
  screen.refresh?.();
  screen.redraw({ isReread: true });
};

/**
 * The merge itself, after the second click: the MR, its issue's boxes and
 * its worktree's tries read again (the overview's copy may be half a minute
 * old), refused while
 * anything blocks, a draft its agent posted the merge for marked ready
 * first, then merged with its own squash setting, the source branch deleted,
 * and only at the commit whose pipeline passed.
 */
const runMerge = async (context, state) => {
  const { mr, issue } = context;
  const canMarkReady = context.canMarkReady ?? Boolean(context.actionId);
  state.phase = "running";
  screen.redraw();
  try {
    const [first, boxes] = await Promise.all([
      readMr(mr.iid),
      readBoxes(issue.number),
    ]);
    const tree = readTree(issue.tree?.dir);
    let fresh = first;
    let blockers = blockersOf(fresh, boxes, { canMarkReady, tree });
    if (blockers.length)
      throw new Error(`can't merge: ${blockers.join(" · ")}`);
    if (fresh.draft) {
      fresh = await markReady(fresh);
      blockers = blockersOf(fresh, boxes, { tree });
      if (blockers.length)
        throw new Error(`marked ready, can't merge: ${blockers.join(" · ")}`);
    }
    const merge = at =>
      glabWrite("PUT", `projects/:fullpath/merge_requests/${mr.iid}/merge`, {
        should_remove_source_branch: true,
        squash: Boolean(at.squash),
        sha: at.sha,
      });
    try {
      await merge(fresh);
    } catch (error) {
      // Right after another merge GitLab still checks: say so, try once more
      // when it is done and says mergeable.
      if (!isNotYet(error)) throw error;
      state.error = reasonOf(error);
      screen.redraw();
      await new Promise(resolve => setTimeout(resolve, RETRY_MS));
      fresh = await readMr(mr.iid);
      if (fresh.detailed_merge_status !== "mergeable")
        throw new Error(
          `can't merge: ${blockersOf(fresh, boxes, { tree }).join(" · ") || reasonOf(error)}`,
        );
      await merge(fresh);
    }
    state.phase = "merged";
    state.error = null;
    await carryOn({ ...context, mr: fresh });
  } catch (error) {
    state.phase = "failed";
    state.error = reasonOf(error);
  }
  // GitLab now, not in half a minute: NEXT moves on, clean up shows.
  screen.refresh?.();
  screen.redraw({ isReread: true });
};

/**
 * A click on a merge action. `context`: `{ issue, mr, boxes, others, file,
 * actionId, canMarkReady }` — the issue, its MR and its boxes as the overview
 * read them, the other open issues in the merge order, the actions file and
 * id of a merge the agent posted (a draft of such a one is marked ready
 * first; `canMarkReady` says so for one the overview found, #242). While anything
 * blocks (an open try of `issue.tree` too) it opens the MR; the first click
 * arms it for ARM_MS, the second merges.
 */
export const clickMerge = context => {
  const { mr, issue, boxes } = context;
  const state = merges.get(mr.iid);
  if (["running", "merged"].includes(state?.phase)) return;
  const canMarkReady = context.canMarkReady ?? Boolean(context.actionId);
  if (blockersOf(mr, boxes, { canMarkReady, tree: issue.tree }).length)
    return show(mr.url, issue.tree?.dir);
  if (state?.phase !== "armed") {
    const armed = { phase: "armed", error: null };
    merges.set(mr.iid, armed);
    setTimeout(() => {
      if (armed.phase !== "armed") return;
      merges.delete(mr.iid);
      screen.redraw();
    }, ARM_MS);
    screen.redraw();
    return;
  }
  runMerge(context, state);
};

/**
 * How a merge action reads after its `!N`: what two clicks do, or grey with
 * everything that blocks them in one line, or where the merge is.
 * `canMarkReady`: its agent posted it, a draft is marked ready first;
 * `tree` its worktree, whose open tries block it.
 */
export const mergeText = (
  mr,
  text,
  boxes = [],
  { canMarkReady = false, tree = null } = {},
) => {
  const state = merges.get(mr.iid);
  const isDraft = Boolean(mr.isDraft);
  const verb = isDraft ? "mark ready and merge" : "merge";
  if (state?.phase === "merged") return green("merged");
  if (state?.phase === "running")
    return `${isDraft ? "marking ready, " : ""}merging …${state.error ? ` ${dim(state.error)}` : ""}`;
  if (state?.phase === "armed") return red(`click again to ${verb} !${mr.iid}`);
  const blockers = blockersOf(mr, boxes, { canMarkReady, tree });
  if (blockers.length)
    return dim(
      `${text} · ${[isDraft && canMarkReady ? "draft" : "", ...blockers].filter(Boolean).join(" · ")}`,
    );
  const toCheck = toCheckOf(boxes, tree);
  const after = toCheck ? ` · ${toCheck} to check after merge` : "";
  return `${text} ${state?.phase === "failed" ? red(state.error) : dim(`· two clicks ${verb}${after}`)}`;
};
