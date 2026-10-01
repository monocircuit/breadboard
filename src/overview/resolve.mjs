// The conflict state of an open MR in MERGE REQUESTS (#246, #290): a button
// that has its agent merge develop in and resolve the conflicts, shown only
// while a click can do that. GitLab's `has_conflicts` counts once its check
// is settled and it has the pushed head; before that the row says it is
// checking. A click the agent can't take is a grey button with the reason;
// after a click the button turns into where the order stands (queued, merging
// develop in, pushed) until GitLab no longer reports conflicts. The cell
// leads the row's second line, before what else the MR lacks (mr-blockers.mjs).
import { dim, green, red, yellow } from "../lib/tty.mjs";
import { deliverTo } from "./answers.mjs";
import { button } from "./boxes.mjs";
import { ago } from "./render-worktree.mjs";
import { screen } from "./screen.mjs";

/** Between the words of the button: a wrap never parts them. */
const GLUE = " ";
/** The button's words, as a line holds them. */
export const RESOLVE = "resolve merge conflicts".replaceAll(" ", GLUE);
/** How long a click's "didn't get in" stays next to the button. */
const NOTE_MS = 10_000;
/** How long "conflicts resolved" stays before the row says "no merge conflicts". */
const RESOLVED_MS = 60_000;
/** GitLab's `merge_status` while it works the conflicts out again (after a push). */
const RECHECK = new Set(["unchecked", "checking", "cannot_be_merged_recheck"]);
/** GitLab's `detailed_merge_status` while it does. */
const CHECKING = new Set(["unchecked", "checking"]);

/**
 * Whether GitLab's `has_conflicts` of `mr` is about its pushed head and
 * settled: its check isn't running, and it has the commit `tree` pushed last
 * (a push it hasn't seen yet makes the answer one about older commits).
 */
export const isConflictSettled = (mr, tree = null) =>
  !RECHECK.has(mr.mergeCheck) &&
  !CHECKING.has(mr.mergeStatus) &&
  !(tree?.pushed && mr.sha && mr.sha !== tree.pushed);
/** Whether GitLab reports conflicts for `mr`, settled (isConflictSettled). */
export const hasSettledConflicts = (mr, tree = null) =>
  Boolean(mr.hasConflicts) && isConflictSettled(mr, tree);

/**
 * What the clicks did, by the MR's iid: `{ isSent, at, pushed, isStarted,
 * resolvedAt }`, `pushed` the commit on origin at the click. A sent order
 * stays until GitLab no longer reports conflicts (then "resolved" for
 * RESOLVED_MS); one that didn't get in is a note for NOTE_MS.
 */
const clicks = new Map();

/** The line its agent gets: merge develop in, resolve, check, push. */
export const resolveLine = mr =>
  `[overview] !${mr.iid} has merge conflicts with develop: merge develop in (git fetch origin, git merge origin/develop), resolve the conflicts, run lint, typecheck and test of the packages they touch, commit the merge and push.`;

/**
 * Why a line can't reach the agent of `tree`, or null when it may:
 * `agents` as agentsOf (empty outside herdr, then nothing is known).
 */
export const noAgentOf = (tree, agents) => {
  if (!tree) return "no worktree, so no agent: merge develop in yourself";
  if (!tree.claude) return "no agent in its worktree";
  if (agents.size === 0) return null;
  const agent = agents.get(tree.claude.id);
  if (!agent) return "its agent isn't running";
  if (agent.status === "blocked")
    return "its agent is at a question: answer it first";
  return null;
};

/** Whether the agent of `tree` took the order of `mr` up: its last prompt is it, after the click. */
const hasStarted = (mr, tree, click) =>
  click.isStarted ||
  Boolean(
    // The button's line or the automatic one (auto-actions.mjs).
    /has (merge )?conflicts with develop/.test(tree?.claude?.prompt ?? "") &&
    tree.claude.prompt.includes(`!${mr.iid} `) &&
    tree.claude.at &&
    tree.claude.at.getTime() >= click.at,
  );

/**
 * Where a sent order of `mr` stands, as words: queued behind the agent's
 * turn, the agent merging develop in, or pushed for GitLab to check.
 */
