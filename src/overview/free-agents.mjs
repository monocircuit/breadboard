// AGENTS of an orchestration (#289): the free agents, their worktree gone,
// waiting in its space for the next issue. Each with its state (#288,
// agent-state.mjs), its context and where it came from; two clicks on a row
// dismiss it (worktree/free.mjs). The agents of the worktrees are on their
// nodes in WORKTREES.
import { bold, dim } from "../lib/tty.mjs";
import { dismissFree } from "../worktree/free.mjs";
import { agentStateOf } from "./agent-state.mjs";
import { button, closeBox, openBox } from "./boxes.mjs";
import { paintOf } from "./herdr-tones.mjs";
import { pink } from "./names.mjs";
import { pushRows, screen } from "./screen.mjs";

/** How long the first click keeps a dismiss armed. */
const ARM_MS = 4000;
/** id → `{ isArmed }`: a dismiss armed by its first click. */
const armed = new Map();

/** `18 %`: how much of its context a free agent has used. */
const percent = share => `${Math.round((share ?? 0) * 100)} %`;
/**
 * The row of a free agent (`{ id, name, share, issues, free }`, view.mjs):
 * its state as agentStateOf names it for an agent without a worktree (`at
 * work` while it takes its line, `asks you`, else `free`), its context, the
 * worktree and MR it came from and how many issues it did. `status`: herdr's.
 */
export const freeRowOf = (agent, status) => {
  const state = agentStateOf({ claude: null, actions: [] }, { status });
  const from = agent.free?.from
    ? `from ${agent.free.from}${agent.free.mr ? ` (!${agent.free.mr})` : ""}`
    : "";
  const issues = agent.issues?.length
    ? `${agent.issues.length} issue${agent.issues.length === 1 ? "" : "s"}`
    : "";
  return {
    state,
    text: [percent(agent.share), from, issues].filter(Boolean).join(" · "),
  };
};
/** What a click on a free agent's row does: the first arms it, the second dismisses it. */
const clickDismiss = (name, agent, agents) => {
  if (!armed.get(agent.id)?.isArmed) {
    const state = { isArmed: true };
    armed.set(agent.id, state);
    setTimeout(() => {
      state.isArmed = false;
      screen.redraw();
    }, ARM_MS);
    screen.redraw();
    return;
  }
  armed.delete(agent.id);
  dismissFree(name, agent.id, agents.get(agent.id)?.tab ?? null);
  screen.redraw({ isReread: true });
};
/**
 * The box AGENTS: one row per free agent of `free` (view.mjs `freeAgents`),
 * none when there is none. `agents`: agentsOf().
 */
export const pushFreeAgents = (lines, name, free, agents = new Map()) => {
  if (!free.length) return;
  const box = openBox(lines);
  const rows = free.map(agent => {
    const { state, text } = freeRowOf(agent, agents.get(agent.id)?.status);
    const dismiss = armed.get(agent.id)?.isArmed
      ? button(`✕ click again to dismiss ${agent.name}`, "red")
      : button("✕ dismiss", "red");
    return [
      `${paintOf(state.tone)(state.text)} ${pink(agent.name)}`,
      `${dim(text)} ${dismiss}`,
    ];
  });
  pushRows(
    lines,
    rows,
    free.map(agent => () => clickDismiss(name, agent, agents)),
  );
  closeBox(
    lines,
    box,
    `${bold("AGENTS")} ${dim(`${free.length} free · wait in their tab for the next issue; two clicks dismiss one`)}`,
  );
};
