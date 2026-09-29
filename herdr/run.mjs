#!/usr/bin/env node
// The one entry point of the herdr plugin (herdr-plugin.toml). It only turns
// what herdr hands over into a call of ../src/*.mjs:
//
//   node run.mjs restore        the main space and a space per worktree (pnpm wt:herdr)
//   node run.mjs open <pane>    an action: remember where it was triggered, open the pane
//   node run.mjs pane <pane>    inside the pane: run it in the worktree of that place
//   node run.mjs status         one line for tab_bar_right: stacks and apps that run
//
// herdr runs plugin commands in the plugin directory, so the place a key was
// pressed in comes from the context herdr passes (HERDR_PLUGIN_CONTEXT_JSON,
// HERDR_ACTIVE_PANE_CWD). An action gets it; a pane it opens may not, so the
// action writes it down in the plugin state directory for the pane to read.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

import { SPACE, mainDir } from "../src/worktree.mjs";

const REPO = path.resolve(import.meta.dirname, "../../..");
const SCRIPTS = path.resolve(import.meta.dirname, "../src");
const PLUGIN = process.env.HERDR_PLUGIN_ID ?? "monocircuit.platform";
const HERDR = process.env.HERDR_BIN_PATH ?? "herdr";
const STATE = process.env.HERDR_PLUGIN_STATE_DIR;
const saved = STATE ? path.join(STATE, "last-context.json") : null;

const readJson = text => {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
};

/** The directory the key was pressed in: the focused pane, else the space, else the repo. */
const whereFrom = context =>
  context?.focused_pane_cwd ??
  process.env.HERDR_ACTIVE_PANE_CWD ??
  context?.worktree?.checkout_path ??
  context?.workspace_cwd ??
  null;

const context = () => {
  const given = readJson(process.env.HERDR_PLUGIN_CONTEXT_JSON ?? "null");
  if (whereFrom(given)) return given;
  return saved && fs.existsSync(saved)
    ? readJson(fs.readFileSync(saved, "utf-8"))
    : given;
};

/** The worktree of this repo the directory lies in, or null: a pane elsewhere has no stack of ours. */
const worktreeOf = dir => {
  if (!dir) return null;
  const top = spawnSync("git", ["rev-parse", "--show-toplevel"], {
    cwd: dir,
    encoding: "utf-8",
  });
  if (top.status !== 0) return null;
  const common = spawnSync(
    "git",
    ["rev-parse", "--path-format=absolute", "--git-common-dir"],
    { cwd: dir, encoding: "utf-8" },
  );
  const repoCommon = spawnSync(
    "git",
    ["rev-parse", "--path-format=absolute", "--git-common-dir"],
    { cwd: REPO, encoding: "utf-8" },
  );
  return common.stdout.trim() === repoCommon.stdout.trim()
    ? top.stdout.trim()
    : null;
};

const node = (script, args, cwd) =>
  spawnSync(process.execPath, [path.join(SCRIPTS, script), ...args], {
    cwd,
    stdio: "inherit",
    // INIT_CWD is where the scripts look for "you are here", like under pnpm.
    env: { ...process.env, INIT_CWD: cwd, FORCE_COLOR: "1" },
  }).status ?? 1;

/** A popup closes when its command ends: keep the output on screen until a key. */
const waitForKey = () => {
  if (!process.stdin.isTTY) return;
  process.stdout.write("\n\x1b[2m  any key closes this\x1b[0m");
  process.stdin.setRawMode(true);
  const buffer = Buffer.alloc(8);
  fs.readSync(0, buffer, 0, 8, null);
};

const PANES = {
  overview: dir => node("overview.mjs", [], dir),
  commands: dir => {
    // The list is longer than a popup: page it, with its colours.
    const list = spawnSync(
      process.execPath,
      [path.join(SCRIPTS, "commands.mjs")],
      {
        cwd: dir,
        encoding: "utf-8",
        env: { ...process.env, INIT_CWD: dir, FORCE_COLOR: "1" },
      },
    );
    return (
      spawnSync("less", ["-R", "-X", "+g"], {
        input: list.stdout,
        stdio: ["pipe", "inherit", "inherit"],
      }).status ?? 1
    );
  },
  // New workflow code into this session; conversations, nvim and servers keep running.
  reload: () => node("herdr.mjs", ["--reload"], REPO),
  "stack-up": dir => node("worktree.mjs", ["control", "up"], dir),
  "stack-down": dir => node("worktree.mjs", ["control", "down"], dir),
  "stack-ps": dir => node("worktree.mjs", ["control", "ps"], dir),
};
const isPopup = name => name !== "overview";

