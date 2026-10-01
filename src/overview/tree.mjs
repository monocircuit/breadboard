// WORKTREES of an orchestration: its worktrees as a folder tree under the
// orchestration, each with where it stands in its life, from what the
// overview reads anyway (git, herdr, GitLab, the record): at work, waiting
// for you, ready, merged and about to go, gone. The ones wt:remove took are
// in the record with their times (markRemoved), under "done and removed".
// A node reads like a folder listing: its name and state, its facts and
// title in the lines below (#223). Every node has its number, its name is
// in the pink of herdr's sidebar (#227).
import {
  bold,
  cyan,
  dim,
  fit,
  green,
  link,
  pad,
  red,
  visible,
  wrap,
  yellow,
} from "../lib/tty.mjs";
import {
  ORCHESTRATOR,
  focusSpace,
  isChangeOpen,
  waitsText,
  wasOf,
} from "../worktree.mjs";
import { historyText, reuseOf, reuseText } from "../worktree/history.mjs";
import { THRESHOLD, gb } from "../worktree/memory.mjs";
import { isOnce } from "./args.mjs";
import { removalOf } from "./auto-actions.mjs";
import { boxInner, closeBox, openBox, pipelineMark } from "./boxes.mjs";
import { toneOf } from "./herdr-tones.mjs";
import { changeFact, unpushedFact } from "./change.mjs";
import { cleanUpOf } from "./clean-up.mjs";
import { show } from "./io.mjs";
import { nextText } from "./handovers.mjs";
import { pink } from "./names.mjs";
import { numbersOf } from "./numbering.mjs";
import { ago, mrState, tokens } from "./render-worktree.mjs";
import { screen } from "./screen.mjs";

/** A time of today as `15:12`, an older one with its day. */
const clock = at => {
  const date = new Date(at);
  const time = date.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
  return date.toDateString() === new Date().toDateString()
    ? time
    : `${date.toLocaleDateString([], { day: "numeric", month: "short" })} ${time}`;
};

/** `10. ` for the tenth, as wide as the last of `count`, so the features line up. */
const placeCell = (count, number) =>
  dim(`${String(number).padStart(String(count).length)}. `);
/**
 * Where a worktree stands in its life: `{ key, text }`. `issue` as issuesOf
 * gives it (its `tree` while the worktree is there, with `ownCommits`: commits
 * develop doesn't have), `mr` its merge request, `status` herdr's state of its
 * agent, `hold` why a handover holds the agent, `removal` the automatic
 * wt:remove's `{ isRunning, error }` (auto-actions.mjs), `merged` the iids
 * known merged (a wait names only the others, #262). Merged comes first:
 * what is left then is the clean-up (clean-up.mjs).
 * - `removable`: merged, nothing left in it: the overview removes it
 * - `kept`: merged, but changes or commits not in develop keep it
 * - `held`: merged, its clean-up waits for its agent (at work, a handover)
 * - `asks`: its agent is blocked on you; `waits`: an action of it is open
 *   (`waits for you: <kinds>`); `waitsForMr`: it waits for MRs (wt:wait)
 * - `working`: its agent is at work; `ready`: the MR is ready, pipeline passed
 * - `idle`: none of those; `removed`: gone (`merged: removed 21:14`); `skipped`: never started, why;
 *   `queued`: waits for an agent (#217, workers.mjs)
 */
