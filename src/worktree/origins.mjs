import fs from "node:fs";
import path from "node:path";

import { herdr, isInHerdr, panesOf, spaces, tabsOf } from "../lib/herdr.mjs";
import { NAME, OVERVIEW, SPACE } from "./constants.mjs";
import {
  orchestratorCheckouts,
  removeOrchestratorCheckout,
} from "./orchestrator.mjs";
import { featureOf, isIdle } from "./tabs.mjs";
import { mainDir, worktrees } from "./worktrees.mjs";

// ---------------------------------------------------------------------------
// Orchestration: one claude session works on several issues at once through
// an agent per worktree (`wt:orchestrate`).
//
//   ⇶ <name>               root · overview (its issues and MRs) · orchestrator (the tab of the orchestrating session)
//   <feature> (per issue)  the usual space, its <feature> tab running a claude on the issue
//
// A worktree started for an orchestration records it (`.claude/origin`), and
// the sidebar marks the orchestrator's space and its worktrees' spaces with
// the same token in the orchestration's colour: `$orch1`, `$orch2`, … one per
// slot, styled in `.herdr/config.toml` (sidebar.mjs, written by pnpm herdr).
// herdr forgets tokens on a restart: the layout reports them again.
// ---------------------------------------------------------------------------

/** The mark of an orchestration: its space is `⇶ <name>`, and its worktrees' spaces show `⇶ <name>`. */
export const ORCHESTRATOR = "⇶";
/** The tab of the orchestrating session in its space. */
export const ORCHESTRATOR_TAB = "orchestrator";
/**
 * One colour per concurrent orchestration, named as in the base config's
 * `[theme.custom]` (with a fallback): the informational accents, the ones a
 * theme doesn't spend on agent states (red blocked, yellow working, green
 * idle). A fourth orchestration shares the first one's colour; its label
 * still tells them apart.
 */
export const ORIGIN_SLOTS = [
  { colour: "blue", fallback: "#89B4FA" },
  { colour: "teal", fallback: "#94E2D5" },
  { colour: "mauve", fallback: "#CBA6F7" },
];
/** The sidebar token of a slot: `orch1` (herdr's config writes it `$orch1`). */
export const originToken = slot => `orch${slot + 1}`;
/** Whose tokens they are, for herdr: the plugin's id. */
export const TOKEN_SOURCE = "monocircuit.platform";
/** The space of an orchestration, in the group of the main space: `⇶ <name>`. */
export const orchestratorLabel = name => `${ORCHESTRATOR} ${name}`;
/** What the spaces of an orchestration's worktrees show: `⇶ <name>`. */
export const originMark = name => `${ORCHESTRATOR} ${name}`;
/** The orchestration of a space's label, also of an older space `Monocircuit/Platform ⇶ <name>`; null for any other. */
export const orchestrationOf = label =>
  label?.match(new RegExp(`^(?:${SPACE} )?${ORCHESTRATOR} (.+)$`))?.[1] ?? null;
/** Where a worktree records the orchestration that started it, next to herdr-session: `{ "name", "slot" }`. */
const originFile = dir => path.join(dir, ".claude", "origin");
/** The orchestration that started a worktree, or null. */
export const originOf = dir => {
  try {
    const { name, slot } = JSON.parse(
      fs.readFileSync(originFile(dir), "utf-8"),
    );
    return NAME.test(name) && Number.isInteger(slot) && slot >= 0
      ? { name, slot: slot % ORIGIN_SLOTS.length }
      : null;
  } catch {
    return null;
  }
};
/** Every worktree an orchestration started, with its origin. */
const orchestrated = () =>
  worktrees()
    .slice(1)
    .map(({ dir }) => ({ dir, origin: originOf(dir) }))
    .filter(({ origin }) => origin);
/**
 * What an orchestration covers, in the ignored `.claude/` of the main
 * checkout: `{ issues: [{ number, title, feature, branch, why }] }`, `why`
 * saying why an issue got no worktree (skipped, failed), null for one that
 * did. It outlives the worktrees, so the overview of the orchestration still
 * shows an issue after its worktree went with the merge (wt:remove notes
 * `removedAt` and `mergedAt` on its entry, markRemoved); it goes with the
 * space `⇶ <name>` (pruneOrchestrators). The orchestrator adds `plan`
 * (`pnpm wt:plan`: `{ work, merge, why }`, issue numbers in order) and its
 * own `actions` (`pnpm wt:action --orchestration`).
 */
export const recordFile = name =>
  path.join(mainDir(), ".claude", "orchestrations", `${name}.json`);
