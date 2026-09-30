// Whether a `try` can be tried now (#237). One on the worktree's app is
// ready once its commits are pushed; one on the test instance (`wt:action try
// --on test`) once its MR is merged and the develop pipeline of the merge
// commit has deployed it: `waits for merge !N` → `deploying (publish, 3 min)`
// → `ready`, or `deploy failed` with the job a click opens.
//
// vita reports no build commit, so the instance itself can't be asked which
// one it runs: `deploy:develop` passed plus DEPLOY_GRACE_MS counts as
// deployed, and the state says so.

/** How long Coolify gets after `deploy:develop` passed to pull and start the image. */
export const DEPLOY_GRACE_MS = 90_000;
/** The jobs of the develop pipeline that deploy: the others (scans, checks) don't hold a try. */
const DEPLOY_JOBS = ["build:develop", "publish:develop", "deploy:develop"];
/** The job whose success means the test instance was told to pull the image. */
const DEPLOY_JOB = "deploy:develop";
/** Without it (no Coolify variables, .gitlab-ci.yml), publishing is the last step. */
const PUBLISH_JOB = "publish:develop";
/** A job's statuses that end it without success. */
const FAILED = new Set(["failed", "canceled"]);
/** A pipeline that no longer runs. */
const DONE = new Set(["success", "failed", "canceled", "skipped"]);

/** `40 s`, `3 min`, `2 h`: how long something runs. */
export const span = ms => {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  if (seconds < 60) return `${seconds} s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min`;
  return `${Math.floor(seconds / 3600)} h`;
};
/** `publish` of `publish:develop`: the step a job is. */
const stepOf = job => job.name.split(":")[0];

/**
 * Where the deployment of a merge commit stands, from the develop pipeline
 * of that commit and its jobs (gitlab.mjs `deployOf`): `{ state, job, since,
 * isFinal }`, `state` one of
 * - `deploying`: no pipeline yet, or it is at `job` (the one running, else
 *   the first to come), since `since`, the pipeline's start;
 * - `deployed`: `deploy:develop` passed DEPLOY_GRACE_MS ago (`publish:develop`
 *   without a deploy job): `job` is that one;
 * - `failed`: a deploying job failed or was cancelled: `job` is that one, a
 *   click opens its `url`.
 * `isFinal`: it won't change any more, what gitlab.mjs caches.
 */
export const deployStateOf = (deploy, now = Date.now()) => {
  const pipeline = deploy?.pipeline ?? null;
  if (!pipeline)
    return { state: "deploying", job: null, since: null, isFinal: false };
  const jobs = (deploy.jobs ?? []).filter(job =>
    DEPLOY_JOBS.includes(job.name),
  );
  const since = pipeline.createdAt ? Date.parse(pipeline.createdAt) : null;
  const failed = jobs.find(job => FAILED.has(job.status));
  if (failed) return { state: "failed", job: failed, since, isFinal: true };
  const last =
    jobs.find(job => job.name === DEPLOY_JOB) ??
    // No deploy job in a pipeline that has ended: publishing was the last step.
    (DONE.has(pipeline.status)
      ? jobs.find(job => job.name === PUBLISH_JOB)
      : null);
  if (last?.status === "success") {
    const passed = last.finishedAt ? Date.parse(last.finishedAt) : now;
    const isDeployed = now - passed >= DEPLOY_GRACE_MS;
    return {
      state: isDeployed ? "deployed" : "deploying",
      job: last,
      since,
      passed,
      isFinal: isDeployed,
    };
  }
  // The pipeline ended without the deploying jobs passing, and none failed.
  if (DONE.has(pipeline.status) && pipeline.status !== "success")
    return {
      state: "failed",
      job: jobs.at(-1) ?? { name: "pipeline", url: pipeline.url },
      since,
      isFinal: true,
    };
  const job =
    jobs.find(candidate => candidate.status === "running") ??
    jobs.find(candidate => candidate.status !== "success") ??
    null;
  return { state: "deploying", job, since, isFinal: false };
};

/**
 * Whether the try `action` can be tried now, and what its row says in front
 * of the text: `{ isReady, label, tone, url }`, `tone` one of `ready`,
 * `waits`, `failed`; `url` what a click opens in place of the spot (the failed
 * job). `tree`: the worktree with pushStateOf's `unpushed` (none for the
 * orchestrator's own), `mr` its MR as readRemoteOf reads it (with
 * `deploy`), `origin` the test instance's (testOrigin, try.mjs),
 * `hasRemote` whether GitLab has answered yet.
 */
export const tryStateOf = (
  action,
  {
    tree = null,
    mr = null,
    origin = null,
    hasRemote = true,
    now = Date.now(),
  } = {},
) => {
  if (action.on !== "test") {
    if (tree?.unpushed)
      return { isReady: false, label: "not pushed", tone: "waits" };
    return { isReady: true, label: "ready", tone: "ready" };
  }
  if (!origin)
    return {
      isReady: false,
      label: "no test instance: VITA_TEST_ORIGIN unset",
      tone: "failed",
    };
  if (!hasRemote) return { isReady: false, label: "GitLab …", tone: "waits" };
  if (!mr) return { isReady: false, label: "waits for its MR", tone: "waits" };
  if (mr.state === "closed")
    return { isReady: false, label: `!${mr.iid} closed`, tone: "failed" };
  if (mr.state !== "merged")
    return {
      isReady: false,
      label: `waits for merge !${mr.iid}`,
      tone: "waits",
    };
  const deploy = deployStateOf(mr.deploy, now);
  if (deploy.state === "failed")
    return {
      isReady: false,
      label: `deploy failed (${deploy.job.name})`,
      tone: "failed",
      url: deploy.job.url ?? null,
    };
  if (deploy.state === "deployed")
    return {
      isReady: true,
      label: `ready (${deploy.job.name} passed ${span(now - deploy.passed)} ago)`,
      tone: "ready",
    };
  const at = deploy.passed
    ? `${deploy.job.name} passed, Coolify starts it`
    : deploy.job
      ? stepOf(deploy.job)
      : "pipeline";
  return {
    isReady: false,
    label: `deploying (${at}${deploy.since ? `, ${span(now - deploy.since)}` : ""})`,
    tone: "waits",
  };
};
