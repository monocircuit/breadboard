// AGENTS of an orchestration (#229): one line per agent, what it does, on
// which issue, how full its context is and what it gets next; below them
// each issue that waits and why. No count holds an issue back any more, so
// this says who works on what instead of how many slots are free.
import { bold, cyan, dim, red } from "../lib/tty.mjs";
import { focusSpace, waitsText, wasOf } from "../worktree.mjs";
import { THRESHOLD, gb } from "../worktree/memory.mjs";
import { closeBox, openBox } from "./boxes.mjs";
import { toneOf } from "./herdr-tones.mjs";
import { pushRows } from "./screen.mjs";
import { isDoneMr } from "./workers.mjs";

/**
 * What an agent does, `{ key, text }`: `asks` (herdr: blocked) and `yours`
 * (an action of it is open) wait for you, `waits` for MRs (wt:wait), `work`
 * at work, `idle` none of those (`done` when its MR waits for no one but
 * the merge click, it is free for a next issue).
 */
export const agentStateOf = (tree, status, mr) => {
  if (status === "blocked") return { key: "asks", text: "waits for you: asks" };
  const open = tree.actions?.filter(action => !action.answer) ?? [];
  if (open.length)
    return {
      key: "yours",
      text: `waits for you: ${[...new Set(open.map(action => action.kind))].join(", ")}`,
    };
  if (tree.wait) return { key: "waits", text: waitsText(tree.wait) };
  if (status === "working") return { key: "work", text: "at work" };
  return { key: "idle", text: isDoneMr(mr) ? "idle, its MR done" : "idle" };
};

/** `after: [232]` as `after !232`, nothing for none. */
const afterText = after =>
  after?.length ? ` after ${after.map(iid => `!${iid}`).join(", ")}` : "";

/**
 * The rows of AGENTS: `{ agents, waiting }`. `agents` one per worktree an
 * agent is bound to, `{ issue, name, was, status, state, share, next }`,
 * `was` its name before its last handover for a day (#251), `status`
 * herdr's state of it (the colour of `state`, herdr-tones.mjs); `waiting` one
 * per issue without an agent, `{ issue, text }`, why it waits.
 * `known`: the record's workers (their names), `handovers`: the announced
 * ones (#227, announcedOf: `{ number, to, after, open }`, the MRs still
 * `open` named), `suggested`: number → `{ worker }`
 * of UP NEXT (workers.mjs).
 */
export const agentRowsOf = (
  issues,
  of,
  agentStatus,
  { known = {}, handovers = [], suggested = new Map(), now = Date.now() } = {},
) => {
  const agents = issues
    .filter(issue => issue.tree?.claude)
    .map(issue => {
      const { tree } = issue;
      const { claude } = tree;
      const name = known[claude.id]?.name ?? tree.name;
      const was = wasOf(known[claude.id], now);
      // Announced under its name before the handover: the same agent.
      const announced = handovers.find(
        handover =>
          handover.to === name || handover.to === known[claude.id]?.was,
      );
      const offered = [...suggested].find(
        ([, { worker }]) => worker.name === name,
      );
      const status = agentStatus.get(claude.id)?.status ?? null;
      return {
        issue,
        name,
        was,
        status,
        state: agentStateOf(tree, status, of(issue).mr),
        share: claude.window ? Math.min(claude.used / claude.window, 1) : null,
        next: announced
          ? `next #${announced.number}${afterText(announced.open ?? announced.after)}`
          : offered
            ? `next #${offered[0]} once its MR is merged`
            : "",
      };
    });
  const waiting = issues
    .filter(issue => !issue.tree?.claude && (issue.queued || issue.tree))
    .map(issue => {
      const announced = handovers.find(
        handover => handover.number === issue.number,
      );
      const offered = suggested.get(issue.number)?.worker;
      return {
        issue,
        text: issue.tree
          ? "no agent runs in its worktree"
          : announced
            ? `goes to ${announced.to}${afterText(announced.open ?? announced.after)}`
            : offered
              ? `goes to ${offered.name} once its MR is merged`
              : "queued (--queue): a click on its start in UP NEXT",
      };
    });
  return { agents, waiting };
};

/** The head line of AGENTS: `3 at work · 1 idle · 0 waiting`. */
export const agentCountsOf = agents => {
  const count = keys =>
    agents.filter(row => keys.includes(row.state.key)).length;
  return `${count(["work"])} at work · ${count(["idle"])} idle · ${count(["asks", "yours", "waits"])} waiting`;
};

/**
 * AGENTS: a row per agent (a click goes to its space), then the issues that
 * wait without one; its title counts them and names the memory threshold
 * below which a fresh agent gets the shared stack (memory.mjs).
 */
export const pushAgents = (lines, issues, of, agentStatus, context, free) => {
  const { agents, waiting } = agentRowsOf(issues, of, agentStatus, context);
  if (agents.length + waiting.length === 0) return;
  const box = openBox(lines);
  pushRows(
    lines,
    [
      ...agents.map(row => [
        "",
        toneOf(row.status)(row.state.text),
        row.was ? `${bold(row.name)} ${dim(`was ${row.was}`)}` : bold(row.name),
        `#${row.issue.number}`,
        row.share === null ? "" : dim(`${Math.round(row.share * 100)} %`),
        row.next ? cyan(`→ ${row.next}`) : "",
      ]),
      ...waiting.map(row => [
        "",
        dim("waits"),
        bold(row.issue.feature ?? ""),
        `#${row.issue.number}`,
        "",
        row.text,
      ]),
    ],
    [...agents, ...waiting].map(
      ({ issue }) =>
        () =>
          issue.tree && focusSpace(issue.tree.dir),
    ),
  );
  const memory =
    free === undefined
      ? ""
      : free < THRESHOLD
        ? ` · ${red(`${gb(free)} free: fresh agents on the shared stack`)}`
        : dim(` · ${gb(free)} free (shared stack below ${gb(THRESHOLD)})`);
  closeBox(
    lines,
    box,
    `${bold("AGENTS")} ${dim(`${agentCountsOf(agents)}${waiting.length ? ` · ${waiting.length} ${waiting.length === 1 ? "issue waits" : "issues wait"}` : ""}`)}${memory}`,
  );
};
