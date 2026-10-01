// The conflict state of an open MR in MERGE REQUESTS (#246): a button that
// has its agent merge develop in and resolve the conflicts while GitLab
// reports some (`has_conflicts`), "no merge conflicts" otherwise. The button
// leads the row's second line, before what else the MR lacks (mr-blockers.mjs).
import { ESCAPES, dim, isColor, red, yellow } from "../lib/tty.mjs";
import { deliverTo } from "./answers.mjs";
import { button } from "./boxes.mjs";
import { ago } from "./render-worktree.mjs";
import { screen } from "./screen.mjs";

/** Between the words of the button: a wrap never parts them. */
const GLUE = " ";
/** The button's words, as a line holds them. */
export const RESOLVE = "resolve merge conflicts".replaceAll(" ", GLUE);
/** How long a click's "no agent" stays next to the button. */
const NOTE_MS = 10_000;

/**
 * What the clicks did, by the MR's iid: `{ isSent, text, at }`. A sent order
 * stays until GitLab no longer reports conflicts (`forgetResolved`), a note
 * that it didn't get in goes after NOTE_MS.
 */
const clicks = new Map();

/** The line its agent gets: merge develop in, resolve, check, push. */
export const resolveLine = mr =>
  `[overview] !${mr.iid} has merge conflicts with develop: merge develop in (git fetch origin, git merge origin/develop), resolve the conflicts, run lint, typecheck and test of the packages they touch, commit the merge and push.`;

/**
 * The conflict state of an open `mr` as the start of its second line: the
 * button, how far it is behind develop and what its last click did, or
 * "no merge conflicts". `behind`: the commits of develop its branch lacks.
 */
export const conflictCell = (mr, { behind = 0, now = Date.now() } = {}) => {
  if (!mr.hasConflicts) return dim("no merge conflicts");
  const click = clicks.get(mr.iid);
  const note = !click
    ? ""
    : click.isSent
      ? dim(`told its agent ${ago(new Date(click.at))}`)
      : now - click.at < NOTE_MS
        ? red(click.text)
        : "";
  return [
    button(RESOLVE, "yellow"),
    behind ? yellow(`${behind} behind`) : "",
    note,
  ]
    .filter(Boolean)
    .join(" ");
};

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

/**
 * A click on the button of `mr`: the order typed into its agent's pane
 * (`[overview] …`, answers.mjs), else a note why it can't get in.
 */
export const clickResolve = ({ mr, tree, agents }) => {
  const why = noAgentOf(tree, agents);
  const isSent = !why && deliverTo(tree.dir, resolveLine(mr));
  clicks.set(mr.iid, {
    isSent,
    text: why ?? "didn't get in to its agent",
    at: Date.now(),
  });
  if (!isSent) setTimeout(() => screen.redraw(), NOTE_MS).unref();
  screen.redraw();
};

/** Forgets the clicks of MRs GitLab no longer reports conflicts for. */
export const forgetResolved = mrs => {
  for (const mr of mrs) if (!mr.hasConflicts) clicks.delete(mr.iid);
};

/**
 * The columns (from 1, on screen) the button takes in `line`, a line of a
 * box as it is pushed (two columns of indent the box replaces), or null when
 * it isn't on it.
 */
export const resolveColumns = line => {
  const at = line.replace(ESCAPES, "").indexOf(RESOLVE);
  if (at === -1) return null;
  // The chip's blank (or `[ `) before the words, and after.
  const pad = isColor ? 1 : 2;
  // Inside a box: an island's margin, or the border and a blank, for the indent.
  const shift = isColor ? 0 : 1;
  return {
    from: at - pad + shift,
    to: at + RESOLVE.length + pad - 1 + shift,
  };
};
