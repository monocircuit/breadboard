// The view of one worktree (--worktree): WORK, AGENT, ISSUE, PIPELINE, DIFF,
// DEVELOP, WHERE, STACK.
import os from "node:os";
import path from "node:path";

import { canGroupTabs } from "../browser.mjs";
import {
  bold,
  cyan,
  dim,
  fit,
  green,
  link,
  red,
  table,
  yellow,
} from "../lib/tty.mjs";
import { barOf } from "./diff.mjs";
import {
  ORCHESTRATOR,
  SPACE,
  STORYBOOK_PORT,
  STUDIO_PORT,
  isInHerdr,
  mainDir,
  startStorybook,
} from "../worktree.mjs";
import {
  isStarting,
  pushAgent,
  pushDevelop,
  pushDisabledRemove,
  pushIssue,
  pushMigrations,
  pushPipeline,
  pushRemoveButton,
  pushStackButton,
  removing,
} from "./actions.mjs";
import { isOnce, only } from "./args.mjs";
import {
  button,
  closeBox,
  openBox,
  pipelineMark,
  titleLines,
} from "./boxes.mjs";
import { editorTrouble, fileLine, openFile, pushFiles } from "./files.mjs";
import { open, show } from "./io.mjs";
import { KEYS, pushRows } from "./screen.mjs";

export const tokens = count =>
  count >= 1_000_000
    ? `${+(count / 1_000_000).toFixed(1)}M`
    : `${Math.round(count / 1000)}k`;
