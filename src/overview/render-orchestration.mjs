// The view of one orchestration (--orchestration): what to do next, what the
// agents ask for, its worktrees and their merge requests, the open ones in
// the merge order the overview computes (merge-order.mjs), the merged below.
import {
  bold,
  cyan,
  dim,
  fit,
  green,
  link,
  red,
  tableRows,
  visible,
  yellow,
} from "../lib/tty.mjs";
import {
  ORCHESTRATOR,
  actionsFile,
  focusSpace,
  waitsText,
} from "../worktree.mjs";
import { writePlan } from "../worktree/origins.mjs";
import { answerCell, clickAction, deliversAnswers } from "./answers.mjs";
import { isOnce, orchestration } from "./args.mjs";
import {
  AUTO_KIND,
  autoActionsOf,
  notifyNewActions,
  tellConflicts,
} from "./auto-actions.mjs";
import {
  boxInner,
  closeBox,
  openBox,
  pipelineMark,
  titleLines,
} from "./boxes.mjs";
import { clickEnd, endText, pushEndButton } from "./end.mjs";
import { pushFeed } from "./feed.mjs";
import { open, show } from "./io.mjs";
import { clickMerge, mergeText, openAfter } from "./merge.mjs";
import { graphOf, mergeOrderOf } from "./merge-order.mjs";
import { orderActions, placeIn } from "./order.mjs";
import { pushAgents } from "./agents.mjs";
import { ago } from "./render-worktree.mjs";
import { workerActionsOf } from "./workers.mjs";
import { KEYS, pushRows, screen } from "./screen.mjs";
import { pushWorktrees } from "./tree.mjs";
import { clickTry, pageOf, trySpot } from "./try.mjs";

/** How an action shows its kind. */
const KIND = {
  merge: green("merge"),
  try: cyan("try"),
  "browser-check": cyan("try"),
  decide: yellow("decide"),
  look: "look",
  plan: cyan("plan"),
  end: red("end"),
  order: cyan("order"),
  "hand over": cyan("hand over"),
  start: cyan("start"),
  ...AUTO_KIND,
};
/** The kinds whose click opens the spot to check; `browser-check` is stored before `try` came. */
const TRY_KINDS = new Set(["try", "browser-check"]);
/** In place of the buttons of an automatic action: it goes by itself when its cause goes. */
const AUTO = dim("auto");

/**
 * Every open action, split by order.mjs: ACTIONS, what needs the user (the
 * orchestrator's and the agents' own, the automatic `waits` and fallback
 * `merge`), by what blocks most; UP NEXT, the automatic follow-ups (`clean
 * up`, `pipeline`, and `end` once all is merged and no worktree is left),
 * by merge order. A merge whose MR is merged is left out.
 * Each with what a click on it does and where its answer goes (answers.mjs).
 */
