// FEED of an orchestration: per agent one line of what it does, so the human
// follows the agents here instead of in their chats. Read in node from the
// agent's note (`pnpm wt:action note`) and its transcript: no model asked.
import { bold, dim, green, red, yellow } from "../lib/tty.mjs";
import { feedLineOf, focusSpace, lastAnswerOf } from "../worktree.mjs";
import { closeBox, openBox } from "./boxes.mjs";
import { pink } from "./names.mjs";
import { ago } from "./render-worktree.mjs";
import { pushRows } from "./screen.mjs";

/** herdr's state of an agent, short. */
const STATE = {
  working: yellow("◐"),
  blocked: red("●"),
  idle: dim("○"),
  done: green("✓"),
};
/** Who of the agents asks, works, waits: the legend of the marks. */
const LEGEND = `${yellow("◐")} at work · ${red("●")} asks you · ${green("✓")} done`;

/**
 * The row of each issue with a worktree, in the order given: its feature, the
 * state herdr gives its agent, how long ago and the line. A note is marked
 * `›`, an answer's first line `"`, to tell what the agent chose to say from
 * where it happened to stop.
 */
export const feedRowsOf = (issues, agents = new Map()) =>
  issues
    .filter(issue => issue.tree)
    .map(issue => {
      const { claude, note, stopped } = issue.tree;
      const answer = lastAnswerOf(claude, stopped);
      const line = feedLineOf(note, answer?.text, answer?.at);
      const status = claude ? agents.get(claude.id)?.status : null;
      return {
        issue,
        cells: [
          "",
          STATE[status] ?? " ",
          pink(bold(issue.feature)),
          dim(line?.at ? ago(line.at) : ""),
          line
            ? `${dim(line.source === "note" ? "›" : '"')} ${line.text}`
            : dim(claude || stopped ? "no answer yet" : "no agent"),
        ],
      };
    });

/** How many lines a row of FEED takes at most: a glance, the rest is in the agent's tab. */
const FEED_LINES = 2;
/** FEED: one row per agent, a click goes to its space. */
export const pushFeed = (lines, issues, agents) => {
  const rows = feedRowsOf(issues, agents);
  if (rows.length === 0) return;
  const box = openBox(lines);
  pushRows(
    lines,
    rows.map(row => row.cells),
    rows.map(
      ({ issue }) =>
        () =>
          focusSpace(issue.tree.dir),
    ),
    { maxLines: FEED_LINES },
  );
  closeBox(
    lines,
    box,
    `${bold("FEED")} ${dim('› note · " last answer · click one to go there')}  ${LEGEND}`,
  );
};
