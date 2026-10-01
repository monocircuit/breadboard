// What GitLab says that can't change any more (#305), kept on disk for every
// overview of the checkout: a merged or closed MR, a closed issue, the jobs
// of a finished pipeline (a failed one's log lines with them), the
// deployment of a merge commit that ended (try-state.mjs `isFinal`). Asked
// once, then never again: a refresh reads only what still moves.
//
// One file in the main checkout's .claude/, shared by all overviews: a save
// merges with what another one wrote meanwhile, the last writer's entry wins
// (both hold the same facts).
import fs from "node:fs";
import path from "node:path";

import { mainDir } from "../worktree.mjs";

/** Bumped when the shape of an entry changes: an older file is dropped. */
const VERSION = 1;
/** Entries kept per kind, the newest: the file doesn't grow forever. */
const KEEP = 2000;

export const cacheFile = () =>
  path.join(mainDir(), ".claude", "gitlab-cache.json");

const read = file => {
  try {
    const data = JSON.parse(fs.readFileSync(file, "utf-8"));
    return data.version === VERSION ? data : { version: VERSION };
  } catch {
    return { version: VERSION };
  }
};
const newest = entries =>
  Object.fromEntries(Object.entries(entries).slice(-KEEP));

/**
 * The cache in `file` (null: in memory only, for tests and a --once print):
 * `get(kind, key)`, `set(kind, key, value)`, `save()` once a read is done.
 */
export const gitlabCache = (file = cacheFile()) => {
  let data = file ? read(file) : { version: VERSION };
  const added = {};
  return {
    get: (kind, key) => data[kind]?.[key],
    set: (kind, key, value) => {
      (data[kind] ??= {})[key] = value;
      (added[kind] ??= {})[key] = value;
    },
    save: () => {
      if (!file || Object.keys(added).length === 0) return;
      // What another overview wrote meanwhile stays; this one's added on top.
      const merged = read(file);
      for (const [kind, entries] of Object.entries(added))
        merged[kind] = newest({ ...merged[kind], ...entries });
      try {
        fs.mkdirSync(path.dirname(file), { recursive: true });
        const temporary = `${file}.${process.pid}.tmp`;
        fs.writeFileSync(temporary, JSON.stringify(merged));
        fs.renameSync(temporary, file);
        data = merged;
        for (const kind of Object.keys(added)) delete added[kind];
      } catch {
        // Not written: asked again next time, nothing lost.
      }
    },
  };
};
