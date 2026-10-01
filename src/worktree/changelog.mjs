// The changelog of an orchestration (#248): what it got done, every merged
// MR newest first, grouped by day and by the Conventional Commit type of its
// title, and the issues closed without one. Written in node from the record
// and GitLab, no model asked, into the ignored `.claude/orchestrations/` of
// the main checkout next to the record: `<name>.changelog.md`. It outlives
// the orchestration: the end removes record and checkout, not this.
//
// Written when an MR of it is merged: the overview of the orchestration sees
// the merge on its next GitLab read (right after its own merge click, else
// within REMOTE_MS), wt:remove and wt:prune after they note `mergedAt`.
// `pnpm wt:orchestrate <name> --changelog` rebuilds it from scratch; the tab
// `changelog` of the space shows it live (changelog-view.mjs).
import fs from "node:fs";
import path from "node:path";

import { glabApi } from "../lib/glab.mjs";
import { recordOf, titleOf } from "./origins.mjs";
import { mainDir } from "./worktrees.mjs";

/** Where the changelog of `name` lives, next to its record. */
export const changelogFile = name =>
  path.join(mainDir(), ".claude", "orchestrations", `${name}.changelog.md`);

/** The Conventional Commit types as the groups of a day, in this order; any other type goes to Other. */
const GROUPS = [
  ["feat", "Features"],
  ["fix", "Fixes"],
  ["perf", "Performance"],
  ["refactor", "Refactoring"],
  ["docs", "Documentation"],
  ["test", "Tests"],
  ["build", "Build"],
  ["ci", "CI"],
  ["chore", "Chores"],
];
const OTHER = "Other";
/** The group of an MR title `feat(breadboard): …`, and the title without its type. */
export const groupOf = title => {
  const match = title.match(/^(\w+)(?:\([^)]*\))?!?:\s*(.*)$/);
  const group = GROUPS.find(([type]) => type === match?.[1]?.toLowerCase());
  return { group: group?.[1] ?? OTHER, text: match ? match[2] : title };
};
/** How many characters of the description's summary an entry keeps. */
const SUMMARY_CHARACTERS = 280;
/**
 * One or two lines of what changed, from an MR description: the section
 * `## Summary` when there is one, else its first paragraph that isn't the
 * title again; its first two sentences (one ends before a capital),
 * markdown links as their text.
 */
export const summaryOf = (description, title = "") => {
  const text = (description ?? "").replace(/\r/g, "");
  const sections = text.split(/^#{1,6}\s+/m);
  const summary = sections.find(section => /^summary\b/i.test(section));
  const body = summary ? summary.replace(/^.*\n/, "") : sections[0];
  const paragraph =
    body
      .split(/\n\s*\n/)
      .map(part => part.trim())
      .find(
        part =>
          part &&
          part !== title.trim() &&
          // Another title (`feat(x): …`) says nothing more than the title.
          !/^\w+(\([^)]*\))?!?: [^\n]*$/.test(part) &&
          !/^(closes|fixes|resolves|part of)\b/i.test(part) &&
          !/^[-*] \[[ x]\]/.test(part),
      ) ?? "";
  const flat = paragraph
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/\s+/g, " ")
    .trim();
  // A sentence ends before a capital: `e.g. \`x\`` and `a.md` go on.
  const sentences = flat.split(/(?<=[.!?])\s+(?=\p{Lu})/u);
  const two = sentences.slice(0, 2).join(" ").trim();
  return two.length > SUMMARY_CHARACTERS
    ? `${two.slice(0, SUMMARY_CHARACTERS - 1).trimEnd()}…`
    : two;
};

const two = n => String(n).padStart(2, "0");
/** `2026-09-30`, the day of `iso` in local time. */
export const dayOf = iso => {
  const at = new Date(iso);
  return `${at.getFullYear()}-${two(at.getMonth() + 1)}-${two(at.getDate())}`;
};
/** `15:27`, the time of `iso` in local time. */
const timeOf = iso => {
  const at = new Date(iso);
  return `${two(at.getHours())}:${two(at.getMinutes())}`;
};
const byNewest = (a, b) => String(b.at).localeCompare(String(a.at));

/**
 * The changelog as Markdown. `merged`: `{ iid, url, title, issue: { number,
 * url } | null, mergedAt, summary }`; `closed`: the issues closed without an
 * MR, `{ number, url, title, closedAt }`. Newest first: the days, in each
 * day the groups in GROUPS' order, in each group the MRs by merge time.
 */
