// Answerable actions: the buttons of an action in ACTIONS, what a click on
// one does, and how the answer reaches the agent — typed into its pane as one
// line, so it goes on without polling. What doesn't get in (the agent is
// blocked, herdr is away) waits in the actions file for the agent's next stop
// (.claude/hooks/stop-answers.mjs) or for its pane to change state. A
// `broken` or `different` (isDraft) is only written into the agent's input,
// its tab focused, for the user to complete and send.
import os from "node:os";
import path from "node:path";

import { herdrDid, paneText, panesOf, spaces, tabsOf } from "../lib/herdr.mjs";
import { dim, visible, yellow } from "../lib/tty.mjs";
import {
  ORCHESTRATOR_TAB,
  actionsOf,
  answerAction,
  answerLine,
  answersOf,
  claimAnswer,
  doneAction,
  draftLine,
  draftWaits,
  featureOf,
  isDraft,
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

/** The agent's tab `to` addresses focused, its space first; true when herdr did. */
const focusAgent = to => {
  const pane = agentPaneOf(to);
  return Boolean(
    pane &&
    herdrDid(["workspace", "focus", pane.workspace_id]) &&
    herdrDid(["tab", "focus", pane.tab_id]),
  );
};
/** The prompt mark of Claude Code at the start of its input line. */
const PROMPT = "❯";
/**
 * What `screen` (the last lines of an agent's pane) has in Claude Code's input:
 * the text after the last `❯` up to the rule below it; "" when empty, null
 * without an input line.
 */
export const inputIn = screen => {
  const lines = screen.split("\n");
  const start = lines.findLastIndex(line =>
    line.trimStart().startsWith(PROMPT),
  );
  if (start === -1) return null;
  const end = lines.findIndex((line, i) => i > start && /^\s*─{3,}/.test(line));
  return lines
    .slice(start, end === -1 ? undefined : end)
    .join("\n")
    .trimStart()
    .slice(PROMPT.length)
    .trim();
};
/** Whether the user has typed into the input of `pane` (an unreadable one counts as empty). */
const hasInput = pane => {
  const screen = paneText(pane.pane_id, 30);
  return Boolean(screen && inputIn(screen));
};
/**
 * The draft of `action` written into the input of `pane`, unsent: the user
 * adds what is wrong and sends it. One that waited for a blocked pane is
 * dropped when the user typed meanwhile: they said it themselves. True when
 * it is done with.
 */
const writeDraft = (pane, action) =>
  (action.waits && hasInput(pane)) ||
  herdrDid(["pane", "send-text", pane.pane_id, draftLine(action)]);
/** Why a draft waits, on its row: the agent is at a question, or has no pane. */
const draftWaitsFor = pane =>
  pane
    ? "the agent is at a question: the draft follows once it's answered"
    : "no agent in its tab: the draft follows once there is one";
/** file → the state of its agent's pane at the last try. */
const tried = new Map();
/**
 * The answers of `file` delivered to `to`, each claimed in the file before
 * it is typed and taken out once it got in: a second overview and the Stop
 * hook skip a claimed one, so none arrives twice. A draft (isDraft) is
 * written, not submitted, and is delivered once written.
 */
export const deliverAnswers = (file, to) => {
  const pane = agentPaneOf(to);
  tried.set(file, pane?.agent_status ?? null);
  const answered = actionsOf(file).filter(action => action.answer);
  if (!canType(pane)) {
    // Text written now would land in the question's dialog.
    const waits = draftWaitsFor(pane);
    for (const action of answered.filter(isDraft))
      if (action.waits !== waits) draftWaits(file, action.id, waits);
    return;
  }
  for (const { id } of answered) {
    const action = claimAnswer(file, id);
    if (!action) continue;
    const isIn = isDraft(action)
      ? writeDraft(pane, action)
      : typeInto(pane, answerLine(action));
    if (isIn) doneAction(file, id);
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

/** The buttons of an action: its answers, ✓ alone when it has none. */
const labelsOf = action => {
  const answers = answersOf(action);
  return answers.length ? answers : ["✓"];
};
/** The cell of an action: its buttons, or the answer on its way. */
export const answerCell = action =>
  action.answer
    ? `${yellow(`→ ${action.answer}`)} ${dim(action.waits ?? "at its next stop")}`
    : labelsOf(action)
        .map(label => button(label, label === "✓" ? "green" : "cyan"))
        .join(" ");
/**
 * The button at `column` (from 1) of a line whose buttons start at column
 * `start`, a column of slack after each; -1 past them.
 */
export const buttonAt = (action, column, start = CELL_START) => {
  let end = start - 1;
  const labels = labelsOf(action);
  for (let i = 0; i < labels.length; i++) {
    end += visible(button(labels[i])) + 1;
    if (column <= end) return i;
  }
  return -1;
};
/**
 * The answer `at` (its index in answersOf) to the action of `item`: `{
 * action, file, to }`; stored, typed into the agent's pane at once, ✓ (no
 * answers) ticks it off.
 */
export const answerWith = (item, at, redraw) => {
  const { action, file, to } = item;
  const answers = answersOf(action);
  if (answers.length === 0) doneAction(file, action.id);
  else {
    answerAction(file, action.id, answers[at]);
    if (isDraft({ ...action, answer: answers[at] })) focusAgent(to);
    deliverAnswers(file, to);
  }
  redraw();
};
/**
 * A click on the buttons line of an action, `item`: `{ action, file, to, go
 * }`, its buttons from column `start`. On a button it answers (answerWith);
 * past the buttons it does the action (`go`). An answered one only goes.
 */
export const clickAction = (item, column, redraw, start = CELL_START) => {
  const at = item.action.answer ? -1 : buttonAt(item.action, column, start);
  if (at === -1) return item.go();
  answerWith(item, at, redraw);
};
