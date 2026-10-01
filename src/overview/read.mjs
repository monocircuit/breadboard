// What there is: local state (worktrees, stacks, apps) every few seconds,
// GitLab (MRs, pipelines, issues) every half minute.
import path from "node:path";

import {
  checkOf,
  envFileOf,
  boxesOf,
  isInside,
  isOverallBehind,
  isPortFree,
  mainDir,
  offsetOf,
  originOf,
  readEnv,
  worktrees,
  writeOverall,
} from "../worktree.mjs";
import { here, only, orchestration } from "./args.mjs";
import { pathsOf } from "./workers.mjs";
import { composeProjects } from "./claude.mjs";
import { deployOf, jobsOf } from "./gitlab.mjs";
import { needsOf } from "./merge-order.mjs";
import { exec, glab, sh } from "./io.mjs";
import { waitingFor } from "./waiting.mjs";

export const readLocal = async () => {
  const main = mainDir();
  const projects = composeProjects();
  const trees = [];
  for (const [index, { dir, branch }] of worktrees().entries()) {
    if (only && dir !== only) continue;
    if (orchestration && originOf(dir)?.name !== orchestration) continue;
    const env = readEnv(envFileOf(dir));
    const offset = index === 0 ? 0 : offsetOf(dir);
    const hasOwnPort = index === 0 || offset > 0;
    const project = env?.COMPOSE_PROJECT_NAME ?? "vita";
    trees.push({
      dir,
      name:
        index === 0
          ? path.basename(main)
          : path.basename(dir).replace(`${path.basename(main)}-`, ""),
      branch: branch ?? "(detached)",
      // The orchestration that started it (wt:orchestrate), or null.
      origin: index === 0 ? null : originOf(dir),
      isHere: isInside(here, dir),
      hasEnv: Boolean(env),
      offset,
      hasOwnPort,
      isUp: Boolean(env) && hasOwnPort && !(await isPortFree(3000 + offset)),
      project,
      env,
      stack: projects?.get(project) ?? null,
      changed: sh("git", ["status", "--porcelain"], dir)
        .stdout.split("\n")
        .filter(Boolean).length,
      ahead:
        Number(
          sh(
            "git",
            ["rev-list", "--count", "origin/develop..HEAD"],
            dir,
          ).stdout.trim(),
        ) || 0,
      // Whether wt:check passed on its HEAD (#298, worktree/check.mjs).
      checked: checkOf(dir),
    });
  }
  const known = new Set(trees.map(tree => tree.project));
  const orphans = projects
    ? [...projects].filter(
        ([name]) => name.startsWith("vita") && !known.has(name),
      )
    : [];
  return { trees, orphans, hasDocker: Boolean(projects) };
};
/** The head of develop the changelog of the repo was last started for: one that fails is not tried again and again. */
let overallTried = null;
/**
 * The changelog of the repo (#261) rewritten in the background when the
 * head of develop moved since it was written: a merge, by the overview's
 * click or anywhere else.
 */
const keepOverall = develop => {
  if (!develop || overallTried === develop || !isOverallBehind(develop)) return;
  overallTried = develop;
  writeOverall({ develop }).catch(() => (overallTried = null));
};
export const readRemote = async () => {
  const [mrs, pipelines, issues, refs] = await Promise.all([
    glab("projects/:fullpath/merge_requests?state=opened&per_page=50"),
    glab("projects/:fullpath/pipelines?per_page=100"),
    glab(
      "projects/:fullpath/issues?state=opened&per_page=100&order_by=updated_at",
    ),
    exec("git", ["ls-remote", "--heads", "origin"], { cwd: mainDir() }).then(
      ({ stdout }) =>
        stdout
          .split("\n")
          .map(line => line.split("\trefs/heads/"))
          .filter(([, branch]) => branch),
    ),
  ]);
  const heads = refs.map(([, branch]) => branch);
  keepOverall(refs.find(([, branch]) => branch === "develop")?.[0]);
  // Newest first: the first pipeline of a ref is its state.
  const pipelineOf = new Map();
  for (const pipeline of pipelines)
    if (!pipelineOf.has(pipeline.ref)) pipelineOf.set(pipeline.ref, pipeline);
  const mrList = mrs.map(mr => ({
    iid: mr.iid,
    title: mr.title.replace(/^Draft:\s*/, ""),
    branch: mr.source_branch,
    isDraft: mr.draft,
    who: mr.assignees?.map(a => a.username).join(",") || mr.author?.username,
    url: mr.web_url,
    pipeline:
      pipelineOf.get(`refs/merge-requests/${mr.iid}/head`) ??
      pipelineOf.get(mr.source_branch) ??
      null,
  }));
  const claimOf = number =>
    heads.find(branch => branch.split("/").at(-1).startsWith(`${number}-`));
  const issueList = issues.map(issue => {
    const branch = claimOf(issue.iid);
    return {
      iid: issue.iid,
      title: issue.title,
      url: issue.web_url,
      labels: issue.labels,
      who: issue.assignees?.map(a => a.username).join(","),
      branch,
      mr: mrList.find(mr => mr.branch === branch)?.iid,
    };
  });
  // Claimed ones first: that is the work in flight.
  issueList.sort(
    (a, b) => Number(Boolean(b.branch)) - Number(Boolean(a.branch)),
  );
  return {
    mrs: mrList,
    issues: issueList,
    develop: pipelineOf.get("develop") ?? null,
    at: new Date(),
  };
};
/**
 * An MR as the views use it, from GitLab's (the list's or the single one's),
 * with `pipeline` its head pipeline (the single one has it as
 * `head_pipeline`). `deploy` is set by the reader for a merged one.
 */
