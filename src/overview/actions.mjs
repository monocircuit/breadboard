// What a click or a key does, and the rows and buttons that offer it: start a
// stack, migrate, open an issue, MR or pipeline, remove a worktree.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import {
  ESCAPES,
  bold,
  dim,
  green,
  link,
  red,
  table,
  yellow,
} from "../lib/tty.mjs";
import { ENTRY, VITA, featureOf, mainDir } from "../worktree.mjs";
import { isOnce } from "./args.mjs";
import { button, closeBox, openBox, pipelineMark } from "./boxes.mjs";
import { show } from "./io.mjs";
import { claudeLine, home, plural, someFiles } from "./render-worktree.mjs";
import { pushRows, screen } from "./screen.mjs";

/**
 * Stacks started from the overview: dir → { isRunning, log, error }. The
 * button shows what `stack:up` is at, and its last line when it failed. Up,
 * the stack gets its tabs from `stack:up` itself (inside herdr).
 */
const starting = new Map();
export const isStarting = dir => Boolean(starting.get(dir)?.isRunning);
/** The last line `stack:up` wrote, without colours and progress redraws. */
const lastLine = file => {
  try {
    const text = fs
      .readFileSync(file, "utf-8")
      .slice(-4096)
      .replace(ESCAPES, "");
    return (
      text
        .split(/[\r\n]/)
        .map(line => line.trim())
        .filter(Boolean)
        .at(-1) ?? ""
    );
  } catch {
    return "";
  }
};
/** Where a start of the stack of `dir` is: null, or `{ isRunning, line, error }`, `line` the last one `stack:up` wrote. */
export const stackProgress = dir => {
  const state = starting.get(dir);
  return state ? { ...state, line: lastLine(state.log) } : null;
};
/**
 * `stack:up` for `dir`, detached and into a log file instead of a pipe:
 * quitting the overview must not break a start halfway, before its tabs.
 * `onUp`: what comes once it is up (not after a failure, nor when a start
 * is already running).
 */
export const startStack = (dir, onUp = null) => {
  if (isStarting(dir)) return;
  const log = path.join(
    os.tmpdir(),
    `overview-stack-${path.basename(dir)}.log`,
  );
  const out = fs.openSync(log, "w");
  const child = spawn(process.execPath, [ENTRY, "control", "up"], {
    cwd: dir,
    env: { ...process.env, INIT_CWD: dir, NO_COLOR: "1" },
    stdio: ["ignore", out, out],
    detached: true,
  });
  fs.closeSync(out);
  child.unref();
  const state = { isRunning: true, log, error: null };
  starting.set(dir, state);
  child.on("exit", code => {
    state.isRunning = false;
    if (code === 0) starting.delete(dir);
    else state.error = lastLine(log) || `stack:up ended with ${code}`;
    if (code === 0) onUp?.();
    screen.redraw();
  });
  screen.redraw();
};
/**
 * `pnpm pg:migrate` in the vita of `dir`, like startStack: detached, into a
 * log file, the button shows its last line. dir → { isRunning, log, error }.
 */
const migrating = new Map();
const migrate = dir => {
  if (migrating.get(dir)?.isRunning) return;
  const log = path.join(
    os.tmpdir(),
    `overview-migrate-${path.basename(dir)}.log`,
  );
  const out = fs.openSync(log, "w");
  const child = spawn("pnpm", ["pg:migrate"], {
    cwd: path.join(dir, VITA),
    env: { ...process.env, NO_COLOR: "1" },
    stdio: ["ignore", out, out],
    detached: true,
  });
  fs.closeSync(out);
  child.unref();
  const state = { isRunning: true, log, error: null };
  migrating.set(dir, state);
  child.on("exit", async code => {
    state.isRunning = false;
    if (code === 0) migrating.delete(dir);
    else state.error = lastLine(log) || `pg:migrate ended with ${code}`;
    await screen.redraw({ isReread: true });
  });
  screen.redraw();
};
/** The migrations line of STACK: all applied, or the ones missing with the button that applies them. */
export const pushMigrations = (lines, dir, migrations) => {
  if (!migrations) return;
  if (migrations.error) {
    lines.push(
      `  ${dim("migrations")}  ${yellow(`the database doesn't answer: ${migrations.error}`)}`,
    );
    return;
  }
  const state = migrating.get(dir);
  if (migrations.pending.length === 0 && !state) {
    lines.push(
      `  ${dim("migrations")}  ${green("✓")} ${dim(`all ${migrations.applied} applied`)}`,
    );
    return;
  }
  const { pending } = migrations;
  lines.push(
    `  ${dim("migrations")}  ${yellow(`⚠ ${plural(pending.length, "migration")} not applied:`)} ${someFiles(pending)}`,
  );
  const chip = state?.isRunning
    ? `${button("◐ migrating …", "yellow")} ${dim(lastLine(state.log))}`
    : `${button("▶ pnpm pg:migrate")}${state?.error ? `  ${red(state.error)}` : ""}`;
  pushRows(lines, [["", "", chip]], [() => migrate(dir)]);
};
/** The first line of a text, for a row: what someone said, not all of it. */
const firstLine = text =>
  text
    .split("\n")
    .find(line => line.trim())
    ?.trim() ?? "";
