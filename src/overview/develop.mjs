// Two more answers for the view of one worktree (#158), both local: what
// develop brought in since the branch started, and whether merging it would
// conflict; and the migrations the database of the worktree's stack lacks.
//
// The overview reads them every 2 s, so both are cached: develop under HEAD
// and origin/develop, the migrations for MIGRATIONS_MS or until the journal
// changes.
import fs from "node:fs";
import path from "node:path";

import { lines, sh as run } from "../lib/proc.mjs";
import { VITA } from "../worktree/constants.mjs";

const git = (args, dir) => run("git", args, dir);

const developCache = new Map();

/**
 * What came into develop since `dir` branched off, newest first: the merge
 * requests merged on develop's first-parent line, from their merge commits
 * (`!n`, the title, the branch), and the commits that came in without one.
 * `conflicts`: the files a `git merge origin/develop` would stop at, from
 * `git merge-tree`, which touches neither the working tree nor a ref; null
 * when git can't tell (older than 2.38).
 */
export const developOf = dir => {
  const key = git(["rev-parse", "HEAD", "origin/develop"], dir).stdout;
  const hit = developCache.get(dir);
  if (hit?.key === key) return hit.value;

  // Fields split by 0x1f, entries by 0x1e: a body has newlines.
  const log = git(
    ["log", "--first-parent", "--format=%s%x1f%b%x1e", "HEAD..origin/develop"],
    dir,
  ).stdout;
  const entries = log
    .split("\x1e")
    .map(entry => entry.trim())
    .filter(Boolean)
    .map(entry => entry.split("\x1f"));
  const merges = [];
  let direct = 0;
  for (const [subject, body = ""] of entries) {
    const branch = subject.match(/^Merge branch '([^']+)' into/)?.[1];
    if (!branch) {
      direct++;
      continue;
    }
    const mr = Number(body.match(/!(\d+)\s*$/m)?.[1]) || null;
    const title =
      lines(body).find(
        line =>
          !/^(Closes|Refs|See merge request|Co-Authored-By)\b/i.test(line),
      ) ?? branch;
    merges.push({ mr, title: title.trim(), branch });
  }

  let conflicts = null;
  if (entries.length > 0) {
    const tree = git(
      [
        "merge-tree",
        "--write-tree",
        "--name-only",
        "--no-messages",
        "HEAD",
        "origin/develop",
      ],
      dir,
    );
    // 0 clean, 1 conflicts (the tree's id, then one file per line), anything else: git can't.
    if (tree.status === 0) conflicts = [];
    if (tree.status === 1) conflicts = lines(tree.stdout).slice(1);
  } else {
    conflicts = [];
  }

  const value = { merges, direct, conflicts };
  developCache.set(dir, { key, value });
  return value;
};

/** How long an answer of the database counts: a query every 2 s would be noise in its log. */
const MIGRATIONS_MS = 15_000;
const migrationsCache = new Map();

/**
 * The migrations of `dir` (products/vita/web/src/server/db/migrations) that the
 * database of its stack hasn't applied, the way drizzle decides it: a
 * migration counts as applied when the newest one in
 * `drizzle.__drizzle_migrations` is at least as new as its `when`.
 * `{ pending: [tag…], applied }`, `{ error }` when the database doesn't answer,
 * null without .env.local, journal or psql.
 */
export const migrationsOf = dir => {
  const vita = path.join(dir, VITA);
  const journalFile = path.join(
    vita,
    "src",
    "server",
    "db",
    "migrations",
    "meta",
    "_journal.json",
  );
  const envFile = path.join(vita, ".env.local");
  if (!fs.existsSync(journalFile) || !fs.existsSync(envFile)) return null;

  const mtime = fs.statSync(journalFile).mtimeMs;
  const hit = migrationsCache.get(dir);
  if (hit && hit.mtime === mtime && Date.now() - hit.at < MIGRATIONS_MS)
    return hit.value;

  const url = fs
    .readFileSync(envFile, "utf-8")
    .match(/^MIGRATION_DATABASE_URL=(.*)$/m)?.[1]
    ?.trim();
  if (!url) return null;
  const answer = run("psql", [
    url,
    "--no-psqlrc",
    "-tAc",
    "select coalesce(max(created_at), 0) from drizzle.__drizzle_migrations",
  ]);
  let value;
  if (answer.error) {
    value = null;
  } else if (answer.status !== 0) {
    // A fresh database has no drizzle schema yet: everything is pending.
    value = /does not exist/.test(answer.stderr)
      ? pendingSince(journalFile, 0)
      : { error: lines(answer.stderr)[0] ?? "no answer" };
  } else {
    value = pendingSince(journalFile, Number(answer.stdout.trim()) || 0);
  }
  migrationsCache.set(dir, { mtime, at: Date.now(), value });
  return value;
};

const pendingSince = (journalFile, newest) => {
  const { entries = [] } = JSON.parse(fs.readFileSync(journalFile, "utf-8"));
  const pending = entries
    .filter(entry => entry.when > newest)
    .map(entry => entry.tag);
  return { pending, applied: entries.length - pending.length };
};
