#!/usr/bin/env node
// Worktrees for parallel work, see the skill parallel-work.
//
//   pnpm wt:new <feature> [--shared] [--fix]  create ../platform-<feature> on feature/<feature>
//                                             with its own dev stack, push it as the claim
//     [--origin <orchestration>]              started for an orchestration: recorded, its space marked
//   pnpm wt:orchestrate <name> <issue>…       a space ⇶ <name> for this session, a worktree per issue
//   pnpm wt:orchestrate <name> --title "<t>"  what it shows instead of its name: ⇶ <t> (--title with issues too)
//     [--shared] [--peer <s>] [--brief <f>]   and in its space a claude briefed on the issue (--dry-run);
//     [--queue]                               each starts at once, in an idle agent that knows its
//                                             files or a fresh one; --queue: the overview hands them out
//   pnpm wt:orchestrate <name> <issue>        the issue to an idle agent of it instead of a fresh one:
//     --to <agent> [--limit <percent>]        its worktree made, the agent moved in and told;
//     [--after !<mr>…]                        announced: the overview does it once the MRs are merged
//   pnpm wt:orchestrate <name> --end          once its worktrees are gone: its space, the orchestrator's
//                                             claude (kept for /resume), its record and checkout away
//   pnpm wt:plan <name> [--work <n>…]         the order of an orchestration's issues and merges
//     [--merge <n>…] [--why <text>]           for its overview (NEXT, WORKTREES, MERGE REQUESTS)
//   pnpm wt:list                            every worktree: branch, url, stack, what runs
//   pnpm wt:stack                             give the current worktree its own vita dev stack
//   pnpm wt:mr [<issue>] [--update]           the draft MR of the current worktree, opened once
//                                             from .claude/mr-description.md; --update sends it again
//   pnpm wt:note <issue> --message-file <f>   comment <f> on the issue, read back
//   pnpm wt:tick [<issue>] [<box>…]           the boxes of the issue, numbered; ticks the given ones
//   pnpm wt:action <kind> <text> [--url <u>]  ask the human (merge, try, decide, look, plan): ACTIONS
//     [--list | --done <id>]                  of the orchestration's overview
//   pnpm wt:action note <line>                what the agent does: its row in FEED
//   pnpm wt:wait !<mr>… [--then <what>]       (in the background) until the MRs are merged, then the
//     [--hours <h>] [--cancel]                line to go on with; recorded for the overview
//   pnpm wt:issues                          issues of merged MRs nobody commented on since the merge
//   pnpm wt:prune [--dry-run]                 worktrees and branches of merged MRs away
//   pnpm wt:herdr                             herdr: root and overview, a space per worktree
//   pnpm wt:remove <feature> [--keep-branch]  own stack, worktree and merged branch away
//                  [--close-space]            and the whole herdr space, conversations too (the ✕ of the overview)
//   pnpm stack:up|down|reset|ps               the vita stack of the worktree that serves <port> (3000+n,
//     [--port <port> | <feature>]             or a port of its services), else of the one you are in
//
// No dependencies: it has to run before `pnpm install` of the new worktree.

import { fileURLToPath } from "node:url";

import { fail } from "./lib/proc.mjs";
import { commands } from "./worktree/commands/index.mjs";

// overview.mjs reads the worktrees and keeps herdr in step through these, herdr.mjs writes
// the sidebar with them; imported, nothing runs.
export { isInHerdr } from "./lib/herdr.mjs";
export {
  ENTRY,
  SPACE,
  STORYBOOK_PORT,
  STUDIO_PORT,
  VITA,
} from "./worktree/constants.mjs";
export {
  boundSession,
  claudeDir,
  rememberSessions,
  transcriptOf,
  usageOf,
} from "./worktree/conversations.mjs";
export {
  actionsFile,
  actionsOf,
  answerAction,
  answerLine,
  answersOf,
  changeOf,
  claimAnswer,
  doneAction,
  releaseAnswer,
  takeAnswers,
} from "./worktree/actions.mjs";
export {
  isChangeOpen,
  isUnpushedLong,
  pushStateOf,
} from "./worktree/change.mjs";
export { endInBackground, endLog } from "./worktree/end.mjs";
export {
  feedFile,
  feedLineOf,
  lastAnswerOf,
  noteOf,
  stoppedOf,
} from "./worktree/feed.mjs";
export { boxesOf } from "./worktree/issues.mjs";
export { layout, startStorybook, syncStackTabs } from "./worktree/layout.mjs";
export {
  ORCHESTRATOR,
  ORCHESTRATOR_TAB,
  orchestratorLabel,
  originOf,
  recordFile,
  recordOf,
  titleOf,
} from "./worktree/origins.mjs";
export { handoversOf } from "./worktree/handovers.mjs";
export { hasOwnCommits, isInside, pruneSpaces } from "./worktree/prune.mjs";
export {
  REPO_TOKEN,
  STATE_TOKEN,
  branchColourOf,
  herdrConfigText,
  sidebarConfig,
} from "./worktree/sidebar.mjs";
export { focusSpace, spaceLabel } from "./worktree/spaces.mjs";
export { compose, isPortFree, offsetOf, readEnv } from "./worktree/stack.mjs";
export { featureOf } from "./worktree/tabs.mjs";
export { isWaiting, waitOf, waitsText } from "./worktree/wait.mjs";
export { CONTEXT_LIMIT, filesOf } from "./worktree/workers.mjs";
export { mainDir, worktrees } from "./worktree/worktrees.mjs";

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [command, feature, ...rest] = process.argv.slice(2);
  const flags = new Set([feature, ...rest].filter(a => a?.startsWith("--")));
  if (!commands[command])
    fail(
      "usage: node packages/breadboard/src/worktree.mjs new|orchestrate|plan|stack|control|mr|note|tick|action|wait|issues|herdr|prune|list|remove …",
    );
  await commands[command](
    feature?.startsWith("--") ? undefined : feature,
    flags,
  );
}
