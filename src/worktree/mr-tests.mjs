// The Tests section of a merge request (#302): only the checks that apply to
// the branch, never an empty box with "no browser change". An unticked box
// reads as open work in GitLab and in every box count.
import fs from "node:fs";
import path from "node:path";

import { git } from "../lib/proc.mjs";

/** A change here needs vita's integration tests (`pnpm test:int`). */
export const SERVER = /^products\/vita\/(?:[^/]+\/)*src\/server\//;
/** A change here is seen in the browser: vita's pages and components, its screens, monolithium. */
export const BROWSER =
  /^(?:products\/vita\/web\/src\/(?:app|components)\/|products\/vita\/ui\/src\/|packages\/monolithium\/src\/)/;

/** What a package's line names, of the scripts it has. */
export const TASKS = ["lint", "typecheck", "test"];

/**
 * The box lines of the Tests section for a branch: `packages` the packages it
 * changes (`{ name, scripts }`), `files` its changed paths (repo-relative).
 */
export function testLines({ packages, files }) {
  const lines = packages.flatMap(({ name, scripts = TASKS }) => {
    const [first, ...rest] = TASKS.filter(task => scripts.includes(task));
    if (!first) return [];
    return [
      `- [ ] ${[`\`pnpm --filter ${name} ${first}\``, ...rest.map(task => `\`${task}\``)].join(" / ")}`,
    ];
  });
  if (files.some(file => SERVER.test(file)))
    lines.push("- [ ] `pnpm test:int` (`src/server` changed)");
  if (files.some(file => BROWSER.test(file)))
    lines.push("- [ ] clicked through in the browser");
  return lines;
}

/**
 * `text` (the template) with its Tests section replaced by `lines` under a
 * comment that says what to do with them. Nothing applies: an empty bullet,
 * which wt:mr refuses until someone says why.
 */
export function withTests(text, lines) {
  const all = text.split("\n");
  const from = all.findIndex(line => /^##\s+Tests\b/.test(line));
  if (from === -1) return text;
  let to = from + 1;
  while (to < all.length && !/^(?:#{1,6}\s|Refs #|Closes #)/.test(all[to]))
    to++;
  const section = [
    all[from],
    "",
    "<!-- Pre-filled from the branch's changes: only what applies. Tick what ran,",
    "     with its result; delete a line that doesn't apply. Something that",
    "     applies but didn't run goes into a sentence below, not an empty box. -->",
    ...(lines.length > 0 ? lines : ["-"]),
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
 * The packages of `files`: `{ name, scripts }` of the nearest `package.json`
 * above each, the root's left out. `packageAt(dir)` reads the package of a
 * directory (null when there is none).
 */
export function packagesOf(files, packageAt) {
  const found = new Map();
  for (const file of files) {
    let dir = path.posix.dirname(file);
    while (dir !== "." && dir !== "/") {
      const pkg = packageAt(dir);
      if (pkg) {
        found.set(pkg.name, pkg);
        break;
      }
      dir = path.posix.dirname(dir);
    }
  }
  return [...found.values()].sort((a, b) => a.name.localeCompare(b.name));
}

/** packageAt for the checkout `top`. */
export const packageIn = top => dir => {
  try {
    const { name, scripts = {} } = JSON.parse(
      fs.readFileSync(path.join(top, dir, "package.json"), "utf-8"),
    );
    return name ? { name, scripts: Object.keys(scripts) } : null;
  } catch {
    return null;
  }
};

/** The Tests lines of the branch in `top`. */
export function testLinesOf(top) {
  const files = changedFiles(top);
  return testLines({ packages: packagesOf(files, packageIn(top)), files });
}
