// The actions the overview of an orchestration finds by itself, from what it
// reads anyway (GitLab, herdr, the transcripts): no agent has to remember to
// post them, no model spends a token on them. Computed on each read, never
// stored: an action goes when its cause goes.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { herdr, isInHerdr } from "../lib/herdr.mjs";
import { ESCAPES, cyan, red, yellow } from "../lib/tty.mjs";
import { featureOf, focusSpace, isChangeOpen, mainDir } from "../worktree.mjs";
import { deliverTo } from "./answers.mjs";
import { unpushedActionOf } from "./change.mjs";
import { cleanUpOf } from "./clean-up.mjs";
import { show } from "./io.mjs";
import { screen } from "./screen.mjs";

/** How the automatic kinds show; `merge` is the kind an agent posts too. */
export const AUTO_KIND = {
  pipeline: red("pipeline"),
  "clean up": cyan("clean up"),
  waits: yellow("waits"),
  push: red("push"),
};

// ---------------------------------------------------------------------------
// herdr and the transcripts
// ---------------------------------------------------------------------------

/**
 * The agents herdr runs, by their conversation's id: `{ pane, tab, status }`,
 * `status` as herdr says it (`working`, `blocked`: it asks you, `idle`, …).
 * One herdr call for every worktree; empty outside herdr.
 */
export const agentsOf = () => {
  if (!isInHerdr()) return new Map();
  const panes = herdr(["pane", "list"])?.panes ?? [];
  return new Map(
    panes
      .filter(pane => pane.agent && pane.agent_session?.value)
      .map(pane => [
        pane.agent_session.value,
        { pane: pane.pane_id, tab: pane.tab_id, status: pane.agent_status },
      ]),
  );
};

