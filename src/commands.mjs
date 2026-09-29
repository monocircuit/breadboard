#!/usr/bin/env node
// Every pnpm command of the repo, with what it is for.
//
//   pnpm commands          the package you stand in, then the commands of the root
//   pnpm commands --all    every package of the workspace
//
// The scripts come from the package.json files, so nothing here goes out of
// date: a script without a line in DESCRIPTIONS is shown with its command.
import fs from "node:fs";
import path from "node:path";

import { bold, cyan, dim } from "./lib/tty.mjs";

/** `<package dir>` → script → what it is for. The root is "". */
const DESCRIPTIONS = {
  "": {
    build: "build every package (turbo)",
    dev: "every dev server (turbo)",
    "dev:vita": "the vita dev server, from anywhere",
    "build:vita": "a production build of vita",
    storybook: "the monolithium Storybook on :6006",
    lint: "lint every package (turbo)",
    typecheck: "typecheck every package (turbo)",
    test: "unit tests of every package",
    changeset: "note a change of monolithium/utilities for the next release",
    commands: "this list (--all: every package)",
    "check:agents":
      "every AGENTS.md, skill and subagent within 150 lines, every link in them resolves",
    herdr:
      "the pinned herdr with the repo's session and plugin (--reload new breadboard code, --stop)",
    "herdr:install":
      "the repo in your own herdr instead (e.g. Omarchy's): plugin, keys, the space Monocircuit/Platform",
    browser:
      "the Chromium extension for a tab group per herdr space: install it, say how to load it",
    overview:
      "live: worktrees, stacks, merge requests and issues (q quits, --worktree <dir> for one, --orchestration <name> for its issues and MRs)",
    "stack:up":
      "start the vita stack — of the worktree on --port <p> / <feature>, else yours",
    "stack:down": "stop that stack, data stays",
    "stack:reset": "throw away the data of that stack and set it up again",
    "stack:ps": "the containers of that stack",
    "wt:new":
      "<feature> a worktree on a new branch with its own stack (--shared, --fix, --origin <orchestration>)",
    "wt:orchestrate":
      "<name> <issue>… a space ⇶ <name> for this session, per issue a worktree with a briefed claude (--dry-run); <name> <issue> --to <agent> hands it to an idle agent; <name> --end ends it once its worktrees are gone",
    "wt:plan":
      "<name> [--work <n>…] [--merge <n>…] [--why <t>] the order of an orchestration's issues and merges, for its overview",
    "wt:stack": "give the worktree you are in its own stack",
    "wt:mr":
      "open the draft MR of this worktree (--update sends the description again)",
    "wt:note": "<issue> --message-file <f> comment a file on an issue",
    "wt:tick":
      "[<issue>] [<box>…] the boxes of the issue, numbered; ticks the given ones",
    "wt:action":
      "<kind> <text> [--url <u> | --run <cmd>] ask the human for merge, try (opens the spot to check), decide, plan or look: a row in ACTIONS of the orchestration's overview (--list, --done <id>, --orchestration <name> for the orchestrator); note <line>: what you do, a row in FEED; change <what>: a change for the MR, draft at once and its merge withdrawn (--mr !N with --orchestration)",
    "wt:wait":
      '!<mr>… [--then "<what>"] run in the background: ends with the line to go on with once the MRs are merged (early when one is closed or stays in conflict, after --hours, default 8); the overview shows the wait (--cancel)',
    "wt:issues": "issues of merged MRs nobody commented on since the merge",
    "wt:prune": "close worktrees and branches of merged MRs (--dry-run)",
    "wt:herdr":
      "herdr: root and overview, a space per worktree, tabs of running stacks; after a restart it all runs again",
    "wt:list": "every worktree: branch, url, stack",
    "wt:remove": "<feature> stack, worktree and merged branch away",
  },
  "products/vita": {
    commands: "this list: vita first, then the root",
    overview: "live: worktrees, stacks, merge requests and issues (q quits)",
    dev: "the app on the port of this worktree, logged in",
    "dev:ready":
      "[<route>…] wait until pnpm dev answers and has built these pages (default /editor)",
    "dev:setup":
      "stack up, migrate, bucket, Keycloak, seed — a fresh stack in one go",
    "dev:webpack": "the app with webpack instead of turbopack",
    "dev:cpu": "the app with a CPU profile",
    logs: "[<path>…] what the browser's devtools logger wrote: entries, --state <name>, --paths, --since 2m",
    build: "production build",
    "build:cpu": "production build with a CPU profile",
    "build:debug": "production build with memory debugging",
    "build:migrate": "bundle the migration runner for the image",
    start: "serve the production build",
    "stack:up": "start Postgres, Keycloak and SeaweedFS of this checkout",
    "stack:down": "stop the containers of this checkout, data stays",
    "stack:reset": "throw away the data of this checkout and set it up again",
    "pg:generate":
      "generate a migration from src/server/db/schema.ts (skill db-migration)",
    "pg:migrate": "apply the migrations",
    "pg:seed": "fill the database with the dev data",
    "pg:studio": "Drizzle Studio on the database",
    "pg:visualizer": "the schema as a diagram, on the port of this worktree",
    "storage:setup": "create the bucket in SeaweedFS",
    "keycloak:setup":
      "the MCP clients, their scopes and the login theme in Keycloak",
    lint: "ESLint",
    typecheck: "next typegen and tsc",
    test: "unit tests",
    "test:coverage": "unit tests with coverage (CI has a floor)",
    "test:int": "the operations against the real Postgres and object storage",
  },
  "packages/monolithium": {
    storybook: "Storybook on :6006",
    "build-storybook": "static Storybook, like CI",
    "check-contrast": "contrast check of the built Storybook",
  },
  "packages/eslint-config": {
    build: "build the configs (dev) and types",
    "build:prod": "build the configs as the hooks and CI load them",
  },
};

