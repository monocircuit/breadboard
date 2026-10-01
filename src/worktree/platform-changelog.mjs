// The changelog of the platform (#291): `CHANGELOG.md` at the root, Keep a
// Changelog 1.1.0, one section per vita release (`vX.Y.Z`). Unlike the
// changelogs of develop (overall-changelog.mjs) and of an orchestration
// (changelog.mjs), it is committed and written by people: every MR adds a
// fragment `.changelog/<issue>-<slug>.md` (type, area, a sentence), and the
// release folds them into its section. No MR edits CHANGELOG.md itself, so
// parallel branches never conflict on it (.changelog/README.md).
//
// Plain node and git only: CI checks the fragments without an install and
// without glab. glab is asked for the MR links at release time, and its
// absence only drops them.
import fs from "node:fs";
import path from "node:path";

import { exec, git } from "../lib/proc.mjs";

/** Where the fragments wait for the next release. */
export const FRAGMENTS = ".changelog";
export const CHANGELOG = "CHANGELOG.md";
/** The label of an MR that needs no entry (CI `check:changelog`). */
export const NO_CHANGELOG = "no-changelog";

/** The types of Keep a Changelog, in its order, with their headings. */
export const TYPES = [
  ["added", "Added"],
  ["changed", "Changed"],
  ["deprecated", "Deprecated"],
  ["removed", "Removed"],
  ["fixed", "Fixed"],
  ["security", "Security"],
];
/** Areas that are no directory of products/ or packages/. */
const OTHER_AREAS = ["ci", "repo"];

/**
 * The areas an entry may name, in the order the changelog lists them: the
 * products, the packages (without `monocircuit-`), then ci and repo.
 */