/** How much of a transcript's end is read for a tool call that waits: it is the last entry. */
const PENDING_BYTES = 1 << 16;
const pendings = new Map();
/** The first line of `text` that says something, shortened to fit a row. */
const firstLine = text =>
  String(text ?? "")
    .split("\n")
    .map(line => line.replace(/^[#>*\s-]+/, "").trim())
    .find(Boolean) ?? "";
/**
 * The tool call a conversation's last answer ends with, when nothing answered
 * it yet, as a line: the question of an `AskUserQuestion`, else the tool and
 * what it is about (a permission prompt). Null when the last entry is not one.
 * `lines`: the transcript's lines, oldest first.
 */
export const pendingOf = lines => {
  for (const line of lines.toReversed()) {
    let entry;
    try {
      entry = JSON.parse(line);
    } catch {
      continue;
    }
    if (entry.isSidechain || !["user", "assistant"].includes(entry.type))
      continue;
    if (entry.type === "user") return null;
    const content = entry.message?.content;
    const call = Array.isArray(content)
      ? content.findLast(part => part.type === "tool_use")
      : null;
    if (!call) return null;
    const input = call.input ?? {};
    if (call.name === "AskUserQuestion")
      return firstLine(input.questions?.[0]?.question) || "a question";
    const about = firstLine(
      input.description ?? input.command ?? input.file_path ?? input.url,
    );
    return `may it run ${call.name}${about ? `: ${about}` : ""}?`;
  }
  return null;
};
/** pendingOf of a transcript file, read again only when it changed. */
const pendingOfFile = transcript => {
  try {
    const { size, mtimeMs } = fs.statSync(transcript);
    const cached = pendings.get(transcript);
    if (cached?.size === size && cached.mtimeMs === mtimeMs)
      return cached.pending;
    const length = Math.min(size, PENDING_BYTES);
    const buffer = Buffer.alloc(length);
    const fd = fs.openSync(transcript, "r");
    fs.readSync(fd, buffer, 0, length, size - length);
    fs.closeSync(fd);
    const lines = buffer.toString("utf-8").split("\n");
    if (length < size) lines.shift(); // cut in the middle
    const pending = pendingOf(lines);
    pendings.set(transcript, { size, mtimeMs, pending });
    return pending;
  } catch {
    return null;
  }
};
/**
 * The question an answer ends on: its last line with a `?`, when that is in
 * its last paragraph. An agent that stops on one waits for the human.
 */
export const questionOf = answer => {
  const paragraphs = String(answer ?? "")
    .trim()
    .split(/\n\s*\n/);
  const line = paragraphs
    .at(-1)
    .split("\n")
    .findLast(candidate => candidate.includes("?"));
  return line ? firstLine(line) : null;
};

// ---------------------------------------------------------------------------
// What the clicks do
// ---------------------------------------------------------------------------

/** The agent's tab in front: its space, then the tab. */
const focusAgent = (dir, agent) => {
  focusSpace(dir);
  if (agent?.tab) herdr(["tab", "focus", agent.tab]);
};

/**
 * `wt:remove` of a worktree, per its dir: `{ isArmed, isRunning, error }`.
 * Two clicks, like the ✕ of the worktree view: the first arms, the second runs.
 */
const removals = new Map();
const ARM_MS = 5000;
const removeWorktree = dir => {
  const log = path.join(
    os.tmpdir(),
    `overview-remove-${path.basename(dir)}.log`,
  );
  const out = fs.openSync(log, "w");
  const child = spawn(
    process.execPath,
    [
      path.join(import.meta.dirname, "..", "worktree.mjs"),
      "remove",
      featureOf(dir),
      "--close-space",
    ],
    {
      cwd: mainDir(),
      env: { ...process.env, NO_COLOR: "1" },
      stdio: ["ignore", out, out],
      detached: true,
    },
  );
  fs.closeSync(out);
  child.unref();
  const state = { isArmed: false, isRunning: true, error: null };
  removals.set(dir, state);
  child.on("exit", code => {
    state.isRunning = false;
    if (code !== 0) {
      const lines = fs.readFileSync(log, "utf-8").trim().split("\n");
      state.error = lines.at(-1) || `wt:remove ended with ${code}`;
    }
    screen.redraw({ isReread: true });
  });
  screen.redraw();
};
/** Where the wt:remove of the worktree `dir` stands: `{ isArmed, isRunning, error }`, or undefined. */
export const removalOf = dir => removals.get(dir);
/**
 * The automatic clean-up (#227, clean-up.mjs): wt:remove of each merged
 * worktree nothing stops, once. One that failed stays for a click (its row
 * in UP NEXT says why). `holds`: dir → why a handover holds it.
 */
export const removeMerged = (issues, of, agents, holds = new Map()) => {
  // herdr told nothing: a working agent would look idle.
  if (agents.size === 0) return;
  for (const issue of issues) {
    const { tree } = issue;
    if (!tree || removals.has(tree.dir)) continue;
    const status = tree.claude ? agents.get(tree.claude.id)?.status : null;
    const cleanUp = cleanUpOf(issue, {
      mr: of(issue).mr,
      status,
      hold: holds.get(tree.dir),
    });
    if (cleanUp?.key === "due") removeWorktree(tree.dir);
  }
};
const clickRemove = dir => {
  const state = removals.get(dir);
  if (state?.isRunning) return;
  if (!state?.isArmed) {
    const armed = { isArmed: true, isRunning: false, error: null };
    removals.set(dir, armed);
    setTimeout(() => {
      armed.isArmed = false;
      screen.redraw();
    }, ARM_MS);
    screen.redraw();
    return;
  }
  removeWorktree(dir);
};
const removeText = (dir, feature) => {
  const state = removals.get(dir);
  if (state?.isRunning) return `removing ${feature} …`;
  if (state?.isArmed)
    return red(
      `click again to remove ${feature}: worktree, branch and its stack's data`,
    );
  return `remove the worktree ${feature}${state?.error ? `  ${red(state.error)}` : ""}`;
};

/**
 * Where a worktree remembers the automatic lines that got in to its agent, in
 * its ignored `.claude/`: `{ sent: [key, …] }`. Their rows stay hidden, also
 * after a restart of the overview; a new cause (another pipeline) has a new key.
 */
export const sentFile = dir => path.join(dir, ".claude", "overview-sent.json");
/** How many keys a worktree keeps, the newest. */
const SENT_KEPT = 50;
export const sentOf = dir => {
  try {
    const { sent } = JSON.parse(fs.readFileSync(sentFile(dir), "utf-8"));
    return new Set(Array.isArray(sent) ? sent : []);
  } catch {
    return new Set();
  }
};
const markSent = (dir, key) => {
  const sent = [...sentOf(dir), key].slice(-SENT_KEPT);
  fs.mkdirSync(path.dirname(sentFile(dir)), { recursive: true });
  fs.writeFileSync(sentFile(dir), `${JSON.stringify({ sent }, null, 2)}\n`);
};
/** `key` taken out of what `dir` remembers as sent: its cause went, a new one sends again. */
const forgetSent = (dir, key) => {
  const sent = [...sentOf(dir)].filter(candidate => candidate !== key);
  fs.writeFileSync(sentFile(dir), `${JSON.stringify({ sent }, null, 2)}\n`);
};
/**
 * What to do about conflicts with develop: of each open MR with a worktree,
 * `{ dir, key, line }` to send when GitLab says it has conflicts and its
 * agent wasn't told yet, `{ dir, key }` to forget once they are gone.
 * `sentOf(dir)`: the keys the worktree remembers as sent.
 */
export const conflictsOf = (issues, of, sentOf) =>
  issues.flatMap(issue => {
    const { mr } = of(issue);
    const dir = issue.tree?.dir;
    if (!dir || mr?.state !== "opened") return [];
    const key = `conflicts:${mr.iid}`;
    const isSent = sentOf(dir).has(key);
    if (mr.hasConflicts && !isSent)
      return [
        {
          dir,
          key,
          line: `[overview] !${mr.iid} has conflicts with develop: merge develop in (git fetch origin, git merge origin/develop)`,
        },
      ];
    if (!mr.hasConflicts && isSent) return [{ dir, key, isGone: true }];
    return [];
  });
/**
 * Tells each agent whose MR got conflicts with develop, one line once per
 * conflict, without anyone noticing it first. A line that doesn't get in
 * (the agent is at a question) is tried again at the next read.
 */
export const tellConflicts = (issues, of) => {
  for (const { dir, key, line, isGone } of conflictsOf(issues, of, sentOf)) {
    if (isGone) forgetSent(dir, key);
    else sendOnce(dir, key, line);
  }
};
/** `line` to the agent of `dir`; once it got in, the row of `key` goes. True when it got in. */
export const sendOnce = (dir, key, line) => {
  if (!deliverTo(dir, line)) return false;
  markSent(dir, key);
  screen.redraw();
  return true;
};

// ---------------------------------------------------------------------------
// The actions
// ---------------------------------------------------------------------------

/**
 * The automatic actions of an orchestration's issues, as the overview lists
 * them: `{ key, kind, text, at, issue, go }`, `key` the same for as long as
 * the cause lasts. `of(issue)`: its part of GitLab (`{ mr, jobs, issue }`);
 * `agents`: agentsOf().
 * - `merge`: the MR is ready and its pipeline passed, unless the agent posted
 *   one or a requested change still blocks it (change.mjs); a draft of
 *   `readyDrafts` too (issue numbers: next in the merge order, green, every
 *   Work box ticked), whose agent could not mark it ready: its merge does;
 * - `push`: its worktree holds commits origin doesn't have for long;
 * - `pipeline`: the pipeline of the open MR failed; the click tells the agent
 *   which job, with its url;
 * - `clean up`: the MR is merged; the worktree to remove (two clicks) when
 *   the automatic removal failed, the issue's open boxes for the agent to tick;
 * a row whose click sent its line to the agent is gone (sendOnce);
 * - `waits`: the agent asks something (a question, a permission) or stopped
 *   on a question; the click goes to its tab.
 */
export const autoActionsOf = (
  issues,
  of,
  agents,
  { readyDrafts = new Set() } = {},
) =>
  issues.flatMap(issue => {
    const { mr, jobs = [], issue: found } = of(issue);
    const { tree } = issue;
    const agent = tree?.claude && agents.get(tree.claude.id);
    const sent = tree ? sentOf(tree.dir) : new Set();
    const actions = [];
    const add = (kind, text, at, go, more = {}) =>
      actions.push({
        key: `${kind}:${issue.branch}`,
        kind,
        text,
        at: new Date(at ?? Date.now()).toISOString(),
        issue,
        go,
        ...more,
      });
    const status = mr?.pipeline?.status;
    const pipelineAt = mr?.pipeline?.finished_at ?? mr?.pipeline?.updated_at;
    const isReadyDraft = Boolean(mr?.isDraft) && readyDrafts.has(issue.number);
    if (
      mr?.state === "opened" &&
      (!mr.isDraft || isReadyDraft) &&
      status === "success" &&
      // GitLab's word, not ours: conflicts, a check still running, … block.
      (mr.mergeStatus === "mergeable" || isReadyDraft) &&
      !tree?.actions?.some(action => action.kind === "merge") &&
      !isChangeOpen(tree, mr)
    )
      add(
        "merge",
        isReadyDraft
          ? "pipeline passed, every Work box ticked, still a draft"
          : "ready, pipeline passed",
        pipelineAt,
        () => show(mr.url, tree?.dir),
        { canMarkReady: isReadyDraft },
      );
    const failed = `pipeline:!${mr?.iid}:${mr?.pipeline?.id ?? pipelineAt}`;
    if (mr?.state === "opened" && status === "failed" && !sent.has(failed)) {
      const job = jobs.find(candidate => candidate.status === "failed");
      const what = job ? `job ${job.name} failed` : "failed";
      add(
        "pipeline",
        `!${mr.iid} ${what}${agent ? ": click to tell its agent" : ""}`,
        pipelineAt,
        // An agent blocked at a question takes no line: the job then opens.
        () =>
          (agent &&
            sendOnce(
              tree.dir,
              failed,
              `The pipeline of MR !${mr.iid} failed: ${job ? `job ${job.name}, ${job.url}` : mr.pipeline.web_url}`,
            )) ||
          show(job?.url ?? mr.url, tree?.dir),
      );
    }
    const unpushed =
      tree &&
      unpushedActionOf(issue, mr, sent, {
        send: sendOnce,
        focus: () => focusAgent(tree.dir, agent),
      });
    if (unpushed) add(unpushed.kind, unpushed.text, unpushed.at, unpushed.go);
    if (mr?.state === "merged") {
      // The automatic one does the rest (removeMerged): a row only for one that failed.
      if (tree && removals.get(tree.dir)?.error)
        actions.push({
          key: `clean up:${issue.branch}:remove`,
          kind: "clean up",
          text: removeText(tree.dir, issue.feature),
          at: new Date(mr.mergedAt ?? Date.now()).toISOString(),
          issue,
          go: () => clickRemove(tree.dir),
        });
      const open = (found?.boxes ?? []).filter(box => !box.isTicked).length;
      const boxes = `boxes:!${mr.iid}`;
      if (open && agent && !sent.has(boxes))
        actions.push({
          key: `clean up:${issue.branch}:boxes`,
          kind: "clean up",
          text: `#${found.iid} has ${open} open boxes: click to have its agent tick the done ones`,
          at: new Date(mr.mergedAt ?? Date.now()).toISOString(),
          issue,
          // Blocked at a question, it takes no line: its tab comes to the front.
          go: () =>
            sendOnce(
              tree.dir,
              boxes,
              `MR !${mr.iid} is merged: tick the boxes of #${found.iid} that are done (pnpm wt:tick), say what stays open.`,
            ) || focusAgent(tree.dir, agent),
        });
    }
    if (agent && tree.claude) {
      const question =
        agent.status === "blocked"
          ? (pendingOfFile(tree.claude.transcript) ??
            (firstLine(tree.claude.answer) || "needs you"))
          : agent.status === "working"
            ? null
            : questionOf(tree.claude.answer);
      if (question)
        add("waits", question, tree.claude.at, () =>
          focusAgent(tree.dir, agent),
        );
    }
    return actions;
  });

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

/** The keys of the actions at the last read, null before GitLab first answered. */
let seen = null;
/**
 * The actions in `actions` that weren't there at the last call. The first
 * call only takes note: what the overview finds at its start is not new.
 */
export const newActionsOf = (before, actions) =>
  before ? actions.filter(action => !before.has(action.key)) : [];
/**
 * A herdr notification for each action new since the last read, posted or
 * automatic (`{ key, who, kind, text }`). The overview of an orchestration is
 * its one notifier; notify.mjs leaves its worktrees to it. Only once GitLab
 * answered: before, every action from it would look new.
 */
export const notifyNewActions = (label, actions, hasRemote) => {
  if (!hasRemote || !isInHerdr()) return;
  const fresh = newActionsOf(seen, actions);
  seen = new Set(actions.map(action => action.key));
  for (const action of fresh)
    herdr([
      "notification",
      "show",
      label,
      "--body",
      `${action.who}: ${action.kind} · ${firstLine(action.text.replace(ESCAPES, ""))}`,
      "--sound",
      "request",
    ]);
};