const [command, name] = process.argv.slice(2);

if (command === "restore") {
  // Plugins are linked for every session of the user. At startup only the
  // session `platform` (pnpm herdr) and one that already had the space get it
  // back — a personal session doesn't get a space it never asked for.
  if (name === "--startup") {
    const isPlatform = (process.env.HERDR_SOCKET_PATH ?? "").includes(
      `${path.sep}sessions${path.sep}platform${path.sep}`,
    );
    const listed = readJson(
      spawnSync(HERDR, ["workspace", "list"], { encoding: "utf-8" }).stdout,
    );
    const hasSpace = listed?.result?.workspaces?.some(w => w.label === SPACE);
    if (!isPlatform && !hasSpace) process.exit(0);
  }
  // No --focus: the space the user was in stays in front.
  process.exit(node("worktree.mjs", ["herdr", "--no-focus"], REPO));
}

if (command === "open") {
  if (!PANES[name]) throw new Error(`unknown pane ${name}`);
  const here = context();
  if (saved) {
    fs.mkdirSync(STATE, { recursive: true });
    fs.writeFileSync(
      saved,
      JSON.stringify({ ...here, focused_pane_cwd: whereFrom(here) }),
    );
  }
  const opened = spawnSync(
    HERDR,
    ["plugin", "pane", "open", "--plugin", PLUGIN, "--entrypoint", name],
    { stdio: "inherit" },
  );
  process.exit(opened.status ?? 1);
}

if (command === "pane") {
  if (!PANES[name]) throw new Error(`unknown pane ${name}`);
  const from = whereFrom(context());
  const tree = worktreeOf(from);
  // Overview and commands make sense anywhere; the stack commands need a worktree of ours.
  const isStack = name.startsWith("stack-");
  if (isStack && !tree) {
    console.log(
      `\n  ${from ?? "this pane"} is not in a worktree of the platform repo, so there is no stack to control.`,
    );
    console.log(
      "  Focus a tab of the repo, or use pnpm stack:<action> --port <p>.",
    );
    waitForKey();
    process.exit(1);
  }
  const dir = tree ? (isStack ? tree : from) : REPO;
  const status = PANES[name](dir);
  if (isPopup(name) && name !== "commands") waitForKey();
  process.exit(status);
}

/**
 * How many agents in the spaces of the repo ask something (herdr: blocked):
 * the main space, its worktrees' spaces and the orchestrations'. Two calls
 * to herdr, nothing to GitLab: the tab bar runs this every few seconds.
 */
const needsYou = () => {
  const ask = args =>
    readJson(spawnSync(HERDR, args, { encoding: "utf-8" }).stdout ?? "")
      ?.result;
  const main = mainDir();
  const ours = new Set(
    (ask(["workspace", "list"])?.workspaces ?? [])
      .filter(
        space =>
          space.worktree?.repo_root === main || space.label.startsWith(SPACE),
      )
      .map(space => space.workspace_id),
  );
  return (ask(["pane", "list"])?.panes ?? []).filter(
    pane => ours.has(pane.workspace_id) && pane.agent_status === "blocked",
  ).length;
};

if (command === "status") {
  // Kept to one docker call: tab_bar_right runs it every few seconds.
  const listed = spawnSync("docker", ["compose", "ls", "--format", "json"], {
    encoding: "utf-8",
  });
  const projects = (readJson(listed.stdout) ?? []).filter(p =>
    p.Name.startsWith("vita"),
  );
  const stacks = projects.length
    ? `⬢ ${projects.length} stack${projects.length === 1 ? "" : "s"}`
    : "⬡ no stack";
  const waiting = needsYou();
  console.log(
    waiting
      ? `${stacks} · ${waiting} need${waiting === 1 ? "s" : ""} you`
      : stacks,
  );
  process.exit(0);
}

console.error(
  "usage: node run.mjs restore | open <pane> | pane <pane> | status",
);
process.exit(1);
