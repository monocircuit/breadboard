#!/usr/bin/env node
// At the cut-over (#314) this file replaces src/herdr.mjs when src/ goes:
// a reload of the Node plugin pulls develop, then runs the pulled
// `node src/herdr.mjs --reload --pulled`. Here that is the binary through
// its launcher (herdr/breadboard.sh builds it first, its progress in the
// reload's popup), which finishes the reload and relinks the plugin onto
// herdr-plugin.toml of the binary. Goes once every checkout has reloaded.
import { spawnSync } from "node:child_process";
import path from "node:path";

const launcher = path.join(import.meta.dirname, "../herdr/breadboard.sh");
const ran = spawnSync("sh", [launcher, "herdr", ...process.argv.slice(2)], {
  stdio: "inherit",
});
process.exit(ran.status ?? 1);
