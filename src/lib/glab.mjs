// GitLab's REST API, read only, with glab's login (#305). A call is one
// request over a kept-alive connection, not a glab process (~0.45 s each, 300
// per refresh of an orchestration): the host and project come from origin,
// the token from GITLAB_TOKEN or glab's config. glab's OAuth token expires
// after two hours and only glab renews it: once it is due (or GitLab answers
// 401), one `glab api user` renews it and the config is read again. Without a
// token it falls back to `glab api`.
//
// A call that fails for a reason that passes (a 429, a 5xx, a timeout, the
// network) is tried again after a pause; what still fails throws a
// GitlabError whose `reason` says why (`HTTP 429, rate limit`, `timeout`).
// Nothing reaches the terminal: an overview draws the reason itself.
import { execFile, spawnSync } from "node:child_process";
import { promisify } from "node:util";

import { mainDir } from "../worktree/worktrees.mjs";

const execAsync = promisify(execFile);

/** Requests in flight at once, per process. */
const PARALLEL = 12;
/** How long one request may take. */
const TIMEOUT_MS = 15_000;
/** The pauses before the second and third try. */
const BACKOFF_MS = [1_000, 4_000];
/** Below this many requests left in GitLab's minute, calls wait for the next one. */
const RESERVE = 100;

/** A call that failed, with why in a few words. */
export class GitlabError extends Error {
  constructor(reason, { status = null, endpoint = "" } = {}) {
    super(`GitLab: ${reason}${endpoint ? ` (${endpoint})` : ""}`);
    this.reason = reason;
    this.status = status;
  }
}

/**
 * What the calls of this process did: `calls` made (tries included),
 * `rate` GitLab's last word on the minute of this user, all its clients
 * together (`{ limit, observed, remaining, resetAt }`), null before one.
 */
export const stats = { calls: 0, rate: null };
/**
 * Where the read in flight stands, for the view (`GitLab: 4/7 · pipelines`):
 * calls `asked` and `answered` since beginRead, the `phase` its reader named.
 * `onChange` is called on every step.
 */
export const progress = {
  asked: 0,
  answered: 0,
  phase: null,
  onChange: () => {},
};
const stepped = () => progress.onChange();
/** A read starts: its calls are counted from here. */
export const beginRead = () => {
  Object.assign(progress, { asked: 0, answered: 0, phase: null });
  stepped();
};
/** What the read waits for now, in a word or two. */
export const phase = name => {
  progress.phase = name;
  stepped();
};

/**
 * Host, project and token, found once: `{ host, project, token, isOauth,
 * expiresAt }`, `token` null without one; a test's `fetch` with it.
 */
let found = null;
/** For tests: a `fetch`, a token, a project and the pauses between tries (`backoff`) of their own; `null` finds them again. */
export const configure = (options = null) => {
  found = options && {
    host: "gitlab.com",
    project: "group/project",
    token: "token",
    isOauth: false,
    expiresAt: null,
    ...options,
  };
};
/** `{ host, project }` of origin's url, ssh (`user@host:path.git`) or https. */
export const remoteOf = url => {
  const match = url
    .trim()
    .match(
      /^(?:[a-z+]+:\/\/)?(?:[^@/]+@)?([^:/]+)(?::\d+)?[:/](.+?)(?:\.git)?\/?$/,
    );
  return match ? { host: match[1], project: match[2] } : null;
};
/**
 * A value of glab's config, null without one. Not synchronous: glab waits
 * for its config's lock while another glab holds it, and an overview reads
 * GitLab in the thread that draws.
 */
const config = (key, host) =>
  execAsync("glab", ["config", "get", key, "--host", host], {
    cwd: mainDir(),
  }).then(
    ({ stdout }) => stdout.trim() || null,
    () => null,
  );
/** The token: GITLAB_TOKEN, else glab's, OAuth with its expiry. */
const tokenOf = async host => {
  if (process.env.GITLAB_TOKEN)
    return { token: process.env.GITLAB_TOKEN, isOauth: false, expiresAt: null };
  const [token, oauth, expiry] = await Promise.all(
    ["token", "is_oauth2", "oauth2_expiry_date"].map(key => config(key, host)),
  );
  const isOauth = oauth === "true";
  const expiresAt = isOauth ? Date.parse(expiry ?? "") : NaN;
  return {
    token,
    isOauth,
    expiresAt: Number.isNaN(expiresAt) ? null : expiresAt,
  };
};
/** Found once, the calls that come meanwhile wait for it. */
let finding = null;
const connection = async () => {
  if (found) return found;
  finding ??= (async () => {
    const remote = remoteOf(
      spawnSync("git", ["remote", "get-url", "origin"], {
        cwd: mainDir(),
        encoding: "utf-8",
        stdio: "pipe",
      }).stdout ?? "",
    ) ?? { host: "gitlab.com", project: null };
    const token = await tokenOf(remote.host);
    found = { ...remote, ...token, token: remote.project ? token.token : null };
    finding = null;
    return found;
  })();
  return finding;
};
/** One renewal at a time: glab renews an expired OAuth token on its next call. */
let renewing = null;
const renew = () =>
  (renewing ??= execAsync("glab", ["api", "user"], { cwd: mainDir() })
    .catch(() => null)
    .then(() => tokenOf(found.host))
    .then(token => Object.assign(found, token))
    .finally(() => (renewing = null)));
