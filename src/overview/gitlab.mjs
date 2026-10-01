// GitLab beyond the MR list: pipeline jobs with the end of a failed log,
// branches, the issues of an orchestration; and the containers of the stacks.
//
// An orchestration's read (#305) asks GitLab in batches and only for what
// can still change: the open MRs in one list, the issues in one list per
// 100, the pipelines in one; what can't change any more comes from the cache
// (gitlab-cache.mjs). A full read (the first, `r`, after a merge) also looks
// again at what moves slowly: the MRs updated lately, branches without an MR,
// the links of the open issues.
import {
  compose,
  handoversOf,
  isChangelogBehind,
  originOf,
  recordOf,
  waitOf,
  worktrees,
  writeChangelog,
} from "../worktree.mjs";
import { gitlabCache } from "./gitlab-cache.mjs";
import { glab, glabText, phase } from "./io.mjs";
import { issueOf, mergeShaOf, mrOf } from "./read.mjs";
import { deployStateOf } from "./try-state.mjs";
import { deliverWaits } from "./waits.mjs";

/** How many lines of a failed job's log the view shows. */
const LOG_LINES = 6;
/** GitLab's section markers and colours in a job log. */
const LOG_NOISE =
  /\x1b\[[0-9;]*[A-Za-z]|section_(start|end):\d+:[\w.-]+(\[[^\]]*\])?\r?/g;
/** The time and stream GitLab puts in front of each line: `2026-09-28T13:02:20.886478Z 01E+`. */
const LOG_STAMP = /^\d{4}-\d\d-\d\dT[\d:.]+Z \d\d[OE]\+? ?/;
/** What the runner writes after the error: not what went wrong. */
const RUNNER_TAIL =
  /^(Cleaning up |Uploading artifacts|Running after_script|Saving cache|ERROR: Job failed|WARNING: )/;
/** A pipeline that no longer runs: its jobs are final (a retry changes its `updated_at`). */
const FINISHED = new Set(["success", "failed", "canceled", "skipped"]);
/** iids per batched call: GitLab's page. */
const BATCH = 100;

/**
 * How a read talks to GitLab: `json` and `text` GETs (lib/glab.mjs), the
 * `cache` of final things; a test passes its own.
 */
let shared = null;
const defaults = () =>
  (shared ??= { json: glab, text: glabText, cache: gitlabCache() });

/** A failed job's log doesn't change any more: job id → its last lines. */
const logs = new Map();
const logOf = async (job, { text }) => {
  if (!logs.has(job.id)) {
    const lines = (await text(`projects/:fullpath/jobs/${job.id}/trace`))
      .replace(LOG_NOISE, "")
      .split("\n")
      .map(line => line.split("\r").at(-1).replace(LOG_STAMP, "").trimEnd())
      .filter(Boolean);
    const end = lines.findLastIndex(line => !RUNNER_TAIL.test(line));
    logs.set(job.id, lines.slice(Math.max(0, end + 1 - LOG_LINES), end + 1));
  }
  return logs.get(job.id);
};
/**
 * The jobs of `pipeline` (GitLab's object, or its id) in the order they
 * ran, the last try of each; a failed one with the last lines of its log,
 * where the error usually is. A finished pipeline's are read once.
 */
export const jobsOf = async (pipeline, context = defaults()) => {
  const id = typeof pipeline === "object" ? pipeline.id : pipeline;
  const isFinished =
    typeof pipeline === "object" && FINISHED.has(pipeline.status);
  const key = `${id}@${pipeline?.updated_at ?? ""}`;
  if (isFinished && context.cache.get("jobs", key))
    return context.cache.get("jobs", key);
  const all = await context.json(
    `projects/:fullpath/pipelines/${id}/jobs?per_page=100&include_retried=false`,
  );
  const jobs = all
    .toSorted((a, b) => a.id - b.id)
    .map(job => ({
      id: job.id,
      name: job.name,
      status: job.status,
      url: job.web_url,
      finishedAt: job.finished_at ?? null,
      log: null,
    }));
  await Promise.all(
    jobs
      .filter(job => job.status === "failed")
      .map(async job => {
        job.log = await logOf(job, context).catch(() => null);
      }),
  );
  if (isFinished && jobs.every(job => job.status !== "failed" || job.log))
    context.cache.set("jobs", key, jobs);
  return jobs;
};
/**
 * The develop pipeline of the merge commit `sha` with its jobs, what a try
 * on the test instance waits for (try-state.mjs): `{ pipeline, jobs }`,
 * `pipeline` null while GitLab has none for it yet. Read once it ended
 * (try-state.mjs `isFinal`).
 */
