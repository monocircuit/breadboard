// Files in the view of a worktree that a click opens in its nvim tab (#257):
// the files of DIFF, the conflicts of DEVELOP, a `file:line` in the log of a
// failed job. At the change: the first line the worktree's work changes in
// the file, or the line the log names.
import { dim, visible } from "../lib/tty.mjs";
import { lines as linesOf, sh } from "../lib/proc.mjs";
import { openInEditor } from "../worktree.mjs";
import { CELL_START } from "./boxes.mjs";
import { screen } from "./screen.mjs";

/** A column on screen (from 1) → the column of a pushed line (from 0, its two-blank indent included). */
export const lineColumn = column => column - CELL_START + 2;
/**
 * `prefix` and a few files by name, the rest as a count (like someFiles),
 * with where each name stands: `{ text, spans: [{ from, to, file }] }`,
 * columns of the line from 0, `to` past the end. `+N more` stands for the
 * first one it hides.
 */
export const fileLine = (prefix, files, { count = 3, suffix = "" } = {}) => {
  let text = prefix;
  const spans = [];
  const add = (shown, file) => {
    spans.push({
      from: visible(text),
      to: visible(text) + visible(shown),
      file,
    });
    text += shown;
  };
  files.slice(0, count).forEach((file, i) => {
    if (i > 0) text += ", ";
    add(file, file);
  });
  if (files.length > count) {
    text += " ";
    add(dim(`+${files.length - count} more`), files[count]);
  }
  return { text: text + suffix, spans };
};
/** The file of `spans` under the column of the line, the nearest one before it, else the first. */
export const fileAt = (spans, at) =>
  spans.findLast(span => span.from <= at)?.file ?? spans[0]?.file ?? null;

/** `@@ -a,b +c,d @@`: the first line of the new side. */
const HUNK = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@/m;
/**
 * The first line of a hunk in `git diff -U0`: where the change is. A hunk that
 * only removes lines names the line before the gap; 1 without a hunk.
 */
export const firstChangeIn = diff => {
  const hunk = diff.match(HUNK);
  if (!hunk) return 1;
  const line = Number(hunk[1]);
  return hunk[2] === "0" ? Math.max(1, line) : line;
};
/** The first line of `file` the work of `dir` changes against develop (diff.mjs), 1 for a new file. */
export const firstChange = (dir, file) => {
  const git = args => sh("git", args, dir).stdout ?? "";
  const base = git(["merge-base", "origin/develop", "HEAD"]).trim();
  return base ? firstChangeIn(git(["diff", "-U0", base, "--", file])) : 1;
};

/** A path in a log line with an extension, then `:line[:col]` or `(line,col)`. */
const LOG_PATH =
  /(?:^|[\s'"(])((?:\.{0,2}\/)?[\w@.+-]+(?:\/[\w@.+-]+)*\.[A-Za-z]{1,5})(?::(\d+)(?::\d+)?|\((\d+),\d+\))?/g;
/** ESLint's lines under the file: `  12:5  error  …`. */
const ESLINT_PROBLEM = /^\s+(\d+):\d+\s+(?:error|warning)\b/;
/**
 * What a click on each line of a failed job's log opens: `{ file, line }` or
 * null. A path with a line, or one with a folder; a line of ESLint's problems
 * takes the file of the line above it that names one alone.
 */
export const logTargets = log => {
  let eslintFile = null;
  return log.map(text => {
    const problem = text.match(ESLINT_PROBLEM);
    if (problem && eslintFile)
      return { file: eslintFile, line: Number(problem[1]) };
    for (const [, file, colon, paren] of text.matchAll(LOG_PATH)) {
      const line = Number(colon ?? paren) || null;
      if (!line && !file.includes("/")) continue;
      eslintFile = !line && text.trim() === file ? file : null;
      return { file, line };
    }
    eslintFile = null;
    return null;
  });
};
/**
 * The file of the checkout a log's path means, among `files` (git ls-files):
 * itself, one ending in it (a path within a package: `src/a.ts`), or the end
 * of it (an absolute path on the runner); the shortest of them.
 */
export const matchFile = (files, logged) => {
  const wanted = logged.replace(/^\.?\//, "");
  if (files.includes(wanted)) return wanted;
  const found = files.filter(
    file => file.endsWith(`/${wanted}`) || wanted.endsWith(`/${file}`),
  );
  return found.sort((a, b) => a.length - b.length)[0] ?? null;
};

/** Why the last click on a file didn't open it, for the view; null once it did. */
export let editorTrouble = null;
/**
 * `file` (relative to `dir`) in the nvim tab of `dir`, at `line` or else at
 * its first change. A path out of a log is matched to the checkout first.
 */
export const openFile = (dir, file, line = null, { isLogged = false } = {}) => {
  const found = isLogged
    ? matchFile(linesOf(sh("git", ["ls-files"], dir).stdout), file)
    : file;
  if (!found) {
    editorTrouble = `${file} is no file of this checkout`;
    return;
  }
  const { why } = openInEditor(dir, found, line ?? firstChange(dir, found));
  editorTrouble = why;
};
/** A `fileLine` onto `lines`: a click opens the file under it in the nvim of `dir`. */
export const pushFiles = (lines, { text, spans }, dir) => {
  screen.clicks[lines.length] = column =>
    openFile(dir, fileAt(spans, lineColumn(column)));
  lines.push(text);
};
