// The command line of the overview: which view (all, --worktree,
// --orchestration), --once, --herdr, and how often it reads.
import path from "node:path";
import { isMainThread, workerData } from "node:worker_threads";

import { isInHerdr, mainDir } from "../worktree.mjs";

export const LOCAL_MS = 2000;
// `git status` every LOCAL_MS takes the index lock to refresh its cache, and a
// commit in the same worktree at that moment fails on `index.lock`. Reading
// needs no lock: every git this process starts inherits it.
process.env.GIT_OPTIONAL_LOCKS = "0";
export const REMOTE_MS = 30_000;
// The read in its worker (reader.mjs) has no terminal: it is told whether
// the overview has one, and where it was started.
const isTTY = isMainThread ? process.stdout.isTTY : workerData?.isTTY;
export const isOnce = process.argv.includes("--once") || !isTTY;
const argv = process.argv.slice(2);
/** The worktree `--worktree <dir>` narrows the view to, null for all of them. */
export const only = argv.includes("--worktree")
  ? path.resolve(argv[argv.indexOf("--worktree") + 1] ?? ".")
  : null;
/** The orchestration `--orchestration <name>` narrows the view to, null for none. */
export const orchestration = argv.includes("--orchestration")
  ? (argv[argv.indexOf("--orchestration") + 1] ?? null)
  : null;
export const isSyncing =
  argv.includes("--herdr") && isInHerdr() && !only && !orchestration && !isOnce;
/** Where it was started, read once: the ✕ of a worktree deletes the directory this view runs in. */
export const here = isMainThread
  ? path.resolve(process.env.INIT_CWD ?? process.cwd())
  : workerData?.here;
// A worker can't change it, and shares the one the overview changed.
if (isMainThread) process.chdir(mainDir());
