// The building blocks of the screen: boxes on islands, the title band with the logo, buttons
// and status marks.
import {
  ESCAPES,
  dim,
  fit,
  green,
  isColor,
  link,
  pad,
  red,
  visible,
  yellow,
} from "../lib/tty.mjs";
import { LOGO_LINES, titleLogo } from "./logo.mjs";

/**
 * Islands: every section is a box with its title in the top border. A box is
 * opened before its content is pushed and closed after, so the content keeps
 * the rows `clicks` gave it; the borders only wrap those lines. Boxes stack,
 * never side by side: a click knows its row, not its column.
 */
/** The boxes take the whole width of the pane: herdr's pane is the frame. */
const boxWidth = () => process.stdout.columns || 120;
/** Boxes follow each other without a blank line: their borders part them, and the view has to fit a pane. */
export const openBox = lines => {
  lines.push("");
  return lines.length - 1;
};
/** The background of an island: a dark grey a step above the terminal's black. */
const ISLAND = "\x1b[48;2;38;38;38m";
/** The background of an island's title line: lighter, the grey of herdr's tab strip, so it reads as a header. */
const ISLAND_HEAD = "\x1b[48;2;58;58;58m";
/** A line on a background, kept after every reset inside it (colours, buttons, links). */
const onIsland = (line, background = ISLAND) =>
  `${background}${line.replaceAll("\x1b[0m", `\x1b[0m${background}`)}\x1b[0m`;
/** How wide a line inside a box can be before closeBox cuts it. */
/** Where a box's cell starts on screen (from 1): after the island's margin, or the border and a blank. */
export const CELL_START = isColor ? 2 : 3;
export const boxInner = () => boxWidth() - (isColor ? 2 : 4);
export const closeBox = (lines, start, title, { isIndented = true } = {}) => {
  const width = boxWidth();
  if (isColor) {
    // An island: the title on its first line, all of it on a dark grey, a blank line after it.
    const inner = boxInner();
    for (let i = start + 1; i < lines.length; i++) {
      const line = isIndented ? lines[i].replace(/^ {2}/, "") : lines[i];
      lines[i] = onIsland(` ${pad(fit(line, inner), inner)} `);
    }
    lines[start] = onIsland(` ${pad(fit(title, inner), inner)} `, ISLAND_HEAD);
    lines.push("");
    return;
  }
  const inner = boxInner();
  for (let i = start + 1; i < lines.length; i++) {
    // The sections indent their rows by two; inside a box the border does that.
    const line = isIndented ? lines[i].replace(/^ {2}/, "") : lines[i];
    lines[i] = `${dim("│")} ${pad(fit(line, inner), inner)} ${dim("│")}`;
  }
  const head = ` ${fit(title, width - 5)} `;
  lines[start] =
    `${dim("╭─")}${head}${dim(`${"─".repeat(Math.max(0, width - 3 - visible(head)))}╮`)}`;
  lines.push(dim(`╰${"─".repeat(width - 2)}╯`));
};
/**
 * The base title of the last render, for the scroll window to draw the band
 * again with its mark (`markedTitleLines`); `isSet` is false for a view drawn
 * without one. A band drawn with a mark never lands here, so the next frame
 * doesn't put its mark in front of this one's.
 */
export const lastTitle = { left: "", right: "", keys: "", isSet: false };
/**
 * How many lines the title band takes: the logo's height and a line above
 * and below it, the same space on both sides. The sticky top of a view
 * (scroll.mjs).
 */
export const titleHeight = () => LOGO_LINES + 2;
/** The band's text: light on its grey, 9:1. */
const BAND_TEXT = "\x1b[38;2;235;235;235m";
/** Time and keys: quieter, still 4.9:1 on the grey. */
const BAND_QUIET = "\x1b[38;2;170;170;170m";
/**
 * `left` on the left and `right` on the right of `width` columns, both plain
 * text; what doesn't fit is cut from the right one first.
 */
const spread = (left, right, width) => {
  // Characters, not UTF-16 units: a character past the BMP is one column too.
  const [lefts, rights] = [[...left], [...right]];
  const room = Math.max(0, width - lefts.length - (left && right ? 2 : 0));
  const shownRight = rights.slice(0, room);
  const shownLeft = lefts.slice(0, Math.max(0, width - shownRight.length));
  return [
    shownLeft.join(""),
    " ".repeat(Math.max(0, width - shownLeft.length - shownRight.length)),
    shownRight.join(""),
  ];
};
/**
 * The title band across the screen, the logo's height and a line above and
 * below it: the logo at the right end with two columns right of it (a line
 * is about two columns wide on screen: the same space as above and below
 * it), on its lines the name on the left of its middle line, the time (and
 * the scroll mark) on the right of its first line and the keys on the right
 * of its last. All of it on the grey of the section heads, the logo's half
 * blocks too, so no black shows between them. Without colours the same
 * lines, plain. A terminal too narrow for name and logo drops the logo.
 * Pushed with `lines.push(...titleLines())`, every line one row.
 */
export const titleLines = (left, right, keys = "", width = boxWidth()) => {
  Object.assign(lastTitle, { left, right, keys, isSet: true });
  return band(left, right, keys, width);
};
/**
 * The band of the last render again, `mark` (what the scroll window cuts off)
 * left of its time; the base title stays as it was.
 */
export const markedTitleLines = (mark, width = boxWidth()) =>
  band(
    lastTitle.left,
    mark ? `${mark}  ${lastTitle.right}` : lastTitle.right,
    lastTitle.keys,
    width,
  );
const band = (left, right, keys, width) => {
  const plain = text => text.replace(ESCAPES, "");
  const [name, time, hint] = [left, right, keys].map(plain);
  const art = titleLogo();
  const artWidth = art.length ? visible(art[0]) : 0;
  // A column of margin on both ends, two between the text and the logo and
  // one more right of the logo: two there, as much as a line above and below.
  const logoRoom = artWidth + 3;
  const hasLogo = artWidth > 0 && width - 2 - logoRoom >= [...name].length;
  const inner = width - 2 - (hasLogo ? logoRoom : 0);
  // The lines of the logo, between the band's first and last line.
  const rows = [
    ["", ""],
    ["", time],
    [name, ""],
    ["", hint],
  ];
  return Array.from({ length: titleHeight() }, (_, i) => {
    const [l, gap, r] = spread(...(rows[i] ?? ["", ""]), inner);
    const mark = hasLogo ? `  ${art[i - 1] ?? " ".repeat(artWidth)} ` : "";
    if (!isColor) return ` ${l}${gap}${r}${mark} `;
    return `${ISLAND_HEAD}${BAND_TEXT} \x1b[1m${l}\x1b[22m${gap}${BAND_QUIET}${r}${BAND_TEXT}${mark} \x1b[0m`;
  });
};
/**
 * A button: a coloured chip, `[ label ]` without colours. `tone` is the
 * background: cyan starts something, red deletes, yellow runs, gray can't yet.
 */
const TONES = {
  cyan: "1;30;46",
  green: "1;30;42",
  red: "1;37;41",
  yellow: "1;30;43",
  gray: "2;37;100",
};
export const button = (label, tone = "cyan") => {
  if (isColor) return `\x1b[${TONES[tone]}m ${label} \x1b[0m`;
  // Greyed out without colours: parentheses instead of brackets.
  return tone === "gray" ? `( ${label} )` : `[ ${label} ]`;
};
/**
 * A chip: a label on a grey a step above the island's, in the colour of
 * `tone`, not bold and not on a coloured ground, so it never reads as a
 * button. Without colours the label alone.
 */
const CHIP_TONES = {
  cyan: 36,
  green: 32,
  red: 31,
  yellow: 33,
  plain: 37,
};
export const chip = (label, tone = "plain") =>
  isColor
    ? `\x1b[${CHIP_TONES[tone] ?? 37};48;2;70;70;70m ${label} \x1b[0m`
    : label;
/** A pipeline (or its status alone) as a mark, linked to the pipeline when its url is known. */
export const pipelineMark = pipeline =>
  link(pipeline?.web_url, statusMark(pipeline?.status ?? pipeline));
const statusMark = status =>
  ({
    success: green("✓ passed"),
    failed: red("✗ failed"),
    running: yellow("◐ running"),
    pending: yellow("… pending"),
    created: yellow("… created"),
    canceled: dim("⊘ canceled"),
    skipped: dim("skipped"),
    manual: dim("▸ manual"),
  })[status] ?? dim(status ?? "no pipeline");
export const stackCell = tree => {
  if (!tree.hasEnv) return dim("no .env.local");
  if (!tree.hasOwnPort) return dim("shared");
  const state = tree.stack;
  const label = tree.offset ? `+${tree.offset}` : "default";
  if (!state) return dim(`${label} down`);
  return state.startsWith("running")
    ? green(`${label} ${state}`)
    : yellow(`${label} ${state}`);
};
