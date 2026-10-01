// The changelog of an orchestration in the terminal (#248): `pnpm
// wt:orchestrate <name> --changelog` rewrites it and prints it once,
// `--watch` shows it live in the tab `changelog` of its space, rendered and
// scrollable like the overview (q quits, wheel j k ↑↓ PgUp PgDn space g G),
// drawn again whenever the file changes. The file itself: changelog.mjs.
import fs from "node:fs";
import path from "node:path";

import { fail, step } from "../lib/proc.mjs";
import { bold, dim, hex, isColor, link, wrap } from "../lib/tty.mjs";
import { changelogFile, writeChangelog } from "./changelog.mjs";
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
 * headings in the theme's colours, `- ` items with a hanging indent and the
 * lines under them indented with them, links clickable (OSC 8).
 */
export const renderMarkdown = (text, width, paint = paintersOf("")) => {
  const lines = [];
  const wrapped = (line, indent = "") =>
    wrap(line, Math.max(10, width - indent.length), Infinity).map(
      (part, i) => (i === 0 ? "" : indent) + part,
    );
  for (const raw of text.replace(/\r/g, "").split("\n")) {
    const heading = raw.match(/^(#{1,3})\s+(.*)$/);
    if (heading) {
      const [, level, title] = heading;
      const painted =
        level === "#"
          ? bold(paint.title(title))
          : level === "##"
            ? bold(paint.day(title))
            : paint.group(title);
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
const read = name => {
  try {
    return fs.readFileSync(changelogFile(name), "utf-8");
  } catch {
    return EMPTY(name);
  }
};

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
const WHEEL = /^\x1b\[<(\d+);\d+;\d+M$/;
/** Lines per notch of the wheel. */
const WHEEL_LINES = 3;
/** How often the file is looked at: a rename (writeChangelog) is seen too. */
const WATCH_MS = 1000;

/**
 * The live view: the rendered changelog on the whole pane, its first line
 * (the title) standing while the rest scrolls, a mark by it of what is cut off.
 */
const watch = name => {
  const paint = paintersOf(herdrConfigText());
  let offset = 0;
  const rows = () => process.stdout.rows || 40;
  const columns = () => process.stdout.columns || 100;
  const draw = () => {
    const [title = "", ...rest] = renderMarkdown(read(name), columns(), paint);
    const room = Math.max(0, rows() - 1);
    const max = Math.max(0, rest.length - room);
    offset = Math.min(Math.max(0, offset), max);
    const mark = [
      offset ? `↑ ${offset}` : "",
      max - offset ? `↓ ${max - offset}` : "",
    ]
      .filter(Boolean)
      .join(" ");
    const shown = [
      mark ? `${title}  ${dim(mark)}` : title,
      ...rest.slice(offset, offset + room),
    ];
    process.stdout.write(
      `\x1b[H\x1b[2J${shown.map(line => `${line}\x1b[K`).join("\r\n")}`,
    );
  };
  const scroll = by => {
    offset = Math.max(0, offset + by);
    if (!Number.isFinite(offset)) offset = Number.MAX_SAFE_INTEGER;
    draw();
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
      const button = Number(key.match(WHEEL)?.[1]);
      // The wheel: 64 up, 65 down, with a modifier (Shift, Alt, Ctrl) too.
      const wheel = { 64: -WHEEL_LINES, 65: WHEEL_LINES }[button & ~28];
      if (wheel) scroll(wheel);
    }
  });
  fs.watchFile(changelogFile(name), { interval: WATCH_MS }, draw);
  draw();
};

/**
 * `pnpm wt:orchestrate <name> --changelog`: rewritten from the record and
 * GitLab, then printed (rendered on a terminal, else as Markdown). Without a
 * record (the orchestration ended) the file as it is. `isLive`: the view of
 * the tab instead.
 */
export const changelogCommand = async (name, { isLive = false } = {}) => {
  if (isLive) return watch(name);
  const file = path.relative(mainDir(), changelogFile(name));
  const written = await writeChangelog(name);
  if (written) step(`wrote ${file} from the record and GitLab`);
  else if (fs.existsSync(changelogFile(name)))
    step(`${name} has no record any more: ${file} as it is`);
  else fail(`${name} has no record and no changelog (${file})`);
  const text = written ?? read(name);
  console.log(
    isColor
      ? renderMarkdown(
          text,
          process.stdout.columns || 100,
          paintersOf(herdrConfigText()),
        ).join("\n")
      : text,
  );
};