const actionsOf = (local, issues, of, remote) => {
  const merge = local.order.numbers;
  const own = (local.ownActions ?? []).map(action => ({
    action,
    who: `${ORCHESTRATOR} orchestrator`,
    file: local.recordFile,
    to: { orchestration },
    rank: -1,
    // The overview stands in the orchestrator's space already, next to its tab;
    // a page of its try is one on the app of the main checkout.
    go: () => action.url && open(pageOf(action.url, 0)),
  }));
  const agents = issues
    .filter(issue => issue.tree?.actions?.length)
    .flatMap(issue =>
      issue.tree.actions.map(action => ({
        action,
        issue,
        who: issue.feature,
        file: actionsFile(issue.tree.dir),
        to: { dir: issue.tree.dir },
        rank: placeIn(merge, issue.number),
        go: () => {
          const { dir } = issue.tree;
          const { mr } = of(issue);
          if (action.kind === "merge" && mr)
            clickMerge({
              issue,
              mr,
              boxes: of(issue).issue?.boxes ?? [],
              others: openAfter(issues, issue.number, merge, of),
              file: actionsFile(dir),
              actionId: action.id,
            });
          else if (TRY_KINDS.has(action.kind))
            clickTry(action, issue.tree, () => focusSpace(dir));
          else focusSpace(dir);
        },
      })),
    )
    .filter(
      ({ action, issue }) =>
        action.kind !== "merge" || of(issue).mr?.state !== "merged",
    );
  // The next issue to an idle agent that knows its code (workers.mjs): first
  // of UP NEXT, and the clean-up of its worktree waits, it would close the agent.
  const handovers = workerActionsOf(
    orchestration,
    issues,
    of,
    local.agents ?? new Map(),
    local.workers,
    // Queued issues go to their agent by themselves, from the one live overview that also delivers the answers.
    { shouldHandOut: Boolean(remote) && deliversAnswers(orchestration) },
  );
  const workers = handovers.actions.map(
    ({ key, go, issue, who, ...action }) => ({
      action,
      issue,
      who,
      key,
      isAuto: true,
      rank: -1,
      go,
    }),
  );
  const auto = autoActionsOf(issues, of, local.agents ?? new Map())
    .filter(
      ({ key, issue }) =>
        !(key.endsWith(":remove") && handovers.held.has(issue.tree?.dir)),
    )
    .map(({ key, go, issue, ...action }) => ({
      action,
      issue,
      who: issue.feature,
      key,
      isAuto: true,
      rank: placeIn(merge, issue.number),
      // A merge found by itself merges on two clicks too.
      go:
        action.kind === "merge"
          ? () =>
              clickMerge({
                issue,
                mr: of(issue).mr,
                boxes: of(issue).issue?.boxes ?? [],
                others: openAfter(issues, issue.number, merge, of),
              })
          : go,
    }));
  // The end: its click is the first of the ✕'s two.
  const end =
    remote &&
    local.trees.length === 0 &&
    !issues.some(issue => issue.queued) &&
    !nextMerge(local, issues, of)
      ? [
          {
            action: { kind: "end", text: endText(orchestration), at: "" },
            who: `${ORCHESTRATOR} orchestrator`,
            key: `end:${orchestration}`,
            isAuto: true,
            rank: Infinity,
            go: () => clickEnd(orchestration),
          },
        ]
      : [];
  // The computed merge order left the plan's: a click writes it into the plan.
  const { order } = local;
  const replan =
    remote && order.isOffPlan
      ? [
          {
            action: {
              kind: "order",
              text: `${local.plan?.merge?.length ? "the merge order left the plan's" : "no merge order in the plan yet"}: ${order.open.map(item => `#${item.number}`).join(" → ")} · click to write it into the plan (wt:plan)`,
              at: "",
            },
            who: `${ORCHESTRATOR} orchestrator`,
            key: `order:${orchestration}:${order.planned.join(",")}`,
            isAuto: true,
            rank: -1,
            go: () => {
              writePlan(orchestration, { merge: order.planned });
              screen.redraw({ isReread: true });
            },
          },
        ]
      : [];
  return {
    ...orderActions([
      ...own,
      ...agents,
      ...auto,
      ...workers,
      ...end,
      ...replan,
    ]),
    // For AGENTS: which agent gets which queued issue next.
    suggested: handovers.suggested,
  };
};
/** How much of an MR's title its row in MERGE REQUESTS shows. */
const TITLE_WIDTH = 40;
/** Whether the merged MRs are folded under their divider, over redraws. */
let isMergedFolded = false;
/**
 * The text of an action, a merge's with its MR in front and what its two
 * clicks do or what blocks them, a try's with what its click opens behind.
 * A merge its agent posted (it has a file) marks a draft ready first.
 */
const actionText = ({ action, issue, file }, of) => {
  const mr = issue && of(issue).mr;
  if (action.kind === "merge" && mr)
    return `${link(mr.url, `!${mr.iid}`)} ${mergeText(mr, action.text, of(issue).issue?.boxes ?? [], { canMarkReady: Boolean(file) })}`;
  if (TRY_KINDS.has(action.kind))
    return `${action.text}  ${trySpot(action, issue?.tree)}`;
  return action.text;
};
/**
 * A click on an action: on a button (its first line) it answers, anywhere
 * else it is done. An automatic one has no buttons: a click does it.
 */
