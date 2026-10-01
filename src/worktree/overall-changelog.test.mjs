import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { after, before, describe, it } from "node:test";

// Days and times of the changelog are local: the same everywhere in this test.
process.env.TZ = "UTC";

const {
  changesetOf,
  changesetsOfDiffs,
  groupOfPath,
  groupsOf,
  inTurns,
  issueOf,
  overallText,
  readOverall,
  releaseNotesOf,
  releaseOf,
  sinceOf,
} = await import("./overall-changelog.mjs");
const { viewLines } = await import("./changelog-view.mjs");
const { mainDir } = await import("./worktrees.mjs");

const URL = "https://gitlab.test/monocircuit/platform/-";

describe("groupsOf", () => {
  it("puts a path under its product or package, the rest under repo", () => {
    assert.equal(
      groupOfPath("products/vita/src/app/page.tsx"),
      "products/vita",
    );
    assert.equal(
      groupOfPath("packages/monolithium/src/x.tsx"),
      "packages/monolithium",
    );
    assert.equal(groupOfPath(".gitlab-ci.yml"), "repo");
    assert.equal(groupOfPath(".claude/skills/gitlab/SKILL.md"), "repo");
    assert.equal(groupOfPath("packages/README.md"), "repo");
  });

  it("places an MR in every group whose paths it changes, once each", () => {
    assert.deepEqual(
      groupsOf([
        "products/vita/a.ts",
        "packages/monolithium/b.tsx",
        "packages/monolithium/c.tsx",
        "package.json",
      ]),
      ["products/vita", "packages/monolithium", "repo"],
    );
  });

  it("counts a changeset for the packages it names, not for repo", () => {
    const changeset = changesetOf(
      '---\n"@monocircuit/monolithium": minor\n"@monocircuit/utilities": minor\n---\n\n- New `Dot`.\n',
    );
    assert.deepEqual(changeset, {
      packages: ["@monocircuit/monolithium", "@monocircuit/utilities"],
      bump: "minor",
      text: "- New `Dot`.",
    });
    assert.deepEqual(
      groupsOf(
        ["packages/monolithium/dot.tsx", ".changeset/dot.md"],
        [changeset],
      ),
      ["packages/monolithium", "packages/utilities"],
    );
  });

  it("reads the changesets an MR adds from its diffs", () => {
    const diffs = [
      {
        new_path: ".changeset/dot.md",
        new_file: true,
        diff: '@@ -0,0 +1,5 @@\n+---\n+"@monocircuit/monolithium": patch\n+---\n+\n+- A fix.\n',
      },
      { new_path: ".changeset/README.md", new_file: false, diff: "+x" },
      { new_path: "packages/monolithium/a.tsx", new_file: true, diff: "+x" },
    ];
    assert.deepEqual(changesetsOfDiffs(diffs), [
      {
        packages: ["@monocircuit/monolithium"],
        bump: "patch",
        text: "- A fix.",
      },
    ]);
  });
});

describe("issueOf", () => {
  it("takes the number the branch starts with, else one the MR closes", () => {
    assert.equal(issueOf("feature/261-overall-changelog", ""), 261);
    assert.equal(issueOf("fix/12-flicker", "Closes #99"), 12);
    assert.equal(issueOf("feature/link-only-dot", "Closes #241"), 241);
    assert.equal(issueOf("feature/link-only-dot", ""), null);
  });
});

describe("sinceOf", () => {
  const now = new Date("2026-10-01T12:00:00Z");
  it("takes days, weeks or a date", () => {
    assert.equal(sinceOf("30d", now).toISOString(), "2026-09-01T12:00:00.000Z");
    assert.equal(sinceOf("2w", now).toISOString(), "2026-09-17T12:00:00.000Z");
    assert.equal(
      sinceOf("2026-09-15", now).toISOString(),
      "2026-09-15T00:00:00.000Z",
    );
    assert.equal(sinceOf("soon", now), null);
  });
});

