import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { describe, it } from "node:test";

import { ENTRY } from "./worktree.mjs";

/** Every .mjs below `dir`, tests aside. */
const sources = dir =>
  fs.readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return sources(file);
    return entry.name.endsWith(".mjs") && !entry.name.endsWith(".test.mjs")
      ? [file]
      : [];
  });
/**
 * `path.join(import.meta.dirname, "…", …)` and `path.resolve(…)` of a file:
 * the paths it builds from its own place, when all the parts are literals.
 */
const CALL = /path\.(?:join|resolve)\(\s*import\.meta\.dirname\s*,([^)]*)\)/g;
const pathsIn = file =>
  [...fs.readFileSync(file, "utf-8").matchAll(CALL)].flatMap(([, args]) => {
    const parts = args
      .split(",")
      .map(part => part.trim())
      .filter(Boolean);
    if (!parts.every(part => /^"[^"]*"$/.test(part))) return [];
    return [path.join(path.dirname(file), ...parts.map(p => p.slice(1, -1)))];
  });

describe("paths from import.meta.dirname", () => {
  const files = sources(import.meta.dirname);

  it("finds the paths it checks", () => {
    assert.ok(files.flatMap(pathsIn).length > 0);
  });

  // A moved file breaks this test, not a button of the overview.
  for (const file of files)
    for (const target of pathsIn(file))
      it(`${path.relative(import.meta.dirname, file)} → ${path.relative(import.meta.dirname, target)} exists`, () => {
        assert.ok(fs.existsSync(target), `${target} doesn't exist`);
      });

  it("ENTRY, the script the overview spawns for stack:up and wt:remove, exists", () => {
    assert.ok(fs.existsSync(ENTRY));
  });
});