const progressOf = (mr, { tree, agents, behind }, click) => {
  if (hasStarted(mr, tree, click)) click.isStarted = true;
  const since = dim(ago(new Date(click.at)));
  if (!click.isStarted) {
    const isWorking = agents.get(tree?.claude?.id ?? "")?.status === "working";
    return `${yellow(isWorking ? "◐ queued for its agent (at work)" : "◐ sent to its agent")} ${since}`;
  }
  const isPushed =
    tree && !tree.unpushed && behind === 0 && tree.pushed !== click.pushed;
  return `${yellow(isPushed ? "◐ pushed, GitLab checks" : "◐ its agent merges develop in")} ${since}`;
};

/**
 * The conflict state of an open `mr` as the start of its second line, `{
 * text, canClick }`: `canClick` when the text holds the button a click on
 * which reaches its agent. `tree` its worktree (`claude`, `unpushed`,
 * `pushed`), `agents` as agentsOf, `behind` the commits of develop its branch
 * lacks.
 */
export const conflictStateOf = (
  mr,
  { tree = null, agents = new Map(), behind = 0, now = Date.now() } = {},
) => {
  const click = clicks.get(mr.iid);
  const isSettled = isConflictSettled(mr, tree);
  if (click?.isSent) {
    if (isSettled && !mr.hasConflicts) {
      click.resolvedAt ??= now;
      if (now - click.resolvedAt < RESOLVED_MS)
        return { text: green("✓ conflicts resolved"), canClick: false };
      clicks.delete(mr.iid);
    } else if (
      isSettled &&
      click.isStarted &&
      tree?.pushed &&
      tree.pushed !== click.pushed
    ) {
      // Pushed, and GitLab still has conflicts at that commit: the button again.
      clicks.delete(mr.iid);
      return {
        text: `${button(RESOLVE, "yellow")} ${dim("conflicts left after its push")}`,
        canClick: true,
      };
    } else
      return {
        text: progressOf(mr, { tree, agents, behind }, click),
        canClick: false,
      };
  }
  if (!isSettled)
    return { text: dim("checking for conflicts…"), canClick: false };
  if (!mr.hasConflicts)
    return { text: dim("no merge conflicts"), canClick: false };
  const grey = why => ({
    text: `${button(RESOLVE, "gray")} ${dim(why)}`,
    canClick: false,
  });
  if (tree?.unpushed)
    return grey(
      `conflicts as of the pushed commits · ${tree.unpushed} not pushed: its agent pushes first`,
    );
  const why = noAgentOf(tree, agents);
  if (why) return grey(why);
  const note =
    click && now - click.at < NOTE_MS ? red("didn't get in to its agent") : "";
  return {
    text: [
      button(RESOLVE, "yellow"),
      behind ? yellow(`${behind} behind`) : "",
      note,
    ]
      .filter(Boolean)
      .join(" "),
    canClick: true,
  };
};

/** The text of conflictStateOf. */
export const conflictCell = (mr, options) => conflictStateOf(mr, options).text;

/**
 * A click on the button of `mr`: the order typed into its agent's pane
 * (`[overview] …`, answers.mjs). Nothing while the button isn't there
 * (conflictStateOf's `canClick`).
 */
export const clickResolve = ({ mr, tree, agents, behind = 0 }) => {
  if (!conflictStateOf(mr, { tree, agents, behind }).canClick) return;
  const isSent = deliverTo(tree.dir, resolveLine(mr));
  clicks.set(mr.iid, {
    isSent,
    at: Date.now(),
    pushed: tree.pushed ?? null,
    isStarted: false,
  });
  if (!isSent) setTimeout(() => screen.redraw(), NOTE_MS).unref();
  screen.redraw();
};

/**
 * The automatic line about the conflicts of `mr` (auto-actions.mjs) got in to
 * the agent of `tree`: its row shows where it stands, as after a click.
 */
export const noteTold = (mr, tree) =>
  clicks.set(mr.iid, {
    isSent: true,
    at: Date.now(),
    pushed: tree?.pushed ?? null,
    isStarted: false,
  });

/** Forgets the clicks of MRs that are no longer open. */
export const forgetResolved = mrs => {
  const open = new Set(mrs.map(mr => mr.iid));
  for (const iid of clicks.keys()) if (!open.has(iid)) clicks.delete(iid);
};