export const lifecycleOf = (
  issue,
  { mr, status, hold, removal, merged } = {},
) => {
  const { tree } = issue;
  if (!tree)
    return issue.queued
      ? { key: "queued", text: cyan("queued: waits for an agent") }
      : issue.why
        ? { key: "skipped", text: yellow(issue.why) }
        : { key: "removed", text: dim("removed") };
  const cleanUp = cleanUpOf(issue, { mr, status, hold });
  if (cleanUp?.key === "kept")
    return { key: "kept", text: yellow(`kept: ${cleanUp.why}`) };
  if (cleanUp?.key === "waits")
    return { key: "held", text: yellow(`waits: ${cleanUp.why}`) };
  if (cleanUp)
    return removal?.error
      ? { key: "removable", text: red(`merged: ${removal.error}`) }
      : {
          key: "removable",
          text: cyan(
            removal?.isRunning
              ? "merged: removing …"
              : "merged: will be removed",
          ),
        };
  // The agent's state in the colour herdr's sidebar gives it (herdr-tones.mjs),
  // what it waits for before what it does (#260).
  const tone = toneOf(status);
  if (status === "blocked")
    return { key: "asks", text: tone("waits for you: asks") };
  const open = tree.actions?.filter(action => !action.answer) ?? [];
  if (open.length)
    return {
      key: "waits",
      text: tone(
        `waits for you: ${[...new Set(open.map(action => action.kind))].join(", ")}`,
      ),
    };
  if (tree.wait)
    return { key: "waitsForMr", text: tone(waitsText(tree.wait, merged)) };
  if (status === "working") return { key: "working", text: tone("at work") };
  const pipeline = mr?.pipeline?.status ?? mr?.pipeline;
  if (
    mr?.state === "opened" &&
    !mr.isDraft &&
    pipeline === "success" &&
    !isChangeOpen(tree, mr)
  )
    return { key: "ready", text: green("ready to merge") };
  return {
    key: "idle",
    text: tree.claude ? tone("idle") : dim("no agent"),
  };
};

/** The issue of a node: its link and state. */
const issueFact = (issue, found, hasRemote) => {
  if (!issue.number) return "";
  const state = found
    ? found.state === "closed"
      ? cyan("closed")
      : "open"
    : dim(hasRemote ? "?" : "…");
  return `${link(found?.url, `#${issue.number}`)} ${state}`;
};
/** The MR of a node: its link and state, the pipeline while it is open, when a merged one was merged. */
const mrFact = (mr, mergedAt) => {
  if (!mr) return mergedAt ? dim(`merged ${clock(mergedAt)}`) : "";
  const pipeline = mr.state === "opened" ? ` ${pipelineMark(mr.pipeline)}` : "";
  const when =
    mr.state === "merged" && mergedAt ? ` ${dim(clock(mergedAt))}` : "";
  return `${link(mr.url, `!${mr.iid}`)}${mrState(mr)}${pipeline}${when}`;
};
/** A worktree's work: commits ahead of develop, uncommitted changes; nothing for a clean one. */
const workFact = tree =>
  [
    tree.ahead ? `${tree.ahead} ahead` : "",
    tree.changed ? yellow(`${tree.changed} changed`) : "",
  ]
    .filter(Boolean)
    .join(" · ");
/** An agent: how full its context is and when it last answered; nothing without one. */
const agentFact = claude => {
  if (!claude) return "";
  if (!claude.at) return dim("no answer yet");
  const share = Math.min(claude.used / claude.window, 1);
  const tone = share >= 0.8 ? red : share >= 0.5 ? yellow : green;
  return `${tone(`${Math.round(share * 100)}%`)} ${dim(`of ${tokens(claude.window)} · ${ago(claude.at)}`)}`;
};
/**
 * The agent of a worktree by its name when that isn't the worktree's (a
 * handover, #251), its name before for a day; nothing else.
 */
const agentNameFact = (tree, known, now) => {
  const worker = tree?.claude && known[tree.claude.id];
  const was = wasOf(worker, now);
  return [
    worker?.name && worker.name !== tree.name
      ? `agent ${bold(worker.name)}`
      : "",
    was ? dim(`was ${was}`) : "",
  ]
    .filter(Boolean)
    .join(" ");
};
/**
 * The facts line of a node: issue, MR and pipeline, work, its agent's name
 * after a handover, which issue of its agent this is (#264) and its context, a wait (#216) its state doesn't name
 * already, a requested change and what isn't pushed (#228), the next issue
 * announced for its agent (#227). What doesn't apply is left out; a worktree
 * without an MR says so.
 */