const clickOf = item => (column, lineOfRow) =>
  item.isAuto || lineOfRow > 0
    ? item.go()
    : clickAction(item, column, () => screen.redraw({ isReread: true }));

/**
 * What there is to merge: every open MR nothing holds up (merge-order.mjs),
 * else the first by the merge order and what holds it up.
 */
const nextMerge = (local, issues, of) => {
  const byNumber = local.withMr;
  const open = local.order.open.filter(item => byNumber.has(item.number));
  if (!open.length) return null;
  const ready = open.filter(item => item.holdUp.length === 0);
  const mrOf = item => of(byNumber.get(item.number)).mr;
  const named = item =>
    `${link(mrOf(item).url, `!${mrOf(item).iid}`)} ${bold(byNumber.get(item.number).feature)}`;
  const first = ready[0] ?? open[0];
  return {
    line: ready.length
      ? `${green(`ready to merge (${ready.length}):`)} ${ready.map(named).join(", ")}`
      : `${named(first)}  ${yellow(first.holdUp.join(" · "))}`,
    go: () => show(mrOf(first).url, byNumber.get(first.number).tree?.dir),
  };
};

/**
 * NEXT, the answer to "what now?": the first action not answered yet (an
 * answered one waits for its agent), else the first of UP NEXT (the end,
 * once no worktree is left, among them), else the MR that is next to merge
 * and what it waits for, else that nothing waits. A clean up comes after
 * the next MR: after a merge NEXT names the next one. A click does it, like
 * on its row.
 */
const pushNext = (lines, local, { actions, upNext }, issues, of, remote) => {
  const merge = remote ? nextMerge(local, issues, of) : null;
  const first =
    actions.find(item => !item.action.answer) ??
    // A clean up and a new plan wait until nothing is left to merge.
    upNext.find(item => !["clean up", "order"].includes(item.action.kind)) ??
    (merge ? null : upNext[0]);
  const line = first
    ? `${KIND[first.action.kind] ?? first.action.kind} ${bold(first.who)}  ${actionText(first, of)}`
    : merge
      ? `${dim("next to merge")} ${merge.line}`
      : dim(
          remote ? "nothing waits for you: the agents are at work" : "GitLab …",
        );
  // NEXT has no ✓: a click on any of its lines always does it.
  const go = first?.go ?? merge?.go;
  const why = local.plan?.merge?.length ? local.plan.why : null;
  // Both go on under the text, not under NEXT.
  const rows = [[` ${bold("NEXT")}`, line]];
  if (why) rows.push(["", dim(why)]);
  const [next, ...rest] = tableRows(rows, {
    width: process.stdout.columns || 120,
  });
  for (const text of next) {
    if (go) screen.clicks[lines.length] = () => go();
    lines.push(text);
  }
  lines.push(...rest.flat());
};

/** ACTIONS: what needs the user, asked for with `pnpm wt:action` or found waiting. */
const pushActions = (lines, actions, of) => {
  const box = openBox(lines);
  if (actions.length === 0) {
    lines.push(dim("  none: an agent asks with pnpm wt:action <kind> <text>"));
  } else {
    pushRows(
      lines,
      actions.map(item => [
        "",
        item.isAuto ? AUTO : answerCell(item.action),
        KIND[item.action.kind] ?? item.action.kind,
        bold(item.who),
        dim(ago(new Date(item.action.at))),
        actionText(item, of),
      ]),
      actions.map(clickOf),
    );
  }
  closeBox(
    lines,
    box,
    `${bold("ACTIONS")} ${dim(actions.length ? `${actions.length} for you · a button answers, the rest of the row does it${actions.some(item => item.isAuto) ? ", auto ones go by themselves" : ""}` : "nothing waits for you")}`,
  );
};
/** UP NEXT: the automatic follow-ups, one click each; left out while there are none. */
const pushUpNext = (lines, upNext, of) => {
  if (upNext.length === 0) return;
  const box = openBox(lines);
  pushRows(
    lines,
    upNext.map(item => [
      "",
      KIND[item.action.kind] ?? item.action.kind,
      bold(item.who),
      item.action.at ? dim(ago(new Date(item.action.at))) : "",
      actionText(item, of),
    ]),
    upNext.map(item => () => item.go()),
  );
  closeBox(
    lines,
    box,
    `${bold("UP NEXT")} ${dim(`${upNext.length} automatic · a click does it, each goes by itself`)}`,
  );
};

