import fs from "node:fs";
import path from "node:path";

import { fail, flagValue, git } from "../../lib/proc.mjs";
import {
  areaOfPath,
  areasOf,
  CHANGELOG,
  emptyChangelog,
  foldInto,
  FRAGMENTS,
  libraryEntryOf,
  mrOfFragment,
  NO_CHANGELOG,
  notesOf,
  problemsOf,
  projectUrl,
  readFragments,
  sectionBody,
  today,
  verdictOf,
  versionsOf,
} from "../platform-changelog.mjs";

const USAGE = `usage:
  pnpm changelog:add [--type <t>] [--area <a>] "<sentence>"   a fragment for this branch's MR
  pnpm changelog:unreleased                                  what the next release would say
  pnpm changelog:release <X.Y.Z>                             fold the fragments into CHANGELOG.md
  pnpm changelog:notes <X.Y.Z>                               the section of a release, for GitLab
  pnpm changelog:check [--mr]                                every fragment valid; --mr: this MR adds one`;

const rootOf = () => {
  const top = git(["rev-parse", "--show-toplevel"]).out;
  if (!top) fail("not inside the repo");
  return top;
};
/** The words of the command line that are no flag and no flag's value. */
const wordsOf = valued => {
  const argv = process.argv.slice(4);
  return argv.filter(
    (arg, at) => !arg.startsWith("--") && !valued.includes(argv[at - 1]),
  );
};
/** The entries of the fragments; fails with every problem when one is invalid. */
const entriesOf = root => {
  try {
    return readFragments(root);
  } catch (error) {
    return fail(`${error.message}\n  Format: ${FRAGMENTS}/README.md`);
  }
};

/** The base the branch left develop at: the MR pipeline's, else the merge base with origin/develop. */
const baseOf = root =>
  process.env.CI_MERGE_REQUEST_DIFF_BASE_SHA ||
  git(["merge-base", "origin/develop", "HEAD"], root).out ||
  null;
const TYPE_OF_COMMIT = { feat: "added", fix: "fixed" };

/**
 * A fragment named after the branch (`<issue>-<slug>.md`, `-2` … when it
 * exists): the area from the paths the branch changes, the type from its
 * first commit (feat → added, fix → fixed, else changed), unless given.
 */
const add = root => {
  const text = wordsOf(["--type", "--area"]).join(" ").trim();
  if (!text) fail(USAGE);
  const branch = git(["branch", "--show-current"], root).out;
  const name = branch.match(/^(?:feature|fix)\/(.+)$/)?.[1];
  if (!name)
    fail(
      `${branch || "HEAD"} is no feature/… or fix/… branch: write ${FRAGMENTS}/<issue>-<slug>.md yourself`,
    );
  const base = baseOf(root);
  const paths = [
    ...(base
      ? git(["diff", "--name-only", base, "HEAD"], root).out.split("\n")
      : []),
    ...git(["diff", "--name-only", "HEAD"], root).out.split("\n"),
  ].filter(file => file && !file.startsWith(`${FRAGMENTS}/`));
  const areas = [...new Set(paths.map(areaOfPath))];
  // repo (docs, config) goes along with any change; one other area is the one.
  const named = areas.filter(found => found !== "repo");
  const guessed = named.length > 0 ? named : areas;
  const area =
    flagValue("--area") ?? (guessed.length === 1 ? guessed[0] : null);
  if (!area)
    fail(
      `which area? The branch changes ${areas.join(", ") || "nothing yet"}: --area <${areasOf(root).join("|")}>`,
    );
  const subject = base
    ? git(["log", "--reverse", "--format=%s", `${base}..HEAD`], root).out.split(
        "\n",
      )[0]
    : "";
  const type =
    flagValue("--type") ??
    TYPE_OF_COMMIT[subject?.match(/^(\w+)/)?.[1]] ??
    "changed";
  const content = `---\ntype: ${type}\narea: ${area}\n---\n\n${text}\n`;
  const problems = problemsOf(name, content, areasOf(root));
  if (problems.length > 0) fail(problems.join("\n"));
  fs.mkdirSync(path.join(root, FRAGMENTS), { recursive: true });
  let file = path.join(FRAGMENTS, `${name}.md`);
  for (let n = 2; fs.existsSync(path.join(root, file)); n++)
    file = path.join(FRAGMENTS, `${name}-${n}.md`);
  fs.writeFileSync(path.join(root, file), content);
  console.log(
    `✓ ${file}\n\n${content}\n  type and area wrong? Edit the file; then commit it with the change.`,
  );
};

