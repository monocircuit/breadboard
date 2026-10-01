// The view of one orchestration (--orchestration): what to do next, what the
// agents ask for, its worktrees and their merge requests, the open ones in
// the merge order the overview computes (merge-order.mjs), the merged below.
import {
  bold,
  columns,
  dim,
  fit,
  green,
  link,
  red,
  visible,
  wrap,
} from "../lib/tty.mjs";
import {
  ORCHESTRATOR,
  actionsFile,
  focusSpace,
  isInHerdr,
} from "../worktree.mjs";
import { writePlan } from "../worktree/origins.mjs";
import { actionBlock, kindChip } from "./action-block.mjs";
import { answerWith, clickAction, deliversAnswers } from "./answers.mjs";
import { isOnce, orchestration } from "./args.mjs";
import {
  autoActionsOf,
  removeMerged,
  notifyNewActions,
  tellConflicts,
  tellFailedPipelines,
} from "./auto-actions.mjs";
import {
  CELL_START,
  boxInner,
  closeBox,
  openBox,
  pipelineMark,
  titleLines,
} from "./boxes.mjs";
import { pushChatButton } from "./chat.mjs";
import { clickEnd, endText, pushEndButton } from "./end.mjs";
import { announcedOf } from "./handovers.mjs";
import { toneOf } from "./herdr-tones.mjs";
import { open, show } from "./io.mjs";
import {
  checksOf,
  clickMerge,
  mergeText,
  openAfter,
  redraftOnBroken,
} from "./merge.mjs";
import { graphOf, mergeOrderOf } from "./merge-order.mjs";
import { blockersText, mrRowLines, rowBlockersOf } from "./mr-blockers.mjs";
import { nextOf, nothingText, staleOf, updatedPlan, whyOf } from "./next.mjs";
import { orderActions, placeIn } from "./order.mjs";
import {
  clickResolve,
  conflictStateOf,
  forgetResolved,
  resolveColumns,
} from "./resolve.mjs";
import { ago } from "./render-worktree.mjs";
import { pushFreeAgents } from "./free-agents.mjs";
import { freeOnCleanUp, workerActionsOf } from "./workers.mjs";
import { KEYS, pushUnder, screen } from "./screen.mjs";
import { foldLine, pushWorktrees } from "./tree.mjs";
import { clickTry, pageOf, testOrigin, trySpot } from "./try.mjs";
import { tryStateOf } from "./try-state.mjs";

/** The kinds whose click opens the spot to check; `browser-check` is stored before `try` came. */
const TRY_KINDS = new Set(["try", "browser-check"]);
/**
 * How a try's state shows (try-state.mjs `tone`), in herdr's state colours
 * (herdr-tones.mjs): ready as done, waiting as working, failed as blocked.
 */
const TRY_STATUS = { ready: "done", waits: "working", failed: "blocked" };
/** Whether an item is a try that can't be tried yet: grey, no answer, NEXT skips it. */
const isWaitingTry = item => item.tryState?.isReady === false;

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
    .filter(({ issue }) => !issue || isMergeOffered(issue, of));
  const agents = issues
    .filter(issue => issue.tree?.actions?.length)
    .flatMap(issue =>
      issue.tree.actions.map(action => {
        const tryState = TRY_KINDS.has(action.kind)
          ? tryStateOf(action, {
              tree: issue.tree,
              mr: of(issue).mr ?? null,
              origin: action.on === "test" ? testOrigin() : null,
              hasRemote: Boolean(remote),
            })
          : null;
        return {
          action,
          issue,
          tryState,
          who: issue.feature,
          file: actionsFile(issue.tree.dir),
          to: { dir: issue.tree.dir },
          rank: placeIn(merge, issue.number),
          // A `broken` puts a ready MR back to draft (#280).
          onAnswer: answer =>
            redraftOnBroken({
              action,
              answer,
              mr: of(issue).mr,
              file: actionsFile(issue.tree.dir),
            }),
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
              clickTry(action, issue.tree, () => focusSpace(dir), tryState);
            else focusSpace(dir);
          },
        };
      }),
    )
    .filter(
      ({ action, issue }) =>
        action.kind !== "merge" || isMergeOffered(issue, of),
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
      awaited: remote?.awaited ?? null,
      work: local.plan?.work ?? [],
      free: local.freeAgents ?? [],
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
    develop: remote?.develop ?? null,
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
  // A stale plan (next.mjs): a click writes the computed merge order into
  // it, the issues closed or gone and a why naming them dropped.
  const { order } = local;
  const states = statesOf(issues, of);
  const stale = remote
    ? staleOf({ plan: local.plan, states, isOffPlan: order.isOffPlan })
    : [];
  const replan = stale.length
    ? [
        {
          action: {
            kind: "order",
            text: `${local.plan?.merge?.length ? `the plan is stale: ${stale.join("; ")}` : "no merge order in the plan yet"}: ${order.open.map(item => `#${item.number}`).join(" → ")} · click to write it into the plan (wt:plan)`,
            at: "",
          },
          who: `${ORCHESTRATOR} orchestrator`,
          key: `order:${orchestration}:${order.planned.join(",")}:${stale.join(";")}`,
          isAuto: true,
          rank: -1,
          go: () => {
            writePlan(
              orchestration,
              updatedPlan({ plan: local.plan, states, planned: order.planned }),
            );
            screen.redraw({ isReread: true });
          },
        },
      ]
    : [];
  // Merged worktrees nothing holds go by themselves, from the one live overview (clean-up.mjs).
  if (remote && isInHerdr() && deliversAnswers(orchestration))
    removeMerged(
      issues,
      of,
      local.agents ?? new Map(),
      handovers.holds,
      issue =>
        freeOnCleanUp(orchestration, issue, of(issue).mr, {
          agents: local.agents ?? new Map(),
          workers: local.workers,
        }),
    );
  return {
    holds: handovers.holds,
    stale,
    ...orderActions([
      ...own,
      ...agents,
      ...auto,
      ...workers,
      ...end,
      ...replan,
    ]),
    // For WORKTREES' queue: which agent gets which queued issue next.
    suggested: handovers.suggested,
    fresh: handovers.fresh,
    setups: handovers.setups,
  };
};
/**
 * Whether a posted merge of `issue` shows in ACTIONS: not once its MR is
 * merged, nor while a try or an acceptance box is open (#280); the MR row
 * names those.
 */