const factsOf = (
  issue,
  found,
  mr,
  hasRemote,
  { handover, state, agent, history, merged },
) => {
  const { tree } = issue;
  const mergedAt = issue.mergedAt ?? mr?.mergedAt;
  return [
    issueFact(issue, found, hasRemote),
    tree ? mrFact(mr, mergedAt) || dim("no MR yet") : mrFact(mr, mergedAt),
    tree ? workFact(tree) : "",
    agent,
    history ? dim(history) : "",
    tree ? agentFact(tree.claude) : "",
    tree?.wait && state.key !== "waitsForMr"
      ? yellow(waitsText(tree.wait, merged))
      : "",
    tree ? changeFact(tree, mr) : "",
    tree ? unpushedFact(tree) : "",
    // Its agent's next issue, announced (#227).
    handover ? cyan(nextText(handover)) : "",
  ]
    .filter(Boolean)
    .join(" · ");
};

/** `→ 242-overview after !256` / `→ 242-overview now`: an announced handover's agent and what it waits for. */
const takenText = ({ to, open, after }) => {
  const mrs = open ?? after ?? [];
  return `→ ${to} ${mrs.length ? `after ${mrs.map(iid => `!${iid}`).join(", ")}` : "now"}`;
};
/**
 * What a queued issue waits for (#250): the agent the orchestrator announced
 * (and after which MRs), the free one UP NEXT suggests (`→ 3-x (free, 18 %)`),
 * else what its fresh start does (`fresh`, #265).
 */
const queuedText = (issue, announced, suggested, fresh) => {
  const handover = announced.find(({ number }) => number === issue.number);
  if (handover) return cyan(takenText(handover));
  const worker = suggested.get(issue.number)?.worker;
  if (worker)
    return cyan(
      worker.isMerged
        ? `→ ${worker.name} (free, ${Math.round(worker.share * 100)} %)`
        : `→ ${worker.name} once its MR is merged`,
    );
  return cyan(fresh.get(issue.number) ?? "start: waits for an agent");
};

/**
 * The tree of an orchestration's issues, each numbered (numbering.mjs):
 * `live`, the worktrees there and the issues that never got one, by their
 * number, `queued`, the issues that wait for an agent in the order they
 * start: the work order, then when they were queued (#250), `removed`, the ones whose worktree is gone, the last removed
 * first. Each `{ issue, state, place, name, facts, title }`: `place` its
 * number in front of `name`, the feature in pink (bold while its worktree
 * is there); `facts` and `title` go below it.
 * `of(issue)`: its part of GitLab; `agents`: agentsOf(); `work`: the work
 * order of wt:plan; `merge`: the open MRs' issues in the computed merge order;
 * `announced`: the announced handovers (handovers.mjs); `holds`: dir → why
 * the clean-up of a worktree waits for a handover (workers.mjs);
 * `suggested`: number → `{ worker }` of UP NEXT, `fresh`: number → what the
 * fresh start of an issue no agent is free for does (workers.mjs); `known`: the
 * record's workers by agent id (their names, #217); `merged`: the iids known
 * merged, left out of a wait (#262).
 */
