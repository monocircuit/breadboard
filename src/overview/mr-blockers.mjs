// What an open MR still lacks before its merge, for its row in MERGE
// REQUESTS (#242): one reason per item, in the order they have to be done,
// each with who acts on it. Every reason once: the draft behind open boxes,
// a `wt:wait` behind the merge order's `after` say nothing new.
import { dim, yellow } from "../lib/tty.mjs";
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
 * posted actions, `wait`, `change`, `unpushed`).
 */
export const rowBlockersOf = ({
  mr,
  boxes = [],
  behind = 0,
  after = [],
  jobs = [],
  tree = null,
}) => {
  const reasons = [];
  const add = (text, who) => reasons.push({ text, who });

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
    if (!named.has(iid)) add(`waits for !${iid}`, `!${iid}`);

  if (tree && isChangeOpen(tree, mr))
    add(`change requested: ${tree.change.text}`, WHO.agent);
  if (tree?.unpushed)
    add(
      `${tree.unpushed} commit${tree.unpushed === 1 ? "" : "s"} not pushed`,
      WHO.agent,
    );
  return reasons;
};

/** The reasons as the row's text below its title: `pipeline running → GitLab · …`. */
export const blockersText = reasons =>
  reasons
    .map(({ text, who }) => `${yellow(text)} ${dim(`→ ${who}`)}`)
    .join(dim(" · "));
