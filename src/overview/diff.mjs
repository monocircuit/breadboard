// How far a worktree is from develop, and where its work collides (#114): what
// the section DIFF of `pnpm overview --worktree <dir>` shows.
//
// The work of a worktree is everything between the merge base with
// origin/develop and its working tree: commits and uncommitted changes. It
// collides with develop where develop changed the same file since the merge
// base, and with another worktree where that one's work touches the file too.
//
// The overview reads it every 2 s, so each worktree's result is cached under
// what it depends on (HEAD, origin/develop, the status) and computed again
// only when that changes.
import { lines, sh } from "../lib/proc.mjs";

const git = (args, dir) => sh("git", args, dir).stdout ?? "";

/**
 * The area a file belongs to. Inside a package the folder below `src`, so most
 * of vita isn't one row: `vita/server`, `vita/app`, `utilities/devtools`; a
 * file at a package's top is the package (`vita`, `vita-core`). Outside the packages the
 * top-level folder or file (`scripts`, `AGENTS.md`).
 */
export const areaOf = file => {
  const parts = file.split("/");
  if (
    !["products", "packages", "tools"].includes(parts[0]) ||
    parts.length < 3
  ) {
    return parts.length > 1 ? parts[0] : file;
  }
  // A product holds packages (`products/vita/web`, `…/core`): the web app
  // keeps the product's name, the others are `vita-core`, ….
  const [name, inside] =
    parts[0] === "products" && parts.length > 3
      ? [
          parts[2] === "web" ? parts[1] : `${parts[1]}-${parts[2]}`,
          parts.slice(3),
        ]
      : [parts[1], parts.slice(2)];
  const below = inside[0] === "src" ? inside.slice(1) : inside;
  return below.length > 1 ? `${name}/${below[0]}` : name;
};

/** A path of `status --porcelain`: the new name of a rename, without quotes. */
const pathOfStatus = line =>
  line.slice(3).split(" -> ").at(-1).replace(/^"|"$/g, "");

/** `top`: the file of an area with the most lines changed, the one a click on the area opens. */
const topOf = (area, file, changed) => {
  if (area.top && changed <= area.topChanged) return;
  area.top = file;
  area.topChanged = changed;
};
/** What the result of `dir` depends on; the same key means the same result. */
const keyOf = dir =>
  `${git(["rev-parse", "HEAD", "origin/develop"], dir)}${git(["status", "--porcelain"], dir)}`;

const cache = new Map();
const cached = (name, dir, compute) => {
  const key = `${name}\n${keyOf(dir)}`;
  const hit = cache.get(`${name}:${dir}`);
  if (hit?.key === key) return hit.value;
  const value = compute();
  cache.set(`${name}:${dir}`, { key, value });
  return value;
};

/** The files a worktree's work touches: committed since the merge base, changed or new. */
export const filesOf = dir =>
  cached("files", dir, () => {
    const base = git(["merge-base", "origin/develop", "HEAD"], dir).trim();
    if (!base) return new Set();
    return new Set([
      ...lines(git(["diff", "--name-only", base], dir)),
      ...lines(git(["status", "--porcelain", "-uall"], dir)).map(pathOfStatus),
    ]);
  });

/**
 * The diff of `dir` against develop, with the overlaps. `others` are the other
 * worktrees ({ dir, name }) whose work is compared; the main checkout is
 * develop itself and belongs in none.
 */
export const diffOf = (dir, others = []) => {
  const own = cached("diff", dir, () => {
    const base = git(["merge-base", "origin/develop", "HEAD"], dir).trim();
    const status = lines(git(["status", "--porcelain", "-uall"], dir));
    const count = range =>
      Number(git(["rev-list", "--count", range], dir).trim()) || 0;
    if (!base) return null;

    const areas = new Map();
    let added = 0;
    let removed = 0;
    const files = [];
    // Against the working tree: the uncommitted changes are part of the work.
    for (const line of lines(git(["diff", "--numstat", base], dir))) {
      const [plus, minus, file] = line.split("\t");
      // A binary file has "-": it counts as a file, not as lines.
      const fileAdded = Number(plus) || 0;
      const fileRemoved = Number(minus) || 0;
      const name = file.includes(" => ")
        ? file
            .replace(/\{([^}]*) => ([^}]*)\}/, "$2")
            .split(" => ")
            .at(-1)
        : file;
      files.push(name);
      added += fileAdded;
      removed += fileRemoved;
      const area = areas.get(areaOf(name)) ?? {
        name: areaOf(name),
        added: 0,
        removed: 0,
        files: 0,
      };
      area.added += fileAdded;
      area.removed += fileRemoved;
      area.files += 1;
      topOf(area, name, fileAdded + fileRemoved);
      areas.set(area.name, area);
    }
    // New files git doesn't track yet: counted as files, their lines as added.
    for (const file of status
      .filter(line => line.startsWith("??"))
      .map(pathOfStatus)) {
      if (files.includes(file)) continue;
      const fileAdded = lines(
        git(["diff", "--no-index", "--numstat", "/dev/null", file], dir),
      )[0]?.split("\t")[0];
      files.push(file);
      added += Number(fileAdded) || 0;
      const area = areas.get(areaOf(file)) ?? {
        name: areaOf(file),
        added: 0,
        removed: 0,
        files: 0,
      };
      area.added += Number(fileAdded) || 0;
      area.files += 1;
      topOf(area, file, Number(fileAdded) || 0);
      areas.set(area.name, area);
    }

    const onDevelop = new Set(
      lines(git(["diff", "--name-only", base, "origin/develop"], dir)),
    );
    return {
      ahead: count("origin/develop..HEAD"),
      behind: count("HEAD..origin/develop"),
      uncommitted: status.length,
      added,
      removed,
      files: files.length,
      areas: [...areas.values()].sort(
        (a, b) => b.added + b.removed - (a.added + a.removed),
      ),
      withDevelop: files.filter(file => onDevelop.has(file)).sort(),
    };
  });
  if (!own) return null;

  const mine = filesOf(dir);
  const withWorktrees = others
    .map(other => ({
      name: other.name,
      files: [...filesOf(other.dir)].filter(file => mine.has(file)).sort(),
    }))
    .filter(other => other.files.length > 0);
  return { ...own, withWorktrees };
};

/**
 * A bar of `width` cells for `added` and `removed`, scaled so that `max`
 * fills it: `█` added, `░` removed, a cell at least for any nonzero part.
 */
export const barOf = (added, removed, max, width) => {
  if (max <= 0) return { added: "", removed: "" };
  const cells = total =>
    total > 0 ? Math.max(1, Math.round((total / max) * width)) : 0;
  return {
    added: "█".repeat(cells(added)),
    removed: "░".repeat(cells(removed)),
  };
};