/**
 * The view of one orchestration, for the space `⇶ <name>`: NEXT, ACTIONS,
 * FEED, its worktrees as a tree in the work order (tree.mjs), then
 * their merge requests in the merge order (`pnpm wt:plan`). An issue stays
 * after its worktree went with the merge (the record of wt:orchestrate).
 */
export const renderOrchestration = (local, remote, error) => {
  const width = process.stdout.columns || 120;
  const label = `${ORCHESTRATOR} ${orchestration}`;
  const lines = [];
  lines.push(
    ...titleLines(label, new Date().toLocaleTimeString(), isOnce ? "" : KEYS),
  );
  const { work, merge } = local.plan ?? {};
  const issues = local.issues.toSorted(
    (a, b) => placeIn(work, a.number) - placeIn(work, b.number),
  );
  if (issues.length === 0) {
    lines.push(
      "",
      dim(
        `  no issues recorded for ${label}: pnpm wt:orchestrate ${orchestration} <issue>…`,
      ),
    );
    return lines.map(line => fit(line, width)).join("\n");
  }
  if (error) lines.push("", red(`GitLab: ${error.split("\n")[0]}`));
  const of = issue => remote?.of.get(issue.branch) ?? {};
  // The merge order, computed on each read; the plan's alone until GitLab answers.
  // By number, only the issues with an MR: another entry with the same
  // number but none (#221) must not stand in for one in the graph or NEXT.
  const withMr = new Map(
    issues
      .filter(issue => issue.number && of(issue).mr)
      .map(issue => [issue.number, issue]),
  );
  const order = mergeOrderOf(
    [...withMr.values()].map(issue => ({
      number: issue.number,
      mr: of(issue).mr,
      needs: of(issue).issue?.needs ?? [],
      files: issue.tree?.files ?? new Set(),
      boxes: of(issue).issue?.boxes ?? [],
      behind: issue.tree?.behind ?? 0,
    })),
    merge,
  );
  const ordered = { ...local, order, withMr };
  // Conflicts with develop reach their agents by themselves: only through
  // the live overview that delivers (never a --once print).
  if (remote && deliversAnswers(orchestration)) tellConflicts(issues, of);

  const actions = actionsOf(ordered, issues, of, remote);
  if (!isOnce)
    notifyNewActions(
      label,
      [...actions.actions, ...actions.upNext].map(item => ({
        key: item.key ?? `${item.file}#${item.action.id}`,
        who: item.who,
        kind: item.action.kind,
        text: item.action.text,
      })),
      Boolean(remote),
    );
  lines.push("");
  pushNext(lines, ordered, actions, issues, of, remote);
  lines.push("");
  // The answer to "what now?" is never scrolled away.
  screen.sticky = lines.length;
  pushActions(lines, actions.actions, of);
  pushUpNext(lines, actions.upNext, of);
  pushFeed(lines, issues, local.agents);
  pushAgents(
    lines,
    issues,
    of,
    local.agents ?? new Map(),
    {
      known: local.workers,
      handovers: local.handovers,
      suggested: actions.suggested,
    },
    local.free,
  );

  pushWorktrees(lines, {
    name: orchestration,
    issues,
    of,
    agents: local.agents,
    work,
    hasRemote: Boolean(remote),
  });

  const byNumber = withMr;
  const holdUpOf = new Map(order.open.map(item => [item.number, item.holdUp]));
  const doneMrs = order.merged.map(number => byNumber.get(number));
  const mrsBox = openBox(lines);
  if (!remote) {
    lines.push(dim(error ? "  GitLab does not answer" : "  GitLab …"));
  } else if (order.open.length + doneMrs.length === 0) {
    lines.push(dim("  none yet"));
  } else {
    // The open ones as a graph (merge-order.mjs), then the merged: one
    // table, so their columns line up across the divider.
    const items = [
      ...graphOf(order.open).map(row =>
        row.number ? { ...row, issue: byNumber.get(row.number) } : row,
      ),
      { isDivider: true },
      ...doneMrs.map(issue => ({ issue, isMerged: true })),
    ];
    const cellsOf = item => {
      const { mr } = item.issue ? of(item.issue) : {};
      // A line of the graph alone, or (never again, #221) a row without its MR.
      if (!mr) return ["", dim(item.graph ?? "")];
      const holdUp = holdUpOf.get(item.issue.number);
      return [
        "",
        dim(item.graph ?? ""),
        link(mr.url, `!${mr.iid}`),
        pipelineMark(mr.pipeline),
        fit(mr.title, TITLE_WIDTH),
        // What holds it up, everything or that nothing does; a merged one: when.
        item.isMerged
          ? dim(`merged ${mr.mergedAt ? ago(new Date(mr.mergedAt)) : ""}`)
          : `${holdUp.length ? yellow(holdUp.join(" · ")) : green("ready to merge")}${
              // What its agent waits for (wt:wait).
              item.issue.tree?.wait
                ? `  ${yellow(waitsText(item.issue.tree.wait))}`
                : ""
            }`,
      ];
    };
    const shown = items.filter(item => !item.isDivider);
    const rows = tableRows(shown.map(cellsOf), { width: boxInner() });
    const isMergedShown = isOnce || !isMergedFolded;
    shown.forEach((item, i) => {
      if (item.isMerged && shown[i - 1] && !shown[i - 1].isMerged) {
        // The divider: a click folds the merged ones, another shows them again.
        if (!isOnce)
          screen.clicks[lines.length] = () => {
            isMergedFolded = !isMergedFolded;
            screen.redraw();
          };
        const text = ` ${doneMrs.length} merged${isOnce ? "" : ` · click to ${isMergedFolded ? "show" : "fold"}`} `;
        lines.push(
          dim(
            `  ──${text}${"─".repeat(Math.max(0, boxInner() - 2 - text.length))}`,
          ),
        );
      }
      if (item.isMerged && !isMergedShown) return;
      rows[i].forEach((text, lineOfRow) => {
        if (item.issue)
          screen.clicks[lines.length] = () =>
            show(of(item.issue).mr.url, item.issue.tree?.dir);
        // Its further lines carry on the lanes that go on below it.
        lines.push(
          lineOfRow > 0 && item.below
            ? `  ${dim(item.below.padEnd(visible(item.graph)))}${text.slice(2 + visible(item.graph))}`
            : text,
        );
      });
    });
  }
  const waiting = issues.filter(
    issue => issue.tree && remote && !of(issue).mr,
  ).length;
  const merged = doneMrs.filter(
    issue => of(issue).mr.state === "merged",
  ).length;
  const counts = [
    `${order.open.length} open`,
    merged ? `${merged} merged` : "",
    waiting ? `${waiting} without one yet` : "",
    order.open.length ? "in merge order, side by side where independent" : "",
  ];
  closeBox(
    lines,
    mrsBox,
    `${bold("MERGE REQUESTS")} ${dim(counts.filter(Boolean).join(" · "))}`,
  );
  pushEndButton(lines, orchestration, [
    ...local.trees.map(tree => tree.name),
    // A queued issue still waits for its worktree: it keeps the orchestration too.
    ...issues
      .filter(issue => issue.queued)
      .map(issue => `#${issue.number} (queued)`),
  ]);

  lines.push(
    "",
    dim(
      ' SendMessage(to: "<feature>") reaches an agent · pnpm overview (everything)',
    ),
  );
  return lines.map(line => fit(line, width)).join("\n");
};
