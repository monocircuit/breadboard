import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

import {
  currentPane,
  herdr,
  isInHerdr,
  spaces,
  tabsOf,
} from "../lib/herdr.mjs";
import { git, quote, run } from "../lib/proc.mjs";
import { CHANGELOG, ENTRY, OVERVIEW, ROOT } from "./constants.mjs";
import { sessionFile } from "./conversations.mjs";
import { exclusive } from "./lock.mjs";
import {
  ORCHESTRATOR,
  ORCHESTRATOR_TAB,
  markSpace,
  orchestrationOf,
  orchestratorLabel,
  slotOf,
} from "./origins.mjs";
import { mainSpace, spaceOf } from "./spaces.mjs";
import {
  dropDuplicates,
  ensureSpace,
  ensureTab,
  ensureWorktreeSpace,
  featureOf,
  idlePaneOf,
  keepOrder,
  moveInto,
} from "./tabs.mjs";
import { claimedElsewhere } from "./stack.mjs";
import { allWorktrees, dirOf, mainDir, worktrees } from "./worktrees.mjs";
import { overviewCommand } from "./tabs.mjs";

/**
 * The checkout an orchestration's space stands on, so herdr hangs the space
 * under the main space like a worktree's: herdr groups linked worktrees only,
 * a second space in the main checkout would stay at the top. A worktree
 * without a branch (`--detach`, on origin/develop) in the ignored
 * `.claude/orchestrations/<name>/` of the main checkout, next to the record;
 * worktrees() leaves it out. Nothing works in it, so it gets no install and
 * no stack. The worktrees of the orchestration are worktrees of the main
 * checkout like any other: their spaces sit in the same group, marked with
 * the orchestration (origins.mjs).
 */
export const checkoutOf = name =>
  path.join(mainDir(), ".claude", "orchestrations", name);
/** The checkouts of every orchestration. */
export const orchestratorCheckouts = () =>
  allWorktrees()
    .map(({ dir }) => dir)
    .filter(dir => path.dirname(dir) === path.dirname(checkoutOf("_")));
/** The checkout of the orchestration `name`, made on first use. Null when git refuses. */
export const orchestratorCheckout = name => {
  const dir = checkoutOf(name);
  if (orchestratorCheckouts().includes(dir)) return dir;
  fs.mkdirSync(path.dirname(dir), { recursive: true });
  return git(
    ["worktree", "add", "--quiet", "--detach", dir, "origin/develop"],
    mainDir(),
  ).ok
    ? dir
    : null;
};
/** The checkout of an orchestration goes with its space; a changed file keeps it (git refuses), and says so. */
export const removeOrchestratorCheckout = name => {
  const dir = checkoutOf(name);
  if (!orchestratorCheckouts().includes(dir)) return null;
  return git(["worktree", "remove", dir], mainDir()).ok
    ? null
    : `kept the checkout of ${orchestratorLabel(name)}, it has changes: ${dir}`;
};
/** The tab order of the space `⇶ <name>`; a function, origins.mjs and this import each other. */
export const orchestratorOrder = () => [
  ROOT,
  OVERVIEW,
  ORCHESTRATOR_TAB,
  CHANGELOG,
];
/** What the tab `changelog` runs: the changelog of the orchestration, live (changelog-view.mjs). */
export const changelogTabCommand = name =>
  `node ${quote(ENTRY)} orchestrate ${name} --changelog --watch`;
/**
 * The space `⇶ <name>`: `root`, its `overview` (the issues and merge
 * requests of the orchestration), the orchestrator's tab and `changelog`
 * (what it got done, #248), in that order. The layout calls it for every
 * such space, so overview and changelog come back after a herdr restart and
 * an older space gets them.
 */
