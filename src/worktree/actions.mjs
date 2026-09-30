// What an agent asks the human to do (`pnpm wt:action`), for the overview of
// its orchestration: merge its MR, try something out, decide.
import fs from "node:fs";
import path from "node:path";

/**
 * The kinds of action and what a click on one in the overview does:
 * - `merge`: the MR of the worktree's branch is ready; two clicks merge it,
 *   the human's (overview/merge.mjs), an agent never does. Gone once merged.
 * - `try`: something only a person can check, with the spot the click opens
 *   (overview/try.mjs): `--run <command>` a terminal running it in the
 *   worktree, `--url <page>` the page on the worktree's app. `browser-check`
 *   is `try --url`, and is stored as a `try`.
 * - `decide`: a decision that is the human's: one question, the options
 *   the agent gave (`--option`, each saying what happens) and the one it
 *   recommends (`--recommend`, marked on its button); a click on one answers it.
 * - `look`: anything else to look at in the agent's space.
 * - `plan`: the agent's plan before larger work, for an `ok` or `different`.
 */
export const ACTION_KINDS = ["merge", "try", "decide", "look", "plan"];
/** Other names of a kind: `browser-check` is a `try --url`. */
export const KIND_ALIASES = { "browser-check": "try" };
/**
 * The answers the overview offers per kind; `decide` offers the options the
 * agent gave. None (a merge, a decision without options): only ✓, which
 * ticks the action off without an answer.
 */
const ANSWERS = {
  try: ["looks good", "broken"],
  // Stored before `try` came.
  "browser-check": ["looks good", "broken"],
  look: ["done"],
  plan: ["ok", "different"],
};
export const answersOf = action =>
  action.kind === "decide"
    ? (action.options ?? [])
    : (ANSWERS[action.kind] ?? []);
/**
 * A decide the user can answer from its row: one question (what is decided,
 * in one sentence; a second one may say why the recommendation), at least
 * two options each saying what happens, not `A` / `B`, and the recommended one.
 */
export const decideProblem = (text, options, recommend) => {
  if (!text.includes("?"))
    return 'a decide asks one question: what is decided, in one sentence, ending in "?"';
  if (options.length < 2)
    return 'a decide offers at least two options: --option "<what happens>" --option "<what happens>"';
  const bare = options.filter(option => option.trim().split(/\s+/).length < 2);
  if (bare.length)
    return `each option says what happens, in a few words, not just ${bare.map(option => `"${option}"`).join(", ")}: --option "keep both lines"`;
  if (!recommend)
    return "a decide names its recommendation: --recommend <option or its number>";
  if (!options.includes(recommend))
    return `--recommend ${recommend} is none of the options: ${options.map((option, i) => `${i + 1} "${option}"`).join(", ")}`;
  return null;
};
/**
 * Where a worktree keeps its actions, in its ignored `.claude/`. The
 * orchestrator's own go into the record of the orchestration (origins.mjs);
 * both hold them under `actions`: `[{ id, kind, text, url, run, options, recommend, pane, at }]`,
 * an answered one with `answer` and `answeredAt` until it is delivered.
 * `pane`: `$HERDR_PANE_ID` of whoever asked, how the Stop hook of the main
 * checkout tells the orchestrator's answers from another's (an identity only:
 * a moved pane gets a new id, its process keeps the old one).
 */
export const actionsFile = dir => path.join(dir, ".claude", "actions.json");
const readFile = file => {
  try {
    const data = JSON.parse(fs.readFileSync(file, "utf-8"));
    return data && typeof data === "object" ? data : {};
  } catch {
    return {};
  }
};
/** The open actions in `file`, oldest first; none when there is no file. */
export const actionsOf = file => {
  const { actions } = readFile(file);
  return Array.isArray(actions) ? actions : [];
};
/** `actions` into `file`, whatever else it holds kept. */
const writeActions = (file, actions) => {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(
    file,
    `${JSON.stringify({ ...readFile(file), actions }, null, 2)}\n`,
  );
};
const LOCK_STALE_MS = 5000;
const pause = ms =>
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
/**
 * `fn(actions)` → `{ actions, result }` as one change of `file`: the
 * overview, the Stop hook and wt:action read and write it from processes of
 * their own, and a read between another's read and write would undo it. The
 * lock is a directory next to the file (mkdir is atomic); `actions` left out
 * writes nothing, `fields` are written next to them. Returns `result`.
 */