const unreleased = root => {
  const entries = entriesOf(root);
  if (entries.length === 0) {
    console.log("Nothing unreleased: no fragments in .changelog/.");
    return;
  }
  console.log(
    `## [Unreleased]\n\n${sectionBody(entries, areasOf(root), projectUrl(root))}`,
  );
};

/**
 * The release: the fragments, with their MR links, as the new section of
 * CHANGELOG.md (started when there is none), a monolithium release since the
 * last one as an entry; the fragments deleted, vita's package.json at the
 * version. Run on the release branch, committed by hand.
 */
const release = async root => {
  const [version] = wordsOf([]);
  if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) fail(USAGE);
  const url = projectUrl(root);
  const file = path.join(root, CHANGELOG);
  const text = fs.existsSync(file)
    ? fs.readFileSync(file, "utf-8")
    : emptyChangelog(url);
  const entries = entriesOf(root);
  const withLinks = await Promise.all(
    entries.map(async entry => ({
      ...entry,
      mr: await mrOfFragment(root, entry.file),
    })),
  );
  const library = libraryEntryOf(root, versionsOf(text)[0] ?? null, url);
  const all = library ? [...withLinks, library] : withLinks;
  if (all.length === 0)
    fail(`no fragments in ${FRAGMENTS}/: nothing to release`);
  let folded;
  try {
    folded = foldInto(text, {
      version,
      date: today(),
      body: sectionBody(all, areasOf(root), url),
      url,
    });
  } catch (error) {
    fail(error.message);
  }
  fs.writeFileSync(file, folded);
  for (const entry of entries) fs.rmSync(path.join(root, entry.file));
  const vita = path.join(root, "products/vita/web/package.json");
  if (fs.existsSync(vita))
    fs.writeFileSync(
      vita,
      fs
        .readFileSync(vita, "utf-8")
        .replace(/"version": "[^"]*"/, `"version": "${version}"`),
    );
  const unlinked = withLinks.filter(entry => !entry.mr).length;
  console.log(
    `✓ ${CHANGELOG}: [${version}] with ${all.length} entries, ${entries.length} fragments deleted, vita at ${version}` +
      (unlinked > 0
        ? `\n  ${unlinked} without an MR link (not merged yet, or glab couldn't say)`
        : "") +
      `\n  Read it, then commit: chore(release): v${version}`,
  );
};

const notes = root => {
  const [version] = wordsOf([]);
  if (!version) fail(USAGE);
  const file = path.join(root, CHANGELOG);
  const found = fs.existsSync(file)
    ? notesOf(fs.readFileSync(file, "utf-8"), version.replace(/^v/, ""))
    : null;
  if (found === null) fail(`${CHANGELOG} has no section ${version}`);
  console.log(found);
};

/**
 * Every fragment valid; with `--mr` also that the MR adds one, carries the
 * label no-changelog, or is a release (it changes CHANGELOG.md).
 */
const check = root => {
  const entries = entriesOf(root);
  console.log(
    `✓ ${entries.length} fragment${entries.length === 1 ? "" : "s"} valid`,
  );
  if (!process.argv.includes("--mr")) return;
  const base = baseOf(root);
  if (!base) fail("no base to compare with (fetch origin/develop)");
  const changed = git(
    ["diff", "--name-status", "--no-renames", base, "HEAD"],
    root,
  )
    .out.split("\n")
    .filter(Boolean)
    .map(line => line.split("\t"));
  const { isOk, reason } = verdictOf(
    changed,
    (process.env.CI_MERGE_REQUEST_LABELS ?? "").split(","),
  );
  if (!isOk)
    fail(
      `${reason}. Add one (pnpm changelog:add "<sentence>", ${FRAGMENTS}/README.md),\n` +
        `  or label the MR ${NO_CHANGELOG} when nobody would miss it in the changelog, then retry this job.`,
    );
  console.log(`✓ ${reason}`);
};

const SUBCOMMANDS = { add, unreleased, release, notes, check };

/**
 * `pnpm changelog:<sub>`: the platform's CHANGELOG.md (#291), Keep a
 * Changelog, from the fragments in .changelog/. `pnpm changelog` (no sub) is
 * the live view of develop instead (overall-changelog.mjs).
 */
export default async function platformChangelogCommand(sub) {
  const run = SUBCOMMANDS[sub];
  if (!run) fail(USAGE);
  await run(rootOf());
}