export const orchestratorSpace = name => {
  const main = mainDir();
  // A new title (`wt:orchestrate <name> --title`): its space is renamed, what runs in it keeps running.
  const label = orchestratorLabel(name);
  const all = spaces();
  const retitled = all.some(w => w.label === label)
    ? null
    : all.find(
        w =>
          w.label.startsWith(`${ORCHESTRATOR} `) &&
          orchestrationOf(w.label) === name,
      );
  if (retitled) herdr(["workspace", "rename", retitled.workspace_id, label]);
  // On its checkout, in the group of the main space; without one (git refused) at the top, in the main checkout.
  const checkout = orchestratorCheckout(name);
  const workspace = checkout
    ? ensureWorktreeSpace(label, checkout, spaceOf(main), { label: ROOT })
    : ensureSpace(label, main, { label: ROOT });
  if (!workspace) return null;
  // An older space of the orchestration (`Monocircuit/Platform ⇶ <name>`) hands its tabs over; its overview starts anew here.
  for (const old of spaces().filter(
    w => w.workspace_id !== workspace && orchestrationOf(w.label) === name,
  )) {
    const overview = tabsOf(old.workspace_id).find(
      tab => tab.label === OVERVIEW,
    );
    if (overview) herdr(["tab", "close", overview.tab_id]);
    moveInto(old.workspace_id, workspace);
    dropDuplicates(workspace, ROOT);
  }
  const tabs = tabsOf(workspace);
  ensureTab(
    workspace,
    { label: OVERVIEW, cwd: main, command: overviewCommand(null, name) },
    tabs,
  );
  ensureTab(workspace, { label: ROOT, cwd: main }, tabs);
  ensureTab(
    workspace,
    { label: CHANGELOG, cwd: main, command: changelogTabCommand(name) },
    tabs,
  );
  keepOrder(workspace, orchestratorOrder());
  return workspace;
};
/**
 * The space `⇶ <name>` of an orchestration, marked in its colour, with the
 * tab running this — the orchestrating session's — moved in as
 * `orchestrator`. When that was `root` of the main space, the main space
 * gets a new one. A move while wt:orchestrate runs, so the space is never
 * empty long enough for pruneOrchestrators to take it.
 */
export const openOrchestrator = exclusive(async name => {
  if (!isInHerdr() || !mainSpace()) return null;
  const label = orchestratorLabel(name);
  const workspace = orchestratorSpace(name);
  if (!workspace) return null;
  const pane = process.env.HERDR_PANE_ID ? currentPane() : null;
  let isMoved = false;
  if (pane && pane.workspace_id !== workspace) {
    const moved = herdr([
      "pane",
      "move",
      pane.pane_id,
      "--workspace",
      workspace,
      "--new-tab",
    ]);
    const created = moved?.move_result?.created_tab;
    if (created) herdr(["tab", "rename", created.tab_id, ORCHESTRATOR_TAB]);
    isMoved = Boolean(created);
  }
  keepOrder(workspace, orchestratorOrder());
  mainSpace();
  const found = spaces().find(w => w.workspace_id === workspace);
  if (found) markSpace(found, { name, slot: slotOf(name) });
  return `space ${label}${isMoved ? `, this tab moved in as ${ORCHESTRATOR_TAB}` : ""}`;
});
/** `Orchestrated work: an orchestrator space, marked …` → `orchestrated-work-orchestrator-space-marked`. */
const STOPWORDS = new Set([
  "a",
  "an",
  "the",
  "of",
  "in",
  "on",
  "for",
  "to",
  "and",
  "or",
  "with",
  "by",
  "at",
  "from",
  "is",
]);
export const slugOf = title => {
  const words = title
    .toLowerCase()
    // é → e + a combining accent, which goes: it would split the word.
    .normalize("NFKD")
    .replace(/\p{M}/gu, "")
    .replace(/[^a-z0-9]+/g, " ")
    .split(" ")
    .filter(word => word && !STOPWORDS.has(word));
  // Up to five whole words, at most 40 characters: the name is typed and read in the sidebar.
  let slug = words[0]?.slice(0, 40) ?? "issue";
  for (const word of words.slice(1, 5)) {
    if (`${slug}-${word}`.length > 40) break;
    slug = `${slug}-${word}`;
  }
  return slug;
};
/**
 * An issue of `wt:orchestrate` (its number, or `<n>-<feature>`) as GitLab
 * has it, with the worktree it gets and `why` it can't get one (closed, its
 * worktree there already, claimed on origin). Null when it can't be read.
 */
