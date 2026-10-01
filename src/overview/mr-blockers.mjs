// The rows of MERGE REQUESTS (#242): what an open MR still lacks before its
// merge, one reason per item in the order they have to be done, each with
// who acts on it, on the lines below its title. Every reason once: the draft
// behind open boxes, a `wt:wait` behind the merge order's `after` say nothing
// new.
import { dim, fit, underRows, visible, yellow } from "../lib/tty.mjs";
import { isChangeOpen } from "../worktree.mjs";
import { isWork, mergeStatusText, wordsOf } from "./merge.mjs";

/** Who acts on a reason; an MR it waits for is named by its `!N`. */
export const WHO = { agent: "agent", you: "you", gitlab: "GitLab" };
/** The pipeline states GitLab is still working on. */
const RUNNING = new Set([
  "created",
  "waiting_for_resource",
  "preparing",
  "pending",
  "running",
  "scheduled",
]);
/** How many open Work boxes a reason names before `+n`. */
const NAMED_BOXES = 2;

/**
 * The reasons an open MR can't be merged yet, `{ text, who }`, in the order
 * they have to be done: pipeline, open Work boxes, the draft, conflicts, the
 * MRs it waits for, a requested change, unpushed commits. None when it is
 * ready to merge. `mr` as the overview reads it, `boxes` its issue's
 * (boxesOf), `behind` how many commits of develop its branch lacks, `after`
 * the MRs it comes directly after (merge-order.mjs, `{ iid, why }`), `jobs`
 * its pipeline's (a failed one names the job), `tree` its worktree (the
 * posted actions, `wait`, `change`, `unpushed`), `done` the iids of the MRs
 * known merged or closed: a wait for one of them is over.
 */
export const rowBlockersOf = ({
  mr,
  boxes = [],
  behind = 0,
  after = [],
  jobs = [],
  tree = null,
  done = new Set(),
}) => {
  const reasons = [];
  const add = (text, who, more = {}) => reasons.push({ text, who, ...more });

  const status = mr.pipeline?.status ?? mr.pipeline ?? null;
  if (!status) add("no pipeline yet", WHO.gitlab);
  else if (RUNNING.has(status)) add(`pipeline ${status}`, WHO.gitlab);
  else if (status === "failed") {
    const job = jobs.find(candidate => candidate.status === "failed");
    add(`pipeline failed${job ? ` at ${job.name}` : ""}`, WHO.agent);
  } else if (status !== "success") add(`pipeline ${status}`, WHO.agent);

  const open = boxes.filter(box => isWork(box) && !box.isTicked);
  if (open.length) {
    const named = open.slice(0, NAMED_BOXES).map(wordsOf).join(", ");
    const more = open.length - NAMED_BOXES;
    add(
      `${open.length} Work box${open.length === 1 ? "" : "es"} open: ${named}${more > 0 ? ` +${more}` : ""}`,
      WHO.agent,
    );
  } else if (mr.isDraft) {
    // All ticked: the merge action marks it ready, the agent's or the
    // overview's own once the pipeline passed.
    const isPosted = tree?.actions?.some(action => action.kind === "merge");
    add(
      "draft: mark ready",
      isPosted || status === "success" ? WHO.you : WHO.agent,
    );
  }

  if (mr.hasConflicts)
    add(
      `conflicts with develop${behind ? ` (${behind} behind)` : ""}: merge develop in`,
      WHO.agent,
      // Its row shows the button that has the agent resolve them (resolve.mjs).
      { isConflict: true },
    );
  else {
    const text = mergeStatusText(mr.mergeStatus, {
      isDraft: mr.isDraft,
      isPipelineBlocked: status !== "success",
    });
    if (text) add(text, text === "GitLab is checking" ? WHO.gitlab : WHO.you);
  }

  for (const pred of after)
    add(`after !${pred.iid} (${pred.why})`, `!${pred.iid}`);
  const named = new Set(after.map(pred => pred.iid));
  for (const iid of tree?.wait?.mrs ?? [])
    if (!named.has(iid) && !done.has(iid)) add(`waits for !${iid}`, `!${iid}`);

  if (tree && isChangeOpen(tree, mr))
    add(`change requested: ${tree.change.text}`, WHO.agent);
  if (tree?.unpushed)
    add(
      `${tree.unpushed} commit${tree.unpushed === 1 ? "" : "s"} not pushed`,
      WHO.agent,
    );
  return reasons;
};

/** Between a reason's last word and who acts: a wrap never parts them. */
const GLUE = "\u00a0";
/** The reasons as the row's text below its title: `pipeline running → GitLab · …`. */
export const blockersText = reasons =>
  reasons
    .map(({ text, who }) => `${yellow(text)}${GLUE}${dim(`→${GLUE}${who}`)}`)
    .join(dim(" · "));

/** The fewest columns an MR's title gets, however narrow the view. */
const MIN_TITLE = 24;
/** The column of MERGE REQUESTS the reasons start under: the title's. */
const TITLE_COLUMN = 4;
/** How many lines the reasons of a row take at most, the first line apart. */
const REASON_LINES = 2;

/**
 * The rows of MERGE REQUESTS as the lines of each: on the first the order
 * mark (`graph`, merge-order.mjs's graphOf), `cells` (!N and the pipeline),
 * the title with the room the width leaves and `status` (ready, merged);
 * below it `text`, what the MR still lacks (blockersText), indented under the
 * title and wrapped at the full `width`, its lines carrying on the lanes
 * that go on below it (`below`). A row without `text` stays one line, one of
 * the graph alone (no `cells`) is its graph.
 */
export const mrRowLines = (rows, { width } = {}) => {
  const leads = rows.map(row => [
    "",
    dim(row.graph ?? ""),
    ...(row.cells ?? []),
  ]);
  const widths = [];
  for (const lead of leads)
    lead.forEach(
      (cell, i) => (widths[i] = Math.max(widths[i] ?? 0, visible(cell))),
    );
  const start = widths
    .slice(0, TITLE_COLUMN)
    .reduce((sum, cellWidth) => sum + cellWidth + 2, 0);
  const status = Math.max(0, ...rows.map(row => visible(row.status ?? "")));
  const room = Math.max(
    MIN_TITLE,
    (width ?? Infinity) - start - (status ? status + 2 : 0),
  );
  const lines = underRows(
    rows.map((row, i) => ({
      head: row.cells
        ? [
            ...leads[i],
            fit(row.title ?? "", room),
            // No status in any row: no column for it.
            ...(status ? [row.status ?? ""] : []),
          ]
        : leads[i],
      text: row.text ?? "",
    })),
    { width, under: TITLE_COLUMN, maxLines: REASON_LINES },
  );
  return lines.map((row, i) => {
    const { graph = "", below } = rows[i];
    const lanes = 2 + visible(graph);
    return row.map((line, at) =>
      at > 0 && below && !line.slice(0, lanes).trim()
        ? `  ${dim(below.padEnd(visible(graph)))}${line.slice(lanes)}`
        : line,
    );
  });
};