const ALWAYS_HIDDEN = new Set(["preinstall"]);

const root = path.resolve(import.meta.dirname, "../../..");
const readJson = file => JSON.parse(fs.readFileSync(file, "utf-8"));

/** The workspace packages, from pnpm-workspace.yaml's `dir/*` globs. */
const packages = () =>
  fs
    .readFileSync(path.join(root, "pnpm-workspace.yaml"), "utf-8")
    .split("\n")
    .map(line => line.match(/^\s*-\s*["']?([^"'*]+)\/\*["']?\s*$/)?.[1])
    .filter(Boolean)
    .flatMap(dir =>
      fs.existsSync(path.join(root, dir))
        ? fs.readdirSync(path.join(root, dir)).map(name => path.join(dir, name))
        : [],
    )
    .filter(dir => fs.existsSync(path.join(root, dir, "package.json")))
    .sort();

/** The package the command was typed in: pnpm runs the script elsewhere, INIT_CWD keeps the place. */
const here = () => {
  let dir = path.resolve(process.env.INIT_CWD ?? process.cwd());
  while (dir.startsWith(root) && dir !== root) {
    if (fs.existsSync(path.join(dir, "package.json")))
      return path.relative(root, dir);
    dir = path.dirname(dir);
  }
  return "";
};

const section = (dir, prefix) => {
  const { name, scripts = {} } = readJson(path.join(root, dir, "package.json"));
  const groupOf = script => script.split(":")[0];
  // Grouped by prefix (dev, pg, stack, wt, …), in the order the groups first appear.
  const order = [...new Set(Object.keys(scripts).map(groupOf))];
  const entries = Object.entries(scripts)
    .filter(([script]) => !ALWAYS_HIDDEN.has(script))
    .sort(([a], [b]) => order.indexOf(groupOf(a)) - order.indexOf(groupOf(b)));
  if (entries.length === 0) return;
  const sizeOf = group =>
    entries.filter(([script]) => groupOf(script) === group).length;
  const said = DESCRIPTIONS[dir] ?? {};
  const width = Math.max(
    ...entries.map(([script]) => `${prefix}${script}`.length),
  );
  console.log(`\n${bold(name)} ${dim(dir || "(root)")}`);
  let group = null;
  for (const [script, command] of entries) {
    // A group of several gets blank lines around it; single scripts stay together.
    const next = groupOf(script);
    if (
      group !== null &&
      next !== group &&
      (sizeOf(next) > 1 || sizeOf(group) > 1)
    )
      console.log("");
    group = next;
    const call = `${prefix}${script}`.padEnd(width);
    console.log(
      `  ${cyan(call)}  ${said[script] ?? dim(command.length > 70 ? `${command.slice(0, 69)}…` : command)}`,
    );
  }
};

const isAll = process.argv.includes("--all");
const current = here();

if (current) section(current, "pnpm ");
section("", current ? "pnpm -w " : "pnpm ");
if (isAll) {
  for (const dir of packages().filter(dir => dir !== current)) {
    const { name } = readJson(path.join(root, dir, "package.json"));
    section(dir, `pnpm --filter ${name} `);
  }
}
console.log(
  dim(
    `\n  ${current ? "pnpm -w <script> runs a root command from here. " : ""}${isAll ? "" : "pnpm commands --all: every package. "}In a package: its own commands first.\n`,
  ),
);
