import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { fail, flagValue, git, run } from "../../lib/proc.mjs";
import {
  boxReport,
  descriptionFile,
  issueOfBranch,
  shown,
  templateToFill,
  unfilled,
} from "../issues.mjs";
import {
  fragmentFiles,
  issueOfName,
  NO_CHANGELOG,
} from "../platform-changelog.mjs";
import { installPushGuard } from "../push-guard.mjs";

/**
 * The draft merge request of this worktree, opened with the first commits so
 * a colleague who takes over sees what exists. Title from the last commit,
 * description written for this branch and handed over with
 * `--description-file`: without one the template is put down for you and
 * nothing is sent, because a merge request that only holds the template
 * says nothing. `--update` sends the description again once the scope has
 * grown — through the same body, so a plain `glab mr update` can no longer
 * drop the `Closes #<n>` that carries the work item.
 */
export default function mrCommand(issue, flags) {
  const isDry = flags.has("--dry-run");
  const top = git(["rev-parse", "--show-toplevel"]).out;
  const branch = git(["branch", "--show-current"], top).out;
  if (!branch || branch === "develop" || branch === "main")
    fail(`${branch || "a detached HEAD"} gets no merge request`);
  // Worktrees made before the guard existed get it here (#184).
  const guard = isDry ? "installed" : installPushGuard(top);
  if (!["installed", "updated"].includes(guard)) console.log(`! ${guard}`);

  const isUpdate = flags.has("--update");
  const found = run("glab", ["mr", "view", branch, "-F", "json"], {
    cwd: top,
    quiet: true,
    allowFail: true,
  });
  const open = found.ok ? JSON.parse(found.out) : null;
  const number =
    issue?.replace(/^#/, "") ??
    issueOfBranch(branch) ??
    open?.description?.match(/Closes #(\d+)/)?.[1];
  if (open && !isDry && !isUpdate) {
    console.log(
      `\n✓ the merge request is open already\n  ${open.title}\n  ${open.web_url}\n\n  Its description grew out of date? pnpm wt:mr --update${number ? boxReport(top, number) : ""}\n`,
    );
    return;
  }

  if (!number || !/^\d+$/.test(number)) {
    fail(
      "a merge request needs its work item: pnpm wt:mr <issue>, or a worktree named after the issue (34-…). No issue yet: glab issue create",
    );
  }

  // The labels and the milestone of the work item are the labels and the
  // milestone of its merge request, so the board reads the same in both
  // places. Labels fall back to the kind of the branch; the milestone does
  // not — an issue outside every epic gets a merge request outside them too.
  const work = run("glab", ["issue", "view", number, "-F", "json"], {
    cwd: top,
    quiet: true,
    allowFail: true,
  });
  if (!work.ok)
    fail(
      `issue #${number} does not exist — a merge request without a work item is none`,
    );
  const {
    labels = [],
    milestone,
    description: issueText,
  } = JSON.parse(work.out);
  const boxes = boxReport(top, number, issueText ?? "");
  if (labels.length === 0)
    labels.push(branch.startsWith("fix/") ? "bug" : "enhancement");
  const milestoneTitle = milestone?.title ?? null;

  // The description is written by whoever opens the merge request, never
  // taken from the template: an MR that holds the template says nothing.
  const given = flagValue("--description-file") ?? descriptionFile(top);
  if (!fs.existsSync(given)) fail(templateToFill(top, number));
  const written = fs.readFileSync(given, "utf-8");
  const unwritten = unfilled(written);
  if (unwritten.length > 0) {
    fail(
      `${shown(top, given)} is still the template, ${unwritten.length} place${unwritten.length === 1 ? "" : "s"} unwritten:\n${unwritten.join("\n")}\n\n  Say per section what this branch changes and why, then run pnpm wt:mr again.`,
    );
  }
  const body = /Closes #\d/.test(written)
    ? written.trimEnd()
    : `${written
        .trimEnd()
        .replace(/Refs #\d*$/, "")
        .trimEnd()}\n\nCloses #${number}`;
  const file = path.join(
    fs.mkdtempSync(path.join(os.tmpdir(), "wt-mr-")),
    "description.md",
  );
  fs.writeFileSync(file, `${body}\n`);

  const title = git(["log", "-1", "--format=%s"], top).out;
  if (isDry) {
    console.log(
      `\n▸ nothing was sent (--dry-run)\n  title     ${title}\n  work item #${number}\n  labels    ${labels.join(", ")}\n  milestone ${milestoneTitle ?? "none — the issue has none"}\n  body      ${file}\n\n${body}\n`,
    );
    return;
  }
  // An MR that is already open only gets the description — the same body,
  // so a description sent later cannot drop the `Closes #<n>` that carries
  // the work item.
  if (open) {
    const updated = run(
      "glab",
      ["mr", "update", String(open.iid), "--description-file", file],
      { cwd: top, quiet: true, allowFail: true },
    );
    if (!updated.ok)
      fail(`glab could not update the description of !${open.iid}`);
    const back = run("glab", ["mr", "view", String(open.iid), "-F", "json"], {
      cwd: top,
      quiet: true,
      allowFail: true,
    });
    const isThere =
      back.ok && JSON.parse(back.out).description?.trim() === body.trim();
    console.log(
      `\n${isThere ? "✓" : "⚠"} description of !${open.iid} updated, closes #${number}${isThere ? "" : " — what GitLab shows back differs, look at it"}\n  ${open.web_url}${boxes}\n`,
    );
    return;
  }

  const created = run(
    "glab",
    // eslint-disable-next-line prettier/prettier
    ["mr", "create", "--draft", "--source-branch", branch, "--target-branch", "develop", "--title", title, "--description-file", file, "--label", labels.join(","), ...(milestoneTitle ? ["--milestone", milestoneTitle] : []), "--assignee", "@me", "--remove-source-branch", "--yes", "--no-editor"],
    { cwd: top, quiet: true, allowFail: true },
  );
  if (!created.ok)
    fail(
      "glab could not create the merge request, is everything pushed? (git push)",
    );
  const carried = `labels: ${labels.join(", ")}${milestoneTitle ? `, milestone: ${milestoneTitle}` : ""}`;
  console.log(
    `\n✓ draft merge request opened, closes #${number}, ${carried}\n  ${title}\n  ${created.out.split("\n").find(l => l.startsWith("http")) ?? ""}${boxes}\n`,
  );
  // check:changelog fails the MR until it has an entry: say so now, not at the pipeline.
  if (!fragmentFiles(top).some(found => issueOfName(found) === Number(number)))
    console.log(
      `  No changelog entry yet: pnpm changelog:add "<what users notice>" (or the label ${NO_CHANGELOG})\n`,
    );
}