export const ago = date => {
  const minutes = Math.floor((Date.now() - date.getTime()) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  if (minutes < 48 * 60) return `${Math.floor(minutes / 60)} h ago`;
  return `${Math.floor(minutes / (24 * 60))} days ago`;
};
const BAR = 20;
/** A path with the home directory as `~`. */
export const home = file =>
  file.startsWith(os.homedir()) ? `~${file.slice(os.homedir().length)}` : file;
/** The conversation of the space: a bar of its context, filled, then model and last answer. */
export const claudeLine = ({ id, model, at, used, window }) => {
  if (!at) return dim(`${id.slice(0, 8)} · no answer yet`);
  const share = Math.min(used / window, 1);
  const tone = share >= 0.8 ? red : share >= 0.5 ? yellow : green;
  const filled = Math.round(share * BAR);
  const bar = `${tone("█".repeat(filled))}${dim("░".repeat(BAR - filled))}`;
  return `${bar} ${tone(`${Math.round(share * 100)}%`)} ${tokens(used)} of ${tokens(window)}  ${dim(`${model.replace(/^claude-/, "")} · ${ago(at)} · ${id.slice(0, 8)}`)}`;
};
/** The view of one worktree: where it runs, its stack, its MR. */
/** Where an MR stands: open as a draft or ready, or done. */
export const mrState = mr => {
  if (mr.state === "merged") return cyan(" merged");
  if (mr.state === "closed") return dim(" closed");
  return mr.isDraft ? dim(" draft") : green(" ready");
};
const MAX_AREAS = 6;
const BAR_WIDTH = 24;
export const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;
/** A few files by name, the rest as a count: a line, not a list. */
export const someFiles = (files, count = 3) =>
  files.slice(0, count).join(", ") +
  (files.length > count ? dim(` +${files.length - count} more`) : "");
/**
 * The section DIFF of a worktree (#114): how far its work is from develop,
 * where it lies, and where it collides with develop or another worktree.
 */
const pushDiff = (lines, diff, dir) => {
  const counts = [
    `${diff.ahead} ahead`,
    diff.behind ? yellow(`${diff.behind} behind`) : dim("0 behind"),
    diff.uncommitted ? yellow(`${diff.uncommitted} uncommitted`) : "",
  ].filter(Boolean);
  const start = openBox(lines);
  if (diff.files === 0) {
    lines.push(dim("  nothing changed yet"));
  } else {
    const total = barOf(
      diff.added,
      diff.removed,
      diff.added + diff.removed,
      BAR_WIDTH,
    );
    lines.push(
      `  ${green(`+${diff.added}`)} ${red(`−${diff.removed}`)} in ${plural(diff.files, "file")}  ${green(total.added)}${red(total.removed)}`,
    );

    const shown = diff.areas.slice(0, MAX_AREAS);
    const rest = diff.areas.slice(MAX_AREAS);
    const max = Math.max(...shown.map(area => area.added + area.removed));
    const rows = shown.map(area => {
      const bar = barOf(area.added, area.removed, max, BAR_WIDTH);
      return [
        "",
        area.name,
        green(`+${area.added}`),
        red(`−${area.removed}`),
        `${green(bar.added)}${red(bar.removed)}`,
        dim(`${plural(area.files, "file")} · ${path.basename(area.top)}`),
      ];
    });
    const actions = shown.map(area => () => openFile(dir, area.top));
    if (rest.length > 0) {
      const sum = key => rest.reduce((total, area) => total + area[key], 0);
      rows.push([
        "",
        dim(`… ${plural(rest.length, "area")}`),
        green(`+${sum("added")}`),
        red(`−${sum("removed")}`),
        "",
        dim(plural(sum("files"), "file")),
      ]);
    }
    lines.push("");
    pushRows(lines, rows, actions);
  }

  if (diff.withDevelop.length > 0 || diff.withWorktrees.length > 0)
    lines.push("");
  if (diff.withDevelop.length > 0) {
    pushFiles(
      lines,
      fileLine(`  ${yellow("⚠")} also changed on develop: `, diff.withDevelop, {
        suffix: `  ${dim("merge develop in while it is small")}`,
      }),
      dir,
    );
  }
  for (const other of diff.withWorktrees) {
    pushFiles(
      lines,
      fileLine(
        `  ${yellow("⚠")} also changed in ${bold(other.name)}: `,
        other.files,
      ),
      dir,
    );
  }
  closeBox(
    lines,
    start,
    `${bold("DIFF")} ${dim("against develop")}  ${counts.join(" · ")}  ${dim("click a file: nvim at the change")}`,
  );
};
/**
 * WAITING FOR: the open MRs this worktree builds on (waiting.mjs), in the
 * order they have to be merged; nothing when it builds on develop alone. A
 * click opens one.
 */
const pushWaiting = (lines, waiting) => {
  if (!waiting?.length) return;
  const start = openBox(lines);
  pushRows(
    lines,
    waiting.map((mr, i) => [
      "",
      dim(`${i + 1}.`),
      `${link(mr.url, `!${mr.iid}`)}${mrState(mr)}`,
      pipelineMark(mr.pipeline),
      mr.title,
    ]),
    waiting.map(mr => () => show(mr.url)),
  );
  const next = waiting[0];
  closeBox(
    lines,
    start,
    `${bold("WAITING FOR")} ${dim(`merge in this order; next: !${next.iid}, then this branch goes on`)}`,
  );
};

export const renderWorktree = (local, remote, error) => {
  const width = process.stdout.columns || 120;
  const tree = local.trees[0];
  const keys = isOnce ? "" : dim(KEYS);
  if (!tree)
    return `${bold(path.basename(only))}  ${red("is not a worktree of the repo any more")}  ${keys}`;

  const lines = [];
  const isMain = tree.dir === mainDir();
  const isShared = !tree.hasOwnPort;
  const offset = isShared ? 0 : tree.offset;
  const env = tree.env ?? {};
  const port = (variable, base) => Number(env[variable]) || base + offset;
  lines.push(
    ...titleLines(
      `${tree.name}  ${tree.branch}`,
      new Date().toLocaleTimeString(),
      isOnce ? "" : KEYS,
    ),
  );
  lines.push(dim(` ${tree.dir}`));
  if (editorTrouble) lines.push(yellow(` nvim: ${editorTrouble}`));

  const work = openBox(lines);

  // A worktree's work gets the section DIFF below issue and MR; the main checkout this line.
  if (!local.diff) {
    const work = [
      tree.ahead ? `${tree.ahead} ahead of develop` : "",
      tree.changed ? yellow(`${tree.changed} changed`) : "",
    ]
      .filter(Boolean)
      .join(" · ");
    lines.push(
      ...table([["", "work", work || dim("clean, nothing ahead of develop")]]),
    );
  }
  if (error) lines.push(red(`GitLab: ${error.split("\n")[0]}`));
  // One table with issue and MR, so their columns line up; the conversation is AGENT's.
  const rows = [];
  const actions = [];
  if (tree.origin) {
    // Started by wt:orchestrate: the agent here reports to that session.
    rows.unshift([
      "",
      "origin",
      `${cyan(`${ORCHESTRATOR} ${tree.origin.name}`)} ${dim("the orchestrating session, its agent reports there")}`,
    ]);
    actions.unshift(null);
  }
  if (remote) {
    if (remote.issue) {
      rows.push([
        "",
        "issue",
        `${link(remote.issue.url, `#${remote.issue.iid}`)} ${remote.issue.title}${remote.issue.state === "closed" ? dim(" (closed)") : ""}`,
      ]);
      actions.push(() => show(remote.issue.url));
    }
    rows.push(
      remote.mr
        ? [
            "",
            "mr",
            `${link(remote.mr.url, `!${remote.mr.iid}`)}${mrState(remote.mr)} ${pipelineMark(remote.mr.pipeline)}  ${dim(remote.mr.url)}`,
          ]
        : ["", "mr", dim(isMain ? "—" : "none yet: pnpm wt:mr")],
    );
    actions.push(remote.mr && (() => show(remote.mr.url)));
  }
  pushRows(lines, rows, actions);
  if (!remote && !error) {
    lines.push(dim("  GitLab …"));
  }
  // Below issue and MR: the button is the last step of a worktree.
  if (local.isDone || removing?.isRunning) {
    pushRemoveButton(lines, tree);
  } else if (!isMain && tree.branch !== "(detached)") {
    pushDisabledRemove(lines, tree, local.ownCommits, remote?.mr);
  }
  closeBox(lines, work, bold("WORK"));
  pushWaiting(lines, remote?.waiting);
  pushAgent(lines, local.claude, local.agentStatus);
  pushIssue(lines, remote?.issue);
  pushPipeline(lines, remote?.mr, remote?.jobs ?? [], tree.dir);
  if (local.diff) pushDiff(lines, local.diff, tree.dir);
  // The project's URL, from any link GitLab gave: `https://…/platform/-/issues/158` → `https://…/platform`.
  pushDevelop(
    lines,
    tree.dir,
    local.develop,
    (remote?.issue?.url ?? remote?.mr?.url)?.split("/-/")[0],
  );

  if (!tree.hasEnv) {
    const start = openBox(lines);
    lines.push(yellow("  no .env.local: no stack, the app can't run here"));
    closeBox(lines, start, bold("STACK"));
    return lines.map(line => fit(line, width)).join("\n");
  }

  const isUp = tree.stack?.includes("running");
  const mark = isOn => (isOn ? green("●") : dim("○"));
  const where = [
    {
      isOn: local.isAppUp,
      name: "app",
      url: `http://localhost:${3000 + offset}`,
      hint: local.isAppUp
        ? isShared
          ? dim("shared: whichever checkout runs pnpm dev")
          : ""
        : dim("cd products/vita/web && pnpm dev"),
    },
    {
      isOn: isUp,
      name: "keycloak",
      url: `http://localhost:${port("KEYCLOAK_PORT", 8080)}/admin`,
    },
    {
      isOn: isUp,
      name: "s3",
      url: `http://localhost:${port("S3_PORT", 8333)}`,
    },
    // Not http: nothing to open.
    {
      isOn: isUp,
      name: "postgres",
      address: `localhost:${port("POSTGRES_PORT", 5432)}`,
    },
    {
      isOn: local.isStudioUp,
      name: "studio",
      url: `https://local.drizzle.studio?port=${STUDIO_PORT + offset}`,
      hint: local.isStudioUp
        ? ""
        : dim(`pnpm pg:studio --port ${STUDIO_PORT + offset}`),
    },
    // Down, a click starts it in a tab of the space; the page of the extension waits for it.
    {
      isOn: local.isStorybookUp,
      name: "storybook",
      url: `http://localhost:${STORYBOOK_PORT + offset}`,
      hint: local.isStorybookUp
        ? ""
        : isInHerdr()
          ? button("▶ start storybook")
          : dim(
              `cd packages/monolithium && pnpm exec storybook dev -p ${STORYBOOK_PORT + offset}`,
            ),
      start: () => startStorybook(tree.dir),
    },
  ];
  const whereBox = openBox(lines);
  pushRows(
    lines,
    where.map(w => [
      "",
      mark(w.isOn),
      w.name,
      w.url ? link(w.url, w.url) : w.address,
      w.hint ?? "",
    ]),
    where.map(w => {
      if (!w.url) return null;
      if (w.isOn || !w.start) return () => open(w.url);
      // Without the extension a browser would only show a connection error: the row turns ● once it runs.
      return () => w.start().then(() => canGroupTabs() && open(w.url));
    }),
  );
  closeBox(
    lines,
    whereBox,
    `${bold("WHERE")} ${dim("click a row to open it")}`,
  );

  const stackBox = openBox(lines);
  if (isShared) {
    lines.push(
      dim(`  its tabs are in the space ${SPACE}; an own stack: pnpm wt:stack`),
    );
  } else if (!local.hasDocker) {
    lines.push(yellow("  docker does not answer"));
  } else {
    lines.push(
      ...table(
        local.containers.map(c => [
          "",
          c.service,
          c.state === "running" ? green(c.state) : yellow(c.state),
          c.health === "healthy"
            ? green(c.health)
            : c.health
              ? yellow(c.health)
              : "",
        ]),
      ),
    );
    // Down, there are no container tabs to start it from: the button is the way.
    if (!isUp || isStarting(tree.dir))
      pushStackButton(lines, tree.dir, "the stack");
  }
  // The database of the stack, own or shared, against the migrations of this checkout.
  if (isUp || isShared) pushMigrations(lines, tree.dir, local.migrations);
  closeBox(
    lines,
    stackBox,
    `${bold("STACK")} ${dim(isShared ? `shared, the one of the main checkout (${tree.project})` : tree.project)}`,
  );

  lines.push(
    "",
    dim(
      " pnpm stack:up|down|reset · pnpm wt:mr --update · pnpm overview (everything)",
    ),
  );
  return lines.map(line => fit(line, width)).join("\n");
};
