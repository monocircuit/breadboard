// breadboard runs in any repo (#342): what it knows of the repo it is
// developed in comes from that repo's breadboard.toml, never from here. No
// file of the package names that repo's products, packages or project.
//
// Not checked: the package's own name (`@monocircuit/breadboard`) and public repository, the licence's copyright holder, the history of the rewrite (docs/rewrite-*, the evidence),
// the spikes, the monorepo's wiring of the package (package.json, the lint
// config), the herdr plugin's manifest and keys (the live plugin of the repo
// breadboard is developed in, herdr loads them as they are; breadboard init
// writes another repo's with its own id), and the tests of the Rust code
// (their worlds are recorded from that repo).
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";

const HERE = path.join(path.dirname(import.meta.filename), "..");
const NAMES = /products\/vita|monocircuit|monolithium|keycloak/i;
/** The package's own name in the workspace it is developed in, and its public repository. */
const OWN =
  /@monocircuit\/breadboard(-gauntlet)?\b|github\.com\/monocircuit\/breadboard\b/g;
const KEPT = [
  /^docs\/rewrite-/,
  /^docs\/stack-evidence\.md$/,
  /^spikes\//,
  /^package\.json$/,
  /^eslint\.config\.mjs$/,
  /^herdr\/herdr-plugin\.toml$/,
  /^herdr\/keys\.toml$/,
  /^src\/generic\.test\.mjs$/,
  // The copyright holder (#343).
  /^LICENSE-MIT$/,
];

/** The lines of a file the check reads: a Rust file's up to its tests. */
const linesOf = (file, text) => {
  const lines = text.split("\n");
  if (!file.endsWith(".rs")) return lines;
  const tests = lines.findIndex(line => line.trim() === "#[cfg(test)]");
  return tests === -1 ? lines : lines.slice(0, tests);
};

test("no file of breadboard names the repo it is developed in", () => {
  const files = execFileSync(
    "git",
    ["ls-files", "--cached", "--others", "--exclude-standard", "."],
    {
      cwd: HERE,
      encoding: "utf8",
    },
  )
    .split("\n")
    .filter(file => file && !KEPT.some(kept => kept.test(file)))
    .filter(file => fs.existsSync(path.join(HERE, file)));
  const found = files.flatMap(file =>
    linesOf(file, fs.readFileSync(path.join(HERE, file), "utf8"))
      .map((line, at) => ({ at: at + 1, line }))
      .filter(({ line }) => NAMES.test(line.replaceAll(OWN, "")))
      .map(({ at, line }) => `${file}:${at}: ${line.trim()}`),
  );
  assert.deepEqual(found, [], "these belong into the repo's breadboard.toml");
});