export const deployOf = async (sha, context = defaults()) => {
  const cached = context.cache.get("deploys", sha);
  if (cached) return cached;
  const [found] = await context.json(
    `projects/:fullpath/pipelines?ref=develop&sha=${sha}&per_page=1`,
  );
  const deploy = {
    pipeline: found
      ? {
          id: found.id,
          status: found.status,
          url: found.web_url,
          createdAt: found.created_at,
        }
      : null,
    jobs: found ? await jobsOf(found, context) : [],
  };
  if (deployStateOf(deploy).isFinal) context.cache.set("deploys", sha, deploy);
  return deploy;
};
/**
 * develop's last finished pipeline, `{ id, status, url, jobs }`, its jobs
 * only when it failed: a job that fails there too is not an MR's doing
 * (auto-actions.mjs `pipelineLineOf`). Null without one or when GitLab fails.
 */
const developPipelineOf = async (context = defaults()) => {
  const [found] = await context.json(
    "projects/:fullpath/pipelines?ref=develop&scope=finished&per_page=1",
  );
  if (!found) return null;
  return {
    id: found.id,
    status: found.status,
    url: found.web_url,
    jobs: found.status === "failed" ? await jobsOf(found, context) : [],
  };
};

/** In chunks of `size`. */
const chunks = (list, size) =>
  Array.from({ length: Math.ceil(list.length / size) }, (_, i) =>
    list.slice(i * size, (i + 1) * size),
  );
/** The first of each `key`, by it: GitLab's lists come newest first. */
const firstBy = (list, key) => {
  const found = new Map();
  for (const item of list)
    if (!found.has(key(item))) found.set(key(item), item);
  return found;
};
const numberOf = branch => Number(branch?.match(/\/(\d+)-/)?.[1]) || null;

/**
 * What a read of an orchestration keeps between reads, for the fast ones:
 * the branches that had no MR, the links of the open issues.
 */
export const readMemory = () => ({ noMr: new Set(), links: new Map() });
const memory = readMemory();

/**
 * Each branch's MR (`mr`, as readRemoteOf has it) with its pipeline's jobs
 * (`jobs`), by branch: open from the list, final from the cache, the rest
 * asked.
 */
const readMrs = async (branches, { isFull, context, kept }) => {
  const { json, cache } = context;
  phase("merge requests");
  const [opened, recent, pipelines] = await Promise.all([
    json("projects/:fullpath/merge_requests?state=opened&per_page=100"),
    // Merged or closed lately: most of the new ones, without a call each.
    isFull
      ? json(
          "projects/:fullpath/merge_requests?state=all&per_page=100&order_by=updated_at",
        )
      : [],
    json("projects/:fullpath/pipelines?per_page=100"),
  ]);
  const openOf = firstBy(opened, mr => mr.source_branch);
  const recentOf = firstBy(recent, mr => mr.source_branch);
  // Newest first: the first pipeline of a commit is its head pipeline.
  const pipelineOf = firstBy(pipelines, pipeline => pipeline.sha);

  // Open, else known as final, else from the lists, else asked: a branch
  // that had none is asked again on a full read only.
  const lookup = async branch => {
    const cached = cache.get("mrs", cache.get("branches", branch));
    if (openOf.has(branch)) return { found: openOf.get(branch) };
    if (cached) return { final: cached };
    if (recentOf.has(branch)) return { found: recentOf.get(branch) };
    if (!isFull && kept.noMr.has(branch)) return {};
    const mrs = await json(
      `projects/:fullpath/merge_requests?source_branch=${encodeURIComponent(branch)}&order_by=updated_at`,
    );
    const found = mrs.find(mr => mr.state === "opened") ?? mrs[0];
    if (found) kept.noMr.delete(branch);
    else kept.noMr.add(branch);
    return { found };
  };
  // An open one's pipeline from the list, by its head commit; one not in it
  // (older than the last 100) and a newly final one from its own read.
  const settle = async (branch, found) => {
    const listed = found.state === "opened" && pipelineOf.get(found.sha);
    if (listed) return { mr: mrOf(found, listed) };
    phase("pipelines");
    const single = await json(`projects/:fullpath/merge_requests/${found.iid}`);
    const mr = mrOf(single);
    if (mr.state !== "merged" && mr.state !== "closed") return { mr };
    const final = { mr, mergeSha: mergeShaOf(single) };
    cache.set("mrs", mr.iid, final);
    cache.set("branches", branch, mr.iid);
    return { final };
  };
  const read = async branch => {
    const looked = await lookup(branch);
    const { mr = null, final = looked.final } = looked.found
      ? await settle(branch, looked.found)
      : {};
    if (final) {
      // Merged: where the deployment of its commit stands (try-state.mjs).
      phase("deploys");
      const copy = { ...final.mr };
      if (copy.state === "merged" && final.mergeSha)
        copy.deploy = await deployOf(final.mergeSha, context).catch(() => null);
      return { mr: copy, jobs: [] };
    }
    if (!mr) return { mr: null, jobs: [] };
    phase("jobs");
    // The jobs are extra: without them issue and MR still show.
    const jobs = mr.pipeline
      ? await jobsOf(mr.pipeline, context).catch(() => [])
      : [];
    return { mr, jobs };
  };
  return new Map(
    await Promise.all(
      branches.map(async branch => [branch, await read(branch)]),
    ),
  );
};
/**
 * The issues of `numbers`, by number: a closed one is known, the others in
 * one call per 100, an open one's links asked again on a full read.
 */
