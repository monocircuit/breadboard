// One snapshot to draw: the local state read, GitLab read without failing, and
// the view that fits the command line.
import path from "node:path";

import { diffOf, filesOf } from "./diff.mjs";
import { developOf, migrationsOf } from "./develop.mjs";
import {
  STORYBOOK_PORT,
  STUDIO_PORT,
  actionsFile,
  actionsOf,
  feedFile,
  handoversOf,
  hasOwnCommits,
  isPortFree,
  mainDir,
  noteOf,
  recordFile,
  recordOf,
  stoppedOf,
  waitOf,
  worktrees,
} from "../worktree.mjs";
import { only, orchestration } from "./args.mjs";
import { retryAnswers } from "./answers.mjs";
import { agentsOf } from "./auto-actions.mjs";
import { agentStatusOf, readClaude } from "./claude.mjs";
import { retryMergeLines } from "./merge.mjs";
import {
  containersOf,
  issuesOf,
  readRemoteOfOrchestration,
} from "./gitlab.mjs";
import { sh } from "./io.mjs";
import { readLocal, readRemote, readRemoteOf } from "./read.mjs";
import { renderOrchestration } from "./render-orchestration.mjs";
import { renderWorktree } from "./render-worktree.mjs";
import { render } from "./render.mjs";
import { screen } from "./screen.mjs";
import { lastTitle, titleHeight } from "./boxes.mjs";

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

/** In the view of one worktree, its containers and its studio too. */
export const readView = async () => {
  const local = await readLocal();
  // An orchestration: its issues, the conversation of each worktree's agent and what it asks for.
  if (orchestration) {
    // Answers that didn't get into an agent's pane yet: again, once it
    // changed state. Only by the live overview: a --once print changes nothing.
    retryAnswers(orchestration, [
      ...local.trees.map(tree => ({
        file: actionsFile(tree.dir),
        to: { dir: tree.dir },
      })),
      { file: recordFile(orchestration), to: { orchestration } },
    ]);
    // The lines after a merge (merge.mjs) that didn't get in either.
    retryMergeLines(local.trees.map(tree => tree.dir));
    const trees = local.trees.map(tree => ({
      ...tree,
      claude: readClaude(tree.dir),
      actions: actionsOf(actionsFile(tree.dir)),
      note: noteOf(feedFile(tree.dir)),
      stopped: stoppedOf(feedFile(tree.dir)),
      // For the merge order (order.mjs): which MRs share files.
      files: filesOf(tree.dir),
      // How far develop moved on: with conflicts, how much there is to merge in.
      behind:
        Number(
          sh(
            "git",
            ["rev-list", "--count", "HEAD..origin/develop"],
            tree.dir,
          ).stdout.trim(),
        ) || 0,
      // What keeps a merged one from wt:remove (clean-up.mjs): `cherry` as wt:remove checks it, counted.
      ownCommits:
        tree.branch === "(detached)"
          ? 0
          : sh("git", ["cherry", "origin/develop", tree.branch], tree.dir)
              .stdout.split("\n")
              .filter(line => line.startsWith("+")).length,
      // What it waits for (wt:wait), for its MR row.
      wait: waitOf(tree.dir),
    }));
    // The orchestrator's order (wt:plan) and its own actions, from the record.
    const file = recordFile(orchestration);
    return {
      ...local,
      trees,
      issues: issuesOf(orchestration, trees),
      plan: recordOf(orchestration)?.plan ?? null,
      // Each agent's name and what it worked on before (#217, worktree/workers.mjs).
      workers: recordOf(orchestration)?.workers ?? {},
      // How many agents work at once (wt:orchestrate --cap); workers.mjs has the default.
      cap: recordOf(orchestration)?.cap,
      // The handovers the orchestrator announced (#227, worktree/handovers.mjs).
      handovers: handoversOf(recordOf(orchestration)),
      ownActions: actionsOf(file),
      recordFile: file,
      // For the automatic actions (auto-actions.mjs) and FEED: what herdr says each agent does.
      agents: agentsOf(),
    };
  }
  const tree = local.trees[0];
  if (!only || !tree) return local;
  const offset = tree.hasOwnPort ? tree.offset : 0;
  const isMain = tree.dir === mainDir();
  const claude = isMain ? null : readClaude(tree.dir);
  return {
    ...local,
    // What wt:remove would refuse stays: changes, commits develop doesn't have.
    isDone:
      !isMain &&
      tree.branch !== "(detached)" &&
      tree.changed === 0 &&
      !hasOwnCommits(tree.branch, tree.dir),
    claude,
    // Not cached with the transcript: herdr knows it before the transcript does.
    agentStatus: claude ? agentStatusOf(claude.id) : null,
    // Why it isn't done yet, for the greyed out button: `cherry` as wt:remove checks it.
    ownCommits: isMain
      ? 0
      : sh("git", ["cherry", "origin/develop", tree.branch], tree.dir)
          .stdout.split("\n")
          .filter(line => line.startsWith("+")).length,
    containers: tree.hasEnv && tree.hasOwnPort ? containersOf(tree.dir) : [],
    // The main checkout is develop: nothing to compare with, and no work of its own.
    diff: isMain
      ? null
      : diffOf(
          tree.dir,
          worktrees()
            .slice(1)
            .filter(other => other.dir !== tree.dir)
            .map(other => ({
              dir: other.dir,
              name: path
                .basename(other.dir)
                .replace(`${path.basename(mainDir())}-`, ""),
            })),
        ),
    develop: isMain ? null : developOf(tree.dir),
    migrations: tree.hasEnv ? migrationsOf(tree.dir) : null,
    isAppUp: !(await isPortFree(3000 + offset)),
    isStudioUp: !(await isPortFree(STUDIO_PORT + offset)),
    isStorybookUp: !(await isPortFree(STORYBOOK_PORT + offset)),
  };
};
export const view = (...args) => {
  screen.clicks = [];
  screen.upTarget = null;
  screen.sticky = titleHeight();
  lastTitle.isSet = false;
  if (orchestration) return renderOrchestration(...args);
  return only ? renderWorktree(...args) : render(...args);
};
export const readRemoteSafely = () =>
  (orchestration
    ? readRemoteOfOrchestration(orchestration)
    : only
      ? readRemoteOf(worktrees().find(w => w.dir === only)?.branch, only)
      : readRemote()
  ).then(
    remote => ({ remote, error: null }),
    error => ({ remote: null, error: error.stderr?.trim() || error.message }),
  );
