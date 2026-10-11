#!/usr/bin/env node
// A reader of the old form (#314, rule 5), all that is left of the Node
// breadboard: a reload of the Node plugin pulls develop, then runs the pulled
// `node src/herdr.mjs --reload --pulled`. Here that is the binary through
// its launcher (herdr/breadboard.sh makes it first, its progress in the
// reload's popup), which finishes the reload and relinks the plugin onto the
// binary's herdr-plugin.toml. Goes once every checkout has reloaded
// (packages/breadboard/docs/rewrite-plan.md).
import { spawnSync } from "node:child_process";
import path from "node:path";

const launcher = path.join(import.meta.dirname, "../herdr/breadboard.sh");
const ran = spawnSync("sh", [launcher, "herdr", ...process.argv.slice(2)], {
  stdio: "inherit",
});
process.exit(ran.status ?? 1);
