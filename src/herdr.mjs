#!/usr/bin/env node
// herdr for this repo in one command, see packages/workflow/herdr/README.md.
//
//   pnpm herdr                    the pinned herdr in .herdr/, the session `platform`
//                                 with the repo's config and plugin, in a new window
//                                 of your terminal without padding (kitty, alacritty, ghostty)
//   pnpm herdr --here             attach in this terminal instead
//   pnpm herdr --no-attach        the same, without attaching to it
//   pnpm herdr --reload           new workflow code into the running herdr: develop
//                                 fast-forwarded, config, overviews restarted; claude,
//                                 nvim and servers keep running (prefix+shift+l). In a
//                                 pane the session it runs in, else each one with the space
//   pnpm herdr --stop             stop the session `platform` (ends its panes)
//   pnpm herdr:install            your own herdr >= 0.9 (Omarchy's too): the plugin linked,
//                                 its keys in your config, the space in your default session
//
// Without herdr:install, your default herdr session and
// ~/.config/herdr/config.toml are never written: the repo's session has its
// own server, and its config lives in .herdr/config.toml of the main checkout.
// With it, your config gets the plugin's keys as one marked block, after a
// question.
//
// No dependencies, like worktree.mjs: it runs before anything is installed.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { fail } from "./lib/proc.mjs";
import { installIntoOwn } from "./session/own.mjs";
import { pullDevelop, reload, reloadTargets } from "./session/reload.mjs";
import {
  PLUGIN,
  SESSION,
  VERSION,
  bin,
  cleanEnv,
  configFile,
  herdr,
  isInPlatform,
  isRunning,
  main,
  pluginDir,
  serverVersion,
  sessionEnv,
  step,
} from "./session/session.mjs";
import {
  ensureBinary,
  linkPlugin,
  startServer,
  terminalWindow,
  writeConfig,
  writeLinked,
} from "./session/setup.mjs";
import { SPACE } from "./worktree.mjs";

const flags = new Set(process.argv.slice(2));
if (!pluginDir) fail("packages/workflow/herdr is missing in this checkout");
if (flags.has("--install")) {
  await installIntoOwn();
  process.exit(0);
}
if (flags.has("--reload")) {
  const targets = reloadTargets();
  if (!targets.length)
    fail(
      `neither the session ${SESSION} nor your herdr runs with the space ${SPACE}: pnpm herdr, or pnpm herdr:install`,
    );
  // The pull can bring new workflow code, but this process runs what it
  // imported before it: the rest goes on in one that runs the pulled code.
  if (!flags.has("--pulled") && pullDevelop()) {
    const pulled = path.join(main, "packages", "workflow", "src", "herdr.mjs");
    const again = spawnSync(
      process.execPath,
      [pulled, "--reload", "--pulled"],
      {
        stdio: "inherit",
      },
    );
    process.exit(again.status ?? 1);
  }
  for (const binary of targets) await reload(binary);
  process.exit(0);
}
if (flags.has("--stop")) {
  if (!fs.existsSync(bin) || !isRunning()) {
    console.log(`\n✓ the session ${SESSION} is not running`);
    process.exit(0);
  }
  step(`stop the session ${SESSION} (its panes end)`);
  herdr(["server", "stop"], { isQuiet: false });
  process.exit(0);
}
const isAttachingHere = flags.has("--here") || !terminalWindow([]);
// Inside a herdr pane, attaching here would nest a herdr in it; a window of its own doesn't.
if (process.env.HERDR_ENV === "1") {
  if (isInPlatform()) fail(`you are in the session ${SESSION} already`);
  if (isAttachingHere && !flags.has("--no-attach")) {
    fail(
      "this is a herdr pane already, and no terminal was found to open a window of its own. In a plain terminal: pnpm herdr — or pnpm herdr:install to add the plugin to this herdr",
    );
  }
}
await ensureBinary();
const { base, isChanged, hasOwnRows } = writeConfig();
step(
  `config ${path.relative(main, configFile)} from ${base.startsWith(main) ? path.relative(main, base) : base.replace(os.homedir(), "~")}`,
);
if (hasOwnRows)
  step(
    "your [ui.sidebar.spaces] stays: add $orch1..$orch3 to its rows to see the orchestrations (packages/workflow/herdr/README.md)",
  );
if (!isRunning()) {
  await startServer();
} else if (serverVersion() && serverVersion() !== VERSION) {
  console.log(
    `! the session runs herdr ${serverVersion()}, pinned is ${VERSION}: pnpm herdr --stop (ends its panes), then pnpm herdr`,
  );
} else if (isChanged) {
  herdr(["server", "reload-config"]);
}
if (linkPlugin(bin)) step(`plugin ${PLUGIN} linked from ${pluginDir}`);
writeLinked(bin);
// The startup hook only runs at a server start; a new or moved link gets the space now.
herdr(["plugin", "action", "invoke", `${PLUGIN}.restore`]);
if (flags.has("--no-attach")) {
  console.log(
    `\n✓ ${SPACE} in the herdr session ${SESSION} is ready: pnpm herdr attaches\n`,
  );
  process.exit(0);
}
if (!isAttachingHere) {
  const [command, args] = terminalWindow([
    process.execPath,
    fileURLToPath(import.meta.url),
    "--here",
  ]);
  spawn(command, args, {
    env: cleanEnv(),
    cwd: main,
    detached: true,
    stdio: "ignore",
  }).unref();
  console.log(
    `\n✓ ${SPACE} in the herdr session ${SESSION}: opened in a window of its own (${command}, no padding)\n  here instead: pnpm herdr --here\n`,
  );
  process.exit(0);
}
console.log(
  `\n✓ ${SPACE} in the herdr session ${SESSION}, attaching (detach: prefix+d, back: pnpm herdr)\n`,
);
const attached = spawnSync(bin, ["--session", SESSION], {
  env: sessionEnv(),
  stdio: "inherit",
});
process.exit(attached.status ?? 0);
