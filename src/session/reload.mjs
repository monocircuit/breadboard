// pnpm herdr --reload: new workflow code into the running herdrs, the session
// `platform` and your own.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { fail, sh } from "../lib/proc.mjs";
import { SPACE } from "../worktree.mjs";
import { ownHerdr, writeOwnBlock } from "./own.mjs";
import {
  PLUGIN,
  SESSION,
  ask,
  bin,
  herdr,
  isInPlatform,
  isRunning,
  main,
  pluginDir,
  step,
} from "./session.mjs";
import {
  linkPlugin,
  linkedFileOf,
  manifestHash,
  writeConfig,
  writeLinked,
} from "./setup.mjs";

/** A herdr whose server runs and which has the space of the repo (not a personal session that never had it). */
const hasSpace = binary =>
  /status: running/.test(herdr(["status", "server"], { binary }).out) &&
  Boolean(
    ask(["workspace", "list"], { binary })?.workspaces?.some(
      w => w.label === SPACE,
    ),
  );
/**
 * The herdrs --reload acts on. Inside herdr (the key, prefix+shift+l): the
 * session it runs in, the session `platform` or your own. From a plain
 * terminal: each of them that runs with the space, so a pull reaches both.
 */
export const reloadTargets = () => {
  const own = ownHerdr();
  if (process.env.HERDR_ENV === "1")
    return isInPlatform() ? [bin] : own ? [own] : [];
  return [
    fs.existsSync(bin) && isRunning() ? bin : null,
    own && hasSpace(own) ? own : null,
  ].filter(Boolean);
};
/** The main checkout to origin/develop when it can without a question: clean, on develop, fast-forward. */
/** Whether it moved develop: then the workflow code on disk is newer than the one running. */
export const pullDevelop = () => {
  const git = args => sh("git", args, main);
  const branch = git(["branch", "--show-current"]).stdout.trim();
  if (branch !== "develop" || git(["status", "--porcelain"]).stdout.trim()) {
    step(
      `the main checkout stays as it is (${branch || "detached"}${branch === "develop" ? ", with changes" : ""})`,
    );
    return false;
  }
  git(["fetch", "--quiet", "origin", "develop"]);
  const before = git(["rev-parse", "HEAD"]).stdout.trim();
  const merged = git(["merge", "--ff-only", "--quiet", "origin/develop"]);
  const after = git(["rev-parse", "HEAD"]).stdout.trim();
  if (merged.status !== 0) {
    step("develop can't fast-forward to origin/develop, it stays as it is");
    return false;
  }
  step(
    before === after
      ? "develop is up to date"
      : `develop moved to ${after.slice(0, 8)}`,
  );
  return before !== after;
};
/**
 * New workflow code into a running herdr, without ending a conversation.
 * What runs our code for long is the overviews: they quit (`q`) and the layout
 * starts them again from the current scripts. Everything else — claude,
 * nvim, `pnpm dev`, the logs — keeps running; the plugin's commands start
 * fresh on every key anyway. The config is written again (the session
 * `platform`: .herdr/config.toml; your own herdr: its marked block, when it
 * has one), and the plugin is linked again last, only when its manifest
 * changed: it is the one step that could end a pane of the plugin, like the
 * popup this runs in.
 */
export const reload = async binary => {
  const isPlatform = binary === bin;
  const name = isPlatform ? `the session ${SESSION}` : `your herdr (${binary})`;
  step(`${name}:`);

  const isChanged = isPlatform
    ? writeConfig().isChanged
    : await writeOwnBlock(
        path.join(os.homedir(), ".config", "herdr", "config.toml"),
        { isRenewOnly: true },
      );
  if (isChanged && herdr(["server", "reload-config"], { binary }).isOk)
    step("config written again and reloaded");

  // A plugin whose directory moved (a pull took the old one) is linked from
  // where it is now first: herdr can't run restore from a directory that is gone.
  if (linkPlugin(binary)) {
    writeLinked(binary);
    step(`plugin ${PLUGIN} linked again from ${pluginDir}, it moved`);
  }

  const processInfo = pane =>
    ask(["pane", "process-info", "--pane", pane.pane_id], { binary })
      ?.process_info;
  const overviews = (ask(["pane", "list"], { binary })?.panes ?? []).filter(
    pane =>
      processInfo(pane)?.foreground_processes?.some(p =>
        p.cmdline?.includes("overview.mjs"),
      ),
  );
  for (const pane of overviews)
    herdr(["pane", "send-keys", pane.pane_id, "q"], { binary });
  // Until their shells idle: the layout only runs a command again in an idle tab.
  for (let waited = 0; waited < 5000; waited += 100) {
    const isRunningStill = overviews.some(pane => {
      const info = processInfo(pane);
      return info && info.foreground_process_group_id !== info.shell_pid;
    });
    if (!isRunningStill) break;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  step(`${overviews.length} overview${overviews.length === 1 ? "" : "s"} quit`);

  // Through the plugin, like pnpm herdr: it runs inside the session, from wherever this was typed.
  const restored = herdr(["plugin", "action", "invoke", `${PLUGIN}.restore`], {
    binary,
  });
  if (!restored.isOk) {
    const again = isPlatform ? "pnpm herdr --no-attach" : "pnpm herdr:install";
    fail(
      `the layout failed: ${restored.out}\n  The overviews quit; ${again} links the plugin and builds the layout again.`,
    );
  }
  step("layout built again, the overviews run the new code");

  const linkedFile = linkedFileOf(binary);
  const linked = fs.existsSync(linkedFile)
    ? fs.readFileSync(linkedFile, "utf-8").trim()
    : null;
  if (manifestHash() !== linked) {
    herdr(["plugin", "unlink", PLUGIN], { binary });
    linkPlugin(binary);
    writeLinked(binary);
    step(`plugin ${PLUGIN} linked again, its manifest changed`);
  }
  console.log(
    `\n✓ ${name} runs the current workflow code; conversations, nvim and servers kept running\n`,
  );
};
