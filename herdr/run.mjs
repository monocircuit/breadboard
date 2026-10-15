#!/usr/bin/env node
// A reader of the old form (#314, rule 5): what still calls `node run.mjs …`
// after the pull that brought the binary (the Node manifest herdr keeps
// until a reload relinks the plugin, a tab bar status written before it)
// runs `sh breadboard.sh herdr …` instead, which makes the binary first when
// it is missing or older than HEAD. Goes once every checkout has reloaded
// onto the binary's herdr-plugin.toml (packages/breadboard/docs/rewrite-plan.md).
import { spawnSync } from "node:child_process";
import path from "node:path";

const launcher = path.join(import.meta.dirname, "breadboard.sh");
const args = process.argv.slice(2);
// The tab bar's status (3 s), an `open` and the restore at a start never wait
// for a build, as in herdr-plugin.toml: the pane an `open` opens builds in front.
const front =
  ["status", "open"].includes(args[0]) || args.includes("--startup")
    ? ["--no-wait"]
    : [];
const ran = spawnSync("sh", [launcher, ...front, "herdr", ...args], {
  stdio: "inherit",
});
process.exit(ran.status ?? 1);
