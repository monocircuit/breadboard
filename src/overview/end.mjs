// The ✕ of an orchestration's overview: two clicks end it (worktree/end.mjs),
// its space and this overview with it.
import fs from "node:fs";

import { dim, red } from "../lib/tty.mjs";
import { ORCHESTRATOR, endInBackground, endLog } from "../worktree.mjs";
import { isOnce } from "./args.mjs";
import { button } from "./boxes.mjs";
import { onButton, pushRows, screen } from "./screen.mjs";

/** How long the first click keeps the button armed, as the ✕ of a worktree. */
const ARM_MS = 4000;
/** The end in hand: armed by a first click, running in the background, or failed. */
let ending = null;
const lastLine = log => {
  try {
    return fs.readFileSync(log, "utf-8").trim().split("\n").at(-1) ?? "";
  } catch {
    return "";
  }
};
/** A click on the button, and on NEXT once all is merged: the first arms it, the second ends. */
export const clickEnd = name => {
  if (ending?.isRunning) return;
  if (!ending?.isArmed) {
    const armed = { isArmed: true };
    ending = armed;
    setTimeout(() => {
      armed.isArmed = false;
      screen.redraw();
    }, ARM_MS);
    screen.redraw();
    return;
  }
  const log = endLog(name);
  ending = { isRunning: true, log };
  const child = endInBackground(name, log);
  // It closes this overview's space: an exit it lives to see is a refusal.
  child.on("exit", code => {
    ending = {
      error: code === 0 ? null : lastLine(log) || `ended with ${code}`,
    };
    screen.redraw();
  });
  screen.redraw();
};
/** The end as a row of UP NEXT: what a click on it does now. */
export const endText = name =>
  ending?.isRunning
    ? `ending … ${dim(lastLine(ending.log))}`
    : ending?.isArmed
      ? red(`click again to end ${ORCHESTRATOR} ${name}`)
      : `all merged, no worktree left: end the orchestration${ending?.error ? `  ${red(ending.error)}` : ""}`;
/**
 * The button that ends the orchestration: offered once none of its worktrees
 * is left (`left`, their names), else greyed out and naming them.
 */
export const pushEndButton = (lines, name, left) => {
  const label = `${ORCHESTRATOR} ${name}`;
  if (left.length) {
    // Without an action, but wrapped like every row: the names are long.
    pushRows(
      lines,
      [
        [
          "",
          `${button("✕ end orchestration", "gray")} ${dim(`not yet, worktree${left.length === 1 ? "" : "s"} left: ${left.join(", ")}`)}`,
        ],
      ],
      [null],
    );
    return;
  }
  const shown = ending?.isRunning
    ? "◐ ending …"
    : ending?.isArmed
      ? `✕ click again to end ${label}`
      : "✕ end orchestration";
  const chip = ending?.isRunning
    ? `${button(shown, "yellow")} ${dim(lastLine(ending.log))}`
    : ending?.isArmed
      ? `${button(shown, "red")} ${dim("its space and the orchestrator's claude close, record and checkout go; the conversation stays in /resume")}`
      : `${button(shown, "red")} ${dim(isOnce ? `pnpm wt:orchestrate ${name} --end` : "no worktree left: its space, record and checkout go")}${ending?.error ? `  ${red(ending.error)}` : ""}`;
  pushRows(lines, [["", chip]], [onButton(shown, () => clickEnd(name))]);
};
