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
  underRows,
  visible,
  wrap,
  yellow,
} from "../lib/tty.mjs";
import {
  ORCHESTRATOR,
  actionsFile,
  focusSpace,
  isInHerdr,
} from "../worktree.mjs";
import { writePlan } from "../worktree/origins.mjs";
import {
  answerCell,
  clickAction,
  deliversAnswers,
  optionsText,
} from "./answers.mjs";
import { isOnce, orchestration } from "./args.mjs";
import {
  AUTO_KIND,
  autoActionsOf,
  removeMerged,
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
import { announcedOf } from "./handovers.mjs";
import { open, show } from "./io.mjs";
import { clickMerge, mergeText, openAfter } from "./merge.mjs";
import { graphOf, mergeOrderOf } from "./merge-order.mjs";
import { blockersText, mrRowLines, rowBlockersOf } from "./mr-blockers.mjs";
import { orderActions, placeIn } from "./order.mjs";
import { pushAgents } from "./agents.mjs";
import { ago } from "./render-worktree.mjs";
import { workerActionsOf } from "./workers.mjs";
import { KEYS, pushUnder, screen } from "./screen.mjs";
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
const actionsOf = (local, issues, of, remote, announced) => {
  const merge = local.order.numbers;
  const own = (local.ownActions ?? [])
    .map(action => {
      // A merge the orchestrator posted for an MR (`--mr !N`) is that MR's.
      const issue =
        action.kind === "merge" && action.mr
          ? issues.find(candidate => of(candidate).mr?.iid === action.mr)
          : null;
      return {
        action,
        issue,
        who: `${ORCHESTRATOR} orchestrator`,
        file: local.recordFile,
        to: { orchestration },
        rank: -1,
        // The overview stands in the orchestrator's space already, next to its tab;
        // a page of its try is one on the app of the main checkout.
        go: () =>
          issue
            ? clickMerge({
                issue,
                mr: of(issue).mr,
                boxes: of(issue).issue?.boxes ?? [],
                others: openAfter(issues, issue.number, merge, of),
                file: local.recordFile,
                actionId: action.id,
              })
            : action.url && open(pageOf(action.url, 0)),
      };
    })
    .filter(({ issue }) => !issue || of(issue).mr?.state !== "merged");
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
    {
      shouldHandOut: Boolean(remote) && deliversAnswers(orchestration),
      announced,
    },
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
  // Drafts nothing but the draft holds up, next in the merge order: their
  // agents may not mark them ready, the merge's click does (#242).
  const readyDrafts = new Set(
    local.order.open
      .filter(item => item.holdUp.length === 1 && item.holdUp[0] === "draft")
      .map(item => item.number),
  );
  const auto = autoActionsOf(issues, of, local.agents ?? new Map(), {
    readyDrafts,
  })
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
                canMarkReady: action.canMarkReady,
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
  // Merged worktrees nothing holds go by themselves, from the one live overview (clean-up.mjs).
  if (remote && isInHerdr() && deliversAnswers(orchestration))
    removeMerged(issues, of, local.agents ?? new Map(), handovers.holds);
  return {
    holds: handovers.holds,
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
/** Whether the merged MRs are folded under their divider, over redraws. */
let isMergedFolded = false;
/**
 * The text of an action, a merge's with its MR in front and what its two
 * clicks do or what blocks them, a try's with what its click opens behind.
 * A merge posted (it has a file) or found for a ready draft marks the draft
 * ready first.
 */
const actionText = ({ action, issue, file }, of) => {
  const mr = issue && of(issue).mr;
  if (action.kind === "merge" && mr)
    return `${link(mr.url, `!${mr.iid}`)} ${mergeText(mr, action.text, of(issue).issue?.boxes ?? [], { canMarkReady: Boolean(file) || Boolean(action.canMarkReady) })}`;
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
    ? `${KIND[first.action.kind] ?? first.action.kind} ${bold(first.who)}${first.action.at ? `  ${dim(ago(new Date(first.action.at)))}` : ""}`
    : merge
      ? dim("next to merge")
      : dim(
          remote ? "nothing waits for you: the agents are at work" : "GitLab …",
        );
  // NEXT has no ✓: a click on any of its lines always does it.
  const go = first?.go ?? merge?.go;
  const why = local.plan?.merge?.length ? local.plan.why : null;
  // Its text under the kind, like in ACTIONS, a decide's options after it;
  // the plan's why under that.
  const width = process.stdout.columns || 120;
  const head = [` ${bold("NEXT")}`, line];
  const text = first
    ? [
        actionText(first, of),
        first.action.kind === "decide" && optionsText(first.action),
      ]
        .filter(Boolean)
        .join("  ")
    : (merge?.line ?? "");
  const [next] = underRows([{ head, text }], { width, under: 1 });
  for (const nextLine of next) {
    if (go) screen.clicks[lines.length] = () => go();
    lines.push(nextLine);
  }
  if (!why) return;
  const indent = " ".repeat(visible(head[0]) + 2);
  lines.push(...wrap(dim(why), width - indent.length).map(l => indent + l));
};

/**
 * A row of ACTIONS: its buttons, kind, agent and age on the first line, its
 * text under the kind (pushUnder with `under: 2`).
 */
export const actionRow = (item, of) => ({
  head: [
    "",
    item.isAuto ? AUTO : answerCell(item.action),
    KIND[item.action.kind] ?? item.action.kind,
    bold(item.who),
    dim(ago(new Date(item.action.at))),
  ],
  text: actionText(item, of),
});
/** ACTIONS: what needs the user, asked for with `pnpm wt:action` or found waiting. */
const pushActions = (lines, actions, of) => {
  const box = openBox(lines);
  if (actions.length === 0) {
    lines.push(dim("  none: an agent asks with pnpm wt:action <kind> <text>"));
  } else {
    pushUnder(
      lines,
      actions.map(item => actionRow(item, of)),
      actions.map(clickOf),
      { under: 2 },
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
  pushUnder(
    lines,
    upNext.map(item => ({
      head: [
        "",
        KIND[item.action.kind] ?? item.action.kind,
        bold(item.who),
        item.action.at ? dim(ago(new Date(item.action.at))) : "",
      ],
      text: actionText(item, of),
    })),
    upNext.map(item => () => item.go()),
    { under: 1 },
  );
  closeBox(
    lines,
    box,
    `${bold("UP NEXT")} ${dim(`${upNext.length} automatic · a click does it, each goes by itself`)}`,
  );
};

/**
 * The view of one orchestration, for the space `⇶ <title>`: NEXT, ACTIONS,
 * AGENTS, its worktrees as a tree in the work order (tree.mjs), then
 * their merge requests in the merge order (`pnpm wt:plan`). An issue stays
 * after its worktree went with the merge (the record of wt:orchestrate).
 */
export const renderOrchestration = (local, remote, error) => {
  const width = process.stdout.columns || 120;
  const label = `${ORCHESTRATOR} ${local.title ?? orchestration}`;
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

  // The handovers the orchestrator announced, where they stand (handovers.mjs).
  const announced = announcedOf(local.handovers, issues, {
    known: local.workers,
    agents: local.agents,
    awaited: remote?.awaited ?? null,
  });
  const actions = actionsOf(ordered, issues, of, remote, announced);
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
  pushAgents(
    lines,
    issues,
    of,
    local.agents ?? new Map(),
    {
      known: local.workers,
      handovers: announced,
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
    merge: order.open.map(item => item.number),
    announced,
    holds: actions.holds,
    hasRemote: Boolean(remote),
  });

  const byNumber = withMr;
  const afterOf = new Map(order.open.map(item => [item.number, item.after]));
  // The MRs a wait may name that are over already.
  const doneIids = new Set(
    issues
      .map(issue => of(issue).mr)
      .filter(mr => mr && mr.state !== "opened")
      .map(mr => mr.iid),
  );
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
    const rowOf = item => {
      const { mr, jobs } = item.issue ? of(item.issue) : {};
      // A line of the graph alone, or (never again, #221) a row without its MR.
      if (!mr) return { graph: item.graph };
      const reasons = item.isMerged
        ? []
        : rowBlockersOf({
            mr,
            boxes: of(item.issue).issue?.boxes ?? [],
            behind: item.issue.tree?.behind ?? 0,
            after: afterOf.get(item.issue.number) ?? [],
            jobs,
            tree: item.issue.tree,
            done: doneIids,
          });
      return {
        graph: item.graph,
        below: item.below,
        cells: [link(mr.url, `!${mr.iid}`), pipelineMark(mr.pipeline)],
        title: mr.title,
        // A merged one: when; an open one nothing holds up: that it is ready.
        status: item.isMerged
          ? dim(`merged ${mr.mergedAt ? ago(new Date(mr.mergedAt)) : ""}`)
          : reasons.length
            ? ""
            : green("ready to merge"),
        text: blockersText(reasons),
      };
    };
    const shown = items.filter(item => !item.isDivider);
    const rows = mrRowLines(shown.map(rowOf), { width: boxInner() });
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
      for (const text of rows[i]) {
        if (item.issue)
          screen.clicks[lines.length] = () =>
            show(of(item.issue).mr.url, item.issue.tree?.dir);
        lines.push(text);
      }
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
