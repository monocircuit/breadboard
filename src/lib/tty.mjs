// Text for the terminal: colours, links, visible width, fitting and tables.
// FORCE_COLOR: the herdr plugin pipes output into less -R and popups.
export const isColor =
  (process.stdout.isTTY || Boolean(process.env.FORCE_COLOR)) &&
  !process.env.NO_COLOR;
const paint = code => text =>
  isColor ? `\x1b[${code}m${text}\x1b[0m` : String(text);
export const bold = paint(1);
export const dim = paint(2);
export const red = paint(31);
export const green = paint(32);
export const yellow = paint(33);
export const cyan = paint(36);
/** A colour of the theme as herdr's config names it, `#FF79C6`: 24 bits, as herdr paints its sidebar. */
export const hex = colour => {
  const [r, g, b] = [1, 3, 5].map(at =>
    Number.parseInt(colour.slice(at, at + 2), 16),
  );
  return paint(`38;2;${r};${g};${b}`);
};
/** An OSC 8 hyperlink: Ctrl-click opens it, in herdr and most terminals. */
export const link = (url, text) =>
  isColor && url ? `\x1b]8;;${url}\x1b\\${text}\x1b]8;;\x1b\\` : String(text);
// ---------------------------------------------------------------------------
// How it looks
// ---------------------------------------------------------------------------

/** Colours and the OSC 8 link sequences: both take no room on screen. */
const ESCAPE = String.raw`\x1b\[[0-9;]*m|\x1b\]8;;[^\x1b]*\x1b\\`;
export const ESCAPES = new RegExp(ESCAPE, "g");
/** The columns a line takes: its characters, not its UTF-16 units (the logo's `🭭` is two of them, one column). */
export const visible = text => [...text.replace(ESCAPES, "")].length;
export const pad = (text, width) =>
  text + " ".repeat(Math.max(0, width - visible(text)));
/** Cut a painted line to `width` visible characters, keeping its colours. */
export const fit = (line, width) => {
  if (visible(line) <= width) return line;
  let out = "";
  let shown = 0;
  for (const token of line.split(new RegExp(`(${ESCAPE})`))) {
    if (token.startsWith("\x1b")) {
      out += token;
      continue;
    }
    const room = width - 1 - shown;
    if (room <= 0) break;
    const characters = [...token];
    out += characters.slice(0, room).join("");
    shown += Math.min(characters.length, room);
  }
  // A cut can end inside a colour or a link: close both.
  return `${out}…${isColor ? "\x1b[0m\x1b]8;;\x1b\\" : ""}`;
};
/** The most lines a wrapped text takes; past them it ends in `…`. */
export const MAX_LINES = 6;
/**
 * Where the lines of `plain` start and end (from, to) at `width`: a break
 * at the last space that fits, a hard cut in a word longer than a line.
 */
const breaks = (plain, width) => {
  const ranges = [];
  let from = 0;
  while (plain.length - from > width) {
    const space = plain.lastIndexOf(" ", from + width);
    const to = space > from ? space : from + width;
    ranges.push([from, to]);
    from = to;
    while (plain[from] === " ") from++;
  }
  ranges.push([from, plain.length]);
  return ranges;
};
/**
 * A painted text as lines of at most `width` visible characters, words kept
 * whole where they fit, at most `maxLines` of them (the last cut with `…`).
 * A colour or link that spans a break is closed at the end of the line and
 * opened again at the start of the next, so every line stands on its own.
 */
export const wrap = (text, width, maxLines = MAX_LINES) => {
  const plain = text.replace(ESCAPES, "");
  if (plain.length <= width) return [text];
  const ranges = breaks(plain, width);
  if (ranges.length > maxLines)
    ranges.splice(maxLines - 1, Infinity, [
      ranges[maxLines - 1][0],
      plain.length,
    ]);
  const lines = [""];
  let colours = [];
  let open = "";
  let shown = 0;
  let at = 0;
  for (const token of text.split(new RegExp(`(${ESCAPE})`))) {
    if (token.startsWith("\x1b[")) {
      colours = /^\x1b\[0?m$/.test(token) ? [] : [...colours, token];
      lines[at] += token;
      continue;
    }
    if (token.startsWith("\x1b]")) {
      open = token === "\x1b]8;;\x1b\\" ? "" : token;
      lines[at] += token;
      continue;
    }
    for (const char of token) {
      if (at + 1 < ranges.length && shown >= ranges[at + 1][0]) {
        if (colours.length) lines[at] += "\x1b[0m";
        if (open) lines[at] += "\x1b]8;;\x1b\\";
        lines.push(colours.join("") + open);
        at++;
      }
      if (shown < ranges[at][1]) lines[at] += char;
      shown += char.length;
    }
  }
  lines[at] = fit(lines[at], width);
  return lines;
};
/** Below this many columns the last cell of a table isn't wrapped but cut. */
const MIN_WRAP = 16;
/**
 * Columns of painted cells, each as wide as its widest cell, as the lines of
 * each row. Given a `width`, a last cell too wide for it goes on over further
 * lines (at most `maxLines`), each indented to where that cell starts.
 */
export const tableRows = (rows, { width, maxLines = MAX_LINES } = {}) => {
  const widths = [];
  for (const row of rows)
    row.forEach(
      (cell, i) => (widths[i] = Math.max(widths[i] ?? 0, visible(cell))),
    );
  return rows.map(row => {
    const head = row
      .slice(0, -1)
      .map((cell, i) => `${pad(cell, widths[i])}  `)
      .join("");
    const room = (width ?? Infinity) - visible(head);
    if (room < MIN_WRAP) return [head + (row.at(-1) ?? "")];
    const indent = " ".repeat(visible(head));
    return wrap(row.at(-1) ?? "", room, maxLines).map(
      (line, i) => (i === 0 ? head : indent) + line,
    );
  });
};
/** Columns of painted cells, each as wide as its widest cell; see `tableRows`. */
export const table = (rows, options) => tableRows(rows, options).flat();