export const readIssue = arg => {
  const number = arg.match(/^\d+/)[0];
  const found = run("glab", ["issue", "view", number, "-F", "json"], {
    cwd: mainDir(),
    quiet: true,
    allowFail: true,
  });
  if (!found.ok) return null;
  const issue = JSON.parse(found.out);
  const feature = /^\d+$/.test(arg) ? `${number}-${slugOf(issue.title)}` : arg;
  const isFix = (issue.labels ?? []).includes("bug");
  // wt:new would stop at it too, but only after the ones before were made.
  const why =
    issue.state !== "opened"
      ? `#${number} is ${issue.state}`
      : fs.existsSync(dirOf(feature))
        ? `${path.basename(dirOf(feature))} exists already`
        : claimedElsewhere(feature) &&
          `claimed by ${claimedElsewhere(feature)}`;
  return {
    number,
    title: issue.title,
    description: issue.description ?? "",
    feature,
    isFix,
    dir: dirOf(feature),
    why,
    branch: `${isFix ? "fix" : "feature"}/${feature}`,
  };
};
/** An issue as the record of an orchestration keeps it. */
export const recordEntry = (item, why = item.why || null) => ({
  number: Number(item.number),
  title: item.title,
  feature: item.feature,
  branch: item.branch,
  why,
});
/**
 * `wt:new <feature> --origin <name>` for an issue of readIssue, its output
 * shown. `{ isThere, failed, status }`: whether the worktree exists, the
 * reason wt:new failed with (it may fail after the setup, #130), its exit code.
 */
export const makeWorktree = (item, name, { isShared = false } = {}) => {
  const args = [ENTRY, "new", item.feature, "--origin", name];
  if (item.isFix) args.push("--fix");
  if (isShared) args.push("--shared");
  const made = spawnSync(process.execPath, args, {
    cwd: mainDir(),
    encoding: "utf-8",
    stdio: ["ignore", "inherit", "pipe"],
  });
  const errors = made.stderr ?? "";
  if (errors.trim()) process.stderr.write(errors);
  return {
    isThere: worktrees().some(({ dir }) => dir === item.dir),
    // fail() of wt:new says `✗ <reason>`: a step that failed, the worktree is half set up at best.
    failed: errors.match(/^✗ (.*)$/m)?.[1] ?? null,
    status: made.status,
  };
};
/** The file an orchestrated agent is briefed with, in the ignored `.claude/` of its worktree. */
const briefFile = dir => path.join(dir, ".claude", "brief.md");
export const writeBrief = (dir, brief) => {
  fs.mkdirSync(path.dirname(briefFile(dir)), { recursive: true });
  fs.writeFileSync(briefFile(dir), brief);
};
/** Its first prompt: short, one line and without quotes, so every shell takes it as typed. */
export const BRIEF_PROMPT =
  "Read .claude/brief.md and work on the issue it describes.";
/**
 * What an orchestrated agent is told. It stays in its worktree, works like
 * any session there, and the overview shows where it is (AGENTS,
 * WORKTREES); SendMessage to the orchestrator only for what needs it.
 */
export const briefOf = ({
  number,
  title,
  dir,
  branch,
  name,
  peer,
  extra,
  before,
}) =>
  `# #${number} ${title}

You work on issue #${number} of monocircuit/platform, started by the orchestration \`${name}\` (herdr space \`${orchestratorLabel(name)}\`).
${
  before
    ? `Your worktree is \`${dir}\` on \`${branch}\`: it exists, its branch is pushed as the claim. You come from another worktree: \`EnterWorktree(path: "${dir}")\` once, if you don't stand in it yet; after that don't run \`wt:new\`, \`EnterWorktree\` or \`ExitWorktree\`.`
    : `Your worktree is \`${dir}\` on \`${branch}\`: it exists, its branch is pushed as the claim, you already stand in it.
Don't run \`wt:new\`, \`EnterWorktree\` or \`ExitWorktree\`.`
}

1. Read the issue: \`glab issue view ${number} --comments\`, then the AGENTS.md files and skills it touches.
2. Work as AGENTS.md says: commit and push per green step, the draft MR with the first commit (\`pnpm wt:mr\`), boxes ticked with \`pnpm wt:tick\`. Push every commit at once, never hold one back: the overview shows unpushed commits, and past 10 minutes they go to its NEXT.
   A change requested for your MR (by the user or the orchestrator): \`pnpm wt:action change "<what>"\` first, it makes the MR a draft and withdraws your merge action, then build it; once it is pushed and green, post the merge again. Merged meanwhile: the command says so, and the change gets its own issue, branch and draft MR at once.
   Your issue started at once, even when it builds on an open MR or its files move in one: when you have to wait for other MRs, run \`pnpm wt:wait !N … --then "<what next>"\` with \`run_in_background\` and stop; don't ask the orchestrator. Its line wakes you once they are merged (or why the wait ended).
3. **The overview shows your work, you don't report it.** Its AGENTS shows herdr's state of your agent and what you wait for, its WORKTREES and MERGE REQUESTS show commits, MR, boxes and pipeline: no note, no status message.
   \`SendMessage\` to the orchestrator only for what needs it, one short message:
   - the plan, when it differs from the issue;
   - a decision that is the user's (options, your recommendation), then wait for the answer instead of guessing; with its \`decide\` action posted, the message is only context: the user answers on the action's buttons, and the orchestrator relays it only while the action is open;
   - what concerns another agent's work (an order, a conflict, a dependency).
   No message for a stage end or the end: the overview has them.
   ${peer ? `The orchestrator is the session \`${peer}\` (\`ListAgents\`).` : `The orchestrator messages you first: reply to the \`from\` of its message. Before that, \`ListAgents\` finds it (it works in \`${mainDir()}\`).`}
