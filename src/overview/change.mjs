// What the overview of an orchestration shows of a requested change and of
// work that isn't pushed (#228): facts on the worktree's node and its MR row,
// and an automatic action once commits sit unpushed for long.
import { red, yellow } from "../lib/tty.mjs";
import { isChangeOpen, isUnpushedLong } from "../worktree.mjs";

/** `change requested: <what>` while the change blocks the MR; nothing else. */
export const changeFact = (tree, mr) =>
  isChangeOpen(tree, mr) ? yellow(`change requested: ${tree.change.text}`) : "";

/** `2 commits not pushed`, red once they sit there long; with `no remote branch` when origin has none. */
export const unpushedFact = tree => {
  if (!tree?.unpushed) return "";
  const text = `${tree.unpushed} commit${tree.unpushed === 1 ? "" : "s"} not pushed${tree.hasRemoteBranch ? "" : " (no remote branch)"}`;
  return isUnpushedLong(tree) ? red(text) : yellow(text);
};

/** How long, as `25 min` or `3 h`. */
const since = at => {
  const minutes = Math.round((Date.now() - Date.parse(at)) / 60_000);
  return minutes < 120 ? `${minutes} min` : `${Math.round(minutes / 60)} h`;
};

/**
 * The automatic `push` of an issue whose worktree holds commits origin
 * doesn't have for longer than UNPUSHED_MS: in ACTIONS, so NEXT names it. Its
 * click tells the agent to push (once per state of the branch), else goes to
 * its space. `{ kind, text, at, go }` or null; `sent`: what the worktree
 * remembers as sent, `send(dir, key, line)`: sendOnce, `focus`: to its
 * agent's tab. When `mr` is merged the push guard refuses the branch: the
 * commits need an issue, branch and draft MR of their own.
 */
export const unpushedActionOf = (issue, mr, sent, { send, focus }) => {
  const { tree } = issue;
  if (!isUnpushedLong(tree)) return null;
  const key = `unpushed:${issue.branch}:${tree.unpushed}:${tree.pushed ?? "none"}`;
  const fact = `${unpushedFact(tree)} for ${since(tree.unpushedAt)}`;
  const isSent = sent.has(key);
  const commits = `${tree.unpushed} commit${tree.unpushed === 1 ? "" : "s"} of ${issue.branch}`;
  const line =
    mr?.state === "merged"
      ? `[overview] ${commits} came after !${mr.iid} was merged and are not pushed: they get their own issue, branch and draft MR now (glab issue create, pnpm wt:new, cherry-pick them, push, pnpm wt:mr), waiting with pnpm wt:wait if they must.`
      : `[overview] ${commits} ${tree.unpushed === 1 ? "is" : "are"} not pushed: push now (git push${tree.hasRemoteBranch ? "" : " -u origin HEAD"}), a draft MR from the first commit (pnpm wt:mr). Waiting for another MR comes after the push.`;
  return {
    kind: "push",
    text: isSent
      ? `${fact}: its agent was told`
      : `${fact}: click to tell its agent`,
    at: tree.unpushedAt,
    go: () => (!isSent && tree.claude && send(tree.dir, key, line)) || focus(),
  };
};
