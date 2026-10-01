// The changelogs in the terminal: an orchestration's (#248, `pnpm
// wt:orchestrate <name> --changelog`) and the repo's (#261, `pnpm
// changelog`) are rewritten and printed once; `--watch` shows one live in
// the tab `changelog` of its space, rendered and scrollable like the
// overview (q quits, wheel j k ↑↓ PgUp PgDn space g G), its `##` sections
// foldable (a click on the heading, z the one on top, Z all) and jumpable
// (] [ the next and the one before, 1–9 the nth), drawn again whenever the
// file changes. The files themselves: changelog.mjs, overall-changelog.mjs.
import fs from "node:fs";
import path from "node:path";

import { fail, step } from "../lib/proc.mjs";
import { bold, dim, hex, isColor, link, wrap } from "../lib/tty.mjs";
import { changelogFile, writeChangelog } from "./changelog.mjs";
import {
  overallFile,
  sinceOf,
  stateOf,
  writeOverall,
} from "./overall-changelog.mjs";
import { orchestratorLabel } from "./origins.mjs";
import { colourIn, herdrConfigText } from "./sidebar.mjs";
import { mainDir } from "./worktrees.mjs";

/** The theme's colours of herdr's config (`[theme.custom]`), with fallbacks. */
const THEME = {
  title: { colour: "pink", fallback: "#FF79C6" },
  day: { colour: "blue", fallback: "#89B4FA" },
  group: { colour: "mauve", fallback: "#CBA6F7" },
  link: { colour: "teal", fallback: "#94E2D5" },
};
/** The painters of the theme in the config text `base`. */
export const paintersOf = base =>
  Object.fromEntries(
    Object.entries(THEME).map(([key, entry]) => [
      key,
      hex(colourIn(base, entry)),
    ]),
  );

/** `[text](url)` as a link in the link colour, `` `code` `` dimmed; the rest as it is. */
const inline = (text, paint) =>
  text
    .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_, label, url) =>
      link(url, paint.link(label)),
    )
    .replace(/`([^`]+)`/g, (_, code) => dim(code))
    .replace(/ · (\d\d:\d\d)$/, (_, time) => ` ${dim(`· ${time}`)}`);
/**
 * The Markdown of a changelog as painted lines of at most `width` columns:
 * headings in the theme's colours (`####` dimmed), `- ` items with a hanging indent and the
 * lines under them indented with them, links clickable (OSC 8).
 */