4. **Whenever you stop and the next step is the user's, leave an action for it** with \`pnpm wt:action\`. The overview of the orchestration answers "what now?" from these (its NEXT line and ACTIONS): the user must never have to ask you what is next.
   - \`pnpm wt:action merge "<what is in it>"\` once the pipeline of the MR passed and it is its turn in the merge order (\`wt:plan\`): mark it ready (\`glab mr update --ready\`) and post it yourself, the overview's automatic one is only a fallback; when your permissions block \`--ready\`, post it anyway with "mark ready + merge" in the text. Never merge yourself;
   - \`pnpm wt:action try "<what to check>" --run "<command>"\` (a terminal, e.g. \`pnpm overview --orchestration <name>\` of your branch) or \`--url <page>\` (\`/atlas/…\` on your worktree's app, its stack started first) for something only a person can judge: the click opens that spot. Every box you leave open for the user's live check gets one, and the box names it (\`… (try #<id>)\`);
   - \`pnpm wt:action decide "<what is decided, one sentence>? <why the recommendation, one line>" --option "<what happens>" --option "<what happens>" --recommend <option>\`: the user answers from the row, so the question says precisely what is decided, each option (a button) says what happens, not \`A\`/\`B\`, and \`--recommend\` marks yours with ★. Together with the SendMessage to the orchestrator that explains them;
   - \`pnpm wt:action plan "<the plan in one line>"\` before larger work, for an \`ok\` or \`different\`;
   - \`pnpm wt:action look "<what>"\` for anything else the user has to do.
   The user's click arrives typed into this tab as one line, \`[overview] #<id> <kind> "<text>" → <answer>\`: go on with it, don't wait or poll for it. The answered action goes by itself. Once one no longer applies (done by the user, overtaken), \`pnpm wt:action --done <id>\` (\`--list\` shows them). While you are working, leave none.
5. The user may type into this tab directly: their word wins over the orchestrator's.
6. Your next issue may come as one line, \`[overview] next issue #<n>: EnterWorktree(path: "<dir>"), then read .claude/brief.md\`: do just that, the brief there is your task from then on.
${before ? `\n## Before this issue\n\n${before}\n` : ""}${extra ? `\n## From the orchestrator\n\n${extra.trim()}\n` : ""}`;
/**
 * The claude of an orchestrated worktree: its brief written, its conversation
 * bound to the space before it starts (an own session id), so a herdr restart
 * resumes it like any other. Inside herdr it starts in the `<feature>` tab —
 * the pane's shell has no CLAUDE* variables (pnpm herdr), so it keeps its own
 * transcript and ListAgents finds it by its name, the feature. Returns what
 * happened, or the command to run by hand.
 */
export const startAgent = (dir, brief) => {
  const feature = featureOf(dir);
  writeBrief(dir, brief);
  const id = crypto.randomUUID();
  const command = `cd ${quote(dir)} && claude --session-id ${id} --name ${feature} ${quote(BRIEF_PROMPT)}`;
  const workspace = isInHerdr() ? spaceOf(dir) : null;
  const pane = workspace ? idlePaneOf(workspace, feature) : null;
  // `pane run` answers with nothing on success, so its exit code, not herdr()'s result.
  const isStarted =
    Boolean(pane) &&
    spawnSync("herdr", ["pane", "run", pane, command], { stdio: "ignore" })
      .status === 0;
  if (isStarted) fs.writeFileSync(sessionFile(dir), `${id}\n`);
  return { isStarted, command };
};
