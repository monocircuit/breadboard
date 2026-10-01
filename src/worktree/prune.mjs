// What is done: worktrees whose branch is merged and gone, issues of merged
// work nobody came back to, spaces of removed worktrees.
import fs from "node:fs";
import path from "node:path";

import { glabApi } from "../lib/glab.mjs";
import { isInHerdr, panesOf, spaces } from "../lib/herdr.mjs";
import { git } from "../lib/proc.mjs";
import {
  leftIn,
  leftText,
  runningClaudes,
  sessionFile,
} from "./conversations.mjs";
import { boxesOf, issueOfBranch } from "./issues.mjs";
import { releaseSpace } from "./layout.mjs";
import { pruneOrchestrators } from "./origins.mjs";
import { spaceLabel } from "./spaces.mjs";
import { dirOf, mainDir, worktrees } from "./worktrees.mjs";

/** `platform-40` is not inside `platform-4`, and the main checkout is not inside its worktrees. */
export const isInside = (here, dir) =>
  here === dir || here.startsWith(`${dir}${path.sep}`);
/** Commits of `branch` that `origin/develop` doesn't have, by patch: a merge brings them in. `cwd`: a checkout of its repo. */
export const hasOwnCommits = (branch, cwd) =>
  git(["cherry", "origin/develop", branch], cwd)
    .out.split("\n")
    .some(line => line.startsWith("+"));
/**
 * The files of a leftover folder that may be someone's work. Not counted: the
 * binding, symlinks (pnpm's point at packages that are gone, #130), what
 * .gitignore ignores (node_modules, .next, build info; not descended into) and
 * a file whose content git already has as a blob, i.e. a tracked file a
 * removal left behind unchanged.
 */
export const ownFiles = dir => {
  const files = [];
  const walk = relative => {
    const entries = fs.readdirSync(path.join(dir, relative), {
      withFileTypes: true,
    });
    // A trailing slash, so `dir/` patterns match a folder the main checkout doesn't have.
    const paths = entries.map(
      entry =>
        path.join(relative, entry.name) + (entry.isDirectory() ? "/" : ""),
    );
    const input = paths.map(name => `${name}\n`).join("");
    const ignored = new Set(
      git(
        ["check-ignore", "--no-index", "--stdin"],
        mainDir(),
        input,
      ).out.split("\n"),
    );
    entries.forEach((entry, i) => {
      const name = paths[i];
      if (ignored.has(name)) return;
      if (entry.isDirectory()) walk(name);
      else if (entry.isFile() && path.join(dir, name) !== sessionFile(dir))
        files.push(name);
    });
  };
  walk("");
  if (files.length === 0) return files;

  const input = files.map(name => `${path.join(dir, name)}\n`).join("");
  const hashes = git(
    ["hash-object", "--no-filters", "--stdin-paths"],
    mainDir(),
    input,
  ).out.split("\n");
  const missing = new Set(
    git(
      ["cat-file", "--batch-check"],
      mainDir(),
      hashes.map(hash => `${hash}\n`).join(""),
    )
      .out.split("\n")
      .filter(line => line.endsWith(" missing"))
      .map(line => line.split(" ")[0]),
  );
  return files.filter((_, i) => !hashes[i] || missing.has(hashes[i]));
};
/**
 * What `wt:prune` would close, each entry with the reason it stays. `why: null`
 * is the actionable case: GitLab is done with it.
 *
 * `wt:new` shows the count of those, so the sweep is offered where someone
 * starts the next thing — an agent is long gone when the merge happens.
 */
export const prunable = () => {
  const here = path.resolve(process.cwd());
  const claudes = runningClaudes();
  const trees = worktrees()
    .slice(1)
    .map(({ dir, branch }) => ({ dir, branch, left: leftIn(dir, claudes) }))
    .map(({ dir, branch, left }) => ({
      name: path.basename(dir).replace(`${path.basename(mainDir())}-`, ""),
      dir,
      why: !branch
        ? "detached HEAD"
        : git(["rev-parse", "--verify", "--quiet", `origin/${branch}`], dir).ok
          ? "its branch is still on origin"
          : hasOwnCommits(branch, dir)
            ? "commits that are not in develop"
            : git(["status", "--porcelain"], dir).out
              ? "uncommitted changes"
              : isInside(here, dir)
                ? "you are working in it"
                : left.length
                  ? `${leftText(left)} still stands in it, not in its next worktree yet`
                  : null,
    }));

  const branches = git([
    "for-each-ref",
    "--format=%(refname:short)|%(upstream:track)|%(worktreepath)",
    "refs/heads",
  ])
    .out.split("\n")
    .map(line => line.split("|"))
    .filter(
      ([name, track, worktree]) =>
        name &&
        !worktree &&
        track === "[gone]" &&
        name !== "develop" &&
        name !== "main",
    )
    .map(([name]) => ({
      name,
      why: hasOwnCommits(name, mainDir())
        ? "gone from origin but not in develop — the only copy of that work"
        : null,
    }));

  // A `platform-*` folder git no longer knows: a removal that stopped halfway, or an
  // overview that wrote the binding back after git deleted the rest. Only the
  // binding in it: nothing of anyone's work, it goes.
  const known = new Set(worktrees().map(({ dir }) => dir));
  const prefix = `${path.basename(mainDir())}-`;
  const folders = fs
    .readdirSync(path.dirname(mainDir()), { withFileTypes: true })
    .filter(entry => entry.isDirectory() && entry.name.startsWith(prefix))
    .map(entry => path.join(path.dirname(mainDir()), entry.name))
    .filter(dir => !known.has(dir))
    .map(dir => {
      const files = ownFiles(dir);
      return {
        name: path.basename(dir).slice(prefix.length),
        dir,
        why: files.length
          ? `not a worktree, but ${files.length} file(s) git doesn't know in it`
          : null,
      };
    });

  return {
    trees,
    branches,
    folders,
    count: [...trees, ...branches, ...folders].filter(entry => !entry.why)
      .length,
  };
};
/** `Closes #12`, `Refs #12`, `fixes #12`: the issues a merge request description says it works on. */
const REFERENCE =
  /\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?|refs?)\s+#(\d+)/gi;
