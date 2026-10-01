import fs from "node:fs";
import path from "node:path";

import { isAgent } from "../../lib/herdr.mjs";
import { fail, flagValue, git, run, step, setHint } from "../../lib/proc.mjs";
import { NAME, VITA } from "../constants.mjs";
import { shareConversations } from "../conversations.mjs";
import { movesTab, openSpace, orchestrationHere } from "../layout.mjs";
import { slotOf, writeOrigin } from "../origins.mjs";
import { prunable, staleIssues } from "../prune.mjs";
import { installPushGuard } from "../push-guard.mjs";
import { openApp } from "../spaces.mjs";
import {
  claimedElsewhere,
  envFileOf,
  createStack,
  handOver,
  leavingWorktree,
  summary,
} from "../stack.mjs";
import { dirOf, mainDir } from "../worktrees.mjs";

export default async function newCommand(feature, flags) {
  const usage =
    "usage: pnpm wt:new <feature-in-kebab-case> [--shared] [--fix] [--origin <orchestration>]";
  if (!feature || !NAME.test(feature)) fail(usage);
  const branch = `${flags.has("--fix") ? "fix" : "feature"}/${feature}`;
  const dir = dirOf(feature);
  if (fs.existsSync(dir)) fail(`${dir} exists already`);
  // Started for an orchestration (wt:orchestrate): recorded right after the worktree exists.
  const originName = flagValue("--origin");
  if (flags.has("--origin") && !NAME.test(originName ?? "")) fail(usage);
  const origin = originName
    ? { name: originName, slot: slotOf(originName) }
    : null;
  // The orchestrator ran wt:new itself: its tab stays in its space, like with --origin (#245).
  const stays = origin ? null : orchestrationHere();
  // Read before openSpace moves the agent's tab out of it. An orchestrator stays where it is.
  const previous = movesTab({
    isAgent: isAgent(),
    origin,
    orchestration: stays,
  })
    ? leavingWorktree()
    : null;

  step("git fetch");
  git(["fetch", "--prune"]);
  if (git(["rev-parse", "--verify", "--quiet", branch]).ok)
    fail(`the branch ${branch} exists already`);
  if (git(["ls-remote", "--exit-code", "--heads", "origin", branch]).ok)
    fail(`origin has a branch ${branch} already`);
  const taken = claimedElsewhere(feature);
  if (taken)
    fail(
      `origin has the branch ${taken} — someone works on that issue already`,
    );

  step(`git worktree add ${dir} -b ${branch} origin/develop`);
  run("git", [
    "worktree",
    "add",
    "--quiet",
    "--no-track",
    dir,
    "-b",
    branch,
    "origin/develop",
  ]);
  // Before the first claude runs there: its conversations go to the project of the main checkout.
  shareConversations();
  if (origin) writeOrigin(dir, origin);

  setHint(
    `the worktree is only half set up: fix the cause and run the step again in ${dir}, or start over with pnpm wt:remove ${feature}`,
  );

  step(`push guard: ${installPushGuard(dir)} (#184)`);
  step(`git push -u origin ${branch} (the empty claim on the issue)`);
  if (
    !run("git", ["push", "--quiet", "-u", "origin", branch], {
      cwd: dir,
      quiet: true,
      allowFail: true,
    }).ok
  ) {
    console.log(
      `! origin refused the branch, push it yourself: git push -u origin ${branch}`,
    );
  }

  step("pnpm install");
  run("pnpm", ["install", "--frozen-lockfile"], {
    cwd: dir,
    quiet: isAgent(),
  });
  step("build the lint and format configs (lint and the hooks load them)");
  run("pnpm", ["--filter", "@monocircuit/prettier-config", "build"], {
    cwd: dir,
    quiet: true,
  });
  run("pnpm", ["--filter", "@monocircuit/eslint-config", "build:prod"], {
    cwd: dir,
    quiet: true,
  }); // like CI: build also runs tsc, which the package lacks

  // An own stack is the default: two worktrees on the shared one can't both
  // run the app, because APP_ORIGIN and the Keycloak realm of the shared
  // .env.local point at port 3000.
  let offset = 0;
  const mainEnv = envFileOf(mainDir());
  if (!fs.existsSync(mainEnv)) {
    console.log(
      `\n! ${mainEnv} is missing, the worktree has no .env.local and no stack`,
    );
  } else if (flags.has("--shared")) {
    step("share the dev stack of the main checkout (copy its .env.local)");
    fs.copyFileSync(mainEnv, path.join(dir, VITA, ".env.local"));
  } else {
    offset = await createStack(dir, feature);
  }
  // GitLab not answering costs the count, never the worktree. An orchestration's worktree skips
  // the sweep: its summary is read by wt:orchestrate, not by someone who would run it.
  const silent = origin
    ? 0
    : await staleIssues().then(
        found => found.length,
        () => 0,
      );
  const space = await openSpace(feature, dir, origin, stays);
  openApp(dir, offset);
  const left = previous && { line: handOver(previous) };
  // The worktree is ready by now: a count that fails costs the hint, not the summary (#130).
  let stale = 0;
  try {
    stale = origin ? 0 : prunable().count;
  } catch (error) {
    console.log(
      `\n! pnpm wt:prune couldn't count what is left over: ${error.message}`,
    );
  }
  summary(dir, branch, offset, space, stale, silent, left, origin);
}