const isMergeOffered = (issue, of) =>
  of(issue).mr?.state !== "merged" &&
  checksOf(of(issue).issue?.boxes ?? [], issue.tree).length === 0;
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
    return `${link(mr.url, `!${mr.iid}`)} ${mergeText(mr, action.text, of(issue).issue?.boxes ?? [], { canMarkReady: Boolean(file) || Boolean(action.canMarkReady), tree: issue.tree })}`;
  if (TRY_KINDS.has(action.kind))
    return `${action.text}  ${trySpot(action, issue?.tree)}`;
  return action.text;
};
/**
 * What a click on a line of an action's block does (action-block.mjs `on`):
 * on its buttons line a button answers, past them it does the action (the
 * buttons from column `start`); on an option that option answers; anywhere
 * else it does the action.
 */
/**
 * Whether a click on the body of `item` can do nothing (#290): a try that
 * isn't ready and has no page to show why (try-state.mjs), a hand-over or
 * start already under way. Its row says why; it takes no click.
 */
export const isInert = item =>
  Boolean(
    (item.tryState && !item.tryState.isReady && !item.tryState.url) ||
    item.action?.isRunning,
  );
/** The click of a line of `item`'s block; none on the body of an inert one. */
const clickOf = (item, on, start = CELL_START) =>
  on !== "buttons" && typeof on !== "number" && isInert(item)
    ? undefined
    : column => {
        const redraw = () => screen.redraw({ isReread: true });
        if (on === "buttons") return clickAction(item, column, redraw, start);
        if (typeof on === "number") return answerWith(item, on, redraw);
        return item.go();
      };

/**
 * Each issue's state by its number, for the plan's staleness (next.mjs):
 * `merged`, `closed` (without a merge) or `open`.
 */
const statesOf = (issues, of) =>
  new Map(
    issues
      .filter(issue => issue.number)
      .map(issue => [
        issue.number,
        of(issue).mr?.state === "merged"
          ? "merged"
          : of(issue).issue?.state === "closed"
            ? "closed"
            : "open",
      ]),
  );

/**
 * The MR next in the merge order (merge-order.mjs): the first nothing holds
 * up (`isReady`: pipeline passed, boxes ticked, tries good, its turn), else the
 * first and what holds it up; null without an open one. Its click opens it.
 */
const nextMerge = (local, issues, of) => {
  const byNumber = local.withMr;
  const open = local.order.open.filter(item => byNumber.has(item.number));
  if (!open.length) return null;
  const ready = open.filter(item => item.holdUp.length === 0);
  const first = ready[0] ?? open[0];
  const issue = byNumber.get(first.number);
  const { mr } = of(issue);
  return {
    isReady: ready.length > 0,
    iid: mr.iid,
    holdUp: first.holdUp,
    line: `${green("ready to merge")}  ${link(mr.url, `!${mr.iid}`)} ${bold(issue.feature)}${ready.length > 1 ? dim(` · ${ready.length - 1} more ready`) : ""}`,
    go: () => show(mr.url, issue.tree?.dir),
  };
};

