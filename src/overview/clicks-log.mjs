// The log of the overview's clicks (#303): where each one landed and what
// ran, so a click that did nothing can be traced afterwards. One line each
// in `$TMPDIR/overview-clicks.log`, shared by every overview running.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export const CLICKS_LOG = path.join(os.tmpdir(), "overview-clicks.log");
/** Past this size the older half of the log goes. */
const MAX_BYTES = 256 * 1024;

/**
 * One line of the log: when, which overview (its pid), the row and column
 * on screen (from 1, as the terminal counts) and the row of the view, what
 * ran (`button <label>`, `line`, `nothing`, or a note such as `release
 * without its press`), and the line as it was drawn, plain.
 */
export const clickLine = (
  { row, column, viewRow = null, ran, line = "" },
  { at = new Date(), pid = process.pid } = {},
) =>
  [
    at.toISOString(),
    `pid ${pid}`,
    `row ${row + 1} col ${column}`,
    viewRow === null ? "" : `view ${viewRow + 1}`,
    `→ ${ran}`,
    line ? `| ${line.slice(0, 160)}` : "",
  ]
    .filter(Boolean)
    .join(" ");

/** `entry` appended to the log (`file`); a log that can't be written is no reason to fail a click. */
export const logClick = (entry, file = CLICKS_LOG) => {
  try {
    const size = fs.statSync(file, { throwIfNoEntry: false })?.size ?? 0;
    if (size > MAX_BYTES) {
      const text = fs.readFileSync(file, "utf-8");
      const half = text.slice(text.length / 2);
      fs.writeFileSync(file, half.slice(half.indexOf("\n") + 1));
    }
    fs.appendFileSync(file, `${clickLine(entry)}\n`);
  } catch {
    // Nothing to do: the click itself ran.
  }
};