const readIssues = async (numbers, { isFull, context, kept }) => {
  const { json, cache } = context;
  const issues = new Map(
    numbers
      .filter(number => cache.get("issues", number))
      .map(number => [number, cache.get("issues", number)]),
  );
  phase("issues");
  const found = (
    await Promise.all(
      chunks(
        numbers.filter(number => !issues.has(number)),
        BATCH,
      ).map(batch =>
        json(
          `projects/:fullpath/issues?state=all&per_page=${BATCH}&${batch.map(number => `iids[]=${number}`).join("&")}`,
        ),
      ),
    )
  ).flat();
  await Promise.all(
    found.map(async issue => {
      if (issue.state === "closed") {
        const done = issueOf(issue);
        cache.set("issues", issue.iid, done);
        issues.set(issue.iid, done);
        return;
      }
      // GitLab's "blocked by" links, for the merge order (order.mjs).
      if (isFull || !kept.links.has(issue.iid))
        kept.links.set(
          issue.iid,
          await json(`projects/:fullpath/issues/${issue.iid}/links`).catch(
            () => kept.links.get(issue.iid) ?? [],
          ),
        );
      issues.set(issue.iid, issueOf(issue, kept.links.get(issue.iid)));
    }),
  );
  return issues;
};
/**
 * The MR, its pipeline's jobs and the issue of each of `branches`
 * (`[branch, { mr, jobs, waiting, issue, at }]`, as readRemoteOf has them),
 * in a few calls, MRs and issues side by side: `isFull` looks again at what
 * moves slowly. `context`: GitLab and the cache, `kept`: what the last reads
 * found (readMemory).
 */
export const readBranches = async (
  branches,
  { isFull = true, context = defaults(), kept = memory } = {},
) => {
  const options = { isFull, context, kept };
  const [mrs, issues] = await Promise.all([
    readMrs(branches, options),
    readIssues([...new Set(branches.map(numberOf).filter(Boolean))], options),
  ]);
  phase(null);
  context.cache.save();
  const at = new Date();
  return branches.map(branch => [
    branch,
    {
      ...mrs.get(branch),
      waiting: [],
      issue: issues.get(numberOf(branch)) ?? null,
      at,
    },
  ]);
};
/**
 * The state of each of `iids` (`{ iid, state }`, null when GitLab fails),
 * from the MRs `found` already read, else asked; a merged or closed one once.
 */
const statesOf = async (iids, found, context = defaults()) => {
  const known = new Map(
    found
      .map(([, remote]) => remote.mr)
      .filter(Boolean)
      .map(mr => [mr.iid, mr.state]),
  );
  return new Map(
    await Promise.all(
      iids.map(async iid => {
        const state =
          known.get(iid) ??
          context.cache.get("states", iid) ??
          (await context.json(`projects/:fullpath/merge_requests/${iid}`).then(
            mr => mr.state,
            () => null,
          ));
        if (state === "merged" || state === "closed")
          context.cache.set("states", iid, state);
        return [iid, state ? { iid, state } : null];
      }),
    ),
  );
};
/**
 * The branches of an orchestration: the ones its record lists (pnpm
 * wt:orchestrate), started or skipped, and those of worktrees started for it
 * by hand (`wt:new --origin`).
 */
