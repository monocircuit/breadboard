// A redraw that throws doesn't kill the overview (#221): the screen keeps the
// last view that worked with the error in its first line, the stack trace goes
// to a log file, and the next redraw tries again.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { red } from "../lib/tty.mjs";
import { screen } from "./screen.mjs";

/** Where a failed redraw writes its stack trace, one file per user. */
export const errorLog = () =>
  path.join(os.tmpdir(), `overview-${os.userInfo().username}-errors.log`);

/** Appends `error`'s stack trace to `log`; without a log the screen still says it. */
const logError = (error, log) => {
  try {
    fs.appendFileSync(
      log,
      `${new Date().toISOString()} ${error?.stack ?? error}\n\n`,
    );
  } catch {
    // No log: the line on screen still says it.
  }
};

const messageOf = error => String(error?.message ?? error).split("\n")[0];

/**
 * `draw()`, which pushes one section's lines onto `lines`, guarded (#306):
 * when it throws, its lines and their clicks go, one line names the section
 * and the error, and the sections around it draw as before.
 */
export const drawSection = (lines, name, draw, log = errorLog()) => {
  const from = lines.length;
  try {
    draw();
  } catch (error) {
    logError(error, log);
    lines.length = from;
    screen.clicks.length = Math.min(screen.clicks.length, from);
    lines.push(
      red(`  ${name} failed to draw: ${messageOf(error)} · trace in ${log}`),
    );
  }
};

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
      logError(error, log);
      const line = red(
        ` the overview failed to draw: ${messageOf(error)} · trace in ${log} · trying again`,
      );
      // In place of the view's first line, not above it: every other line
      // keeps its row, and with it its click.
      return [line, ...last.split("\n").slice(1)].join("\n");
    }
  };
};
