// Issue checkboxes and the merge request description: parse, count, check what
// is still template.
import fs from "node:fs";
import path from "node:path";

import { run } from "../lib/proc.mjs";
import { testLinesOf, withTests } from "./mr-tests.mjs";

/**
 * The places of a merge request description nobody wrote: a bullet that stayed
 * empty and a comment of the template that stayed in. Checkboxes (`- [ ]`) are
 * the test list and belong there, and code — fenced or inline — is text the
 * author wrote, even when it shows a comment.
 */
export function unfilled(text) {
  const left = [];
  let isInComment = false;
  let isInCode = false;
  for (const [index, raw] of text.split("\n").entries()) {
    const at = `  ${String(index + 1).padStart(4)}  `;
    if (/^\s*```/.test(raw)) {
      isInCode = !isInCode;
      continue;
    }
    if (isInCode) continue;
    const line = raw.replace(/`[^`]*`/g, "");
    if (isInComment) {
      if (line.includes("-->")) isInComment = false;
      continue;
    }
    if (line.includes("<!--")) {
      left.push(`${at}${raw.trim()}`);
      if (!line.includes("-->")) isInComment = true;
    } else if (/^\s*[-*]\s*$/.test(line)) {
      left.push(`${at}${raw.trim()} (empty bullet)`);
    }
  }
  return left;
}
/**
 * The checkboxes of an issue description, numbered in order: its line, whether
 * it is ticked, its text and the heading it stands under. A box in fenced code
 * is an example, not a task, and gets no number.
 */
export function boxesOf(text) {
  const boxes = [];
  let heading = null;
  let isInCode = false;
  for (const [line, raw] of text.split("\n").entries()) {
    if (/^\s*```/.test(raw)) {
      isInCode = !isInCode;
      continue;
    }
    if (isInCode) continue;
    const title = raw.match(/^#{1,6}\s+(.*)$/);
    if (title) heading = title[1].trim();
    const box = raw.match(/^\s*[-*+]\s+\[([ xX])\]\s+(.*)$/);
    if (box)
      boxes.push({
        line,
        isTicked: box[1] !== " ",
        text: box[2].trim(),
        heading,
      });
  }
  return boxes;
}
/** `3/9` */
export const tally = boxes =>
  `${boxes.filter(box => box.isTicked).length}/${boxes.length}`;
/** The boxes numbered under their headings, the way `wt:tick` shows them. `marked` are the ones just ticked. */
export function printBoxes(boxes, marked = new Set()) {
  let heading;
  for (const [index, box] of boxes.entries()) {
    if (box.heading !== heading) {
      heading = box.heading;
      console.log(`\n  ${heading ?? "(no heading)"}`);
    }
    const text = box.text.length > 100 ? `${box.text.slice(0, 99)}…` : box.text;
    console.log(
      `  ${String(index + 1).padStart(3)} ${box.isTicked ? "[x]" : "[ ]"} ${text}${marked.has(box) ? "   ← ticked now" : ""}`,
    );
  }
}
/**
 * The line `wt:mr` ends with: how far the issue's boxes are ticked, and the
 * command that ticks them. Without `text` the issue is read first.
 */
export function boxReport(top, number, text) {
  if (text === undefined) {
    const found = run("glab", ["issue", "view", number, "-F", "json"], {
      cwd: top,
      quiet: true,
      allowFail: true,
    });
    if (!found.ok) return "";
    text = JSON.parse(found.out).description ?? "";
  }
  const boxes = boxesOf(text);
  if (boxes.length === 0) return "";
  const left = boxes.filter(box => !box.isTicked).length;
  return left
    ? `\n\n  #${number}: ${tally(boxes)} boxes ticked. Done one? pnpm wt:tick ${number} lists them, pnpm wt:tick ${number} <box>… ticks`
    : `\n\n  #${number}: all ${boxes.length} boxes ticked`;
}
/** The issue a branch claims: `feature/34-…` → `34`. */
export const issueOfBranch = branch => branch?.match(/\/(\d+)-/)?.[1];
/**
 * Where the description of this worktree lives: in the worktree, under the
 * ignored `.claude/`, not in a temp directory — a temp directory is cleared
 * under the session and takes the written text with it.
 */
export function descriptionFile(top) {
  return path.join(top, ".claude", "mr-description.md");
}
/** A path as it is worth reading: relative inside the worktree, whole outside. */
export function shown(top, file) {
  const inside = path.relative(top, file);
  return inside.startsWith("..") ? file : inside;
}
/**
 * No description written yet: the template goes into the worktree, so the
 * answer to the failure is "fill this in", not "find the template". Its Tests
 * section is filled from the branch: only the checks that apply (#302).
 */
export function templateToFill(top, number) {
  const file = descriptionFile(top);
  if (!fs.existsSync(file)) {
    const template = fs.readFileSync(
      path.join(top, ".gitlab", "merge_request_templates", "Default.md"),
      "utf-8",
    );
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(
      file,
      `${withTests(template, testLinesOf(top))
        .trimEnd()
        .replace(/Refs #$/, "")}Closes #${number}\n`,
    );
  }
  return `a merge request needs a description written for this branch — the template is waiting in\n\n  ${shown(top, file)}\n\n  Fill every section (no empty bullets, no <!-- comments -->), then run pnpm wt:mr again.`;
}