const changeActions = (file, fn) => {
  const lock = `${file}.lock`;
  fs.mkdirSync(path.dirname(file), { recursive: true });
  for (;;) {
    try {
      fs.mkdirSync(lock);
      break;
    } catch {
      const age =
        Date.now() -
        (fs.statSync(lock, { throwIfNoEntry: false })?.mtimeMs ?? Date.now());
      if (age > LOCK_STALE_MS)
        fs.rmSync(lock, { recursive: true, force: true });
      else pause(10);
    }
  }
  try {
    const { actions, result, fields } = fn(actionsOf(file));
    if (actions) writeActions(file, actions);
    if (fields)
      fs.writeFileSync(
        file,
        `${JSON.stringify({ ...readFile(file), ...fields }, null, 2)}\n`,
      );
    return result;
  } finally {
    fs.rmSync(lock, { recursive: true, force: true });
  }
};
/**
 * A new action in `file`, numbered after the highest there. The same kind
 * and text again replaces the older one: an agent that announces twice waits
 * for one thing, not two. A merge ends the requested change (`change`): its
 * agent says it is built, pushed and green.
 */
export const addAction = (
  file,
  {
    kind,
    text,
    url = null,
    run = null,
    options = [],
    recommend = null,
    pane = null,
  },
) =>
  changeActions(file, actions => {
    const action = {
      id: Math.max(0, ...actions.map(other => other.id)) + 1,
      kind,
      text,
      url,
      run,
      options,
      recommend,
      pane,
      at: new Date().toISOString(),
    };
    return {
      actions: [
        ...actions.filter(other => other.kind !== kind || other.text !== text),
        action,
      ],
      result: action,
      ...(kind === "merge" ? { fields: { change: null } } : {}),
    };
  });
/** The action `id` in `file` done: it goes. False when there is none. */
export const doneAction = (file, id) =>
  changeActions(file, actions =>
    actions.some(action => action.id === id)
      ? { actions: actions.filter(action => action.id !== id), result: true }
      : { result: false },
  );
/** The action `id` in `file` answered, until it is delivered. False when there is none. */
export const answerAction = (file, id, answer) =>
  changeActions(file, actions => {
    if (!actions.some(action => action.id === id)) return { result: false };
    const answeredAt = new Date().toISOString();
    return {
      actions: actions.map(action =>
        action.id === id ? { ...action, answer, answeredAt } : action,
      ),
      result: true,
    };
  });
/** How an answer reaches its agent, one line: `[overview] #3 decide "A or B" → B`. */
export const answerLine = action =>
  `[overview] #${action.id} ${action.kind} "${action.text}" → ${action.answer}`;
/**
 * How long a claim holds: an answer whose deliverer died while typing it is
 * free again after that.
 */
const CLAIM_MS = 30_000;
const isClaimed = action =>
  Boolean(action.deliveringAt) &&
  Date.now() - Date.parse(action.deliveringAt) < CLAIM_MS;
/**
 * The answer of action `id` in `file` claimed for typing it into its agent's
 * pane (`deliveringAt`): no one else takes it meanwhile. The action, or null
 * when it has no answer or another holds it. After typing: `doneAction` once
 * it got in, `releaseAnswer` when it didn't.
 */
export const claimAnswer = (file, id) =>
  changeActions(file, actions => {
    const action = actions.find(other => other.id === id);
    if (!action?.answer || isClaimed(action)) return { result: null };
    const claimed = { ...action, deliveringAt: new Date().toISOString() };
    return {
      actions: actions.map(other => (other.id === id ? claimed : other)),
      result: claimed,
    };
  });
/** The claim on the answer of action `id` in `file` given back: it didn't get in. */
export const releaseAnswer = (file, id) =>
  changeActions(file, actions => ({
    actions: actions.map(action =>
      action.id === id ? { ...action, deliveringAt: undefined } : action,
    ),
  }));
/**
 * The answered actions in `file`, taken out of it: whoever takes them
 * delivers them, so each answer arrives once. One claimed by another
 * (`claimAnswer`) stays. `pane`: only those asked from that pane (the
 * orchestrator's in a record that others share).
 */
export const takeAnswers = (file, { pane } = {}) =>
  changeActions(file, actions => {
    const taken = actions.filter(
      action =>
        action.answer &&
        !isClaimed(action) &&
        (pane === undefined || action.pane === pane),
    );
    return taken.length
      ? {
          actions: actions.filter(action => !taken.includes(action)),
          result: taken,
        }
      : { result: [] };
  });
/**
 * The change requested for a worktree's MR (`wt:action change`), in the same
 * file as its actions: `{ text, at, mr, head }`, `head` the commit of the
 * origin branch when it was asked for; null when none is open.
 */
export const changeOf = file => readFile(file).change ?? null;
/**
 * `change` requested in `file`: the merge actions there are withdrawn, the MR
 * is no longer to be merged until the change is pushed and green. Returns
 * the merge actions it withdrew.
 */
export const requestChange = (file, change) =>
  changeActions(file, actions => ({
    actions: actions.filter(action => action.kind !== "merge"),
    result: actions.filter(action => action.kind === "merge"),
    fields: { change },
  }));
