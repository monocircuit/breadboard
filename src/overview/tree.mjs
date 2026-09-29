// WORKTREES of an orchestration: its worktrees as a folder tree under the
// orchestration, each with where it stands in its life, from what the
// overview reads anyway (git, herdr, GitLab, the record): at work, waiting
// for you, ready, merged and about to go, gone. The ones wt:remove took are
// in the record with their times (markRemoved), under "done and removed".
import {
  bold,
  cyan,
  dim,
  green,
  link,
  red,
  tableRows,
  yellow,
} from "../lib/tty.mjs";
import { ORCHESTRATOR, focusSpace } from "../worktree.mjs";
import { isOnce } from "./args.mjs";
import { boxInner, closeBox, openBox, pipelineMark } from "./boxes.mjs";
import { show } from "./io.mjs";
import { ago, mrState, tokens } from "./render-worktree.mjs";
import { screen } from "./screen.mjs";

/** An agent in a cell: when it last answered and how full its context is. */
const agentCell = claude => {
  if (!claude) return dim("no agent");
  if (!claude.at) return dim("no answer yet");
  const share = Math.min(claude.used / claude.window, 1);
  const tone = share >= 0.8 ? red : share >= 0.5 ? yellow : green;
  return `${tone(`${Math.round(share * 100)}%`)} ${dim(`of ${tokens(claude.window)} · ${ago(claude.at)}`)}`;
};
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

/** `10. ` for the tenth of the work order, as wide as its last; blanks for one it leaves out, so the features line up. */
const placeCell = (work, place) => {
  const width = String(work.length).length + 2;
  return place === -1
    ? work.length
      ? " ".repeat(width)
      : ""
    : dim(`${String(place + 1).padStart(width - 2)}. `);
};
/**
 * Where a worktree stands in its life: `{ key, text }`. `issue` as issuesOf
 * gives it (its `tree` while the worktree is there, with `ownCommits`: commits
 * develop doesn't have), `mr` its merge request, `status` herdr's state of its
 * agent. Merged comes first: what is left then is the clean-up.
 * - `removable`: merged, nothing left in it: what wt:remove / clean up takes
 * - `kept`: merged, but changes or commits not in develop keep it
 * - `asks`: its agent is blocked on you; `waits`: an action of it is open
 * - `working`: its agent is at work; `ready`: the MR is ready, pipeline passed
 * - `idle`: none of those; `removed`: gone; `skipped`: never started, why;
 *   `queued`: waits for an agent (#217, workers.mjs)
 */
export const lifecycleOf = (issue, { mr, status } = {}) => {
  const { tree } = issue;
  if (!tree)
    return issue.queued
      ? { key: "queued", text: cyan("queued: waits for an agent") }
      : issue.why
        ? { key: "skipped", text: yellow(issue.why) }
        : { key: "removed", text: dim("removed") };
  if (mr?.state === "merged") {
    const why = [
      tree.changed ? `${tree.changed} changed` : "",
      tree.ownCommits ? "commits not in develop" : "",
    ]
      .filter(Boolean)
      .join(", ");
    return why
      ? { key: "kept", text: yellow(`merged: kept, ${why}`) }
      : { key: "removable", text: cyan("merged: will be removed") };
  }
  if (status === "blocked") return { key: "asks", text: red("asks you") };
  if (tree.actions?.some(action => !action.answer))
    return { key: "waits", text: yellow("waits for you") };
  if (status === "working") return { key: "working", text: yellow("working") };
  const pipeline = mr?.pipeline?.status ?? mr?.pipeline;
  if (mr?.state === "opened" && !mr.isDraft && pipeline === "success")
    return { key: "ready", text: green("ready to merge") };
  return { key: "idle", text: dim(tree.claude ? "idle" : "no agent") };
};

/** The issue of a row: its link and state. */
const issueCell = (issue, found, hasRemote) => {
  if (!issue.number) return dim("—");
  const state = found
    ? found.state === "closed"
      ? cyan("closed")
      : "open"
    : dim(hasRemote ? "?" : "…");
  return `${link(found?.url, `#${issue.number}`)} ${state}`;
};
/** The MR of a row: its link and state, the pipeline while it is open. */
const mrCell = mr =>
  mr
    ? `${link(mr.url, `!${mr.iid}`)}${mrState(mr)}${mr.state === "opened" ? ` ${pipelineMark(mr.pipeline)}` : ""}`
    : "";
/** A worktree's work: commits ahead of develop, uncommitted changes. */
const workCell = tree =>
  [
    tree.ahead ? `${tree.ahead} ahead` : "",
    tree.changed ? yellow(`${tree.changed} changed`) : "",
  ]
    .filter(Boolean)
    .join(" · ") || dim("clean");
