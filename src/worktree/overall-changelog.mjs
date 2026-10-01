// The changelog of the whole repo (#261): what got into develop, grouped by
// product and package — `products/vita`, `packages/monolithium`, … and
// `repo` for the rest (CI, `.claude`, root config) — not per orchestration
// (that one is changelog.mjs). Every merged MR of the window, placed in each
// group whose paths it changes; in a group the days newest first, in a day
// the Conventional Commit types, and the releases as markers: vita's
// `vX.Y.Z` tags in products/vita, `monolithium-vX.Y.Z` in monolithium and
// utilities, with their CHANGELOG.md entries.
//
// Written in node from GitLab, no model asked, into the ignored `.claude/`
// of the main checkout: `changelog.md`, next to `changelog.json` (the
// window and what a merged MR changed, which never changes again). The
// main overview rewrites it when develop's head moves; `pnpm changelog
// [--since 30d|<date>]` rewrites and prints it; the tab `changelog` of the
// main space shows it live (changelog-view.mjs).
import fs from "node:fs";
import path from "node:path";

import { glabApi } from "../lib/glab.mjs";
import { dayOf, groupOf, summaryOf } from "./changelog.mjs";
import { ORCHESTRATOR, titleOf } from "./origins.mjs";
import { mainDir } from "./worktrees.mjs";

/** Where the changelog of the repo lives, and what it keeps between rewrites. */
export const overallFile = () =>
  path.join(mainDir(), ".claude", "changelog.md");
const stateFile = () => path.join(mainDir(), ".claude", "changelog.json");

/** The window when none was chosen: the last 30 days. */
export const DEFAULT_SINCE = "30d";
/** The group of what is neither a product nor a package. */
export const REPO = "repo";
/** The start of the window `since` (`30d`, `2w`, `2026-09-01`) at `now`; null when it is neither. */
export const sinceOf = (since, now = new Date()) => {
  const relative = String(since).match(/^(\d+)([dw])$/);
  if (relative) {
    const days = Number(relative[1]) * (relative[2] === "w" ? 7 : 1);
    return new Date(now.getTime() - days * 86_400_000);
  }
  const at = new Date(since);
  return /^\d{4}-\d\d-\d\d/.test(since) && !Number.isNaN(at.getTime())
    ? at
    : null;
};

