import { fail, flagValue, git, run } from "../../lib/proc.mjs";
import { actionsFile, requestChange } from "../actions.mjs";
import { remoteHeadOf } from "../change.mjs";
import { mainDir, worktrees } from "../worktrees.mjs";

/** The MR `ref` (its number or branch) as GitLab has it, null without one. */
const mrOf = ref => {
  const found = run("glab", ["mr", "view", String(ref), "-F", "json"], {
    cwd: mainDir(),
    quiet: true,
    allowFail: true,
  });
  return found.ok ? JSON.parse(found.out) : null;
};

/** What to do when the MR took its last commit already: the change is work of its own. */
const lateText = (mr, text) =>
  `!${mr.iid} is ${mr.state} already: "${text}" can't go into it any more. It gets its own at once, pushed and visible:
  glab issue create --title "<what>" --description "<why, Related: !${mr.iid}>"
  pnpm wt:new <n>-<feature>   (with --origin <orchestration> for its overview)
  commit, push, pnpm wt:mr    the draft MR from the first commit
  pnpm wt:wait !N … --then "…" when it has to wait for others: pushed first, waiting second`;

/**
 * `pnpm wt:action change "<what>"`: a change is requested for an MR (#228).
 * Before any code is written the MR goes back to draft, the merge actions of
 * its worktree are withdrawn and the overview shows `change requested` on
 * the MR and the worktree, until a commit after it is pushed and green.
 * In a worktree: its own MR; the orchestrator names it: `--orchestration
 * <name> --mr !N`. A merged MR takes no change: that needs its own issue.
 */
export default function changeCommand(text, orchestration) {
  let dir;
  let mr;
  if (orchestration) {
    const iid = flagValue("--mr")?.replace(/^!/, "");
    if (!iid) fail("which MR? --mr !N");
    mr = mrOf(iid);
    if (!mr) fail(`GitLab knows no MR !${iid}`);
    dir =
      worktrees()
        .slice(1)
        .find(tree => tree.branch === mr.source_branch)?.dir ?? null;
  } else {
    dir = git(["rev-parse", "--show-toplevel"]).out;
    if (!dir || dir === mainDir())
      fail(
        "run it inside a worktree, or name the MR: --orchestration <name> --mr !N",
      );
    const branch = git(["branch", "--show-current"], dir).out;
    mr = mrOf(branch);
  }
  if (mr && mr.state !== "opened") fail(lateText(mr, text));

  // First blocked, then built: the user can't merge it meanwhile.
  const draft = mr
    ? run("glab", ["mr", "update", String(mr.iid), "--draft"], {
        cwd: mainDir(),
        quiet: true,
        allowFail: true,
      })
    : null;
  const withdrawn = dir
    ? requestChange(actionsFile(dir), {
        text,
        at: new Date().toISOString(),
        mr: mr?.iid ?? null,
        head: remoteHeadOf(
          dir,
          mr?.source_branch ?? git(["branch", "--show-current"], dir).out,
        ),
      })
    : [];
  const name = mr ? `!${mr.iid}` : "this worktree (no MR yet)";
  console.log(
    `\n✓ change requested for ${name}: ${text}\n${
      mr
        ? draft.ok
          ? `  !${mr.iid} is a draft now: it can't be merged until the change is pushed and green\n`
          : ""
        : "  no MR to block yet: open it with the first commit (pnpm wt:mr)\n"
    }${withdrawn.length ? `  withdrawn: ${withdrawn.map(action => `merge #${action.id}`).join(", ")}\n` : ""}${
      dir
        ? "  the overview shows it on the MR and the worktree until a commit after it is pushed and green; then post the merge again (pnpm wt:action merge …)\n"
        : `  no worktree of this orchestration has ${mr.source_branch}: the MR is blocked, the change needs one\n`
    }`,
  );
  if (mr && !draft.ok)
    fail(
      `glab could not make !${mr.iid} a draft: do it by hand before the change is built (glab mr update ${mr.iid} --draft)`,
    );
}