/** When a removed one was merged and removed, as far as the record or GitLab know. */
const timesCell = (issue, mr) =>
  dim(
    [
      (issue.mergedAt ?? mr?.mergedAt)
        ? `merged ${clock(issue.mergedAt ?? mr.mergedAt)}`
        : "",
      issue.removedAt ? `removed ${clock(issue.removedAt)}` : "",
    ]
      .filter(Boolean)
      .join(" · "),
  );

/**
 * The tree of an orchestration's issues, in the order given (the work
 * order): `live`, the worktrees there and the issues that never got one,
 * `removed`, the ones whose worktree is gone, the last removed first. Each
 * `{ issue, state, cells }`, the cells without the tree's lines: the
 * feature, state, issue, MR, work, agent or times, title (the state early:
 * on a narrow screen the last ones are cut).
 * `of(issue)`: its part of GitLab; `agents`: agentsOf(); `work`: the order.
 */
export const treeOf = (
  issues,
  of,
  agents = new Map(),
  work = [],
  hasRemote = true,
) => {
  const rows = issues.map(issue => {
    const { mr, issue: found } = of(issue);
    const { tree } = issue;
    const status = tree?.claude ? agents.get(tree.claude.id)?.status : null;
    const state = lifecycleOf(issue, { mr, status });
    const place = work.indexOf(issue.number);
    return {
      issue,
      state,
      cells: [
        `${placeCell(work, place)}${tree ? bold(issue.feature) : issue.feature}`,
        state.text,
        issueCell(issue, found, hasRemote),
        mrCell(mr),
        tree ? workCell(tree) : "",
        tree ? agentCell(tree.claude) : timesCell(issue, mr),
        found?.title ?? issue.title ?? dim(issue.branch),
      ],
    };
  });
  return {
    live: rows.filter(row => row.state.key !== "removed"),
    removed: rows
      .filter(row => row.state.key === "removed")
      .toSorted((a, b) =>
        (b.issue.removedAt ?? "").localeCompare(a.issue.removedAt ?? ""),
      ),
  };
};

/** Past this many, "done and removed" starts folded into its one line. */
const FOLD_OVER = 3;
/** Whether the user unfolded (true) or folded (false) "done and removed", over redraws; null: by FOLD_OVER. */
let isRemovedOpen = null;

/**
 * WORKTREES: the orchestration `name` at the root, a branch per worktree
 * with its state, then "done and removed" with the gone ones, folded into
 * one line when long (a click shows or folds them). A click on a row goes to
 * the worktree's space, on a gone one to its issue.
 */
export const pushWorktrees = (
  lines,
  { name, issues, of, agents, work, hasRemote },
) => {
  const { live, removed } = treeOf(issues, of, agents, work, hasRemote);
  const box = openBox(lines);
  lines.push(
    `  ${bold(`${ORCHESTRATOR} ${name}`)}  ${dim(`.claude/orchestrations/${name}.json`)}`,
  );
  const isOpen = isRemovedOpen ?? removed.length <= FOLD_OVER;
  const shown = isOpen ? removed : [];
  // One table for both levels: their columns line up.
  const rows = [
    ...live.map((row, i) => ({
      ...row,
      branch: i === live.length - 1 && removed.length === 0 ? "└─ " : "├─ ",
    })),
    ...shown.map((row, i) => ({
      ...row,
      branch: i === shown.length - 1 ? "   └─ " : "   ├─ ",
    })),
  ];
  const go = issue => () => {
    if (issue.tree && focusSpace(issue.tree.dir)) return;
    const url = of(issue).issue?.url;
    if (url) show(url, issue.tree?.dir);
  };
  const drawn = tableRows(
    rows.map(row => [
      "",
      `${dim(row.branch)}${row.cells[0]}`,
      ...row.cells.slice(1),
    ]),
    { width: boxInner() },
  );
  const pushRow = (rowLines, action) =>
    rowLines.forEach(line => {
      screen.clicks[lines.length] = action;
      lines.push(line);
    });
  drawn
    .slice(0, live.length)
    .forEach((rowLines, i) => pushRow(rowLines, go(live[i].issue)));
  if (removed.length) {
    const hint = isOnce ? "" : ` · click to ${isOpen ? "fold" : "show"}`;
    pushRow(
      [
        `  ${dim("└─ ")}${dim(`${isOpen ? "▾" : "▸"} done and removed (${removed.length})${hint}`)}`,
      ],
      () => {
        isRemovedOpen = !isOpen;
        screen.redraw();
      },
    );
  }
  drawn
    .slice(live.length)
    .forEach((rowLines, i) => pushRow(rowLines, go(shown[i].issue)));
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
