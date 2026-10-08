#!/usr/bin/env node
// At the cut-over (#314) this file replaces herdr/run.mjs: what still calls
// `node run.mjs …` after the pull that brings the binary (the Node manifest
// herdr keeps until the reload relinks the plugin, a tab bar status written
// before it) runs `sh breadboard.sh herdr …` instead, which builds the
// binary first when it is missing or older than HEAD. Goes once every
// checkout has reloaded onto herdr-plugin.toml of the binary.
import { spawnSync } from "node:child_process";
import path from "node:path";

const launcher = path.join(import.meta.dirname, "breadboard.sh");
const args = process.argv.slice(2);
// The tab bar's status (3 s) and an `open` never wait for a build, as in
// herdr-plugin.rs.toml: the pane an `open` opens builds in front.
const front = ["status", "open"].includes(args[0]) ? ["--no-wait"] : [];
const ran = spawnSync("sh", [launcher, ...front, "herdr", ...args], {
  stdio: "inherit",
});
process.exit(ran.status ?? 1);
