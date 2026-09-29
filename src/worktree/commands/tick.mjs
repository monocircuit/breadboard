import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { fail, git, run } from "../../lib/proc.mjs";
import { boxesOf, issueOfBranch, printBoxes, tally } from "../issues.mjs";

/**
 * The checkboxes of an issue, numbered; given box numbers, those are ticked
 * in the description and read back. Ticking by hand is a JSON read, an edited
 * file and `glab issue update --description-file` — so it got skipped, and
 * the done work lived in a comment's own `- [x]` list instead.
 *
 * The issue is the first number, the boxes the rest; without any, the issue
 * of the worktree's branch is listed.
 */
export default function tickCommand(_issue) {
  const usage =
    "usage: pnpm wt:tick [<issue>] [<box>…]   (the issue first when you tick)";
  const top = git(["rev-parse", "--show-toplevel"]).out;
  const argv = process.argv.slice(3).filter(arg => !arg.startsWith("--"));
  const number = (
    argv[0] ?? issueOfBranch(git(["branch", "--show-current"], top).out)
  )?.replace(/^#/, "");
  if (!number || !/^\d+$/.test(number))
    fail(`${usage}\n  This branch carries no issue number, so name the issue.`);
  const wanted = argv.slice(1);
  if (wanted.some(box => !/^\d+$/.test(box))) fail(usage);

  const read = () => {
    const found = run("glab", ["issue", "view", number, "-F", "json"], {
      cwd: top,
      quiet: true,
      allowFail: true,
    });
    if (!found.ok) fail(`issue #${number} could not be read`);
    return JSON.parse(found.out);
  };
  // Read right before writing: someone may have changed the description since.
  const issue = read();
  const description = issue.description ?? "";
  const boxes = boxesOf(description);
  if (boxes.length === 0)
    fail(`#${number} has no checkboxes in its description`);

  if (wanted.length === 0) {
    console.log(`\n#${number} ${issue.title} — ${tally(boxes)} ticked`);
    printBoxes(boxes);
    console.log(`\n  pnpm wt:tick ${number} <box>… ticks them\n`);
    return;
  }

  const outside = wanted.filter(
    box => Number(box) < 1 || Number(box) > boxes.length,
  );
  if (outside.length)
    fail(
      `#${number} has boxes 1 to ${boxes.length}, not ${outside.join(", ")}`,
    );
  const chosen = new Set(wanted.map(box => boxes[Number(box) - 1]));
  const fresh = [...chosen].filter(box => !box.isTicked);
  if (fresh.length === 0) {
    console.log(
      `\n✓ already ticked, nothing sent — #${number}: ${tally(boxes)}\n`,
    );
    return;
  }

  const lines = description.split("\n");
  for (const box of fresh)
    lines[box.line] = lines[box.line].replace("[ ]", "[x]");
  const file = path.join(
    fs.mkdtempSync(path.join(os.tmpdir(), "wt-tick-")),
    "description.md",
  );
  fs.writeFileSync(file, lines.join("\n"));
  const updated = run(
    "glab",
    ["issue", "update", number, "--description-file", file],
    { cwd: top, quiet: true, allowFail: true },
  );
  if (!updated.ok) fail(`glab could not update the description of #${number}`);

  // Read it back: the same boxes, the chosen ones ticked. A body that became
  // a path has no boxes at all, so this catches that too.
  const back = boxesOf(read().description ?? "");
  const isThere =
    back.length === boxes.length &&
    fresh.every(box => back[boxes.indexOf(box)].isTicked);
  console.log(
    `\n${isThere ? "✓" : "⚠"} #${number} ${issue.title} — ${tally(back)} ticked${isThere ? "" : " — what GitLab shows back differs, look at it"}`,
  );
  printBoxes(back, new Set(fresh.map(box => back[boxes.indexOf(box)])));
  console.log("");
}