/** The group of a changed path: `products/<x>`, `packages/<x>`, else REPO. */
export const groupOfPath = file => {
  const match = file.match(/^(products|packages)\/([^/]+)\//);
  return match ? `${match[1]}/${match[2]}` : REPO;
};
/** The package directory of a package name in a changeset, `@monocircuit/monolithium` → packages/monolithium. */
const groupOfPackage = name => `packages/${name.split("/").at(-1)}`;
/**
 * The groups an MR belongs to, from the paths it changes. A changeset
 * (`.changeset/*.md`) counts for the packages it names, not for REPO.
 */
export const groupsOf = (paths, changesets = []) => {
  const groups = new Set(
    paths
      .filter(file => !/^\.changeset\/[^/]+\.md$/.test(file))
      .map(groupOfPath),
  );
  for (const changeset of changesets)
    for (const name of changeset.packages) groups.add(groupOfPackage(name));
  return [...groups];
};

/** A changeset's packages (`"@x/y": minor`) and its text, from the file's content. */
export const changesetOf = text => {
  const match = text
    .replace(/\r/g, "")
    .match(/^---\n([\s\S]*?)\n---\n?([\s\S]*)$/);
  if (!match) return null;
  const packages = [
    ...match[1].matchAll(/^\s*["']?([^"':]+)["']?\s*:\s*(\w+)/gm),
  ];
  return {
    packages: packages.map(([, name]) => name.trim()),
    bump: packages.map(([, , bump]) => bump)[0] ?? null,
    text: match[2].trim(),
  };
};
/** The changesets an MR adds, from its diffs: the added lines of a new `.changeset/*.md`. */
export const changesetsOfDiffs = diffs =>
  diffs
    .filter(
      diff =>
        diff.new_file && /^\.changeset\/[^/]+\.md$/.test(diff.new_path ?? ""),
    )
    .map(diff =>
      changesetOf(
        (diff.diff ?? "")
          .split("\n")
          .filter(line => line.startsWith("+") && !line.startsWith("+++"))
          .map(line => line.slice(1))
          .join("\n"),
      ),
    )
    .filter(Boolean);

/** The issue of an MR: the number its branch starts with (`feature/261-…`), else one it closes. */
export const issueOf = (branch, description) => {
  const number =
    String(branch ?? "").match(/^(?:feature|fix)\/(\d+)-/)?.[1] ??
    String(description ?? "").match(/\b(?:closes|fixes|resolves) #(\d+)/i)?.[1];
  return number ? Number(number) : null;
};

/** The release a tag is, with the groups it marks: vita `vX.Y.Z`, the library `monolithium-vX.Y.Z`. */
export const releaseOf = tag => {
  const vita = tag.match(/^v(\d+\.\d+\.\d+.*)$/);
  if (vita)
    return {
      tag,
      version: vita[1],
      package: "vita",
      groups: ["products/vita"],
    };
  const library = tag.match(/^monolithium-v(\d+\.\d+\.\d+.*)$/);
  if (library)
    return {
      tag,
      version: library[1],
      package: "monolithium",
      groups: ["packages/monolithium", "packages/utilities"],
    };
  return null;
};
/** How many lines of a release's CHANGELOG.md entry the changelog shows. */
const RELEASE_LINES = 6;
/** The entry of `version` in a CHANGELOG.md (changesets: `## 1.2.0`, then its sections), as its list lines. */
export const releaseNotesOf = (text, version) => {
  const sections = (text ?? "").replace(/\r/g, "").split(/^## /m);
  const section = sections.find(
    part => part.startsWith(`${version}\n`) || part.startsWith(`${version} `),
  );
  if (!section) return [];
  return section
    .split("\n")
    .slice(1)
    .map(line => line.match(/^\s*[-*] (.*)$/)?.[1])
    .filter(Boolean)
    .map(line => line.replace(/^[0-9a-f]{7,}: /, ""))
    .slice(0, RELEASE_LINES);
};

const two = n => String(n).padStart(2, "0");
const timeOf = iso => {
  const at = new Date(iso);
  return `${two(at.getHours())}:${two(at.getMinutes())}`;
};
/** The name a group's heading shows: the directory, `vita`, `monolithium`, `repo`. */
export const nameOf = group => group.split("/").at(-1);
/** The order of the groups: products, packages, each by name, then REPO. */
const groupOrder = group =>
  `${group.startsWith("products/") ? 0 : group.startsWith("packages/") ? 1 : 2}${group}`;
/** The type groups of a day, in the order changelog.mjs knows them. */
const TYPES = [
  "Features",
  "Fixes",
  "Performance",
  "Refactoring",
  "Documentation",
  "Tests",
  "Build",
  "CI",
  "Chores",
  "Other",
];
const RELEASES = "Releases";

/**
 * The changelog of the repo as Markdown. `merged`: `{ iid, url, title, issue:
 * { number, url } | null, mergedAt, summary, groups, changesets,
 * orchestration }`; `releases`: `{ tag, version, url, at, groups, notes }`.
 * A section per group (`## vita`), in it the days newest first (`###`), in a
 * day its releases and then the types (`####`), the MRs newest first.
 */
export const overallText = ({ since, merged = [], releases = [] }) => {
  const lines = [
    "# Monocircuit/Platform: changelog",
    "",
    `Merged into develop since ${dayOf(since)}, by product and package.`,
    "",
  ];
  const groups = [
    ...new Set([
      ...merged.flatMap(mr => mr.groups),
      ...releases.flatMap(release => release.groups),
    ]),
  ].toSorted((a, b) => groupOrder(a).localeCompare(groupOrder(b)));
  if (groups.length === 0) lines.push("Nothing merged in this window.", "");
  for (const group of groups) {
    const ofGroup = merged
      .filter(mr => mr.groups.includes(group))
      .map(mr => ({ ...mr, at: mr.mergedAt, ...groupOf(mr.title) }));
    const marks = releases
      .filter(release => release.groups.includes(group))
      .map(release => ({ ...release, group: RELEASES }));
    const count = `${ofGroup.length} merge request${ofGroup.length === 1 ? "" : "s"}`;
    lines.push(`## ${nameOf(group)}`, "", `\`${group}\` · ${count}`, "");
    const all = [...ofGroup, ...marks].toSorted((a, b) =>
      String(b.at).localeCompare(String(a.at)),
    );
    for (const day of [...new Set(all.map(entry => dayOf(entry.at)))]) {
      lines.push(`### ${day}`, "");
      const ofDay = all.filter(entry => dayOf(entry.at) === day);
      for (const type of [RELEASES, ...TYPES]) {
        const inType = ofDay.filter(entry => entry.group === type);
        if (inType.length === 0) continue;
        lines.push(`#### ${type}`, "");
        for (const entry of inType) {
          if (type === RELEASES) {
            const name = entry.url ? `[${entry.tag}](${entry.url})` : entry.tag;
            lines.push(
              `- 🏷 ${name} · ${timeOf(entry.at)}`,
              ...entry.notes.map(note => `  ${note}`),
            );
            continue;
          }
          const refs = [
            `[!${entry.iid}](${entry.url})`,
            entry.issue ? `[#${entry.issue.number}](${entry.issue.url})` : null,
          ].filter(Boolean);
          const from = entry.orchestration
            ? ` · ${ORCHESTRATOR} ${entry.orchestration}`
            : "";
          const changesets = entry.changesets
            .filter(changeset =>
              changeset.packages.some(name => groupOfPackage(name) === group),
            )
            .map(
              changeset =>
                `  changeset${changeset.bump ? ` (${changeset.bump})` : ""}: ${summaryOf(changeset.text.replace(/^\s*[-*] /gm, ""))}`,
            );
          lines.push(
            `- ${refs.join(" ")} ${entry.text}${from} · ${timeOf(entry.at)}`,
            ...(entry.summary ? [`  ${entry.summary}`] : []),
            ...changesets,
          );
        }
        lines.push("");
      }
    }
  }
  return lines.join("\n");
};

const readJson = file => {
  try {
    return JSON.parse(fs.readFileSync(file, "utf-8"));
  } catch {
    return null;
  }
};
/** What the last rewrite kept: `{ since, develop, mrs: { iid: { paths, changesets } } }`. */
export const stateOf = () => {
  const state = readJson(stateFile()) ?? {};
  return {
    since: state.since ?? DEFAULT_SINCE,
    develop: state.develop ?? null,
    mrs: state.mrs ?? {},
  };
};
const writeAtomic = (file, text) => {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  // Through a temporary file: the tab reading it never sees half of it.
  const temporary = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(temporary, text);
  fs.renameSync(temporary, file);
};

/**
 * The orchestration each MR came from, iid → its title: the MRs its
 * changelog lists (it outlives the orchestration), and the branches of a
 * record still there.
 */
export const orchestrationsOf = (mrs = []) => {
  const dir = path.join(mainDir(), ".claude", "orchestrations");
  const files = fs.existsSync(dir) ? fs.readdirSync(dir) : [];
  const found = new Map();
  for (const file of files.filter(file => file.endsWith(".changelog.md"))) {
    let text = "";
    try {
      text = fs.readFileSync(path.join(dir, file), "utf-8");
    } catch {
      continue;
    }
    const title =
      text.match(new RegExp(`^# ${ORCHESTRATOR} (.*): changelog$`, "m"))?.[1] ??
      file.replace(/\.changelog\.md$/, "");
    for (const [, iid] of text.matchAll(/\[!(\d+)\]/g))
      found.set(Number(iid), title);
  }
  for (const file of files.filter(file => file.endsWith(".json"))) {
    const name = path.basename(file, ".json");
    const branches = new Set(
      (readJson(path.join(dir, file))?.issues ?? []).map(issue => issue.branch),
    );
    for (const mr of mrs)
      if (branches.has(mr.source_branch) && !found.has(mr.iid))
        found.set(mr.iid, titleOf(name));
  }
  return found;
};

/** Pages of a GitLab list until one comes back short. */
const PER_PAGE = 100;
/** At most this many pages of one list: a window of years is cut there. */
const MAX_PAGES = 20;
const allOf = async (endpoint, gitlab) => {
  const all = [];
  for (let page = 1; page <= MAX_PAGES; page++) {
    const found = await gitlab(
      `${endpoint}${endpoint.includes("?") ? "&" : "?"}per_page=${PER_PAGE}&page=${page}`,
    );
    all.push(...found);
    if (found.length < PER_PAGE) break;
  }
  return all;
};
/** How many reads of GitLab run side by side: a first window of a month reads hundreds of MRs. */
const AT_ONCE = 8;
/** `run` for each of `items`, AT_ONCE at a time, the results in their order. */
export const inTurns = async (items, run, atOnce = AT_ONCE) => {
  const results = [];
  let next = 0;
  const worker = async () => {
    while (next < items.length) {
      const at = next++;
      results[at] = await run(items[at]);
    }
  };
  await Promise.all(
    Array.from({ length: Math.min(atOnce, items.length) }, worker),
  );
  return results;
};
/** What a merged MR changed, `{ paths, changesets }`: from the state, else its diffs. */
const changesOf = async (iid, known, gitlab) => {
  if (known[iid]) return known[iid];
  const diffs = await allOf(
    `projects/:fullpath/merge_requests/${iid}/diffs`,
    gitlab,
  );
  return {
    // A renamed file counts where it went, a deleted one where it was.
    paths: [
      ...new Set(
        diffs
          .map(diff => (diff.deleted_file ? diff.old_path : diff.new_path))
          .filter(Boolean),
      ),
    ],
    changesets: changesetsOfDiffs(diffs),
  };
};
/** The CHANGELOG.md of a package directory in the main checkout, empty when there is none. */
const changelogOf = group => {
  try {
    return fs.readFileSync(
      path.join(mainDir(), group, "CHANGELOG.md"),
      "utf-8",
    );
  } catch {
    return "";
  }
};

/**
 * What the changelog of the repo lists, from GitLab: the MRs merged into
 * develop since `since` with what they changed (`known`: iid → changes
 * read before), and the release tags of the window.
 */
export const readOverall = async ({
  since = DEFAULT_SINCE,
  known = {},
  gitlab = glabApi,
  now = new Date(),
} = {}) => {
  const start = sinceOf(since, now) ?? sinceOf(DEFAULT_SINCE, now);
  const found = (
    await allOf(
      `projects/:fullpath/merge_requests?state=merged&target_branch=develop&updated_after=${encodeURIComponent(start.toISOString())}`,
      gitlab,
    )
  ).filter(mr => mr.merged_at && new Date(mr.merged_at) >= start);
  const changes = new Map(
    await inTurns(found, async mr => [
      mr.iid,
      await changesOf(mr.iid, known, gitlab),
    ]),
  );
  const orchestrations = orchestrationsOf(found);
  const merged = found.map(mr => {
    const { paths, changesets } = changes.get(mr.iid);
    const number = issueOf(mr.source_branch, mr.description);
    return {
      iid: mr.iid,
      url: mr.web_url,
      title: mr.title.replace(/^Draft:\s*/, ""),
      issue: number
        ? {
            number,
            url: mr.web_url.replace(/merge_requests\/\d+$/, `issues/${number}`),
          }
        : null,
      mergedAt: mr.merged_at,
      summary: summaryOf(mr.description, mr.title),
      groups: groupsOf(paths, changesets),
      changesets,
      orchestration: orchestrations.get(mr.iid) ?? null,
    };
  });
  const tags = await allOf("projects/:fullpath/repository/tags", gitlab).catch(
    () => [],
  );
  const base = found[0]?.web_url?.replace(/\/merge_requests\/\d+$/, "") ?? null;
  const releases = tags
    .map(tag => ({
      ...releaseOf(tag.name),
      at:
        tag.created_at ?? tag.commit?.committed_date ?? tag.commit?.created_at,
    }))
    .filter(
      release => release.tag && release.at && new Date(release.at) >= start,
    )
    .map(release => ({
      ...release,
      url: base ? `${base}/tags/${release.tag}` : null,
      notes: releaseNotesOf(changelogOf(release.groups[0]), release.version),
    }));
  return {
    since: start.toISOString(),
    merged,
    releases,
    changes: Object.fromEntries(changes),
  };
};
/**
 * The changelog of the repo rewritten from GitLab, in the window `since`
 * (else the one chosen last, else DEFAULT_SINCE). `develop`: the head it is
 * written for, so the overview knows when to write it again. Returns its text.
 */
export const writeOverall = async ({
  since,
  develop = null,
  gitlab = glabApi,
  now = new Date(),
} = {}) => {
  const state = stateOf();
  const window = since && sinceOf(since, now) ? since : state.since;
  const read = await readOverall({
    since: window,
    known: state.mrs,
    gitlab,
    now,
  });
  const text = overallText(read);
  writeAtomic(overallFile(), text);
  writeAtomic(
    stateFile(),
    `${JSON.stringify({ since: window, develop: develop ?? state.develop, mrs: read.changes }, null, 2)}\n`,
  );
  return text;
};
/** Whether the changelog of the repo was written for another head of develop than `develop` (or not at all). */
export const isOverallBehind = develop =>
  !fs.existsSync(overallFile()) ||
  (Boolean(develop) && stateOf().develop !== develop);