/** herdr's state of an agent, as the title of AGENT says it. */
const AGENT_STATES = {
  working: yellow("◐ at work"),
  blocked: red("● asks you something"),
  idle: dim("waiting for you"),
  done: green("✓ done, waiting for you"),
};
/**
 * AGENT: the conversation bound to the space. What herdr says it does, its
 * context, how to get it back in an emergency (herdr gone, the tab closed),
 * the last prompt and the last answer.
 */
export const pushAgent = (lines, claude, status) => {
  if (!claude) return;
  const start = openBox(lines);
  const rows = [
    ["", dim("context"), claudeLine(claude)],
    [
      "",
      dim("resume"),
      dim(`cd ${home(claude.cwd)} && claude --resume ${claude.id}`),
    ],
    ["", dim("stored"), dim(home(claude.transcript))],
  ];
  if (claude.prompt) rows.push(["", dim("asked"), firstLine(claude.prompt)]);
  if (claude.answer) rows.push(["", dim("said"), firstLine(claude.answer)]);
  lines.push(...table(rows));
  closeBox(lines, start, `${bold("AGENT")} ${AGENT_STATES[status] ?? ""}`);
};
/** How many open boxes ISSUE lists before it counts the rest. */
const MAX_OPEN = 10;
/**
 * ISSUE: its boxes under their headings, each heading with its count; the
 * open boxes by name, the ticked ones only counted — they are done.
 */
export const pushIssue = (lines, issue) => {
  if (!issue?.boxes.length) return;
  const start = openBox(lines);
  const headings = [...new Set(issue.boxes.map(box => box.heading))];
  let shown = 0;
  let rest = 0;
  for (const heading of headings) {
    const boxes = issue.boxes.filter(box => box.heading === heading);
    const done = boxes.filter(box => box.isTicked).length;
    const count =
      done === boxes.length
        ? green(`${done}/${boxes.length} ✓`)
        : `${done}/${boxes.length}`;
    lines.push(`  ${bold(heading ?? "—")}  ${count}`);
    for (const box of boxes.filter(box => !box.isTicked)) {
      if (shown < MAX_OPEN) lines.push(`    ${dim("○")} ${box.text}`);
      else rest++;
      shown++;
    }
  }
  if (rest) lines.push(dim(`    … ${rest} more open`));
  const done = issue.boxes.filter(box => box.isTicked).length;
  closeBox(
    lines,
    start,
    `${bold("ISSUE")} ${link(issue.url, `#${issue.iid}`)}  ${done === issue.boxes.length ? green(`${done}/${issue.boxes.length} done`) : `${done}/${issue.boxes.length} done`}`,
  );
};
/** PIPELINE: every job with its mark on one line; a failed one with the last lines of its log. */
export const pushPipeline = (lines, mr, jobs) => {
  if (!mr?.pipeline || jobs.length === 0) return;
  const start = openBox(lines);
  const short = status =>
    ({
      success: green("✓"),
      failed: red("✗"),
      running: yellow("◐"),
      pending: yellow("…"),
      created: yellow("…"),
      manual: dim("▸"),
    })[status] ?? dim("·");
  const marks = jobs.map(
    job => `${short(job.status)} ${link(job.url, job.name)}`,
  );
  lines.push(`  ${marks.join("   ")}`);
  for (const job of jobs.filter(job => job.log)) {
    pushRows(
      lines,
      [["", red("✗"), bold(job.name), dim("click to open the job")]],
      [() => show(job.url)],
    );
    for (const line of job.log) lines.push(`      ${dim(line)}`);
  }
  closeBox(lines, start, `${bold("PIPELINE")} ${pipelineMark(mr.pipeline)}`);
};
/** How many merge requests DEVELOP lists before it counts the rest. */
const MAX_MERGES = 6;
/**
 * DEVELOP: what came in since the branch started, and whether merging it now
 * would conflict. `base`: the project's URL, for the links to the MRs.
 */