export const areasOf = root => {
  const dirs = parent =>
    fs.existsSync(path.join(root, parent))
      ? fs
          .readdirSync(path.join(root, parent), { withFileTypes: true })
          .filter(entry => entry.isDirectory())
          .map(entry => entry.name.replace(/^monocircuit-/, ""))
          .toSorted()
      : [];
  return [...dirs("products"), ...dirs("packages"), ...OTHER_AREAS];
};
/** The area of a changed path: its product or package, else ci for the pipeline, else repo. */
export const areaOfPath = file => {
  const match = file.match(/^(?:products|packages)\/([^/]+)\//);
  if (match) return match[1].replace(/^monocircuit-/, "");
  return /^\.gitlab(-ci\.yml|\/ci\/)/.test(file) ? "ci" : "repo";
};

/** A fragment's frontmatter fields and text, `{ fields, text }`; null without frontmatter. */
export const parseFragment = content => {
  const match = content
    .replace(/\r/g, "")
    .match(/^---\n([\s\S]*?)\n---\n?([\s\S]*)$/);
  if (!match) return null;
  const fields = Object.fromEntries(
    match[1]
      .split("\n")
      .map(line => line.replace(/\s+#.*$/, "").match(/^(\w+):\s*(.*?)\s*$/))
      .filter(Boolean)
      .map(([, key, value]) => [key, value]),
  );
  return { fields, text: match[2].trim() };
};
/** The issue a fragment's file name starts with (`291-platform-changelog.md`), else null. */
export const issueOfName = name => {
  const match = path.basename(name).match(/^(\d+)-/);
  return match ? Number(match[1]) : null;
};
/** What is wrong with a fragment, as sentences; none for a valid one. */
export const problemsOf = (name, content, areas) => {
  const parsed = parseFragment(content);
  if (!parsed)
    return [
      `${name}: starts with a frontmatter block (--- type: … area: … ---)`,
    ];
  const { fields, text } = parsed;
  const problems = [];
  const types = TYPES.map(([type]) => type);
  if (!types.includes(fields.type))
    problems.push(
      `${name}: type "${fields.type ?? ""}" is none of ${types.join(", ")}`,
    );
  if (!areas.includes(fields.area))
    problems.push(
      `${name}: area "${fields.area ?? ""}" is none of ${areas.join(", ")}`,
    );
  const unknown = Object.keys(fields).filter(
    key => key !== "type" && key !== "area",
  );
  if (unknown.length > 0)
    problems.push(`${name}: unknown field ${unknown.join(", ")}`);
  if (!text) problems.push(`${name}: says nothing below the frontmatter`);
  if (/^#/m.test(text))
    problems.push(`${name}: has a heading; an entry is a sentence or two`);
  return problems;
};

/** The fragment files of `root`, by name (README.md is the explanation, not one). */
export const fragmentFiles = root => {
  const dir = path.join(root, FRAGMENTS);
  if (!fs.existsSync(dir)) return [];
  return fs
    .readdirSync(dir)
    .filter(name => name.endsWith(".md") && name !== "README.md")
    .toSorted()
    .map(name => path.join(FRAGMENTS, name));
};
/** The fragments of `root` as entries `{ file, type, area, text, issue }`; fails on an invalid one. */
export const readFragments = root => {
  const areas = areasOf(root);
  const files = fragmentFiles(root);
  const problems = files.flatMap(file =>
    problemsOf(file, fs.readFileSync(path.join(root, file), "utf-8"), areas),
  );
  if (problems.length > 0) throw new Error(problems.join("\n"));
  return files.map(file => {
    const { fields, text } = parseFragment(
      fs.readFileSync(path.join(root, file), "utf-8"),
    );
    return {
      file,
      type: fields.type,
      area: fields.area,
      text,
      issue: issueOfName(file),
    };
  });
};

/** The project's web URL, from CI or from the origin remote (ssh or https). */
export const projectUrl = root => {
  if (process.env.CI_PROJECT_URL) return process.env.CI_PROJECT_URL;
  const remote = git(["remote", "get-url", "origin"], root).out;
  const match = remote.match(
    /^(?:git@|ssh:\/\/git@|https?:\/\/)([^:/]+)[:/](.+?)(?:\.git)?$/,
  );
  return match ? `https://${match[1]}/${match[2]}` : null;
};

/** The links of an entry: its issue and its MR, as `([#n](…), [!m](…))`; empty without either. */
const linksOf = (entry, url) => {
  const links = [
    entry.issue && url
      ? `[#${entry.issue}](${url}/-/issues/${entry.issue})`
      : null,
    entry.mr ? `[!${entry.mr.iid}](${entry.mr.url})` : null,
  ].filter(Boolean);
  return links.length > 0 ? ` (${links.join(", ")})` : "";
};
/** One list item: `- **area:** text (links)`, further lines of the text indented under it. */
const itemOf = (entry, url) => {
  const [first, ...rest] = entry.text.split("\n");
  const more = rest.map(line => (line.trim() ? `  ${line.trim()}` : ""));
  const last = more.length > 0 ? more.length - 1 : -1;
  const links = linksOf(entry, url);
  if (last === -1) return [`- **${entry.area}:** ${first}${links}`];
  more[last] += links;
  return [`- **${entry.area}:** ${first}`, ...more];
};
/**
 * The body of a section: per type in Keep a Changelog's order a `### Added`
 * … with its entries, sorted by area (in `areas`' order), then by file name.
 */
export const sectionBody = (entries, areas, url) => {
  const rank = area => {
    const at = areas.indexOf(area);
    return at === -1 ? areas.length : at;
  };
  return TYPES.flatMap(([type, heading]) => {
    const ofType = entries
      .filter(entry => entry.type === type)
      .toSorted(
        (a, b) =>
          rank(a.area) - rank(b.area) ||
          String(a.file ?? "").localeCompare(String(b.file ?? "")),
      );
    if (ofType.length === 0) return [];
    return [
      `### ${heading}`,
      "",
      ...ofType.flatMap(entry => itemOf(entry, url)),
      "",
    ];
  })
    .join("\n")
    .trimEnd();
};

/** The file a first release starts from. */
export const emptyChangelog = url =>
  `# Changelog

All notable changes to Monocircuit/Platform are documented in this file. The
format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the versions are those of vita's releases (\`vX.Y.Z\`, [Semantic
Versioning](https://semver.org/spec/v2.0.0.html)). Entries are collected in
[\`.changelog/\`](.changelog/README.md) until the release folds them in here.

## [Unreleased]
${url ? `\n[Unreleased]: ${url}/-/commits/develop\n` : ""}`;

const VERSION = /^\d+\.\d+\.\d+$/;
/** The versions of a changelog's sections (`## [1.2.0] - …`), newest first. */
export const versionsOf = text =>
  [...text.matchAll(/^## \[(\d+\.\d+\.\d+)\]/gm)].map(match => match[1]);
/**
 * The changelog with a new section `## [version] - date` right below
 * `## [Unreleased]`, and the link references at its end: Unreleased compares
 * from the new tag, the new version from the one before (or is its tag).
 */
export const foldInto = (text, { version, date, body, url }) => {
  if (!VERSION.test(version)) throw new Error(`"${version}" is no X.Y.Z`);
  if (versionsOf(text).includes(version))
    throw new Error(`${CHANGELOG} has a section ${version} already`);
  const previous = versionsOf(text)[0] ?? null;
  const unreleased = text.match(/^## \[Unreleased\][^\n]*\n/m);
  if (!unreleased) throw new Error(`${CHANGELOG} has no "## [Unreleased]"`);
  const at = unreleased.index + unreleased[0].length;
  // Whatever stands under Unreleased by hand goes on standing there.
  const rest = text.slice(at);
  const next = rest.search(/^(## \[|\[[^\]]+\]: )/m);
  const kept = (next === -1 ? rest : rest.slice(0, next)).trim();
  const after = next === -1 ? "" : rest.slice(next);
  const section = `## [${version}] - ${date}\n\n${body}\n`;
  let result = `${text.slice(0, at)}\n${kept ? `${kept}\n\n` : ""}${section}\n${after}`;
  if (!url) return `${result.trimEnd()}\n`;
  const references = [
    `[Unreleased]: ${url}/-/compare/v${version}...develop`,
    previous
      ? `[${version}]: ${url}/-/compare/v${previous}...v${version}`
      : `[${version}]: ${url}/-/tags/v${version}`,
  ];
  result = result.replace(/^\[Unreleased\]: .*\n?/m, "");
  const firstReference = result.search(/^\[\d+\.\d+\.\d+\]: /m);
  result =
    firstReference === -1
      ? `${result.trimEnd()}\n\n${references.join("\n")}\n`
      : `${result.slice(0, firstReference)}${references.join("\n")}\n${result.slice(firstReference)}`;
  return `${result.trimEnd()}\n`;
};
/** The body of `version`'s section, for the GitLab release; null when there is none. */
export const notesOf = (text, version) => {
  const start = text.search(
    new RegExp(`^## \\[${version.replaceAll(".", "\\.")}\\]`, "m"),
  );
  if (start === -1) return null;
  const rest = text.slice(start).replace(/^.*\n/, "");
  const end = rest.search(/^(## \[|\[[^\]]+\]: )/m);
  return (end === -1 ? rest : rest.slice(0, end)).trim();
};

/** The MR that added `file` to develop, `{ iid, url }`, through glab; null when git or glab can't say. */
export const mrOfFragment = async (root, file) => {
  const sha = git(
    ["log", "--diff-filter=A", "--format=%H", "-1", "--", file],
    root,
  ).out;
  if (!sha) return null;
  try {
    const { stdout } = await exec(
      "glab",
      ["api", `projects/:fullpath/repository/commits/${sha}/merge_requests`],
      { cwd: root },
    );
    const mr =
      JSON.parse(stdout).find(found => found.state === "merged") ?? null;
    return mr ? { iid: mr.iid, url: mr.web_url } : null;
  } catch {
    return null;
  }
};
/** The `version` of a package.json at `ref` (a tag), null when it isn't there. */
const versionAt = (root, ref, file) => {
  const shown = git(["show", `${ref}:${file}`], root);
  if (!shown.ok) return null;
  try {
    return JSON.parse(shown.out).version ?? null;
  } catch {
    return null;
  }
};
const LIBRARY = "packages/monolithium";
/**
 * The entry for a release of monolithium since the vita release before
 * `previous`: its version changed (`changeset version`), so the section says
 * where its own changelog has the details. None for the first release.
 */
export const libraryEntryOf = (root, previous, url) => {
  if (!previous) return null;
  const file = `${LIBRARY}/package.json`;
  const before = versionAt(root, `v${previous}`, file);
  const now = JSON.parse(
    fs.readFileSync(path.join(root, file), "utf-8"),
  ).version;
  if (!before || before === now) return null;
  const changelog = `${LIBRARY}/CHANGELOG.md`;
  const link = url
    ? `[${changelog}](${url}/-/blob/monolithium-v${now}/${changelog})`
    : changelog;
  return {
    type: "changed",
    area: "monolithium",
    text: `released ${now} (with utilities), see ${link}`,
    issue: null,
  };
};

/** Today in local time, `2026-10-01`. */
export const today = () => {
  const now = new Date();
  const two = n => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${two(now.getMonth() + 1)}-${two(now.getDate())}`;
};

const isFragment = file =>
  file.startsWith(`${FRAGMENTS}/`) &&
  file.endsWith(".md") &&
  file !== `${FRAGMENTS}/README.md`;
/**
 * Whether an MR may go without a new fragment, from its changes
 * (`[status, path]` of `git diff --name-status`) and labels: it adds one, it
 * is labelled no-changelog, or it is a release (it changes CHANGELOG.md and
 * deletes the fragments it folded in). `{ isOk, reason }`.
 */
export const verdictOf = (changed, labels) => {
  const added = changed.filter(
    ([status, file]) => status === "A" && isFragment(file),
  );
  if (added.length > 0)
    return {
      isOk: true,
      reason: `adds ${added.map(([, file]) => file).join(", ")}`,
    };
  if (labels.includes(NO_CHANGELOG))
    return { isOk: true, reason: `labelled ${NO_CHANGELOG}: no entry needed` };
  const isRelease =
    changed.some(([, file]) => file === CHANGELOG) &&
    changed.some(([status, file]) => status === "D" && isFragment(file));
  if (isRelease)
    return { isOk: true, reason: `folds the fragments into ${CHANGELOG}` };
  return { isOk: false, reason: `this MR adds no ${FRAGMENTS}/*.md` };
};
