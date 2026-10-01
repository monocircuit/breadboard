// The commands of worktree.mjs, one file each: `pnpm wt:<name>`, and
// `pnpm stack:*` through `control`.
import actionCommand from "./action.mjs";
import changelogCommand from "./changelog.mjs";
import controlCommand from "./control.mjs";
import herdrCommand from "./herdr.mjs";
import issuesCommand from "./issues.mjs";
import listCommand from "./list.mjs";
import mrCommand from "./mr.mjs";
import newCommand from "./new.mjs";
import noteCommand from "./note.mjs";
import orchestrateCommand from "./orchestrate.mjs";
import planCommand from "./plan.mjs";
import platformChangelogCommand from "./platform-changelog.mjs";
import pruneCommand from "./prune.mjs";
import removeCommand from "./remove.mjs";
import stackCommand from "./stack.mjs";
import tickCommand from "./tick.mjs";
import waitCommand from "./wait.mjs";

export const commands = {
  new: newCommand,
  orchestrate: orchestrateCommand,
  plan: planCommand,
  mr: mrCommand,
  note: noteCommand,
  tick: tickCommand,
  action: actionCommand,
  wait: waitCommand,
  issues: issuesCommand,
  stack: stackCommand,
  herdr: herdrCommand,
  prune: pruneCommand,
  list: listCommand,
  control: controlCommand,
  remove: removeCommand,
  changelog: changelogCommand,
  "platform-changelog": platformChangelogCommand,
};
