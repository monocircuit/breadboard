// GitLab beyond the MR list: pipeline jobs with the end of a failed log,
// branches, the issues of an orchestration; and the containers of the stacks.
import {
  compose,
  handoversOf,
  isChangelogBehind,
  mainDir,
  originOf,
  recordOf,
  worktrees,
  writeChangelog,
} from "../worktree.mjs";
import { exec, glab } from "./io.mjs";
import { readRemoteOf } from "./read.mjs";
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
/** A failed job's log doesn't change any more: job id → its last lines. */
const logs = new Map();
/**
 * The jobs of a pipeline in the order they ran, the last try of each; a
 * failed one with the last lines of its log, where the error usually is.
 */
export const jobsOf = async pipeline => {
  const all = await glab(
    `projects/:fullpath/pipelines/${pipeline}/jobs?per_page=100&include_retried=false`,
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
        if (!logs.has(job.id)) {
          const { stdout } = await exec(
            "glab",
            ["api", `projects/:fullpath/jobs/${job.id}/trace`],
            { cwd: mainDir(), maxBuffer: 1 << 24 },
          );
          const lines = stdout
            .replace(LOG_NOISE, "")
            .split("\n")
            .map(line =>
              line.split("\r").at(-1).replace(LOG_STAMP, "").trimEnd(),
            )
            .filter(Boolean);
          const end = lines.findLastIndex(line => !RUNNER_TAIL.test(line));
          logs.set(
            job.id,
            lines.slice(Math.max(0, end + 1 - LOG_LINES), end + 1),
          );
        }
        job.log = logs.get(job.id);
      }),
  );
  return jobs;
};
/** Deployments that won't change any more (try-state.mjs `isFinal`): merge commit → `{ pipeline, jobs }`. */
const deploys = new Map();
/**
 * The develop pipeline of the merge commit `sha` with its jobs, what a try
 * on the test instance waits for (try-state.mjs): `{ pipeline, jobs }`,
 * `pipeline` null while GitLab has none for it yet.
 */
export const deployOf = async sha => {
  if (deploys.has(sha)) return deploys.get(sha);
  const [found] = await glab(
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
    jobs: found ? await jobsOf(found.id) : [],
  };
  if (deployStateOf(deploy).isFinal) deploys.set(sha, deploy);
  return deploy;
};
/**
 * develop's last finished pipeline, `{ id, status, url, jobs }`, its jobs
 * only when it failed: a job that fails there too is not an MR's doing
 * (auto-actions.mjs `pipelineLineOf`). Null without one or when GitLab fails.
 */
const developPipelineOf = async () => {
  const [found] = await glab(
    "projects/:fullpath/pipelines?ref=develop&scope=finished&per_page=1",
  );
  if (!found) return null;
  return {
    id: found.id,
    status: found.status,
    url: found.web_url,
    jobs: found.status === "failed" ? await jobsOf(found.id) : [],
  };
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
 * keeps its changelog up to date.
 */
export const readRemoteOfOrchestration = async name => {
  const [found, develop] = await Promise.all([
    Promise.all(
      branchesOf(name).map(async branch => [
        branch,
        await readRemoteOf(branch),
      ]),
    ),
    developPipelineOf().catch(() => null),
  ]);
  keepChangelog(name, found);
  // With GitLab read anyway: the waits whose command is gone (waits.mjs).
  await deliverWaits(
    name,
    worktrees()
      .slice(1)
      .filter(({ dir }) => originOf(dir)?.name === name)
      .map(({ dir }) => dir),
  );
  // The MRs the announced handovers wait for (worktree/handovers.mjs), by iid.
  const iids = [
    ...new Set(handoversOf(recordOf(name)).flatMap(({ after }) => after)),
  ];
  const awaited = new Map(
    await Promise.all(
      iids.map(async iid => [
        iid,
        await glab(`projects/:fullpath/merge_requests/${iid}`).then(
          mr => ({ iid, state: mr.state }),
          () => null,
        ),
      ]),
    ),
  );
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