export const treeOf = (
  issues,
  of,
  {
    agents = new Map(),
    work = [],
    merge = [],
    announced = [],
    holds = new Map(),
    suggested = new Map(),
    fresh = new Map(),
    hasRemote = true,
    known = {},
    merged = new Set(),
    now = Date.now(),
  } = {},
) => {
  const numbers = numbersOf(issues, { work, merge, of });
  const handoverOf = tree =>
    tree && announced.find(handover => handover.tree?.dir === tree.dir);
  const rows = issues.map(issue => {
    const { mr, issue: found } = of(issue);
    const { tree } = issue;
    const status = tree?.claude ? agents.get(tree.claude.id)?.status : null;
    const lifecycle = lifecycleOf(issue, {
      mr,
      status,
      hold: tree && holds.get(tree.dir),
      removal: tree && removalOf(tree.dir),
      merged,
    });
    const state =
      lifecycle.key === "queued"
        ? { ...lifecycle, text: queuedText(issue, announced, suggested, fresh) }
        : lifecycle.key === "removed" && issue.removedAt
          ? {
              ...lifecycle,
              text: dim(
                `${issue.mergedAt || mr?.state === "merged" ? "merged: " : ""}removed ${clock(issue.removedAt)}`,
              ),
            }
          : lifecycle;
    return {
      issue,
      state,
      number: numbers.get(issue),
      place: placeCell(issues.length, numbers.get(issue)),
      name: pink(tree ? bold(issue.feature) : issue.feature),
      facts: factsOf(issue, found, mr, hasRemote, {
        handover: handoverOf(tree),
        state,
        agent: agentNameFact(tree, known, now),
        history: historyText(known, issue.number, tree?.claude),
        merged,
      }),
      title: dim(found?.title ?? issue.title ?? issue.branch),
    };
  });
  const rank = row => {
    const at = work.indexOf(row.issue.number);
    return at === -1 ? Infinity : at;
  };
  return {
    live: rows
      .filter(row => !["removed", "queued"].includes(row.state.key))
      .toSorted((a, b) => a.number - b.number),
    queued: rows
      .filter(row => row.state.key === "queued")
      .toSorted(
        (a, b) =>
          (rank(a) === rank(b) ? 0 : rank(a) < rank(b) ? -1 : 1) ||
          (a.issue.queuedAt ?? "").localeCompare(b.issue.queuedAt ?? "") ||
          a.number - b.number,
      ),
    removed: rows
      .filter(row => row.state.key === "removed")
      .toSorted((a, b) =>
        (b.issue.removedAt ?? "").localeCompare(a.issue.removedAt ?? ""),
      ),
  };
};

/** How many lines the facts, and the title, of a node may wrap over each. */
const MAX_BELOW = 3;
/**
 * The lines of a node, `width` wide: the branch glyph, place and name with
 * its state at the right end, then its facts and title indented under the
 * name, `stem` in front (the tree's `│`, blanks at its last node).
 */
export const nodeLines = (row, { branch, stem, width }) => {
  const head = `${dim(branch)}${row.place}`;
  const room = width - visible(head);
  const state = fit(row.state.text, Math.max(8, Math.floor(room / 2)));
  const name = fit(row.name, Math.max(1, room - visible(state) - 2));
  const first = `${head}${pad(name, room - visible(state))}${state}`;
  const indent = `${dim(stem)}${" ".repeat(visible(head) - visible(stem))}`;
  const below = Math.max(1, width - visible(indent));
  return [
    first,
    ...[row.facts, row.title]
      .filter(text => visible(text) > 0)
      .flatMap(text => wrap(text, below, MAX_BELOW))
      .map(line => `${indent}${line}`),
  ];
};

/**
 * The head line of WORKTREES after its title (#260): how many agents work,
 * wait (for you or for MRs) and idle, the ready ones, the ones about to go,
 * queued and removed, then the free memory and the threshold below which a
 * fresh agent gets the shared stack (memory.mjs), red below it.
 */
export const headOf = ({ live, queued, removed }, free) => {
  const count = (...keys) =>
    live.filter(row => keys.includes(row.state.key)).length;
  const counts = [
    [count("working"), "working"],
    [count("asks", "waits", "waitsForMr"), "waiting"],
    [count("idle"), "idle"],
    [count("ready"), "ready"],
    [count("removable"), "will be removed"],
    [queued.length, "queued"],
    [removed.length, "removed"],
  ]
    .filter(([n]) => n)
    .map(([n, what]) => `${n} ${what}`)
    .join(" · ");
  const memory =
    free === undefined
      ? ""
      : free < THRESHOLD
        ? red(`${gb(free)} free: fresh agents on the shared stack`)
        : dim(`${gb(free)} free (shared stack below ${gb(THRESHOLD)})`);
  return [counts && dim(counts), memory].filter(Boolean).join(dim(" · "));
};

/** Past this many, "done and removed" starts folded into its one line. */
const FOLD_OVER = 3;
/** Whether the user unfolded (true) or folded (false) "done and removed", over redraws; null: by FOLD_OVER. */
let isRemovedOpen = null;
/** Whether the user folded the queue's node, over redraws. */
let isQueueFolded = false;
/** Fold the queue's node, or show it again (its click). */
export const toggleQueue = () => {
  isQueueFolded = !isQueueFolded;
};

