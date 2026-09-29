// Which idle agent fits an issue (#217): the one whose worktrees touched the
// files the issue names. wt:orchestrate asks it before it starts a fresh
// agent (#229), the overview for the issues queued with --queue.
import { costOf, stackFor } from "./memory.mjs";

/** A file path an issue names: with a folder or not, with an extension of the repo's files. */
const PATH =
  /(?:[\w@.-]+\/)*[\w@-][\w@.-]*\.(?:mjs|cjs|js|jsx|ts|tsx|json|md|css|scss|sql|toml|ya?ml)\b/g;
/** The file paths `text` (an issue's description) names, each once. */
export const pathsOf = text => [...new Set(String(text ?? "").match(PATH))];
/** The paths of `paths` a file of `files` is: the same, or `files` has it under a folder. */
export const sharedOf = (paths, files) =>
  paths.filter(name =>
    files.some(file => file === name || file.endsWith(`/${name}`)),
  );
/**
 * For each issue (`{ number, paths }`, in the order they wait), the free
 * worker (`{ name, share, files }`) that shares the most of its paths, the
 * one with the most room in its context on a tie: `number → { worker,
 * shared }`. Each worker gets one issue; one sharing nothing gets none.
 */
export const suggestionsOf = (issues, workers) => {
  const taken = new Set();
  const suggested = new Map();
  for (const issue of issues) {
    const [best] = workers
      .filter(worker => !taken.has(worker))
      .map(worker => ({ worker, shared: sharedOf(issue.paths, worker.files) }))
      .filter(({ shared }) => shared.length > 0)
      .toSorted(
        (a, b) =>
          b.shared.length - a.shared.length || a.worker.share - b.worker.share,
      );
    if (!best) continue;
    taken.add(best.worker);
    suggested.set(issue.number, best);
  }
  return suggested;
};

/**
 * How each issue of wt:orchestrate starts (`items`: `{ number, description }`,
 * in the given order), no count holding one back (#229): item → `{ worker,
 * shared }` for one an idle agent of `idle` fits (suggestionsOf), else
 * `{ stack }` (memory.mjs) for a fresh agent, the memory of the fresh ones
 * before it counted as taken.
 */
export const startsOf = (items, idle, { isShared = false, free }) => {
  const suggested = suggestionsOf(
    items.map(item => ({
      number: item.number,
      paths: pathsOf(item.description),
    })),
    idle,
  );
  const starts = new Map();
  let reserved = 0;
  for (const item of items) {
    const found = suggested.get(item.number);
    if (found) {
      starts.set(item, found);
      continue;
    }
    const stack = stackFor({ isShared, free, reserved });
    reserved += costOf(stack);
    starts.set(item, { stack });
  }
  return starts;
};
