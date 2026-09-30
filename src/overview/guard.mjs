// A redraw that throws doesn't kill the overview (#221): the screen keeps the
// last view that worked with the error in its first line, the stack trace goes
// to a log file, and the next redraw tries again.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { red } from "../lib/tty.mjs";

/** Where a failed redraw writes its stack trace, one file per user. */
export const errorLog = () =>
  path.join(os.tmpdir(), `overview-${os.userInfo().username}-errors.log`);

/**
 * `render()` as a guarded draw: its text, or when it throws, the last text
 * that worked (nothing before the first) with a line with the error in place
 * of its first, the
 * trace appended to `log`.
 */
export const guarded = (render, log = errorLog()) => {
  let last = "";
  return () => {
    try {
      last = render();
      return last;
    } catch (error) {
      try {
        fs.appendFileSync(
          log,
          `${new Date().toISOString()} ${error?.stack ?? error}\n\n`,
        );
      } catch {
        // No log: the line on screen still says it.
      }
      const line = red(
        ` the overview failed to draw: ${String(error?.message ?? error).split("\n")[0]} · trace in ${log} · trying again`,
      );
      // In place of the view's first line, not above it: every other line
      // keeps its row, and with it its click.
      return [line, ...last.split("\n").slice(1)].join("\n");
    }
  };
};
