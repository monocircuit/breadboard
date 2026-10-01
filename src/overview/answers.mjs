// Answerable actions: the buttons of an action in ACTIONS, what a click on
// one does, and how the answer reaches the agent — typed into its pane as one
// line, so it goes on without polling. What doesn't get in (the agent is
// blocked, herdr is away) waits in the actions file for the agent's next stop
// (.claude/hooks/stop-answers.mjs) or for its pane to change state.
import os from "node:os";
import path from "node:path";

import { herdrDid, panesOf, spaces, tabsOf } from "../lib/herdr.mjs";
import { dim, visible, yellow } from "../lib/tty.mjs";
import {
  ORCHESTRATOR_TAB,
  actionsOf,
  answerAction,
  answerLine,
  answersOf,
  claimAnswer,
  doneAction,
  featureOf,
  focusSpace,
  orchestratorLabel,
  releaseAnswer,
  spaceLabel,
} from "../worktree.mjs";
import { holds } from "../worktree/lock.mjs";
import { isOnce } from "./args.mjs";
import { CELL_START, button } from "./boxes.mjs";

/**
 * Where a line for an agent goes: `{ dir }` a worktree's agent, the tab
 * `<feature>` of its space; `{ orchestration }` the orchestrator, the tab
 * `orchestrator` of `⇶ <name>`.
 */
const tabOf = to =>
  to.orchestration
    ? { space: orchestratorLabel(to.orchestration), tab: ORCHESTRATOR_TAB }
    : { space: spaceLabel(to.dir), tab: featureOf(to.dir) };
/** The pane of the agent `to` addresses, `{ pane_id, agent_status }`, or null without one. */
const agentPaneOf = to => {
  const { space, tab } = tabOf(to);
  const workspace = spaces().find(w => w.label === space)?.workspace_id;
  if (!workspace) return null;
  const tabId = tabsOf(workspace).find(t => t.label === tab)?.tab_id;
  if (!tabId) return null;
  return (
    panesOf(workspace).find(pane => pane.tab_id === tabId && pane.agent) ?? null
  );
};
/** Whether a line can go into `pane`: there is one, and its agent isn't blocked at a question. */
const canType = pane => Boolean(pane) && pane.agent_status !== "blocked";
/** What every line typed into an agent's prompt starts with. */
export const OVERVIEW_PREFIX = "[overview] ";
/**
 * `line` as it is typed: with `[overview] ` in front unless it has it. The
 * prompt of Claude Code takes a line starting with `!` as a shell command,
 * `/` as a slash command, `#` as a memory: a line from the overview is none.
 */
export const overviewLine = line =>
  line.startsWith(OVERVIEW_PREFIX) ? line : `${OVERVIEW_PREFIX}${line}`;
/**
 * `line` typed into the agent's pane (as overviewLine) and submitted: an idle
 * agent goes on, a working one queues it. herdr refuses an agent blocked at a
 * question, so nothing lands in a dialog. True when it got in.
 */
const typeInto = (pane, line) =>
  canType(pane) &&
  herdrDid(["agent", "prompt", pane.pane_id, overviewLine(line)]);
const deliver = (to, line) => typeInto(agentPaneOf(to), line);
/** One line into the pane of the agent of the worktree `dir`; true when it got in. */
export const deliverTo = (dir, line) => deliver({ dir }, line);
/** One line into the orchestrator's pane of the orchestration `name`; true when it got in. */
export const deliverToOrchestrator = (name, line) =>
  deliver({ orchestration: name }, line);

/** file → the state of its agent's pane at the last try. */
const tried = new Map();
/**
 * The answers of `file` delivered to `to`, each claimed in the file before
 * it is typed and taken out once it got in: a second overview and the Stop
 * hook skip a claimed one, so none arrives twice.
 */
