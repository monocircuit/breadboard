// What the drawing, the click actions and the loop share.
import { tableRows, underRows } from "../lib/tty.mjs";
import { boxInner } from "./boxes.mjs";

/**
 * What the drawing, the click actions and the loop share:
 * - `clicks`: row of the whole view (from 0) → what a click on it does,
 *   handed the column clicked (from 1) and the button drawn there (`{ label
 *   }`, null off the buttons; frame.mjs reads it off the drawn line); a
 *   line with buttons gets its click from `onButtons`. `view` fills it
 *   while it draws, so it always matches the view: every line of a view is
 *   one row. The loop maps a screen row to a view row with `sticky` and
 *   `scroll` (scroll.mjs).
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
 * The click of a line with buttons (#303): on the button `label` it runs
 * `byLabel[label]`, anywhere else on the line `rest` (null: nothing).
 * `buttons` names the labels it answers: the test that clicks every drawn
 * button of every view finds a button no click answers.
 */
export const onButtons = (byLabel, rest = null) =>
  Object.assign(
    (column, hit) => {
      const run =
        hit && Object.hasOwn(byLabel, hit.label) && byLabel[hit.label];
      return run ? run(column) : rest?.(column, hit);
    },
    { buttons: Object.keys(byLabel) },
  );
/** `onButtons` for a line with one button, `label`: the button and the rest of the line do the same. */
export const onButton = (label, run) => onButtons({ [label]: run }, run);
/**
 * `rows` as a table onto `lines`; a click on any line of row i runs
 * `actions[i]` like a click of `clicks`.
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
    rowLines.forEach(line => {
      if (action) screen.clicks[lines.length] = action;
      lines.push(line);
    });
  }
};
/**
 * Rows of `{ head, text }` onto `lines` (underRows): the head cells on the
 * first line, the text below it from column `under`, wrapped at `width`. A
 * click on any line of row i runs `actions[i]` like in pushRows.
 */
export const pushUnder = (
  lines,
  rows,
  actions,
  { width = boxInner(), under, maxLines } = {},
) => {
  for (const [i, rowLines] of underRows(rows, {
    width,
    under,
    maxLines,
  }).entries()) {
    const action = actions[i];
    rowLines.forEach(line => {
      if (action) screen.clicks[lines.length] = action;
      lines.push(line);
    });
  }
};
export const KEYS =
  "q quit · r refresh GitLab · click a row · wheel j k ↑↓ PgUp PgDn space g G scroll";