/**
 * The issues a merged MR touched — the number of its branch, the `Closes #n` /
 * `Refs #n` of its description — that are still open or have a box left open
 * and no comment since the last of those merges, with their ticked/total
 * boxes. System notes ("mentioned in …",
 * "closed via …") don't count: GitLab writes them, nobody caught anything up.
 *
 * The merge comes from a human, long after the agent's session ended, so
 * nobody is there to write the closing comment. `wt:new` counts these next to
 * the stale worktrees, where someone starts the next thing.
 */
export const staleIssues = async () => {
  const mrs = await glabApi(
    "projects/:fullpath/merge_requests?state=merged&order_by=merged_at&sort=desc&per_page=100",
  );
  /** issue → the MR that merged last of those touching it */
  const lastMerge = new Map();
  for (const mr of mrs) {
    const numbers = new Set([
      issueOfBranch(mr.source_branch),
      ...[...(mr.description ?? "").matchAll(REFERENCE)].map(m => m[1]),
    ]);
    for (const number of numbers) {
      if (!number) continue;
      const known = lastMerge.get(number);
      if (!known || known.merged_at < mr.merged_at) lastMerge.set(number, mr);
    }
  }
  if (lastMerge.size === 0) return [];

  const numbers = [...lastMerge.keys()];
  const issues = [];
  for (let at = 0; at < numbers.length; at += 50) {
    const iids = numbers
      .slice(at, at + 50)
      .map(n => `iids[]=${n}`)
      .join("&");
    issues.push(
      ...(await glabApi(`projects/:fullpath/issues?${iids}&per_page=50`)),
    );
  }

  // A closed issue with every box ticked says all there is: the merge closed
  // it, the boxes show what it did. Open ones and ones with a box left open
  // need the word on what's done and what's open.
  const open = issues
    .map(issue => ({
      issue,
      mr: lastMerge.get(String(issue.iid)),
      boxes: boxesOf(issue.description ?? ""),
    }))
    .filter(
      ({ issue, boxes }) =>
        issue.state === "opened" || boxes.some(box => !box.isTicked),
    );

  const stale = [];
  // Ten at a time: one request per issue, and GitLab does not like a burst of a hundred.
  for (let at = 0; at < open.length; at += 10) {
    const found = await Promise.all(
      open.slice(at, at + 10).map(async entry => {
        const notes = await glabApi(
          `projects/:fullpath/issues/${entry.issue.iid}/notes?sort=desc&order_by=created_at&per_page=100`,
        );
        return notes.some(
          note => !note.system && note.created_at > entry.mr.merged_at,
        )
          ? null
          : entry;
      }),
    );
    stale.push(...found.filter(Boolean));
  }
  return stale.sort((a, b) => b.mr.merged_at.localeCompare(a.mr.merged_at));
};
/**
 * The spaces of worktrees that are gone close, whoever removed them: a
 * `git worktree remove` by hand, wt:remove outside herdr, one that stopped
 * halfway. A space counts only when its label names no worktree and every
 * pane in it stands in that worktree's directory, now deleted — a space of
 * someone's own or of a live checkout is never touched. It goes like after
 * wt:remove: a tab an agent still runs in stays until the agent ends. So does
 * the space of an orchestration whose last worktree is gone, once nothing runs
 * in it (pruneOrchestrators). The overview of the main space calls this on
 * every read. Returns what closed.
 */
export const pruneSpaces = () => {
  if (!isInHerdr()) return [];
  const live = new Set(worktrees().map(({ dir }) => spaceLabel(dir)));
  const closed = [];
  for (const workspace of spaces()) {
    if (live.has(workspace.label)) continue;
    const dir = dirOf(workspace.label);
    if (fs.existsSync(dir)) continue;
    const panes = panesOf(workspace.workspace_id);
    const isOrphan =
      panes.length > 0 &&
      panes.every(
        p =>
          p.cwd &&
          (p.cwd === dir ||
            p.cwd.startsWith(`${dir}/`) ||
            p.cwd.startsWith(`${dir} `)),
      );
    if (!isOrphan) continue;
    const released = releaseSpace(workspace.label);
    if (released) closed.push(released);
  }
  closed.push(...pruneOrchestrators());
  return closed;
};
