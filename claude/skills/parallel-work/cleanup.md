# Cleaning up after the merge

Part of the skill [`parallel-work`](SKILL.md).

```sh
{{run}} wt:remove <feature>                 # from outside that worktree
{{run}} wt:remove <feature> --keep-branch   # remove the worktree, keep an unmerged branch
```

It refuses while the worktree has uncommitted changes, and while the branch
has commits that are not in `{{branch}}` (checked with `git cherry`, so
cherry-picked content counts as merged). Stop an app running there
first. Ask before removing a worktree you did not create. The remote branch is
deleted by GitLab on merge (see the `gitlab` skill, "After the merge").

**`wt:new` asks for it**: it ends with a line like `2 worktrees/branches of
merged work left: {{run}} wt:prune` when there is something to close, and one
like `1 issue of merged work without a comment since the merge: {{run}}
wt:issues` for issues nobody came back to. That is the moment someone is
there — nobody is around after the merge.

**Or forget the name and let it sweep:**

```sh
{{run}} wt:prune --dry-run   # what it would close
{{run}} wt:prune             # close it
```

`prune` fetches with `--prune` and then closes every worktree GitLab is done
with: the branch is gone from origin **and** all of its commits are in
`{{branch}}`. The local branches such a merge left behind go the same way. What
it never touches, and lists with the reason instead: own commits that are not
in `{{branch}}` (a closed MR is the only copy of that work), uncommitted changes,
a worktree you are standing in, a branch still open on origin.

A worktree of an orchestration takes its origin along; the space `⇶ <name>`
closes with its last worktree, once nothing runs in it but its overview, and
the record of its issues with it ([orchestrate.md](orchestrate.md)).

It also deletes a `{{main}}-<feature>` folder git no longer knows as a
worktree when all that is left in it is `.claude/herdr-session` — what a
removal that stopped halfway leaves behind. Any other file in it keeps the
folder, listed.