const branchesOf = name =>
  [
    ...new Set([
      ...(recordOf(name)?.issues ?? []).map(issue => issue.branch),
      ...worktrees()
        .slice(1)
        .filter(({ dir }) => originOf(dir)?.name === name)
        .map(({ branch }) => branch),
    ]),
  ].filter(Boolean);
/** What a rewrite of the changelog was last started for, by orchestration: one that lists it still not is not tried again and again. */
const changelogTried = new Map();
/**
 * The changelog of `name` (#248) rewritten in the background when a merged
 * MR or an issue closed without one isn't in it yet: a merge by the
 * overview's click (its read follows at once) or anywhere else.
 */
const keepChangelog = (name, found) => {
  const remotes = found.map(([, remote]) => remote).filter(Boolean);
  const done = {
    mrs: remotes
      .filter(remote => remote.mr?.state === "merged")
      .map(remote => remote.mr.iid),
    closed: remotes
      .filter(
        remote =>
          remote.mr?.state !== "merged" && remote.issue?.state === "closed",
      )
      .map(remote => remote.issue.iid),
  };
  const key = JSON.stringify(done);
  if (changelogTried.get(name) === key || !isChangelogBehind(name, done))
    return;
  changelogTried.set(name, key);
  writeChangelog(name).catch(() => changelogTried.delete(name));
};
/**
 * Of every branch of an orchestration: its merge request (merged ones too)
 * with its pipeline, and its issue; develop's last finished pipeline.
 * Delivers the lines of the waits that ended without their command, and
 * keeps its changelog up to date. `isFull`: what moves slowly too
 * (readBranches).
 */
export const readRemoteOfOrchestration = async (
  name,
  { isFull = true } = {},
) => {
  const [found, develop] = await Promise.all([
    readBranches(branchesOf(name), { isFull }),
    developPipelineOf().catch(() => null),
  ]);
  keepChangelog(name, found);
  // Its worktrees, for the waits.
  const dirs = worktrees()
    .slice(1)
    .filter(({ dir }) => originOf(dir)?.name === name)
    .map(({ dir }) => dir);
  // With GitLab read anyway: the waits whose command is gone (waits.mjs).
  await deliverWaits(name, dirs);
  // The MRs the announced handovers (worktree/handovers.mjs) and the waits
  // of its worktrees (wt:wait, #262) wait for, by iid.
  const iids = [
    ...new Set([
      ...handoversOf(recordOf(name)).flatMap(({ after }) => after),
      // And the queued issues (`--queue --after`, #272).
      ...(recordOf(name)?.issues ?? [])
        .filter(issue => issue.queued)
        .flatMap(issue => issue.after ?? []),
      ...dirs.flatMap(dir => waitOf(dir)?.mrs ?? []),
    ]),
  ];
  const awaited = await statesOf(iids, found);
  defaults().cache.save();
  return { of: new Map(found), awaited, develop, at: new Date() };
};
/**
 * The issues of an orchestration, each with its worktree while that is there
 * (`trees`: the worktrees readLocal found for it), by number.
 */
export const issuesOf = (name, trees) => {
  const issues = (recordOf(name)?.issues ?? []).map(issue => ({
    ...issue,
    tree: trees.find(tree => tree.branch === issue.branch) ?? null,
  }));
  for (const tree of trees) {
    if (issues.some(issue => issue.tree === tree)) continue;
    const number = Number(tree.branch.match(/\/(\d+)-/)?.[1]) || null;
    issues.push({
      number,
      title: null,
      feature: tree.name,
      branch: tree.branch,
      why: null,
      tree,
    });
  }
  return issues.toSorted(
    (a, b) => (a.number ?? Infinity) - (b.number ?? Infinity),
  );
};
/** The containers of one checkout's stack: service, state, health. */
export const containersOf = dir => {
  const listed = compose(dir, ["ps", "--all", "--format", "json"], {
    quiet: true,
    allowFail: true,
  }).out;
  return listed
    .split("\n")
    .filter(Boolean)
    .map(line => {
      try {
        return JSON.parse(line);
      } catch {
        return null;
      }
    })
    .filter(Boolean)
    .map(c => ({ service: c.Service, state: c.State, health: c.Health || "" }));
};