export const renderMarkdown = (text, width, paint = paintersOf("")) => {
  const lines = [];
  const wrapped = (line, indent = "") =>
    wrap(line, Math.max(10, width - indent.length), Infinity).map(
      (part, i) => (i === 0 ? "" : indent) + part,
    );
  for (const raw of text.replace(/\r/g, "").split("\n")) {
    const heading = raw.match(/^(#{1,4})\s+(.*)$/);
    if (heading) {
      const [, level, title] = heading;
      const painted =
        level === "#"
          ? bold(paint.title(title))
          : level === "##"
            ? bold(paint.day(title))
            : level === "###"
              ? paint.group(title)
              : dim(paint.group(title));
      lines.push(...wrapped(painted));
      continue;
    }
    const item = raw.match(/^- (.*)$/);
    if (item) {
      lines.push(...wrapped(`• ${inline(item[1], paint)}`, "  "));
      continue;
    }
    const under = raw.match(/^ {2}(.*)$/);
    if (under) {
      lines.push(...wrapped(`  ${inline(under[1], paint)}`, "  "));
      continue;
    }
    lines.push(...(raw ? wrapped(inline(raw, paint)) : [""]));
  }
  while (lines.at(-1) === "") lines.pop();
  return lines;
};

/** What the tab shows before the first merge wrote the file. */
const EMPTY = name =>
  `# ${orchestratorLabel(name)}: changelog\n\nNothing merged yet: the first merge writes ${path.relative(mainDir(), changelogFile(name))}.\n`;
const readOr = (file, empty) => {
  try {
    return fs.readFileSync(file, "utf-8");
  } catch {
    return empty;
  }
};
const read = name => readOr(changelogFile(name), EMPTY(name));
/** What the tab of the main space shows before the overview wrote the file. */
const EMPTY_OVERALL = () =>
  `# Monocircuit/Platform: changelog\n\nNot written yet: the overview of the main space writes ${path.relative(mainDir(), overallFile())}, or \`pnpm changelog\`.\n`;

/** The keys that scroll, given the rows of a page. */
const SCROLL = {
  j: () => 1,
  k: () => -1,
  "\x1b[B": () => 1,
  "\x1b[A": () => -1,
  "\x1b[6~": page => page,
  "\x1b[5~": page => -page,
  " ": page => page,
  g: () => -Infinity,
  G: () => Infinity,
};
const KEY = /\x1b\[<\d+;\d+;\d+[Mm]|\x1b\[[\d;]*[~A-Za-z]|[\s\S]/g;
const MOUSE = /^\x1b\[<(\d+);\d+;(\d+)M$/;
/** Lines per notch of the wheel. */
const WHEEL_LINES = 3;
/** How often the file is looked at: a rename (writeChangelog) is seen too. */
const WATCH_MS = 1000;
/** The mark of a `##` section in the view: open, folded. */
const OPEN = "▾";
const FOLDED = "▸";

/**
 * The lines of the view: the rendered text before the first `##` (its title
 * first), then each `##` section, its heading marked open or folded and only
 * that line when it is in `folded` (by heading). `{ line, section }` each,
 * `section` the heading of its section, null before the first; `heading`
 * true on the heading's own line.
 */
export const viewLines = (text, width, folded = new Set(), paint) => {
  const parts = text.replace(/\r/g, "").split(/^(?=## )/m);
  const head = parts[0].startsWith("## ") ? "" : parts.shift();
  const lines = renderMarkdown(head, width, paint).map(line => ({
    line,
    section: null,
    isHeading: false,
  }));
  for (const part of parts) {
    const [first, ...body] = part.split("\n");
    const section = first.slice(3).trim();
    const isFolded = folded.has(section);
    const rendered = renderMarkdown(
      [
        `## ${isFolded ? FOLDED : OPEN} ${section}`,
        ...(isFolded ? [] : body),
      ].join("\n"),
      width,
      paint,
    );
    if (lines.length && lines.at(-1).line !== "")
      lines.push({ line: "", section: null, isHeading: false });
    lines.push(
      ...rendered.map((line, i) => ({ line, section, isHeading: i === 0 })),
    );
  }
  return lines;
};

/**
 * The live view of `file`: the rendered changelog on the whole pane, its first
 * line (the title) standing while the rest scrolls, a mark by it of what is
 * cut off and of the keys. `empty`: what it shows while there is no file.
 */
const watch = ({ file, empty }) => {
  const paint = paintersOf(herdrConfigText());
  let offset = 0;
  const folded = new Set();
  let shown = [];
  const rows = () => process.stdout.rows || 40;
  const columns = () => process.stdout.columns || 100;
  const lines = () => viewLines(readOr(file, empty), columns(), folded, paint);
  const draw = () => {
    const [title = { line: "" }, ...rest] = lines();
    const room = Math.max(0, rows() - 1);
    const max = Math.max(0, rest.length - room);
    offset = Math.min(Math.max(0, offset), max);
    const sections = rest.filter(line => line.isHeading).length;
    const mark = [
      offset ? `↑ ${offset}` : "",
      max - offset ? `↓ ${max - offset}` : "",
      sections > 1 ? "] [ 1–9 jump · z Z fold" : "",
    ]
      .filter(Boolean)
      .join("  ");
    shown = rest.slice(offset, offset + room);
    process.stdout.write(
      `\x1b[H\x1b[2J${[mark ? `${title.line}  ${dim(mark)}` : title.line, ...shown.map(({ line }) => line)].map(line => `${line}\x1b[K`).join("\r\n")}`,
    );
  };
  const scroll = by => {
    offset = Math.max(0, offset + by);
    if (!Number.isFinite(offset)) offset = Number.MAX_SAFE_INTEGER;
    draw();
  };
  /** The section headings with their line in the scrolled part, in order. */
  const headings = () =>
    lines()
      .slice(1)
      .map((line, at) => ({ ...line, at }))
      .filter(line => line.isHeading);
  const jumpTo = heading => {
    if (!heading) return;
    offset = heading.at;
    draw();
  };
  /** The section the line on top of the view is in, else the first. */
  const onTop = () =>
    shown.find(line => line.section)?.section ?? headings()[0]?.section;
  const toggle = section => {
    if (!section) return;
    if (folded.has(section)) folded.delete(section);
    else folded.add(section);
    // The heading stays where it was looked at.
    jumpTo(headings().find(heading => heading.section === section));
  };
  const leave = () => {
    process.stdout.write("\x1b[?1000l\x1b[?1006l\x1b[?7h\x1b[?25h\x1b[?1049l");
    process.exit(0);
  };
  process.stdout.write("\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[?1000h\x1b[?1006h");
  process.on("SIGINT", leave);
  process.on("SIGTERM", leave);
  process.stdout.on("resize", draw);
  process.stdin.setRawMode?.(true);
  process.stdin.resume();
  process.stdin.on("data", chunk => {
    for (const [key] of chunk.toString().matchAll(KEY)) {
      if (key === "q" || key === "\u0003") leave();
      if (SCROLL[key]) scroll(SCROLL[key](Math.max(1, rows() - 2)));
      if (key === "]") jumpTo(headings().find(heading => heading.at > offset));
      if (key === "[")
        jumpTo(headings().findLast(heading => heading.at < offset));
      if (/^[1-9]$/.test(key)) jumpTo(headings()[Number(key) - 1]);
      if (key === "z" || key === "\r") toggle(onTop());
      if (key === "Z") {
        const all = headings().map(heading => heading.section);
        const isAllFolded = all.every(section => folded.has(section));
        for (const section of all)
          if (isAllFolded) folded.delete(section);
          else folded.add(section);
        offset = 0;
        draw();
      }
      const [, button, row] = key.match(MOUSE)?.map(Number) ?? [];
      // A click on a heading folds it; row 1 is the title.
      if (button === 0 && shown[row - 2]?.isHeading)
        toggle(shown[row - 2].section);
      // The wheel: 64 up, 65 down, with a modifier (Shift, Alt, Ctrl) too.
      const wheel = { 64: -WHEEL_LINES, 65: WHEEL_LINES }[button & ~28];
      if (wheel) scroll(wheel);
    }
  });
  fs.watchFile(file, { interval: WATCH_MS }, draw);
  draw();
};

/** `text` on stdout: rendered on a terminal, else as Markdown. */
const print = text =>
  console.log(
    isColor
      ? renderMarkdown(
          text,
          process.stdout.columns || 100,
          paintersOf(herdrConfigText()),
        ).join("\n")
      : text,
  );

/**
 * `pnpm wt:orchestrate <name> --changelog`: rewritten from the record and
 * GitLab, then printed (rendered on a terminal, else as Markdown). Without a
 * record (the orchestration ended) the file as it is. `isLive`: the view of
 * the tab instead.
 */
export const changelogCommand = async (name, { isLive = false } = {}) => {
  if (isLive) return watch({ file: changelogFile(name), empty: EMPTY(name) });
  const file = path.relative(mainDir(), changelogFile(name));
  const written = await writeChangelog(name);
  if (written) step(`wrote ${file} from the record and GitLab`);
  else if (fs.existsSync(changelogFile(name)))
    step(`${name} has no record any more: ${file} as it is`);
  else fail(`${name} has no record and no changelog (${file})`);
  print(written ?? read(name));
};
/**
 * `pnpm changelog [--since 30d|2w|<date>]`: the changelog of the repo
 * rewritten from GitLab, in that window (else the one chosen last), then
 * printed. `isLive`: the view of the main space's tab instead.
 */
export const overallChangelogCommand = async ({
  since = null,
  isLive = false,
} = {}) => {
  if (isLive) return watch({ file: overallFile(), empty: EMPTY_OVERALL() });
  if (since && !sinceOf(since))
    fail(
      `--since takes days (30d), weeks (2w) or a date (2026-09-01), not ${since}`,
    );
  const text = await writeOverall({ since });
  step(
    `wrote ${path.relative(mainDir(), overallFile())} from GitLab, since ${stateOf().since}`,
  );
  print(text);
};