describe("releases", () => {
  it("knows vita's tags and the library's, and where they mark", () => {
    assert.deepEqual(releaseOf("v1.4.0").groups, ["products/vita"]);
    assert.deepEqual(releaseOf("monolithium-v0.9.0"), {
      tag: "monolithium-v0.9.0",
      version: "0.9.0",
      package: "monolithium",
      groups: ["packages/monolithium", "packages/utilities"],
    });
    assert.equal(releaseOf("nightly"), null);
  });

  it("takes the entry of a version from a CHANGELOG.md", () => {
    const text =
      "# @monocircuit/monolithium\n\n## 0.9.0\n\n### Minor Changes\n\n- abc1234: New `Dot`.\n- `Sheet` snaps.\n\n## 0.8.0\n\n- old\n";
    assert.deepEqual(releaseNotesOf(text, "0.9.0"), [
      "New `Dot`.",
      "`Sheet` snaps.",
    ]);
    assert.deepEqual(releaseNotesOf(text, "1.0.0"), []);
  });
});

describe("inTurns", () => {
  it("runs a few at a time and keeps the order", async () => {
    let running = 0;
    let most = 0;
    const results = await inTurns(
      [5, 1, 3, 2],
      async n => {
        most = Math.max(most, ++running);
        await new Promise(resolve => setTimeout(resolve, n));
        running--;
        return n * 10;
      },
      2,
    );
    assert.deepEqual(results, [50, 10, 30, 20]);
    assert.equal(most, 2);
  });
});

const mr = (iid, title, mergedAt, groups, more = {}) => ({
  iid,
  url: `${URL}/merge_requests/${iid}`,
  title,
  issue: null,
  mergedAt,
  summary: `What !${iid} changed.`,
  groups,
  changesets: [],
  orchestration: null,
  ...more,
});

