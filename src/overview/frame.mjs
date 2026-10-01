// The frame on screen (#225): the rendered view is kept until its data
// changes, scrolling only cuts a new window out of it; input of one burst
// adds up and draws once per frame; only the rows that changed are written.
// A click resolves against the frame the user sees, not the next one.
import { ESCAPES } from "../lib/tty.mjs";
import { buttonOn, hovered } from "./boxes.mjs";
import { screen } from "./screen.mjs";
import { viewRowOf, windowOf } from "./scroll.mjs";

/** Autowrap off and on: a line too wide for the pane is cut, never pushed onto a second row. */
export const WRAP_OFF = "\x1b[?7l";
export const WRAP_ON = "\x1b[?7h";
/** At most one draw per this many ms, about a frame of the screen. */
export const FRAME_MS = 16;
/** How often the pane's size is asked for, in ms, in case no resize event came (#253). */
export const SIZE_MS = 250;

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
 * the pane, `size()` its size as anything comparable (a view rendered at
 * another size is laid out again, never only cut), `write` takes the escapes.
 * - `changed()`: the data changed (a read, GitLab, a click, a resize): render
 *   again, at the next frame.
 * - `scrolled()`: only the window moved: the kept view is cut again, at the
 *   next frame; any number of calls before it draw once.
 * - `viewRowAt(row)`: the line of the view on the row (from 0) of the frame on
 *   screen.
 * - `actionAt(row)`: what a click on the row (from 0) of the frame on screen
 *   does, handed the column and the button there.
 * - `targetAt(row, column)`: what a click there hits, `{ click, hit, ran }`:
 *   the row's click (none: nothing), the button drawn under it (`{ label }`,
 *   read off the line on screen, null off the buttons), and what runs, `button
 *   <label>`, `line` or `nothing`; `clickAt(row, column)` runs it and hands
 *   it to `log` with the row, the column and the line (#303).
 * - `hoverAt(row, column)`: the pointer is there (row from 0, column from 1),
 *   `hoverAt(null)` gone (#277): what a click there hits is drawn lit
 *   (boxes.mjs hovered), on the next frame and only when that changed, not
 *   on every move; after a redraw or a scroll it is found again.
 * - `repaint()`: the screen was cleared (a resize): all rows again.
 */
export const painter = ({
  render,
  rows,
  size = () => "",
  write,
  now = Date.now,
  later = setTimeout,
  log = () => {},
}) => {
  let text = null;
  let shown = [];
  let drawn = { sticky: screen.sticky, offset: screen.scroll, clicks: [] };
  let timer = null;
  let last = -Infinity;
  let isCleared = false;
  let sized = null;
  /** The rows of the frame on screen as cut, before the hover is drawn on them. */
  let cut = [];
  /** Where the pointer is, `{ row, column }`, null while it is away. */
  let pointer = null;
  /** What is lit on screen (`litOf`), null for nothing. */
  let lit = null;
  const paint = () => {
    timer = null;
    last = now();
    const current = size();
    if (text !== null && current !== sized) {
      // The pane changed size without a repaint(): the kept rows have the old width.
      text = null;
      shown = [];
      isCleared = true;
    }
    if (text === null) {
      text = render();
      sized = current;
    }
    cut = windowOf(text, rows());
    drawn = {
      sticky: screen.sticky,
      offset: screen.scroll,
      clicks: screen.clicks,
    };
    const hover = hoverOf();
    lit = hover?.key ?? null;
    const lines = hover ? cut.with(pointer.row, hover.line) : cut;
    write(`${isCleared ? "\x1b[2J" : ""}${frameUpdate(shown, lines)}`);
    isCleared = false;
    shown = lines;
  };
  const schedule = () => {
    if (timer) return;
    timer = later(paint, Math.max(0, FRAME_MS - (now() - last)));
  };
  const viewRowAt = row => viewRowOf(row, drawn);
  const actionAt = row => drawn.clicks[viewRowAt(row)];
  const targetAt = (row, column) => {
    const click = actionAt(row);
    const hit = cut[row] === undefined ? null : buttonOn(cut[row], column);
    const ran = !click
      ? "nothing"
      : hit && click.buttons?.includes(hit.label)
        ? `button ${hit.label}`
        : "line";
    return { click, hit, ran };
  };
  /**
   * What the pointer is on in the frame on screen, `{ key, line }`: only a
   * row a click does something on, `key` with its row; null for nothing.
   */
  const hoverOf = () => {
    if (!pointer || cut[pointer.row] === undefined || !actionAt(pointer.row))
      return null;
    const hover = hovered(cut[pointer.row], pointer.column);
    return { ...hover, key: `${pointer.row} ${hover.key}` };
  };
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
    targetAt,
    clickAt: (row, column) => {
      const { click, hit, ran } = targetAt(row, column);
      log({
        row,
        column,
        viewRow: viewRowAt(row),
        ran,
        button: hit?.label ?? null,
        line: (cut[row] ?? "").replace(ESCAPES, "").trim(),
      });
      return click?.(column, hit);
    },
    hoverAt: (row, column) => {
      pointer = row === null ? null : { row, column };
      if ((hoverOf()?.key ?? null) !== lit) schedule();
    },
  };
};

/**
 * Asks the terminal of `stream` for its size every `every` ms (#253): a pane
 * resized while its tab was hidden may never send SIGWINCH, and Node's
 * `columns` and `rows` only change with one. A changed size sets them and
 * emits `resize`, as SIGWINCH would. The timer doesn't keep the process alive.
 */
export const watchSize = (
  stream,
  { every = SIZE_MS, repeat = setInterval } = {},
) => {
  if (!stream.isTTY || typeof stream._refreshSize !== "function") return null;
  const timer = repeat(() => stream._refreshSize(), every);
  timer?.unref?.();
  return timer;
};