const isDue = ({ isOauth, expiresAt }) =>
  isOauth && expiresAt !== null && expiresAt - Date.now() < 60_000;

// At most PARALLEL requests at once; the others queue.
let running = 0;
const queue = [];
const slot = async () => {
  if (running >= PARALLEL) await new Promise(resolve => queue.push(resolve));
  running++;
};
const release = () => {
  running--;
  queue.shift()?.();
};
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

/** GitLab's rate limit headers, kept in `stats.rate`. */
const noteRate = headers => {
  const limit = Number(headers.get("ratelimit-limit"));
  if (!limit) return;
  stats.rate = {
    limit,
    observed: Number(headers.get("ratelimit-observed")) || 0,
    remaining: Number(headers.get("ratelimit-remaining")) || 0,
    resetAt: Number(headers.get("ratelimit-reset")) * 1000 || null,
  };
};
/** Before a call: with the minute nearly used up (by all clients of the user), wait for the next. */
const holdForRate = async () => {
  const rate = stats.rate;
  if (!rate || rate.remaining > RESERVE || !rate.resetAt) return;
  const wait = rate.resetAt - Date.now();
  if (wait > 0) await sleep(Math.min(wait, 60_000));
  stats.rate = null;
};

const isPassing = status => status === 429 || status >= 500;
const reasonOf = (status, statusText) =>
  status === 429
    ? "HTTP 429, rate limit"
    : `HTTP ${status}${statusText ? ` ${statusText}` : ""}`;

/** One request with its tries: the body as text. */
const request = async (url, endpoint) => {
  let last = null;
  let hasRenewed = false;
  const backoff = found.backoff ?? BACKOFF_MS;
  for (let attempt = 0; attempt <= backoff.length; attempt++) {
    if (attempt > 0) await sleep(last?.retryAfter ?? backoff[attempt - 1]);
    await holdForRate();
    if (isDue(found)) await renew();
    stats.calls++;
    try {
      const response = await (found.fetch ?? fetch)(url, {
        headers: found.isOauth
          ? { Authorization: `Bearer ${found.token}` }
          : { "PRIVATE-TOKEN": found.token },
        signal: AbortSignal.timeout(TIMEOUT_MS),
      });
      noteRate(response.headers);
      if (response.ok) return await response.text();
      // An OAuth token GitLab no longer takes: renewed once, the same try again.
      if (response.status === 401 && found.isOauth && !hasRenewed) {
        hasRenewed = true;
        await renew();
        attempt--;
        continue;
      }
      const retryAfter = Number(response.headers.get("retry-after")) * 1000;
      last = {
        error: new GitlabError(reasonOf(response.status, response.statusText), {
          status: response.status,
          endpoint,
        }),
        retryAfter: retryAfter > 0 ? Math.min(retryAfter, 30_000) : null,
      };
      if (!isPassing(response.status)) break;
    } catch (error) {
      const reason =
        error?.name === "TimeoutError" || error?.name === "AbortError"
          ? "timeout"
          : `network: ${error?.cause?.code ?? error?.message ?? error}`;
      last = { error: new GitlabError(reason, { endpoint }), retryAfter: null };
    }
  }
  throw last.error;
};

/** `glab api`, what is left without a token. */
const viaGlab = async endpoint => {
  stats.calls++;
  try {
    return (
      await execAsync("glab", ["api", endpoint], {
        cwd: mainDir(),
        maxBuffer: 1 << 24,
      })
    ).stdout;
  } catch (error) {
    throw new GitlabError(
      error.stderr?.trim().split("\n").at(-1) || error.message,
      { endpoint },
    );
  }
};

/**
 * A GET of `endpoint` as `glab api` takes it (`projects/:fullpath/…`,
 * `:id` alike), the body as text.
 */
export const glabText = async endpoint => {
  const { host, project, token } = await connection();
  progress.asked++;
  stepped();
  try {
    if (!token) return await viaGlab(endpoint);
    const url = `https://${host}/api/v4/${endpoint.replace(
      /:fullpath|:id/g,
      encodeURIComponent(project),
    )}`;
    await slot();
    try {
      return await request(url, endpoint);
    } finally {
      release();
    }
  } finally {
    progress.answered++;
    stepped();
  }
};
/** `glab api <endpoint>` (a GET), parsed. Async, so a sweep over many issues runs side by side. */
export const glabApi = async endpoint => JSON.parse(await glabText(endpoint));
