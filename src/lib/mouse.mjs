// The mouse of a full-screen view (the overview, the changelog tab): what it
// reports and how a line under the pointer looks (#277).
import { isColor, pad } from "./tty.mjs";

/**
 * Mouse reporting as `ESC [ < button ; column ; row M` (SGR, 1006): clicks,
 * the wheel and every move of the pointer (any-event, 1003), the focus of
 * the pane coming and going as `ESC [ I` / `ESC [ O` (1004). herdr hands
 * them to the pane once the app asks for them, a move only while the pointer
 * is over the pane (#277: nothing comes when it leaves, the focus going
 * does); Ctrl-click still goes to herdr, which opens the link under it.
 */
export const MOUSE_ON = "\x1b[?1003h\x1b[?1006h\x1b[?1004h";
export const MOUSE_OFF = "\x1b[?1003l\x1b[?1006l\x1b[?1004l";
/** One key or mouse report each: a chunk of input can hold several (a fast wheel, a move). */
export const KEY = /\x1b\[<\d+;\d+;\d+[Mm]|\x1b\[[\d;]*[~A-Za-z]|[\s\S]/g;
/**
 * A report or key cut off at the end of a chunk, the rest of it in the next
 * one: from its escape on, `ESC` or `ESC [` alone too (#303: a click cut
 * there was read as loose characters and lost). A lone Escape key waits for
 * the next input; the views don't use it.
 */
const CUT = /\x1b(?:\[[<\d;]*)?$/;
/**
 * The keys of the input, chunk by chunk (`KEY` each): a report a chunk ends
 * in the middle of is kept for the next, so a fast move never turns into
 * single characters.
 */
export const keyReader = () => {
  let rest = "";
  return chunk => {
    const text = rest + chunk;
    rest = text.match(CUT)?.[0] ?? "";
    return [...text.slice(0, text.length - rest.length).matchAll(KEY)].map(
      ([key]) => key,
    );
  };
};
/** The pane lost the focus: the pointer went elsewhere. */
export const FOCUS_OUT = "\x1b[O";
const REPORT = /^\x1b\[<(\d+);(\d+);(\d+)([Mm])$/;
/** The modifier bits of a mouse button (Shift, Alt, Ctrl). */
const MODIFIERS = 4 | 8 | 16;
/** The bit of a report that the pointer moved, a button held or not. */
const MOTION = 32;
/**
 * What a key is for the mouse, `row` and `column` from 1: `{ kind: "move" }`
 * the pointer moved (never a click, whatever button is held), `{ kind:
 * "wheel", by }` a notch up (-1) or down (1), with a modifier too, `{ kind:
 * "click" }` the left button pressed alone (no modifier, no motion, no
 * wheel), `{ kind: "release" }` it let go (a release without its press
 * tells of a click lost on the way, #303); null for anything else (a key,
 * another button).
 */
export const mouseOf = key => {
  const report = key.match(REPORT);
  if (!report) return null;
  const [button, column, row] = report.slice(1, 4).map(Number);
  if (report[4] === "m")
    return button === 0 ? { kind: "release", row, column } : null;
  if (button & MOTION) return { kind: "move", row, column };
  const by = { 64: -1, 65: 1 }[button & ~MODIFIERS];
  if (by) return { kind: "wheel", by, row, column };
  if (button === 0) return { kind: "click", row, column };
  return null;
};
/** The ground of a clickable line under the pointer: a step lighter than the island's. */
export const HOVER = "\x1b[48;2;54;54;54m";
/**
 * `line` on the hover's ground, padded to `width` (the pane's), kept after
 * every reset inside it. Without colours the line as it is.
 */
export const onHover = (line, width, ground = HOVER) =>
  isColor
    ? `${ground}${pad(line, width).replaceAll("\x1b[0m", `\x1b[0m${ground}`)}\x1b[0m`
    : line;