/** What NEXT shows (next.mjs `nextOf`), a try that isn't ready skipped. */
export const nextFor = (actions, upNext, merge) =>
  nextOf({ actions, upNext, merge, isWaiting: isWaitingTry });
/** How many of the orchestration's agents are at work. */
const atWorkOf = (issues, agents = new Map()) =>
  issues.filter(
    issue =>
      issue.tree?.claude &&
      agents.get(issue.tree.claude.id)?.status === "working",
  ).length;
/**
 * NEXT, the answer to "what now?", only what the user can do now (#278):
 * the first action not answered yet, else the first clickable row of UP
 * NEXT, else the merge that is ready, each as in its own section and a
 * click there does the same; else one dim line that nothing is for the
 * user. The plan's why under a merge only, never once the plan is stale.
 */
const pushNext = (
  lines,
  local,
  { actions, upNext, stale },
  issues,
  of,
  remote,
) => {
  const merge = remote ? nextMerge(local, issues, of) : null;
  const next = nextFor(actions, upNext, merge);
  const why = whyOf(local.plan, stale, next);
  const width = columns();
  const label = ` ${bold("NEXT")}  `;
  const indent = " ".repeat(visible(label));
  if (next?.kind === "action") {
    blockOf(next.item, of, width - indent.length).forEach(({ line, on }, i) => {
      screen.clicks[lines.length] = clickOf(next.item, on, indent.length + 1);
      lines.push(`${i === 0 ? label : indent}${line}`);
    });
  } else if (next) {
    // An UP NEXT row or the ready merge: a click anywhere on it does it.
    const { item } = next;
    const text =
      next.kind === "merge"
        ? merge.line
        : `${kindChip(item.action.kind)} ${bold(item.who)}  ${actionText(item, of)}`;
    const go = next.kind === "merge" ? merge.go : item.go;
    const isLive = next.kind === "merge" || !isInert(item);
    wrap(text, width - indent.length).forEach((line, i) => {
      if (isLive) screen.clicks[lines.length] = () => go();
      lines.push(`${i === 0 ? label : indent}${line}`);
    });
  } else {
    // Nothing for the user: one dim line in place of the band.
    lines.push(
      ` ${dim(
        remote
          ? nothingText({
              atWork: atWorkOf(issues, local.agents),
              merge,
            })
          : "GitLab …",
      )}`,
    );
  }
  if (!why) return;
  lines.push(
    ...wrap(dim(why), width - indent.length).map(line => indent + line),
  );
};

/**
 * The block of an action (action-block.mjs), `width` wide: a try with its
 * state in the head (try-state.mjs), grey but for it while it isn't ready.
 */
export const blockOf = (item, of, width) =>
  actionBlock(item, {
    text: actionText(item, of),
    width,
    state: item.tryState
      ? toneOf(TRY_STATUS[item.tryState.tone])(item.tryState.label)
      : "",
    isWaiting: isWaitingTry(item),
  });
/**
 * ACTIONS: what needs the user, asked for with `pnpm wt:action` or found
 * waiting, each a block of its own (#273), a thin line between them.
 */
const pushActions = (lines, actions, of) => {
  const box = openBox(lines);
  const width = boxInner();
  if (actions.length === 0) {
    lines.push(dim("  none: an agent asks with pnpm wt:action <kind> <text>"));
  }
  actions.forEach((item, i) => {
    if (i > 0) lines.push(`  ${dim("─".repeat(width))}`);
    for (const { line, on } of blockOf(item, of, width)) {
      screen.clicks[lines.length] = clickOf(item, on);
      lines.push(`  ${line}`);
    }
  });
  closeBox(
    lines,
    box,
    `${bold("ACTIONS")} ${dim(actions.length ? `${actions.length} for you · a button or an option answers, the rest of a block does it${actions.some(item => item.isAuto) ? " · auto ones go by themselves" : ""}` : "nothing waits for you")}`,
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
        kindChip(item.action.kind),
        bold(item.who),
        item.action.at ? dim(ago(new Date(item.action.at))) : "",
      ],
      text: actionText(item, of),
    })),
    upNext.map(item => (isInert(item) ? null : () => item.go())),
    // Its text whole, never cut with `…` at the edge (#273).
    { under: 1, maxLines: 8 },
  );
  closeBox(
    lines,
    box,
    `${bold("UP NEXT")} ${dim(`${upNext.length} automatic · a click does it, each goes by itself`)}`,
  );
};

/**
 * The view of one orchestration, for the space `⇶ <title>`: NEXT, ACTIONS,
 * its worktrees as a tree in the work order with their agents (tree.mjs), then
 * their merge requests in the merge order (`pnpm wt:plan`). An issue stays
 * after its worktree went with the merge (the record of wt:orchestrate).
 */