export const pushDevelop = (lines, develop, base) => {
  if (!develop || (develop.merges.length === 0 && develop.direct === 0)) return;
  const start = openBox(lines);
  const { conflicts } = develop;
  if (conflicts === null)
    lines.push(
      dim("  git can't tell whether a merge would conflict (git < 2.38)"),
    );
  else if (conflicts.length === 0)
    lines.push(
      `  ${green("✓")} merges cleanly: ${dim("git fetch && git merge origin/develop")}`,
    );
  else
    lines.push(`  ${red("✗")} a merge conflicts in ${someFiles(conflicts, 4)}`);
  const merges = develop.merges.slice(0, MAX_MERGES);
  const url = mr => base && mr && `${base}/-/merge_requests/${mr}`;
  pushRows(
    lines,
    merges.map(m => [
      "",
      m.mr ? link(url(m.mr), `!${m.mr}`) : "",
      m.title,
      dim(m.branch),
    ]),
    merges.map(m => (url(m.mr) ? () => show(url(m.mr)) : null)),
  );
  const more = [
    develop.merges.length > MAX_MERGES
      ? plural(develop.merges.length - MAX_MERGES, "more merge request")
      : "",
    develop.direct ? plural(develop.direct, "commit") + " without one" : "",
  ].filter(Boolean);
  if (more.length) lines.push(dim(`  … ${more.join(", ")}`));
  closeBox(
    lines,
    start,
    `${bold("DEVELOP")} ${dim("since the branch started")}  ${plural(develop.merges.length, "merge request")}`,
  );
};
/**
 * `wt:remove` of the viewed worktree: null, or { isArmed, isRunning, log,
 * error }. The first click only arms the button, the second one runs it.
 */
export let removing = null;
const ARM_MS = 5000;
/**
 * `wt:remove`, detached and from the main checkout: it removes the directory
 * this view runs in, and closes the space with this tab. Without the pane of
 * this tab herdr would take that tab for the one running the command and keep it.
 */
const removeWorktree = dir => {
  const log = path.join(
    os.tmpdir(),
    `overview-remove-${path.basename(dir)}.log`,
  );
  const out = fs.openSync(log, "w");
  const env = { ...process.env };
  delete env.HERDR_PANE_ID;
  delete env.HERDR_TAB_ID;
  // --close-space: the space goes as a whole, its conversation's tab too; the click chose to delete it all.
  const args = [ENTRY, "remove", featureOf(dir), "--close-space"];
  const child = spawn(process.execPath, args, {
    cwd: mainDir(),
    env: { ...env, NO_COLOR: "1" },
    stdio: ["ignore", out, out],
    detached: true,
  });
  fs.closeSync(out);
  child.unref();
  removing = { isArmed: false, isRunning: true, log, error: null };
  child.on("exit", code => {
    removing.isRunning = false;
    if (code !== 0)
      removing.error = lastLine(log) || `wt:remove ended with ${code}`;
    screen.redraw();
  });
  screen.redraw();
};
const clickRemove = dir => {
  if (removing?.isRunning) return;
  if (!removing?.isArmed) {
    const armed = { isArmed: true, isRunning: false, log: null, error: null };
    removing = armed;
    setTimeout(() => {
      armed.isArmed = false;
      screen.redraw();
    }, ARM_MS);
    screen.redraw();
    return;
  }
  removeWorktree(dir);
};
/** The delete button of a worktree with nothing left in it. */
export const pushRemoveButton = (lines, tree) => {
  const chip = removing?.isRunning
    ? `${button("◐ deleting …", "yellow")} ${dim(lastLine(removing.log))}`
    : removing?.isArmed
      ? `${button(`✕ click again to delete ${tree.name}`, "red")} ${dim("worktree, branch and the data of its stack")}`
      : `${button("✕ delete worktree", "red")} ${dim(isOnce ? `pnpm wt:remove ${tree.name}` : "nothing left in it: no changes, all of it in develop")}${removing?.error ? `  ${red(removing.error)}` : ""}`;
  pushRows(lines, [["", chip]], [() => clickRemove(tree.dir)]);
};
/**
 * The delete button while something would be lost, greyed out and without an
 * action, with what keeps it: uncommitted files, commits develop hasn't got.
 * A merged MR whose commits still look missing means origin/develop here is
 * older than the merge: the overview doesn't fetch.
 */
export const pushDisabledRemove = (lines, tree, ownCommits, mr) => {
  const reasons = [
    tree.changed ? plural(tree.changed, "uncommitted file") : "",
    ownCommits ? `${plural(ownCommits, "commit")} not in develop` : "",
  ].filter(Boolean);
  const isStale = mr?.state === "merged" && ownCommits > 0;
  const why = isStale
    ? "merged on GitLab, but develop here is older: git fetch"
    : `not yet: ${reasons.join(", ")}`;
  lines.push(`  ${button("✕ delete worktree", "gray")} ${dim(why)}`);
};
/** The button of a stack that is down, `what` naming it; it and `u` start it. */
export const pushStackButton = (lines, dir, what) => {
  const state = starting.get(dir);
  const chip = state?.isRunning
    ? `${button(`◐ starting ${what} …`, "yellow")} ${dim(lastLine(state.log))}`
    : `${button(`▶ start ${what}`)} ${dim(isOnce ? "pnpm stack:up" : "click or u")}${state?.error ? `  ${red(state.error)}` : ""}`;
  pushRows(lines, [["", chip]], [() => startStack(dir)]);
  screen.upTarget = dir;
};
