// Agents are workers (#217): an orchestration hands its next issue to an idle
// agent that knows the code already, instead of starting a fresh one. The
// issue keeps its worktree, branch and MR; the agent moves from one worktree
// to the next (`wt:orchestrate <name> <issue> --to <agent>`).
import fs from "node:fs";

import { glabApi } from "../lib/glab.mjs";
import { herdr, herdrDid } from "../lib/herdr.mjs";
import { git, lines } from "../lib/proc.mjs";
import { EDITOR, OVERVIEW, ROOT } from "./constants.mjs";
import { boundSession, contextShareOf, sessionFile } from "./conversations.mjs";
import { originOf, recordOf, writeWorker } from "./origins.mjs";
import { spaceOf } from "./spaces.mjs";
import { dropDuplicates, featureOf, keepOrder } from "./tabs.mjs";
import { worktrees } from "./worktrees.mjs";

/** The share of its context above which an agent gets no next issue: it would run out in the middle of it. */
export const CONTEXT_LIMIT = 0.5;
/** How long AGENTS names an agent's name before its last handover (`was <old>`): messages to that one can be explained. */
export const WAS_FOR = 24 * 60 * 60 * 1000;

/**
 * The agents of the orchestration `name`, one per worktree of it with a
 * conversation bound: `{ dir, feature, id, name, was }`, `name` the one it
 * goes by (the record's, set by its last handover, else the feature of the
 * worktree it is in), `was` the one before that handover, or null.
 */
export const workersOf = name => {
  const known = recordOf(name)?.workers ?? {};
  return worktrees()
    .slice(1)
    .filter(({ dir }) => originOf(dir)?.name === name)
    .map(({ dir }) => ({ dir, feature: featureOf(dir), id: boundSession(dir) }))
    .filter(worker => worker.id)
    .map(worker => ({
      ...worker,
      name: known[worker.id]?.name ?? worker.feature,
      was: known[worker.id]?.was ?? null,
    }));
};
/**
 * The worker `who` names among `workers`, or undefined: its name, else the
 * feature of the worktree it is in, else its name before the last handover
 * (a handover announced under that one still finds it).
 */
export const findWorker = (workers, who) =>
  workers.find(worker => worker.name === who) ??
  workers.find(worker => worker.feature === who) ??
  workers.find(worker => worker.was === who);

/** The pane herdr runs the conversation `id` in, or null. */
export const paneOfConversation = id =>
  (herdr(["pane", "list"])?.panes ?? []).find(
    pane => pane.agent_session?.value === id,
  ) ?? null;
/** What an agent in each of herdr's states is doing, when that keeps it from a next issue. */
const BUSY = {
  working: "it is working",
  blocked: "it waits for an answer",
};
/**
 * Why an agent can't take a next issue, or null: no pane runs it, it works
 * or waits for an answer (herdr's `agent_status`; `idle` and `done` are
 * free), or its context `share` (0 to 1) is above `limit`.
 */
export const refusalOf = ({ pane, share, limit = CONTEXT_LIMIT }) => {
  if (!pane)
    return "no claude runs it (herdr knows no pane with its conversation)";
  const status = pane.agent_status ?? "unknown";
  if (BUSY[status]) return BUSY[status];
  if (!["idle", "done"].includes(status))
    return `herdr can't tell what it does (${status})`;
  if (share !== null && share > limit)
    return `its context is at ${Math.round(share * 100)} %, above the limit of ${Math.round(limit * 100)} %`;
  return null;
};

/**
 * The line an agent gets its next issue with, typed into its prompt: its
 * new name (#251), the feature of the worktree, comes with it.
 */
export const nextIssueLine = (number, dir) =>
  `[overview] next issue #${number}, your name is ${featureOf(dir)} now: EnterWorktree(path: "${dir}"), then read .claude/brief.md`;
/** The line typed before it: Claude Code's own name of the conversation, which ListAgents and SendMessage go by. */
export const renameLine = dir => `/rename ${featureOf(dir)}`;
/**
 * What the brief of a reused agent says about its issues before, in one
 * line, so it carries no assumptions over. `issues`: `[{ number, title }]`.
 */
export const beforeLine = issues =>
  issues.length
    ? `You are a reused agent: you worked on ${issues.map(({ number, title }) => `#${number}${title ? ` (${title})` : ""}`).join(", ")} before. Treat this issue on its own, from develop as it is now: nothing of that work carries over but what is merged.`
    : null;
/** The issues `worker` worked on, as the record of `name` knows them (`[{ number, title }]`). */
export const issuesBefore = (name, worker) => {
  const record = recordOf(name);
  const numbers = record?.workers?.[worker.id]?.issues ?? [
    record?.issues.find(issue => issue.feature === worker.feature)?.number,
  ];
  return numbers.filter(Boolean).map(number => ({
    number,
    title: record?.issues.find(issue => issue.number === number)?.title ?? null,
  }));
};

/**
 * The agent `worker` moves to the worktree `dir` (made, its brief written):
 * its pane into the worktree's space as the tab `<feature>` (the spare one
 * wt:new left there closes), the conversation bound to the new worktree
 * instead of the old one, and it is renamed after the feature (#251): the
 * pane's label, and `/rename <feature>` typed into its prompt before the
 * line of its next issue. Its old worktree keeps its space until the
 * clean-up. Returns what happened, `{ isMoved, isTold }`.
 */
