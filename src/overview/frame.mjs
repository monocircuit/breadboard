// The frame on screen (#225): the rendered view is kept until its data
// changes, scrolling only cuts a new window out of it; input of one burst
// adds up and draws once per frame; only the rows that changed are written.
// A click resolves against the frame the user sees, not the next one.
import { screen } from "./screen.mjs";
import { viewRowOf, windowOf } from "./scroll.mjs";

/** Autowrap off and on: a line too wide for the pane is cut, never pushed onto a second row. */
export const WRAP_OFF = "\x1b[?7l";
export const WRAP_ON = "\x1b[?7h";
/** At most one draw per this many ms, about a frame of the screen. */
export const FRAME_MS = 16;

/**
 * The escapes that turn the screen showing `previous` into `next`: every row
 * that changed, at its row, cleared before it is written; the rows past the
 * end of `next` cleared. Each line has its row, so a line that takes more
 * room than it should can't move the others (with WRAP_OFF it is cut).
 */
export const frameUpdate = (previous, next) => {
  let out = "";
  next.forEach((line, row) => {
    if (previous[row] !== line) out += `\x1b[${row + 1};1H\x1b[2K${line}`;
  });
  if (previous.length > next.length) out += `\x1b[${next.length + 1};1H\x1b[J`;
  return out;
};

/**
 * The drawing loop of the live overview. `render()` gives the whole view
 * (and fills `screen`'s clicks, sticky and upTarget), `rows()` the height of
 * the pane, `write` takes the escapes.
 * - `changed()`: the data changed (a read, GitLab, a click, a resize): render
 *   again, at the next frame.
 * - `scrolled()`: only the window moved: the kept view is cut again, at the
 *   next frame; any number of calls before it draw once.
 * - `viewRowAt(row)`: the line of the view on the row (from 0) of the frame on
 *   screen.
 * - `actionAt(row)`: what a click on the row (from 0) of the frame on screen
 *   does, handed the column; `clickAt(row, column)` runs it.
 * - `repaint()`: the screen was cleared (a resize): all rows again.
 */
export const painter = ({
  render,
  rows,
  write,
  now = Date.now,
  later = setTimeout,
}) => {
  let text = null;
  let shown = [];
  let drawn = { sticky: screen.sticky, offset: screen.scroll, clicks: [] };
  let timer = null;
  let last = -Infinity;
  let isCleared = false;
  const paint = () => {
    timer = null;
    last = now();
    if (text === null) text = render();
    const lines = windowOf(text, rows());
    write(`${isCleared ? "\x1b[2J" : ""}${frameUpdate(shown, lines)}`);
    isCleared = false;
    shown = lines;
    drawn = {
      sticky: screen.sticky,
      offset: screen.scroll,
      clicks: screen.clicks,
    };
  };
  const schedule = () => {
    if (timer) return;
    timer = later(paint, Math.max(0, FRAME_MS - (now() - last)));
  };
  const viewRowAt = row => viewRowOf(row, drawn);
  const actionAt = row => drawn.clicks[viewRowAt(row)];
  return {
    changed: () => {
      text = null;
      schedule();
    },
    scrolled: schedule,
    repaint: () => {
      shown = [];
      isCleared = true;
      text = null;
      schedule();
    },
    viewRowAt,
    actionAt,
    clickAt: (row, column) => actionAt(row)?.(column),
  };
};