/** The record of an orchestration, or null. */
export const recordOf = name => {
  try {
    const record = JSON.parse(fs.readFileSync(recordFile(name), "utf-8"));
    return Array.isArray(record.issues) ? record : null;
  } catch {
    return null;
  }
};
/** Every orchestration with a record. */
const recorded = () => {
  const dir = path.join(mainDir(), ".claude", "orchestrations");
  return fs.existsSync(dir)
    ? fs
        .readdirSync(dir)
        .filter(file => file.endsWith(".json"))
        .map(file => path.basename(file, ".json"))
    : [];
};
/**
 * `issues` into the record of `name`, by number. A second run of the same
 * orchestration adds to it; an issue it skips because the first run started
 * it (its worktree exists) keeps the entry of that run.
 */
export const writeRecord = (name, issues) => {
  const known = new Map(
    (recordOf(name)?.issues ?? []).map(issue => [issue.number, issue]),
  );
  for (const issue of issues) {
    if (issue.why && known.has(issue.number) && !known.get(issue.number).why)
      continue;
    known.set(issue.number, issue);
  }
  fs.mkdirSync(path.dirname(recordFile(name)), { recursive: true });
  fs.writeFileSync(
    recordFile(name),
    `${JSON.stringify({ ...recordOf(name), issues: [...known.values()] }, null, 2)}\n`,
  );
};
/**
 * `record` with the worktree of `branch` marked as removed: `removedAt` and
 * the MR's `mergedAt` (null when unknown) on its issue entry. A worktree
 * started for the orchestration by hand (`wt:new --origin`) has no entry
 * yet: it gets one, so the overview still lists it once it is gone.
 */
export const withRemoved = (
  record,
  { branch, feature, removedAt, mergedAt = null },
) => {
  const issues = record?.issues ?? [];
  const known = issues.some(issue => issue.branch === branch);
  const number = Number(branch.match(/\/(\d+)-/)?.[1]) || null;
  return {
    ...record,
    issues: known
      ? issues.map(issue =>
          issue.branch === branch
            ? {
                ...issue,
                removedAt,
                mergedAt: mergedAt ?? issue.mergedAt ?? null,
              }
            : issue,
        )
      : [
          ...issues,
          {
            number,
            title: null,
            feature,
            branch,
            why: null,
            removedAt,
            mergedAt,
          },
        ],
  };
};
/** Notes in the record of `name` that the worktree of `branch` was removed (wt:remove). */
export const markRemoved = (name, entry) => {
  fs.mkdirSync(path.dirname(recordFile(name)), { recursive: true });
  fs.writeFileSync(
    recordFile(name),
    `${JSON.stringify(withRemoved(recordOf(name), entry), null, 2)}\n`,
  );
};
/**
 * The order the orchestrator set (`pnpm wt:plan`): `work`, the issues in the
 * order they are worked on, `merge`, the order their MRs go in, `why` in one
 * line. A list left out keeps the one there.
 */
export const writePlan = (name, { work, merge, why }) => {
  const record = recordOf(name);
  const plan = {
    work: work ?? record.plan?.work ?? [],
    merge: merge ?? record.plan?.merge ?? [],
    why: why ?? record.plan?.why ?? null,
  };
  fs.writeFileSync(
    recordFile(name),
    `${JSON.stringify({ ...record, plan }, null, 2)}\n`,
  );
  return plan;
};
/**
 * An agent of the orchestration as a worker (#217), by its conversation's
 * id: `{ name, issues }`, `name` the one it was started with (`claude
 * --name`, ListAgents finds it by it), `issues` the numbers it worked on, in
 * order. `worker` is merged into what the record has.
 */
export const writeWorker = (name, id, worker) => {
  const record = recordOf(name);
  const workers = {
    ...record.workers,
    [id]: { ...record.workers?.[id], ...worker },
  };
  fs.writeFileSync(
    recordFile(name),
    `${JSON.stringify({ ...record, workers }, null, 2)}\n`,
  );
};
/**
 * Fields of the whole record of `name` set, e.g. `cap` (how many of its
 * agents work at once, #217), `peer` and `brief` (of wt:orchestrate, for an
 * issue started later from the queue). An undefined one is left as it is.
 */
export const updateRecord = (name, fields) => {
  const record = recordOf(name) ?? { issues: [] };
  const set = Object.fromEntries(
    Object.entries(fields).filter(([, value]) => value !== undefined),
  );
  fs.mkdirSync(path.dirname(recordFile(name)), { recursive: true });
  fs.writeFileSync(
    recordFile(name),
    `${JSON.stringify({ ...record, ...set }, null, 2)}\n`,
  );
};
/** The slot a space's tokens show, or undefined. */
const slotOfTokens = tokens =>
  ORIGIN_SLOTS.findIndex((_, slot) => tokens?.[originToken(slot)]);
