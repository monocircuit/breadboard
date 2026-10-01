// What an orchestration changed in the code (#256): the files of each of its
// MRs, merged and open, and all of them together, each with the commits to
// diff between. Merged: the MR's merge commit on develop against its first
// parent; open: the branch's head against where it left develop (what
// GitLab's diff shows). So only what its MRs brought, not what develop got
// meanwhile. `pnpm wt:orchestrate <name> --changes` prints it as JSON for the
// tab `changes` of its space (nvim/orchestration.lua), which diffs and shows
// the files itself.
import { glabApi } from "../lib/glab.mjs";
import { git } from "../lib/proc.mjs";
import { recordOf, titleOf } from "./origins.mjs";
import { mainDir } from "./worktrees.mjs";

/** `git <args>` in `cwd`: its output, null when git fails. */
const read = (args, cwd) => {
  const done = git(["-c", "core.quotePath=false", ...args], cwd);
  return done.ok ? done.out : null;
};
/** The commit `ref` names, null when it isn't there. */
const commitOf = (ref, cwd) =>
  read(["rev-parse", "--verify", "--quiet", `${ref}^{commit}`], cwd)?.trim() ||
  null;

/** `A`, `M` or `D` per path between two commits; renames as a deletion and a new file. */
const statusesOf = (base, head, cwd) =>
  new Map(
    (read(["diff", "--no-renames", "--name-status", base, head], cwd) ?? "")
      .split("\n")
      .filter(Boolean)
      .map(line => {
        const [status, file] = line.split("\t");
        return [file, status[0]];
      }),
  );
/** `{ added, removed }` per path between two commits; null counts for a binary file. */
const countsOf = (base, head, cwd) =>
  new Map(
    (read(["diff", "--no-renames", "--numstat", base, head], cwd) ?? "")
      .split("\n")
      .filter(Boolean)
      .map(line => {
        const [added, removed, file] = line.split("\t");
        return [
          file,
          added === "-"
            ? { added: null, removed: null }
            : { added: Number(added), removed: Number(removed) },
        ];
      }),
  );
/** The files between two commits, by path: `{ path, status: new|changed|deleted, added, removed }`. */
export const filesBetween = (base, head, cwd) => {
  const counts = countsOf(base, head, cwd);
  return [...statusesOf(base, head, cwd)]
    .map(([file, status]) => ({
      path: file,
      status: status === "A" ? "new" : status === "D" ? "deleted" : "changed",
      ...(counts.get(file) ?? { added: null, removed: null }),
    }))
    .toSorted((a, b) => a.path.localeCompare(b.path));
};

/**
 * The commits an MR's change lies between: `{ base, head }`, null when git
 * lacks them (not fetched yet, the branch gone). Merged: its merge (or
 * squash) commit against its first parent; open: the head of its branch
 * against where it left develop.
 */
export const rangeOf = (mr, cwd) => {
  if (mr.state === "merged") {
    const sha = mr.merge_commit_sha ?? mr.squash_commit_sha;
    const head = sha && commitOf(sha, cwd);
    const base = head && commitOf(`${head}^1`, cwd);
    return base ? { base, head } : null;
  }
  const head =
    (mr.sha && commitOf(mr.sha, cwd)) ??
    commitOf(`origin/${mr.source_branch}`, cwd);
  const develop = commitOf(`origin/${mr.target_branch ?? "develop"}`, cwd);
  const base = head && develop && read(["merge-base", develop, head], cwd);
  return base ? { base: base.trim(), head } : null;
};

/** The MRs of `branch`, merged and open (closed ones changed nothing); none when GitLab doesn't answer. */
const mrsOf = async (branch, gitlab) => {
  const found = await gitlab(
    `projects/:fullpath/merge_requests?source_branch=${encodeURIComponent(branch)}`,
  ).catch(() => []);
  return found.filter(mr => mr.state === "merged" || mr.state === "opened");
};
/** Merged ones first, by merge time, then the open ones by number: the order their changes come in. */
const byOrder = (a, b) =>
  (a.mergedAt ? 0 : 1) - (b.mergedAt ? 0 : 1) ||
  String(a.mergedAt ?? "").localeCompare(String(b.mergedAt ?? "")) ||
  a.iid - b.iid;

/**
 * All files of `groups` together, by path: the first group's base, the last
 * one's head (so a diff shows the file before the orchestration against what
 * it has made of it), the counts summed, `mrs` the iids that touched it. New
 * when the first one added it, deleted when the last one removed it; one
 * that came and went is left out.
 */
export const combine = groups => {
  const byPath = new Map();
  for (const group of groups)
    for (const file of group.files) {
      const known = byPath.get(file.path);
      if (!known) {
        byPath.set(file.path, {
          ...file,
          base: group.base,
          head: group.head,
          mrs: [group.iid],
          first: file.status,
        });
        continue;
      }
      known.head = group.head;
      known.mrs.push(group.iid);
      known.last = file.status;
      known.added =
        known.added === null || file.added === null
          ? null
          : known.added + file.added;
      known.removed =
        known.removed === null || file.removed === null
          ? null
          : known.removed + file.removed;
    }
  return [...byPath.values()]
    .map(({ first, last = first, ...file }) => {
      const didExist = first !== "new";
      const isThere = last !== "deleted";
      if (!didExist && !isThere) return null;
      return {
        ...file,
        status: !didExist ? "new" : !isThere ? "deleted" : "changed",
      };
    })
    .filter(Boolean)
    .toSorted((a, b) => a.path.localeCompare(b.path));
};

/**
 * What the orchestration `name` changed, from its record, GitLab and git in
 * `cwd`: `{ name, title, root, groups, combined, missing }`. A group is one
 * MR, `{ iid, url, title, state, issue, mergedAt, base, head, files }`;
 * `missing` the MRs git lacks the commits of. Null without a record.
 * `gitlab`: a GET of the API, parsed; `shouldFetch`: origin fetched first,
 * so a push or a merge just made is in.
 */
export const readChangeSet = async (
  name,
  { gitlab = glabApi, cwd = mainDir(), shouldFetch = true } = {},
) => {
  const record = recordOf(name);
  if (!record) return null;
  if (shouldFetch) git(["fetch", "--quiet", "--prune", "origin"], cwd);
  const branches = [
    ...new Set(record.issues.map(issue => issue.branch).filter(Boolean)),
  ];
  const found = await Promise.all(
    branches.map(async branch => {
      const issue = record.issues.find(entry => entry.branch === branch);
      return (await mrsOf(branch, gitlab)).map(mr => ({ mr, issue }));
    }),
  );
  const groups = [];
  const missing = [];
  for (const { mr, issue } of found.flat()) {
    const range = rangeOf(mr, cwd);
    if (!range) {
      missing.push(mr.iid);
      continue;
    }
    groups.push({
      iid: mr.iid,
      url: mr.web_url,
      title: mr.title.replace(/^Draft:\s*/, ""),
      state: mr.state,
      issue: issue?.number ?? null,
      mergedAt: mr.merged_at ?? null,
      ...range,
      files: filesBetween(range.base, range.head, cwd),
    });
  }
  groups.sort(byOrder);
  return {
    name,
    title: titleOf(name),
    root: cwd,
    groups,
    combined: combine(groups),
    missing: missing.toSorted((a, b) => a - b),
  };
};