const deliverAnswers = (file, to) => {
  const pane = agentPaneOf(to);
  tried.set(file, pane?.agent_status ?? null);
  if (!canType(pane)) return;
  for (const { id } of actionsOf(file).filter(action => action.answer)) {
    const action = claimAnswer(file, id);
    if (!action) continue;
    if (typeInto(pane, answerLine(action))) doneAction(file, id);
    else releaseAnswer(file, id);
  }
};
/**
 * Whether this process delivers the answers of the orchestration `name`
 * that didn't get in: only a live overview (never a `--once` print, which
 * only reads), and of several live ones the first; another takes over when
 * it ends. A pid file per orchestration and user.
 */
export const deliversAnswers = name =>
  !isOnce &&
  holds(
    path.join(
      os.tmpdir(),
      `platform-overview-${os.userInfo().uid}-${name.replaceAll(/[^\w-]/g, "_")}.pid`,
    ),
  );
/**
 * On a read of the orchestration `name`: answers that didn't get in are
 * tried again once their agent's pane changed state (blocked → working,
 * idle, …), not on every read; after a start of the overview once. Only by
 * the one that delivers them (`deliversAnswers`). `targets`: `[{ file, to }]`.
 */
export const retryAnswers = (name, targets) => {
  if (!deliversAnswers(name)) return;
  for (const { file, to } of targets) {
    if (!actionsOf(file).some(action => action.answer)) {
      tried.delete(file);
      continue;
    }
    const state = agentPaneOf(to)?.agent_status ?? null;
    if (tried.has(file) && tried.get(file) === state) continue;
    deliverAnswers(file, to);
  }
};

/** Answers after which the human has more to say: the click also goes to the agent's space. */
const GOES_ON = new Set(["broken", "different"]);
/** The buttons of an action: its answers, ✓ alone when it has none. */
const labelsOf = action => {
  const answers = answersOf(action);
  return answers.length ? answers : ["✓"];
};
/** What marks the option a decide recommends (`--recommend`), on its button and in NEXT. */
const RECOMMENDED = "★";
const isRecommended = (action, label) =>
  Boolean(action.recommend) && label === action.recommend;
/** A label as its button shows it: the recommended one marked. */
const shownLabel = (action, label) =>
  isRecommended(action, label) ? `${RECOMMENDED} ${label}` : label;
/** The cell of an action: its buttons, or the answer on its way. */
export const answerCell = action =>
  action.answer
    ? `${yellow(`→ ${action.answer}`)} ${dim("at its next stop")}`
    : labelsOf(action)
        .map(label =>
          button(
            shownLabel(action, label),
            label === "✓" || isRecommended(action, label) ? "green" : "cyan",
          ),
        )
        .join(" ");
/** A decide's options in a line, for NEXT (which has no buttons): the recommended one marked. */
export const optionsText = action =>
  answersOf(action).length
    ? `${dim("options:")} ${answersOf(action)
        .map(label => shownLabel(action, label))
        .join(dim(" · "))}`
    : "";
/** The button at `column` (from 1), a column of slack after each; -1 past them. */
export const buttonAt = (action, column) => {
  let end = CELL_START - 1;
  const labels = labelsOf(action);
  for (let i = 0; i < labels.length; i++) {
    end += visible(button(shownLabel(action, labels[i]))) + 1;
    if (column <= end) return i;
  }
  return -1;
};
/**
 * A click on an action row, `item`: `{ action, file, to, go }`. On a button
 * it answers (✓ ticks it off), the answer typed into the agent's pane at
 * once; past the buttons it does the action (`go`). An answered one only goes.
 */
export const clickAction = (item, column, redraw) => {
  const { action, file, to, go } = item;
  const at = action.answer ? -1 : buttonAt(action, column);
  if (at === -1) return go();
  const answers = answersOf(action);
  if (answers.length === 0) doneAction(file, action.id);
  else {
    answerAction(file, action.id, answers[at]);
    deliverAnswers(file, to);
    // The orchestrator's tab is next to this overview already.
    if (GOES_ON.has(answers[at]) && to.dir) focusSpace(to.dir);
  }
  redraw();
};