describe("overallText", () => {
  const text = overallText({
    since: "2026-09-01T00:00:00Z",
    merged: [
      mr(10, "feat(vita): offline", "2026-09-28T18:00:00Z", ["products/vita"]),
      mr(
        11,
        "fix(monolithium): a dot",
        "2026-09-30T19:00:00Z",
        ["products/vita", "packages/monolithium"],
        {
          issue: { number: 5, url: `${URL}/issues/5` },
          orchestration: "@monocircuit/breadboard",
          changesets: [
            {
              packages: ["@monocircuit/monolithium"],
              bump: "patch",
              text: "- The dot fits.",
            },
          ],
        },
      ),
      mr(12, "ci: cache pnpm", "2026-09-29T08:00:00Z", ["repo"]),
    ],
    releases: [
      {
        tag: "v1.4.0",
        version: "1.4.0",
        url: `${URL}/tags/v1.4.0`,
        at: "2026-09-30T20:00:00Z",
        groups: ["products/vita"],
        notes: ["Offline reading."],
      },
    ],
  });
  const section = name =>
    text.split(/^(?=## )/m).find(part => part.startsWith(`## ${name}\n`)) ?? "";

  it("has a section per group: products, packages, then repo", () => {
    const headings = [...text.matchAll(/^## (.*)$/gm)].map(m => m[1]);
    assert.deepEqual(headings, ["vita", "monolithium", "repo"]);
    assert.match(text, /^Merged into develop since 2026-09-01/m);
  });

  it("lists an MR in each of its groups, newest first", () => {
    assert.match(section("vita"), /`products\/vita` · 2 merge requests/);
    assert.ok(
      section("vita").indexOf("[!11]") < section("vita").indexOf("[!10]"),
    );
    assert.match(section("monolithium"), /\[!11\]/);
    assert.doesNotMatch(section("monolithium"), /\[!10\]/);
    assert.match(
      section("repo"),
      /#### CI\n\n- \[!12\]\(\S+\) cache pnpm · 08:00/,
    );
  });

  it("shows issue, type, orchestration, time, summary and the changeset of the group", () => {
    assert.match(
      section("monolithium"),
      /#### Fixes\n\n- \[!11\]\(\S+\) \[#5\]\(\S+\) a dot · ⇶ @monocircuit\/breadboard · 19:00\n {2}What !11 changed\.\n {2}changeset \(patch\): The dot fits\./,
    );
    // vita is not the changeset's package.
    assert.doesNotMatch(section("vita"), /changeset/);
  });

  it("marks a release in its group, on its day, before the types", () => {
    assert.match(
      section("vita"),
      /### 2026-09-30\n\n#### Releases\n\n- 🏷 \[v1\.4\.0\]\(\S+\) · 20:00\n {2}Offline reading\.\n\n#### Fixes/,
    );
    assert.doesNotMatch(section("monolithium"), /v1\.4\.0/);
  });

  it("says when nothing was merged", () => {
    assert.match(
      overallText({ since: "2026-09-01T00:00:00Z" }),
      /Nothing merged in this window/,
    );
  });
});

describe("readOverall", () => {
  const name = `overall-test-${process.pid}`;
  const file = path.join(
    mainDir(),
    ".claude",
    "orchestrations",
    `${name}.changelog.md`,
  );
  const merged = [
    {
      iid: 21,
      title: "feat(breadboard): a changelog tab",
      source_branch: "feature/261-changelog",
      merged_at: "2026-09-30T10:00:00Z",
      web_url: `${URL}/merge_requests/21`,
      description: "The main space shows what got into develop.",
    },
    {
      iid: 22,
      title: "fix(vita): an old one",
      source_branch: "fix/7-old",
      merged_at: "2026-08-01T10:00:00Z",
      web_url: `${URL}/merge_requests/22`,
      description: "",
    },
  ];
  const diffs = {
    21: [
      {
        old_path: "packages/workflow/a.mjs",
        new_path: "packages/breadboard/a.mjs",
      },
      { old_path: "package.json", new_path: "package.json" },
      {
        old_path: "products/vita/gone.ts",
        new_path: "products/vita/gone.ts",
        deleted_file: true,
      },
    ],
  };
  const asked = [];
  const gitlab = async endpoint => {
    asked.push(endpoint);
    if (/page=[2-9]/.test(endpoint)) return [];
    if (endpoint.includes("merge_requests?")) return merged;
    const iid = endpoint.match(/merge_requests\/(\d+)\/diffs/)?.[1];
    if (iid) return diffs[iid] ?? [];
    if (endpoint.includes("repository/tags"))
      return [
        { name: "v1.0.0", created_at: "2026-09-30T12:00:00Z" },
        { name: "v0.9.0", created_at: "2026-07-01T12:00:00Z" },
        { name: "nightly", created_at: "2026-09-30T12:00:00Z" },
      ];
    throw new Error(`unexpected ${endpoint}`);
  };
  before(() => {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(
      file,
      `# ⇶ A test: changelog\n\n- [!21](${URL}/merge_requests/21) x\n`,
    );
  });
  after(() => fs.rmSync(file, { force: true }));

  it("reads the window's merged MRs with their groups, issue, orchestration and releases", async () => {
    const read = await readOverall({
      since: "30d",
      gitlab,
      now: new Date("2026-10-01T00:00:00Z"),
    });
    assert.deepEqual(
      read.merged.map(entry => entry.iid),
      [21],
    );
    const [entry] = read.merged;
    assert.deepEqual(entry.groups, [
      "packages/breadboard",
      "repo",
      "products/vita",
    ]);
    assert.equal(entry.issue.number, 261);
    assert.equal(entry.orchestration, "A test");
    assert.equal(entry.summary, "The main space shows what got into develop.");
    assert.deepEqual(
      read.releases.map(release => [release.tag, release.groups, release.url]),
      [["v1.0.0", ["products/vita"], `${URL}/tags/v1.0.0`]],
    );
    assert.ok(read.changes[21].paths.includes("packages/breadboard/a.mjs"));
  });

  it("doesn't read the diffs of an MR it knows", async () => {
    asked.length = 0;
    await readOverall({
      since: "30d",
      gitlab,
      now: new Date("2026-10-01T00:00:00Z"),
      known: { 21: { paths: ["products/vita/a.ts"], changesets: [] } },
    });
    assert.ok(!asked.some(endpoint => endpoint.includes("/diffs")));
  });
});

describe("viewLines", () => {
  const text =
    "# Title\n\nintro\n\n## vita\n\n### 2026-09-30\n\n- one\n\n## monolithium\n\n- two\n";
  const plain = lines =>
    lines.map(({ line }) => line.replace(/\x1b\[[0-9;]*m/g, ""));

  it("marks each section open and knows its heading", () => {
    const lines = viewLines(text, 60);
    assert.deepEqual(
      lines.filter(line => line.isHeading).map(line => line.section),
      ["vita", "monolithium"],
    );
    assert.ok(plain(lines).includes("▾ vita"));
    assert.ok(plain(lines).includes("• one"));
  });

  it("shows only the heading of a folded section", () => {
    const shown = plain(viewLines(text, 60, new Set(["vita"])));
    assert.ok(shown.includes("▸ vita"));
    assert.ok(!shown.includes("• one"));
    assert.ok(shown.includes("• two"));
  });
});