/**
 * The colour slot of the orchestration `name`: the one its worktrees or its
 * space carry already, else the first no other orchestration holds (the
 * least used one when all are taken). Sequential calls agree, so the space
 * and every worktree of one orchestration get the same.
 */
export const slotOf = name => {
  const trees = orchestrated();
  const own = trees.find(({ origin }) => origin.name === name);
  if (own) return own.origin.slot;
  const marked = isInHerdr()
    ? spaces().filter(w => orchestrationOf(w.label))
    : [];
  const space = marked.find(w => orchestrationOf(w.label) === name);
  if (space && slotOfTokens(space.tokens) >= 0)
    return slotOfTokens(space.tokens);
  const uses = ORIGIN_SLOTS.map(() => 0);
  for (const { origin } of trees) uses[origin.slot]++;
  for (const w of marked)
    if (slotOfTokens(w.tokens) >= 0) uses[slotOfTokens(w.tokens)]++;
  return uses.indexOf(Math.min(...uses));
};
/** Records who started a worktree. */
export const writeOrigin = (dir, origin) => {
  fs.mkdirSync(path.dirname(originFile(dir)), { recursive: true });
  fs.writeFileSync(originFile(dir), `${JSON.stringify(origin)}\n`);
};
/**
 * The origin token of a space as `origin` wants it: its slot's token set to
 * `⇶ <name>`, the other slots' cleared. Nothing is sent when it shows that
 * already; `workspace` is an entry of `workspace list`, which has the tokens.
 */
export const markSpace = (workspace, origin) => {
  const args = [];
  for (const [slot] of ORIGIN_SLOTS.entries()) {
    const token = originToken(slot);
    const wanted = origin?.slot === slot ? originMark(origin.name) : null;
    const shown = workspace.tokens?.[token] ?? null;
    if (wanted && shown !== wanted) args.push("--token", `${token}=${wanted}`);
    if (!wanted && shown) args.push("--clear-token", token);
  }
  // herdr wants the space first: `--source` in front takes the id for an option.
  if (args.length)
    herdr([
      "workspace",
      "report-metadata",
      workspace.workspace_id,
      "--source",
      TOKEN_SOURCE,
      ...args,
    ]);
};
/**
 * Every space shows the orchestration it belongs to: a worktree's space its
 * origin, an orchestrator's space its own. The layout calls it, so the marks
 * come back after a herdr restart.
 */
export const markOrigins = () => {
  const byFeature = new Map(
    orchestrated().map(({ dir, origin }) => [featureOf(dir), origin]),
  );
  for (const workspace of spaces()) {
    const name = orchestrationOf(workspace.label);
    markSpace(
      workspace,
      name
        ? { name, slot: slotOf(name) }
        : (byFeature.get(workspace.label) ?? null),
    );
  }
};
/**
 * The space of an orchestration closes once its last worktree is gone — but
 * never while something runs in it: the orchestrating claude, or the
 * wt:orchestrate that is still creating the worktrees. Its overview runs as
 * long as the space: it keeps nothing open. The record goes with the space,
 * and so does one whose space and worktrees are both gone. Returns what closed.
 */
export const pruneOrchestrators = () => {
  const live = new Set(orchestrated().map(({ origin }) => origin.name));
  const closed = [];
  const open = new Set();
  for (const workspace of spaces()) {
    const name = orchestrationOf(workspace.label);
    if (!name) continue;
    open.add(name);
    if (live.has(name)) continue;
    const overview = tabsOf(workspace.workspace_id).find(
      tab => tab.label === OVERVIEW,
    );
    const panes = panesOf(workspace.workspace_id).filter(
      p => p.tab_id !== overview?.tab_id,
    );
    if (panes.some(p => p.agent || !isIdle(p.pane_id))) continue;
    if (herdr(["workspace", "close", workspace.workspace_id])) {
      open.delete(name);
      closed.push(
        `closed the space ${workspace.label}, its worktrees are gone`,
      );
      const kept = removeOrchestratorCheckout(name);
      if (kept) closed.push(kept);
    }
  }
  for (const name of recorded()) {
    if (live.has(name) || open.has(name)) continue;
    fs.rmSync(recordFile(name), { force: true });
  }
  // A checkout whose space is gone and whose worktrees are too (a record may never have been written).
  for (const dir of orchestratorCheckouts()) {
    const name = path.basename(dir);
    if (live.has(name) || open.has(name)) continue;
    const kept = removeOrchestratorCheckout(name);
    if (kept) closed.push(kept);
  }
  return closed;
};
