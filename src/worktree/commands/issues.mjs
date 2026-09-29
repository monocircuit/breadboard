import { fail } from "../../lib/proc.mjs";
import { tally } from "../issues.mjs";
import { staleIssues } from "../prune.mjs";

/**
 * The issues of merged work nobody came back to: no comment since the merge.
 * Catching one up is a tick of what the MR did (`wt:tick`) and a comment
 * with the MR, what is done and what is open (`wt:note`).
 */
export default async function issuesCommand() {
  const stale = await staleIssues().catch(error =>
    fail(`GitLab could not be read: ${error.message}`),
  );
  if (stale.length === 0) {
    console.log(
      "\n✓ every issue of a merged MR has a comment since its merge\n",
    );
    return;
  }
  console.log(
    `\n${stale.length} issue${stale.length === 1 ? "" : "s"} of merged work without a comment since the merge:\n`,
  );
  const rows = stale.map(({ issue, mr, boxes }) => [
    `#${issue.iid}`,
    issue.state,
    boxes.length ? `${tally(boxes)} ticked` : "no boxes",
    `!${mr.iid} ${mr.merged_at.slice(0, 10)}`,
    issue.title,
  ]);
  const widths = rows[0].map((_, column) =>
    Math.max(...rows.map(row => row[column].length)),
  );
  for (const row of rows)
    console.log(
      `  ${row.map((cell, column) => (column < row.length - 1 ? cell.padEnd(widths[column]) : cell)).join("  ")}`,
    );
  console.log(
    `\n  Catch one up: pnpm wt:tick <issue> (then the boxes the MR did), and a comment with the MR,\n  what is done and what is open: pnpm wt:note <issue> --message-file <f>\n`,
  );
}
