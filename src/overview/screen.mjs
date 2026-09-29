// What the drawing, the click actions and the loop share.
import { tableRows } from "../lib/tty.mjs";
import { boxInner } from "./boxes.mjs";

/**
 * What the drawing, the click actions and the loop share:
 * - `clicks`: row of the whole view (from 0) → what a click on it does,
 *   handed the column clicked (from 1). `view` fills it while it draws, so it
 *   always matches the view: every line of a view is one row. The loop maps
 *   a screen row to a view row with `sticky` and `scroll` (scroll.mjs).
 * - `sticky`: how many lines on top of the view stay while the rest scrolls
 *   (the title bar with its logo; in an orchestration NEXT too), set while
 *   drawing.
 * - `scroll`: how far the rest is scrolled, kept over redraws.
 * - `upTarget`: the stack `u` starts, the one whose button is on screen, set
 *   while drawing like `clicks`.
 * - `redraw`: set by the loop; draws the screen again, after reading the local
 *   state again with `isReread`.
 */
export const screen = {
  clicks: [],
  sticky: 1,
  scroll: 0,
  upTarget: null,
  redraw: async () => {},
};
/**
 * `rows` as a table onto `lines`; a click on any line of row i runs
 * `actions[i]` with the column and the line within the row (0 for the first).
 * A last cell wider than `width` (the inside of a box) wraps, over at most
 * `maxLines` lines.
 */
export const pushRows = (
  lines,
  rows,
  actions,
  { width = boxInner(), maxLines } = {},
) => {
  for (const [i, rowLines] of tableRows(rows, { width, maxLines }).entries()) {
    const action = actions[i];
    rowLines.forEach((line, lineOfRow) => {
      if (action)
        screen.clicks[lines.length] = column => action(column, lineOfRow);
      lines.push(line);
    });
  }
};
export const KEYS =
  "q quit · r refresh GitLab · click a row · wheel j k ↑↓ PgUp PgDn space g G scroll";
