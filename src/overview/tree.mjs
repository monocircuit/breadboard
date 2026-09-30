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
} from "../worktree.mjs";
import { isOnce } from "./args.mjs";
import { removalOf } from "./auto-actions.mjs";
import { boxInner, closeBox, openBox, pipelineMark } from "./boxes.mjs";
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
 * wt:remove's `{ isRunning, error }` (auto-actions.mjs). Merged comes first:
 * what is left then is the clean-up (clean-up.mjs).
 * - `removable`: merged, nothing left in it: the overview removes it
 * - `kept`: merged, but changes or commits not in develop keep it
 * - `held`: merged, its clean-up waits for its agent (at work, a handover)
 * - `asks`: its agent is blocked on you; `waits`: an action of it is open
 * - `working`: its agent is at work; `ready`: the MR is ready, pipeline passed
 * - `idle`: none of those; `removed`: gone (`merged: removed 21:14`); `skipped`: never started, why;
 *   `queued`: waits for an agent (#217, workers.mjs)
 */
export const lifecycleOf = (issue, { mr, status, hold, removal } = {}) => {
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
  if (status === "blocked") return { key: "asks", text: red("asks you") };
  if (tree.actions?.some(action => !action.answer))
    return { key: "waits", text: yellow("waits for you") };
  if (status === "working") return { key: "working", text: yellow("working") };
  const pipeline = mr?.pipeline?.status ?? mr?.pipeline;
  if (
    mr?.state === "opened" &&
    !mr.isDraft &&
    pipeline === "success" &&
    !isChangeOpen(tree, mr)
  )
    return { key: "ready", text: green("ready to merge") };
  return { key: "idle", text: dim(tree.claude ? "idle" : "no agent") };
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
 * The facts line of a node: issue, MR and pipeline, work, agent, a wait
 * (#216), a requested change and what isn't pushed (#228), the next issue
 * announced for its agent (#227). What doesn't apply is left out; a worktree
 * without an MR says so.
 */
const factsOf = (issue, found, mr, hasRemote, handover) => {
  const { tree } = issue;
  const mergedAt = issue.mergedAt ?? mr?.mergedAt;
  return [
    issueFact(issue, found, hasRemote),
    tree ? mrFact(mr, mergedAt) || dim("no MR yet") : mrFact(mr, mergedAt),
    tree ? workFact(tree) : "",
    tree ? agentFact(tree.claude) : "",
    tree?.wait ? yellow(waitsText(tree.wait)) : "",
    tree ? changeFact(tree, mr) : "",
    tree ? unpushedFact(tree) : "",
    // Its agent's next issue, announced (#227).
    handover ? cyan(nextText(handover)) : "",
  ]
    .filter(Boolean)
    .join(" · ");
};

/**
 * The tree of an orchestration's issues, each numbered (numbering.mjs):
 * `live`, the worktrees there and the issues that never got one, by their
 * number, `removed`, the ones whose worktree is gone, the last removed
 * first. Each `{ issue, state, place, name, facts, title }`: `place` its
 * number in front of `name`, the feature in pink (bold while its worktree
 * is there); `facts` and `title` go below it.
 * `of(issue)`: its part of GitLab; `agents`: agentsOf(); `work`: the work
 * order of wt:plan; `merge`: the open MRs' issues in the computed merge order;
 * `announced`: the announced handovers (handovers.mjs); `holds`: dir → why
 * the clean-up of a worktree waits for a handover (workers.mjs).
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
    hasRemote = true,
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
    });
    const state =
      lifecycle.key === "removed" && issue.removedAt
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
      facts: factsOf(issue, found, mr, hasRemote, handoverOf(tree)),
      title: dim(found?.title ?? issue.title ?? issue.branch),
    };
  });
  return {
    live: rows
      .filter(row => row.state.key !== "removed")
      .toSorted((a, b) => a.number - b.number),
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

/** Past this many, "done and removed" starts folded into its one line. */
const FOLD_OVER = 3;
/** Whether the user unfolded (true) or folded (false) "done and removed", over redraws; null: by FOLD_OVER. */
let isRemovedOpen = null;

/**
 * WORKTREES: the orchestration `name` at the root, a node per worktree with
 * its state, its facts and title below it, then "done and removed" with the
 * gone ones, folded into one line when long (a click shows or folds them).
 * A click on any line of a node goes to the worktree's space, on a gone one
 * to its issue.
 */
export const pushWorktrees = (
  lines,
  { name, issues, of, agents, work, merge, announced, holds, hasRemote },
) => {
  const { live, removed } = treeOf(issues, of, {
    agents,
    work,
    merge,
    announced,
    holds,
    hasRemote,
  });
  const box = openBox(lines);
  lines.push(
    `  ${bold(`${ORCHESTRATOR} ${name}`)}  ${dim(`.claude/orchestrations/${name}.json`)}`,
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
    pushNode(row, i === live.length - 1 && removed.length === 0),
  );
  if (removed.length) {
    const hint = isOnce ? "" : ` · click to ${isOpen ? "fold" : "show"}`;
    screen.clicks[lines.length] = () => {
      isRemovedOpen = !isOpen;
      screen.redraw();
    };
    lines.push(
      `  ${dim("└─ ")}${dim(`${isOpen ? "▾" : "▸"} done and removed (${removed.length})${hint}`)}`,
    );
  }
  shown.forEach((row, i) => pushNode(row, i === shown.length - 1, "   "));
  const count = key => live.filter(row => row.state.key === key).length;
  const counts = [
    count("working") ? `${count("working")} working` : "",
    count("asks") + count("waits")
      ? `${count("asks") + count("waits")} wait for you`
      : "",
    count("ready") ? `${count("ready")} ready` : "",
    count("removable") ? `${count("removable")} will be removed` : "",
    count("queued") ? `${count("queued")} queued` : "",
    removed.length ? `${removed.length} removed` : "",
  ];
  closeBox(
    lines,
    box,
    `${bold("WORKTREES")} ${dim([...counts, "click one to go to its space"].filter(Boolean).join(" · "))}`,
  );
};
