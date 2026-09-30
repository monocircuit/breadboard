// The scroll window: a view taller than the pane shows a part of it. The
// first `screen.sticky` lines (the title band with its logo, in an orchestration
// NEXT too)
// stay on top, the rest scrolls under them.
import { lastTitle, titleLines } from "./boxes.mjs";
import { screen } from "./screen.mjs";

/**
 * The part of `lines` that fits `rows`: the first `sticky` lines, then the
 * rest from `offset` on. `offset` is clamped to what the view has (a view that
 * got shorter moves it up); `above` and `below` count the lines cut off.
 */
export const scrollWindow = (lines, rows, { sticky = 1, offset = 0 } = {}) => {
  const top = Math.min(sticky, lines.length, rows);
  const room = Math.max(0, rows - top);
  const max = Math.max(0, lines.length - top - room);
  const at = Math.min(Math.max(0, offset), max);
  return {
    lines: [...lines.slice(0, top), ...lines.slice(top + at, top + at + room)],
    offset: at,
    above: at,
    below: max - at,
  };
};

/** The row of the whole view (from 0) under screen row `row` (from 0). */
export const viewRowOf = (row, { sticky = 1, offset = 0 } = {}) =>
  row < sticky ? row : row + offset;

/** `↑ 3 ↓ 12 more`, or nothing when the whole view fits. */
export const moreMark = (above, below) =>
  [above ? `↑ ${above}` : "", below ? `↓ ${below}` : ""]
    .filter(Boolean)
    .join(" ") + (above || below ? " more" : "");

/**
 * The view `text` cut to the pane (`screen.scroll` clamped on the way), its
 * title band with the mark of what is cut off, by its time.
 */
export const windowOf = (text, rows) => {
  const shown = scrollWindow(text.split("\n"), rows, {
    sticky: screen.sticky,
    offset: screen.scroll,
  });
  screen.scroll = shown.offset;
  const mark = moreMark(shown.above, shown.below);
  if (mark && lastTitle.isSet) {
    // The band again with the mark by the time, as far as the pane shows it.
    const band = titleLines(
      lastTitle.left,
      `${mark}  ${lastTitle.right}`,
      lastTitle.keys,
    );
    const count = Math.min(band.length, screen.sticky, shown.lines.length);
    shown.lines.splice(0, count, ...band.slice(0, count));
  }
  return shown.lines;
};

/**
 * Scrolls by `by` lines, or to the top (`-Infinity`) or the bottom
 * (`Infinity`); the next draw clamps it.
 */
export const scrollBy = by => {
  screen.scroll = Math.max(0, screen.scroll + by);
  if (!Number.isFinite(screen.scroll)) screen.scroll = Number.MAX_SAFE_INTEGER;
};

/** The lines a page scrolls: what the pane shows under the sticky lines, less one to keep the context. */
export const pageOf = rows => Math.max(1, rows - screen.sticky - 1);