export const changelogText = ({ title, merged = [], closed = [] }) => {
  const lines = [`# ⇶ ${title}: changelog`, ""];
  if (merged.length === 0 && closed.length === 0)
    lines.push("Nothing merged yet.", "");
  const entries = merged
    .map(mr => ({ ...mr, at: mr.mergedAt, ...groupOf(mr.title) }))
    .toSorted(byNewest);
  const order = [...GROUPS.map(([, group]) => group), OTHER];
  for (const day of [...new Set(entries.map(entry => dayOf(entry.at)))]) {
    lines.push(`## ${day}`, "");
    const ofDay = entries.filter(entry => dayOf(entry.at) === day);
    for (const group of order) {
      const inGroup = ofDay.filter(entry => entry.group === group);
      if (inGroup.length === 0) continue;
      lines.push(`### ${group}`, "");
      for (const entry of inGroup) {
        const refs = [
          `[!${entry.iid}](${entry.url})`,
          entry.issue ? `[#${entry.issue.number}](${entry.issue.url})` : null,
        ].filter(Boolean);
        lines.push(
          `- ${refs.join(" ")} ${entry.text} · ${timeOf(entry.at)}`,
          ...(entry.summary ? [`  ${entry.summary}`] : []),
        );
      }
      lines.push("");
    }
  }
  if (closed.length) {
    lines.push("## Closed without a merge request", "");
    for (const issue of closed
      .map(issue => ({ ...issue, at: issue.closedAt }))
      .toSorted(byNewest))
      lines.push(
        `- [#${issue.number}](${issue.url}) ${issue.title}${issue.at ? ` · ${dayOf(issue.at)} ${timeOf(issue.at)}` : ""}`,
      );
    lines.push("");
  }
  return lines.join("\n");
};

/** The MR merged from `branch`, the last one when there are several; null when none or GitLab doesn't answer. */
const mergedMrOf = async (branch, gitlab) => {
  const found = await gitlab(
    `projects/:fullpath/merge_requests?state=merged&source_branch=${encodeURIComponent(branch)}`,
  ).catch(() => []);
  return (
    found.toSorted((a, b) =>
      String(b.merged_at).localeCompare(String(a.merged_at)),
    )[0] ?? null
  );
};
/** The issue `number` as GitLab has it, null when it doesn't answer. */
const issueOf = (number, gitlab) =>
  gitlab(`projects/:fullpath/issues/${number}`).catch(() => null);
/**
 * What the changelog of `name` lists, from its record and GitLab: `{ title,
 * merged, closed }` for changelogText. Null without a record. `gitlab`:
 * a GET of the API, parsed.
 */
export const readChangelog = async (name, gitlab = glabApi) => {
  const record = recordOf(name);
  if (!record) return null;
  const read = await Promise.all(
    record.issues
      .filter(issue => issue.branch)
      .map(async issue => {
        const mr = await mergedMrOf(issue.branch, gitlab);
        // Its link, and whether it was closed without an MR.
        const found = issue.number ? await issueOf(issue.number, gitlab) : null;
        return { issue, mr, found };
      }),
  );
  const merged = read
    .filter(({ mr }) => mr)
    .map(({ issue, mr, found }) => ({
      iid: mr.iid,
      url: mr.web_url,
      title: mr.title.replace(/^Draft:\s*/, ""),
      issue: issue.number
        ? {
            number: issue.number,
            url:
              found?.web_url ??
              mr.web_url.replace(
                /merge_requests\/\d+$/,
                `issues/${issue.number}`,
              ),
          }
        : null,
      mergedAt: mr.merged_at,
      summary: summaryOf(mr.description, mr.title),
    }));
  const closed = read
    .filter(({ mr, found }) => !mr && found?.state === "closed")
    .map(({ found }) => ({
      number: found.iid,
      url: found.web_url,
      title: found.title,
      closedAt: found.closed_at ?? null,
    }));
  return { title: titleOf(name), merged, closed };
};
/**
 * The changelog of `name` rewritten from scratch, record and GitLab. Returns
 * its text, or null without a record: a changelog there already stays as it
 * is (the orchestration ended, its record is gone).
 */
export const writeChangelog = async (name, gitlab = glabApi) => {
  const read = await readChangelog(name, gitlab);
  if (!read) return null;
  const text = changelogText(read);
  fs.mkdirSync(path.dirname(changelogFile(name)), { recursive: true });
  // Through a temporary file: the tab reading it never sees half of it.
  const temporary = `${changelogFile(name)}.${process.pid}.tmp`;
  fs.writeFileSync(temporary, text);
  fs.renameSync(temporary, changelogFile(name));
  return text;
};
/** The MRs (`!n`) and issues (`#n`) the changelog of `name` lists now. */
export const listedIn = name => {
  let text = "";
  try {
    text = fs.readFileSync(changelogFile(name), "utf-8");
  } catch {
    // None yet: it lists nothing.
  }
  const numbers = mark =>
    new Set(
      [...text.matchAll(new RegExp(`\\[${mark}(\\d+)\\]`, "g"))].map(m =>
        Number(m[1]),
      ),
    );
  return { mrs: numbers("!"), issues: numbers("#") };
};
/**
 * Whether the changelog of `name` misses one of the merged MRs `mrs` (iids)
 * or of the issues `closed` without one (numbers) of the orchestration.
 */
export const isChangelogBehind = (name, { mrs = [], closed = [] }) => {
  const listed = listedIn(name);
  return (
    mrs.some(iid => !listed.mrs.has(Number(iid))) ||
    closed.some(number => !listed.issues.has(Number(number)))
  );
};
