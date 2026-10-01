// The Tests section of a merge request (#302): only the checks that apply to
// the branch, never an empty box with "no browser change". An unticked box
// reads as open work in GitLab and in every box count.
import fs from "node:fs";
import path from "node:path";

import { git } from "../lib/proc.mjs";
import { readCheck, treeOf } from "./check.mjs";

/** A change here needs vita's integration tests (`pnpm test:int`). */
export const SERVER = /^products\/vita\/(?:[^/]+\/)*src\/server\//;
/** A change here is seen in the browser: vita's pages and components, its screens, monolithium. */
export const BROWSER =
  /^(?:products\/vita\/web\/src\/(?:app|components)\/|products\/vita\/ui\/src\/|packages\/monolithium\/src\/)/;

/**
 * The wt:check line (#298): ticked with its result when the last check that
 * passed tested HEAD's tree (`check`: `{ sha, packages, duration }`), else
 * the check still to run on `packages` (the names of the changed ones). It
 * stands for every package's lint, typecheck and test.
 */
export function checkLine({ check, packages }) {
  if (check) {
    const count = check.packages.length;
    const what =
      count === 0
        ? "no package changed"
        : `lint / typecheck / test of ${count} package${count === 1 ? "" : "s"} (${check.packages.join(", ")})`;
    return `- [x] \`pnpm wt:check\` passed on ${check.sha.slice(0, 8)}: ${what}${check.duration ? `, ${check.duration} s` : ""}`;
  }
  if (packages.length === 0) return null;
  return `- [ ] \`pnpm wt:check\`: lint / typecheck / test of ${packages.join(", ")} and what depends on them`;
}

/**
 * The box lines of the Tests section for a branch: `packages` the names of
 * the packages it changes, `files` its changed paths (repo-relative),
 * `check` the wt:check that passed on HEAD, if one did.
 */
export function testLines({ packages, files, check = null }) {
  const lines = [checkLine({ check, packages })].filter(Boolean);
  if (files.some(file => SERVER.test(file)))
    lines.push("- [ ] `pnpm test:int` (`src/server` changed)");
  if (files.some(file => BROWSER.test(file)))
    lines.push("- [ ] clicked through in the browser");
  return lines;
}

/** A template box the pre-fill decides on: the package checks, test:int, the browser. */
export const PREFILLED = /pnpm --filter|wt:check|test:int|browser/;

/**
 * `text` (the template) with its Tests section replaced by `lines` under a
 * comment that says what to do with them. The template's other boxes (the
 * changelog entry) apply to every MR and stay. Nothing applies: an empty bullet,
 * which wt:mr refuses until someone says why.
 */
export function withTests(text, lines) {
  const all = text.split("\n");
  const from = all.findIndex(line => /^##\s+Tests\b/.test(line));
  if (from === -1) return text;
  let to = from + 1;
  while (to < all.length && !/^(?:#{1,6}\s|Refs #|Closes #)/.test(all[to]))
    to++;
  const kept = all
    .slice(from + 1, to)
    .filter(line => /^\s*[-*+]\s+\[ \]/.test(line) && !PREFILLED.test(line));
  const boxes = [...lines, ...kept];
  const section = [
    all[from],
    "",
    "<!-- Pre-filled from the branch's changes: only what applies. Tick what ran,",
    "     with its result; delete a line that doesn't apply. Something that",
    "     applies but didn't run goes into a sentence below, not an empty box. -->",
    ...(boxes.length > 0 ? boxes : ["-"]),
    "",
  ];
  return [...all.slice(0, from), ...section, ...all.slice(to)].join("\n");
}

/**
 * The boxes left unticked under Tests whose text says they don't apply
 * (`: no browser change`, `n/a`): such a line is deleted, not kept as an
 * empty box. Returned as the lines to show, with their number.
 */
export function inapplicable(text) {
  const found = [];
  let isInTests = false;
  for (const [index, raw] of text.split("\n").entries()) {
    const heading = raw.match(/^#{1,6}\s+(.*)$/);
    if (heading) isInTests = /^tests\b/i.test(heading[1].trim());
    if (!isInTests) continue;
    const box = raw.match(/^\s*[-*+]\s+\[ \]\s+(.*)$/);
    if (!box) continue;
    const says = box[1].replace(/`[^`]*`/g, "");
    if (
      /[:—–(]\s*(?:no|not|nothing)\b.*\b(?:change[sd]?|touched|apply|applies|applicable|needed)\b/i.test(
        says,
      ) ||
      /\b(?:n\/a|doesn't apply|does not apply|not applicable)\b/i.test(says)
    )
      found.push(`  ${String(index + 1).padStart(4)}  ${raw.trim()}`);
  }
  return found;
}

/**
 * The files the branch in `top` changes against develop, committed or not
 * (untracked ones included): repo-relative paths.
 */
export function changedFiles(top) {
  const base = git(["merge-base", "origin/develop", "HEAD"], top).out;
  const changed = base
    ? git(["diff", "--name-only", base], top).out.split("\n")
    : [];
  const untracked = git(
    ["ls-files", "--others", "--exclude-standard"],
    top,
  ).out.split("\n");
  return [...new Set([...changed, ...untracked].filter(Boolean))].sort();
}

/**
 * The packages of `files`: the name in the nearest `package.json` above
 * each, the root's left out. `nameAt(dir)` reads the name of a package
 * directory (null when there is none).
 */
export function packagesOf(files, nameAt) {
  const names = new Set();
  for (const file of files) {
    let dir = path.posix.dirname(file);
    while (dir !== "." && dir !== "/") {
      const name = nameAt(dir);
      if (name) {
        names.add(name);
        break;
      }
      dir = path.posix.dirname(dir);
    }
  }
  return [...names].sort();
}

/** nameAt for the checkout `top`. */
export const packageNameIn = top => dir => {
  try {
    return (
      JSON.parse(fs.readFileSync(path.join(top, dir, "package.json"), "utf-8"))
        .name ?? null
    );
  } catch {
    return null;
  }
};

/** The wt:check of `top` that passed on its HEAD, or null. */
export function passedCheckOf(top) {
  const passed = readCheck(top).passed;
  return passed && passed.tree === treeOf(top) ? passed : null;
}

/** The Tests lines of the branch in `top`. */
export function testLinesOf(top) {
  const files = changedFiles(top);
  return testLines({
    packages: packagesOf(files, packageNameIn(top)),
    files,
    check: passedCheckOf(top),
  });
}