export const mrOf = (found, pipeline = found.head_pipeline ?? null) => ({
  iid: found.iid,
  title: found.title.replace(/^Draft:\s*/, ""),
  isDraft: found.draft,
  state: found.state,
  mergedAt: found.merged_at ?? null,
  hasConflicts: Boolean(found.has_conflicts),
  // GitLab's word on whether it can be merged: only `mergeable` can.
  mergeStatus: found.detailed_merge_status ?? null,
  // Whether GitLab's conflicts are settled (resolve.mjs): `checking` after a push.
  mergeCheck: found.merge_status ?? null,
  // The head GitLab has: one older than the pushed commit answers for older commits.
  sha: found.sha ?? null,
  url: found.web_url,
  pipeline,
  // Merged: the develop pipeline of the merge commit, what a try on the test instance waits for (try-state.mjs).
  deploy: null,
});
/** The commit a merged MR put on develop. */
export const mergeShaOf = found =>
  found?.merge_commit_sha ?? found?.squash_commit_sha ?? null;
/** An issue as the views use it, from GitLab's with its links. */
export const issueOf = (issue, links = []) => ({
  iid: issue.iid,
  title: issue.title,
  url: issue.web_url,
  state: issue.state,
  boxes: boxesOf(issue.description ?? ""),
  // The issues it names as prerequisites, or is blocked by.
  needs: [
    ...new Set([
      ...needsOf(issue.description),
      ...links
        .filter(link => link.link_type === "is_blocked_by")
        .map(link => link.iid),
    ]),
  ],
  // The files it names: the agent that knows them is suggested for it (workers.mjs).
  paths: pathsOf(issue.description),
});
/**
 * One worktree's part of GitLab: its MR (open, else the latest) with the
 * pipeline's jobs, its issue, and with `dir` the open MRs it waits for
 * (waiting.mjs), with their pipelines.
 */
export const readRemoteOf = async (branch, dir = null) => {
  const number = branch?.match(/\/(\d+)-/)?.[1];
  const [mrs, issue, links, open, pipelines] = await Promise.all([
    // Every state: after the merge the branch still has its MR, just not an open one.
    branch
      ? glab(
          `projects/:fullpath/merge_requests?source_branch=${encodeURIComponent(branch)}&order_by=updated_at`,
        )
      : [],
    number
      ? glab(`projects/:fullpath/issues/${number}`).catch(() => null)
      : null,
    // GitLab's "blocked by" links, for the merge order (order.mjs).
    number
      ? glab(`projects/:fullpath/issues/${number}/links`).catch(() => [])
      : [],
    dir
      ? glab("projects/:fullpath/merge_requests?state=opened&per_page=100")
      : [],
    dir ? glab("projects/:fullpath/pipelines?per_page=100") : [],
  ]);
  const latest = mrs.find(mr => mr.state === "opened") ?? mrs[0];
  const found = latest
    ? await glab(`projects/:fullpath/merge_requests/${latest.iid}`)
    : null;
  const mr = found && mrOf(found);
  const mergeSha = mergeShaOf(found);
  if (mr?.state === "merged" && mergeSha)
    mr.deploy = await deployOf(mergeSha).catch(() => null);
  // The jobs are extra: without them issue and MR still show.
  const jobs = mr?.pipeline ? await jobsOf(mr.pipeline).catch(() => []) : [];
  const pipelineOf = new Map();
  for (const pipeline of pipelines)
    if (!pipelineOf.has(pipeline.ref)) pipelineOf.set(pipeline.ref, pipeline);
  const waiting = dir
    ? waitingFor(
        dir,
        branch,
        open.map(candidate => ({
          iid: candidate.iid,
          title: candidate.title.replace(/^Draft:\s*/, ""),
          branch: candidate.source_branch,
          isDraft: candidate.draft,
          state: candidate.state,
          url: candidate.web_url,
          pipeline:
            pipelineOf.get(`refs/merge-requests/${candidate.iid}/head`) ??
            pipelineOf.get(candidate.source_branch) ??
            null,
        })),
      )
    : [];
  return {
    mr,
    jobs,
    waiting,
    issue: issue && issueOf(issue, links),
    at: new Date(),
  };
};