export const moveWorker = (worker, pane, number, dir) => {
  const feature = featureOf(dir);
  const workspace = spaceOf(dir);
  let isMoved = false;
  // A pane moved into another space gets a new id there; the old one takes no prompt (#222).
  let target = pane.pane_id;
  if (workspace) {
    const moved = herdr([
      "pane",
      "move",
      pane.pane_id,
      "--workspace",
      workspace,
      "--new-tab",
    ]);
    target = moved?.move_result?.pane?.pane_id ?? target;
    const created = moved?.move_result?.created_tab;
    if (created) {
      herdr(["tab", "rename", created.tab_id, feature]);
      dropDuplicates(workspace, feature);
      keepOrder(workspace, [ROOT, OVERVIEW, feature, EDITOR]);
      isMoved = true;
    }
  }
  // The binding follows the conversation: a restart resumes it in its new space, answers go there.
  fs.writeFileSync(sessionFile(dir), `${worker.id}\n`);
  fs.rmSync(sessionFile(worker.dir), { force: true });
  // The rename first: a stale pane id refuses it, and the next one is tried.
  const tell = paneId => {
    if (!paneId || !herdrDid(["agent", "prompt", paneId, renameLine(dir)]))
      return false;
    herdrDid(["pane", "rename", paneId, feature]);
    return herdrDid(["agent", "prompt", paneId, nextIssueLine(number, dir)]);
  };
  // Without the new id in herdr's answer: the pane that runs the conversation now.
  let isTold = tell(target);
  if (!isTold) {
    const now = paneOfConversation(worker.id)?.pane_id;
    isTold = now !== target && tell(now);
  }
  return { isMoved, isTold };
};
/**
 * The record's entry of `worker` after its handover to the issue `number`
 * in the worktree `dir`: named after its feature, `was` its name before
 * (kept through a handover that doesn't rename it) with `renamed`, when.
 * `before`: the numbers of its issues before, `files` what it knows.
 */
export const handedOver = (worker, { number, dir, before, files, at }) => {
  const name = featureOf(dir);
  const isRenamed = name !== worker.name;
  return {
    name,
    was: isRenamed ? worker.name : (worker.was ?? null),
    renamed: isRenamed ? at : (worker.renamed ?? null),
    issues: [...before.filter(n => n !== number), number],
    files,
  };
};
/** The record of `name` learns that `worker` works on `number` in `dir` now, under the name of its feature. */
export const recordHandover = (name, worker, number, dir) => {
  const known = recordOf(name)?.workers?.[worker.id] ?? {};
  writeWorker(
    name,
    worker.id,
    handedOver(
      { ...worker, renamed: known.renamed },
      {
        number,
        dir,
        before: issuesBefore(name, worker).map(issue => issue.number),
        // What it knows from the worktree it leaves: that one goes with its clean-up.
        files: [...new Set([...(known.files ?? []), ...filesOf(worker.dir)])],
        at: new Date().toISOString(),
      },
    ),
  );
};
/** Its name before the last handover, for AGENTS, while that was less than WAS_FOR ago; else null. */
export const wasOf = (known, now = Date.now()) =>
  known?.was && now - Date.parse(known.renamed ?? "") < WAS_FOR
    ? known.was
    : null;

/**
 * The agents of the orchestration `name` free for a next issue now, for
 * wt:orchestrate: herdr sees them idle (or done), their context below the
 * limit and their MR merged (an agent only ready may still be needed for
 * it). `{ name, share, files, dir, isMerged }`, `files` those of its
 * worktree and, from the record, of its earlier ones (fit.mjs).
 */
export const idleWorkersNow = async name => {
  const known = recordOf(name)?.workers ?? {};
  const branchOf = new Map(worktrees().map(({ dir, branch }) => [dir, branch]));
  const candidates = workersOf(name)
    .map(worker => ({ ...worker, share: contextShareOf(worker.id) }))
    .filter(
      worker =>
        refusalOf({
          pane: paneOfConversation(worker.id),
          share: worker.share,
        }) === null,
    );
  const merged = await Promise.all(
    candidates.map(({ dir }) =>
      glabApi(
        `projects/:fullpath/merge_requests?state=merged&source_branch=${encodeURIComponent(branchOf.get(dir) ?? "")}`,
      ).then(
        mrs => mrs.length > 0,
        () => false,
      ),
    ),
  );
  return candidates
    .filter((_, i) => merged[i])
    .map(worker => ({
      name: worker.name,
      share: worker.share ?? 0,
      files: [
        ...new Set([
          ...(known[worker.id]?.files ?? []),
          ...filesOf(worker.dir),
        ]),
      ],
      dir: worker.dir,
      isMerged: true,
    }));
};
/** dir → `{ head, files }`: the files of a worktree's commits, read again after a new commit. */
const filesCache = new Map();
/**
 * The files the commits of a worktree's branch touched: from where the
 * branch was created (the oldest entry of its reflog, wt:new made it) to
 * HEAD, along the first parent and without merges, so what `develop`
 * brought in doesn't count. It stays the same after the merge.
 */
export const filesOf = dir => {
  const head = git(["rev-parse", "HEAD"], dir).out;
  const cached = filesCache.get(dir);
  if (cached?.head === head) return cached.files;
  const branch = git(["symbolic-ref", "--quiet", "HEAD"], dir).out;
  const created = branch
    ? git(["reflog", "show", "--format=%H", branch], dir).out.split("\n").at(-1)
    : "";
  const files = created
    ? lines(
        git(
          [
            "log",
            "--first-parent",
            "--no-merges",
            "--format=",
            "--name-only",
            `${created}..HEAD`,
          ],
          dir,
        ).out,
      )
    : [];
  const unique = [...new Set(files)];
  filesCache.set(dir, { head, files: unique });
  return unique;
};