export const renderOrchestration = (local, remote, error) => {
  const width = columns();
  const label = `${ORCHESTRATOR} ${local.title ?? orchestration}`;
  const lines = [];
  lines.push(
    ...titleLines(label, new Date().toLocaleTimeString(), isOnce ? "" : KEYS),
  );
  pushChatButton(lines, orchestration, local.chat ?? null);
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
      tree: issue.tree ?? null,
    })),
    merge,
  );
  const ordered = { ...local, order, withMr };
  // Conflicts with develop reach their agents by themselves: only through
  // the live overview that delivers (never a --once print).
  // So do failed pipelines, before the rows: one whose line got in has none.
  if (remote && deliversAnswers(orchestration)) {
    tellConflicts(issues, of);
    tellFailedPipelines(
      issues,
      of,
      local.agents ?? new Map(),
      remote.develop ?? null,
    );
  }

  // The handovers the orchestrator announced, where they stand (handovers.mjs).
  const announced = announcedOf(local.handovers, issues, {
    known: local.workers,
    free: local.freeAgents ?? [],
    agents: local.agents,
    awaited: remote?.awaited ?? null,
  });
  const actions = actionsOf(ordered, issues, of, remote, announced);
  if (!isOnce)
    notifyNewActions(
      label,
      // A try counts once it is ready: the one that turns ready notifies then.
      [...actions.actions, ...actions.upNext]
        .filter(item => !isWaitingTry(item))
        .map(item => ({
          key: item.key ?? `${item.file}#${item.action.id}`,
          who: item.who,
          kind: item.tryState ? "try, ready" : item.action.kind,
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
  // The MRs a wait may name that are over already: the orchestration's own
  // and the awaited ones (#262).
  const overMrs = [
    ...issues.map(issue => of(issue).mr),
    ...(remote?.awaited?.values() ?? []),
  ].filter(mr => mr && mr.state !== "opened");
  const doneIids = new Set(overMrs.map(mr => mr.iid));
  const mergedIids = new Set(
    overMrs.filter(mr => mr.state === "merged").map(mr => mr.iid),
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
    suggested: actions.suggested,
    fresh: actions.fresh,
    setups: actions.setups,
    hasRemote: Boolean(remote),
    known: local.workers,
    free: local.free,
    merged: mergedIids,
  });
  // The free agents, their worktree gone (#289).
  pushFreeAgents(lines, orchestration, local.freeAgents ?? [], local.agents);

  const byNumber = withMr;
  const afterOf = new Map(order.open.map(item => [item.number, item.after]));
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
    forgetResolved(
      issues.map(issue => of(issue).mr).filter(mr => mr?.state === "opened"),
    );
    const agents = local.agents ?? new Map();
    // Once per row and frame: the state moves on as it is read.
    const conflicts = new Map();
    const conflictOf = item => {
      if (!conflicts.has(item))
        conflicts.set(
          item,
          conflictStateOf(of(item.issue).mr, {
            tree: item.issue.tree,
            agents,
            behind: item.issue.tree?.behind ?? 0,
          }),
        );
      return conflicts.get(item);
    };
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
        // An open one's conflict state leads what it lacks: the button
        // that has its agent resolve them, the conflicts' reason in it.
        text: item.isMerged
          ? ""
          : [
              conflictOf(item).text,
              blockersText(reasons.filter(reason => !reason.isConflict)),
            ]
              .filter(Boolean)
              .join(dim(" · ")),
      };
    };
    const shown = items.filter(item => !item.isDivider);
    const rows = mrRowLines(shown.map(rowOf), { width: boxInner() });
    const isMergedShown = isOnce || !isMergedFolded;
    shown.forEach((item, i) => {
      if (item.isMerged && shown[i - 1] && !shown[i - 1].isMerged) {
        // The heading of the merged ones: a click folds them, another shows them again.
        if (!isOnce)
          screen.clicks[lines.length] = () => {
            isMergedFolded = !isMergedFolded;
            screen.redraw();
          };
        lines.push(
          `  ${foldLine({
            branch: "",
            isOpen: isMergedShown,
            text: `${doneMrs.length} merged`,
            width: boxInner(),
          })}`,
        );
      }
      if (item.isMerged && !isMergedShown) return;
      for (const text of rows[i]) {
        // On the button its agent gets the order, anywhere else the MR opens;
        // a grey one (or none) does nothing of its own.
        const resolve =
          item.isMerged || !conflictOf(item).canClick
            ? null
            : resolveColumns(text);
        if (item.issue)
          screen.clicks[lines.length] = column =>
            resolve && column >= resolve.from && column <= resolve.to
              ? clickResolve({
                  mr: of(item.issue).mr,
                  tree: item.issue.tree,
                  agents,
                  behind: item.issue.tree?.behind ?? 0,
                })
              : show(of(item.issue).mr.url, item.issue.tree?.dir);
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