/**
 * A node that folds what it holds, like "done and removed": `▾` open, `▸`
 * folded, the hint for its click (none in --once), cut to `width`.
 */
export const foldLine = ({ branch, isOpen, text, width }) =>
  dim(
    fit(
      `${branch}${isOpen ? "▾" : "▸"} ${text}${isOnce ? "" : ` · click to ${isOpen ? "fold" : "show"}`}`,
      width,
    ),
  );

/**
 * WORKTREES: the orchestration `name` at the root, a node per worktree with
 * its state, its facts and title below it, then the queue as a node of the
 * tree (#259, a click folds it) with each issue and what it waits for under
 * it, then "done and removed" with the gone ones, folded into one line when
 * long (a click shows or folds them).
 * A click on any line of a node goes to the worktree's space, on a gone one
 * to its issue. Its head line counts them and names the free memory (#260).
 */
export const pushWorktrees = (
  lines,
  {
    name,
    issues,
    of,
    agents,
    work,
    merge,
    announced,
    holds,
    suggested,
    fresh,
    hasRemote,
    known,
    free,
    merged,
  },
) => {
  const { live, queued, removed } = treeOf(issues, of, {
    merged,
    agents,
    work,
    merge,
    announced,
    holds,
    suggested,
    fresh,
    hasRemote,
    known,
  });
  const box = openBox(lines);
  lines.push(
    `  ${bold(`${ORCHESTRATOR} ${name}`)}  ${dim(`.claude/orchestrations/${name}.json`)}`,
  );
  // How reuse works (#264), as the root's facts: agents, their issues, reused ones, failed handovers.
  lines.push(
    `  ${dim(fit(`${live.length + queued.length + removed.length ? "│" : " "}  ${reuseText(reuseOf(issues, known))}`, boxInner()))}`,
  );
  const isOpen = isRemovedOpen ?? removed.length <= FOLD_OVER;
  const shown = isOpen ? removed : [];
  const width = boxInner();
  const go = issue => () => {
    if (issue.tree && focusSpace(issue.tree.dir)) return;
    const url = of(issue).issue?.url;
    if (url) show(url, issue.tree?.dir);
  };
  const pushNode = (row, isLast, level = "") =>
    nodeLines(row, {
      branch: `${level}${isLast ? "└─ " : "├─ "}`,
      stem: `${level}${isLast ? "   " : "│  "}`,
      width,
    }).forEach(line => {
      screen.clicks[lines.length] = go(row.issue);
      lines.push(`  ${line}`);
    });
  live.forEach((row, i) =>
    pushNode(
      row,
      i === live.length - 1 && queued.length + removed.length === 0,
    ),
  );
  if (queued.length) {
    const isLast = removed.length === 0;
    if (!isOnce)
      screen.clicks[lines.length] = () => {
        toggleQueue();
        screen.redraw();
      };
    lines.push(
      `  ${foldLine({
        branch: isLast ? "└─ " : "├─ ",
        isOpen: !isQueueFolded,
        text: `queued (${queued.length}) · in the order they start`,
        width,
      })}`,
    );
    const queuedShown = isQueueFolded ? [] : queued;
    queuedShown.forEach((row, i) =>
      pushNode(row, i === queuedShown.length - 1, isLast ? "   " : "│  "),
    );
  }
  if (removed.length) {
    screen.clicks[lines.length] = () => {
      isRemovedOpen = !isOpen;
      screen.redraw();
    };
    lines.push(
      `  ${foldLine({
        branch: "└─ ",
        isOpen,
        text: `done and removed (${removed.length})`,
        width,
      })}`,
    );
  }
  shown.forEach((row, i) => pushNode(row, i === shown.length - 1, "   "));
  closeBox(
    lines,
    box,
    `${bold("WORKTREES")} ${headOf({ live, queued, removed }, free)}`,
  );
};
