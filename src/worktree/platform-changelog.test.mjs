import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, before, describe, it } from "node:test";

const {
  areaOfPath,
  areasOf,
  emptyChangelog,
  foldInto,
  issueOfName,
  notesOf,
  parseFragment,
  problemsOf,
  readFragments,
  sectionBody,
  versionsOf,
} = await import("./platform-changelog.mjs");

const URL = "https://gitlab.test/monocircuit/platform";
const AREAS = ["vita", "breadboard", "monolithium", "ci", "repo"];
const fragment = (type, area, text) =>
  `---\ntype: ${type}\narea: ${area}\n---\n\n${text}\n`;

describe("fragments", () => {
  it("reads type, area and text, comments after a value ignored", () => {
    assert.deepEqual(
      parseFragment(
        "---\ntype: added   # added | changed\narea: vita\n---\n\nA sentence.\n",
      ),
      { fields: { type: "added", area: "vita" }, text: "A sentence." },
    );
    assert.equal(parseFragment("A sentence."), null);
  });

  it("takes the issue from the file name", () => {
    assert.equal(issueOfName(".changelog/291-platform-changelog.md"), 291);
    assert.equal(issueOfName(".changelog/291-platform-changelog-2.md"), 291);
    assert.equal(issueOfName(".changelog/fonts.md"), null);
  });

  it("names every problem of a fragment", () => {
    assert.deepEqual(
      problemsOf("x.md", fragment("added", "vita", "Ok."), AREAS),
      [],
    );
    const problems = problemsOf(
      "x.md",
      "---\ntype: feat\narea: nowhere\nscope: vita\n---\n\n",
      AREAS,
    );
    assert.equal(problems.length, 4);
    assert.match(problems[0], /type "feat"/);
    assert.match(problems[1], /area "nowhere"/);
    assert.match(problems[2], /unknown field scope/);
    assert.match(problems[3], /says nothing/);
    assert.match(problemsOf("x.md", "no frontmatter", AREAS)[0], /frontmatter/);
    assert.match(
      problemsOf("x.md", fragment("added", "vita", "### Added\n- x"), AREAS)[0],
      /heading/,
    );
  });

  it("puts a path under its product or package, the pipeline under ci, the rest under repo", () => {
    assert.equal(areaOfPath("products/vita/web/src/app/page.tsx"), "vita");
    assert.equal(
      areaOfPath("packages/monocircuit-keycloak-theme/src/x.ts"),
      "keycloak-theme",
    );
    assert.equal(areaOfPath(".gitlab-ci.yml"), "ci");
    assert.equal(areaOfPath("AGENTS.md"), "repo");
  });
});

describe("sectionBody", () => {
  it("lists the types in Keep a Changelog's order, entries by area, with links", () => {
    const body = sectionBody(
      [
        {
          file: "b.md",
          type: "fixed",
          area: "vita",
          text: "Fonts load.",
          issue: 271,
          mr: null,
        },
        {
          file: "a.md",
          type: "added",
          area: "breadboard",
          text: "A changelog.",
          issue: 291,
          mr: { iid: 300, url: `${URL}/-/merge_requests/300` },
        },
        {
          file: "c.md",
          type: "added",
          area: "vita",
          text: "Groups.\nMove together.",
          issue: null,
          mr: null,
        },
      ],
      AREAS,
      URL,
    );
    assert.equal(
      body,
      [
        "### Added",
        "",
        "- **vita:** Groups.",
        "  Move together.",
        `- **breadboard:** A changelog. ([#291](${URL}/-/issues/291), [!300](${URL}/-/merge_requests/300))`,
        "",
        "### Fixed",
        "",
        `- **vita:** Fonts load. ([#271](${URL}/-/issues/271))`,
      ].join("\n"),
    );
  });
});

describe("foldInto", () => {
  const body = "### Added\n\n- **vita:** Groups.";

  it("starts the history with the first release, its tag as link", () => {
    const text = foldInto(emptyChangelog(URL), {
      version: "1.0.0",
      date: "2026-10-14",
      body,
      url: URL,
    });
    assert.match(
      text,
      /## \[Unreleased\]\n\n## \[1\.0\.0\] - 2026-10-14\n\n### Added\n\n- \*\*vita:\*\* Groups\.\n/,
    );
    assert.match(
      text,
      new RegExp(
        `\\[Unreleased\\]: ${URL}/-/compare/v1\\.0\\.0\\.\\.\\.develop\n\\[1\\.0\\.0\\]: ${URL}/-/tags/v1\\.0\\.0\n$`,
      ),
    );
    assert.deepEqual(versionsOf(text), ["1.0.0"]);
  });

  it("puts the next release on top, compared with the one before, and refuses one twice", () => {
    const first = foldInto(emptyChangelog(URL), {
      version: "1.0.0",
      date: "2026-10-14",
      body,
      url: URL,
    });
    const second = foldInto(first, {
      version: "1.1.0",
      date: "2026-10-20",
      body: "### Fixed\n\n- **ci:** x.",
      url: URL,
    });
    assert.deepEqual(versionsOf(second), ["1.1.0", "1.0.0"]);
    assert.equal(second.match(/^\[Unreleased\]: /gm).length, 1);
    assert.match(
      second,
      new RegExp(
        `\\[1\\.1\\.0\\]: ${URL}/-/compare/v1\\.0\\.0\\.\\.\\.v1\\.1\\.0\n\\[1\\.0\\.0\\]: `,
      ),
    );
    assert.throws(
      () => foldInto(second, { version: "1.1.0", date: "x", body, url: URL }),
      /already/,
    );
    assert.throws(
      () => foldInto(second, { version: "1.2", date: "x", body, url: URL }),
      /X\.Y\.Z/,
    );
  });

  it("keeps what stands under Unreleased by hand", () => {
    const text = emptyChangelog(URL).replace(
      "## [Unreleased]\n",
      "## [Unreleased]\n\nNothing yet.\n",
    );
    const folded = foldInto(text, {
      version: "1.0.0",
      date: "2026-10-14",
      body,
      url: URL,
    });
    assert.match(
      folded,
      /## \[Unreleased\]\n\nNothing yet\.\n\n## \[1\.0\.0\]/,
    );
  });

  it("gives the body of a version for its GitLab release", () => {
    const first = foldInto(emptyChangelog(URL), {
      version: "1.0.0",
      date: "2026-10-14",
      body,
      url: URL,
    });
    const second = foldInto(first, {
      version: "1.1.0",
      date: "2026-10-20",
      body: "### Fixed\n\n- **ci:** x.",
      url: URL,
    });
    assert.equal(notesOf(second, "1.1.0"), "### Fixed\n\n- **ci:** x.");
    assert.equal(notesOf(second, "1.0.0"), body);
    assert.equal(notesOf(second, "2.0.0"), null);
  });
});

describe("readFragments", () => {
  let root;
  before(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "platform-changelog-"));
    for (const dir of [
      "products/vita",
      "packages/breadboard",
      "packages/monocircuit-keycloak-theme",
      ".changelog",
    ])
      fs.mkdirSync(path.join(root, dir), { recursive: true });
    fs.writeFileSync(path.join(root, ".changelog/README.md"), "# How\n");
  });
  after(() => fs.rmSync(root, { recursive: true, force: true }));

  it("knows the areas from the directories", () => {
    assert.deepEqual(areasOf(root), [
      "vita",
      "breadboard",
      "keycloak-theme",
      "ci",
      "repo",
    ]);
  });

  it("reads every fragment but the README, and refuses an invalid one", () => {
    fs.writeFileSync(
      path.join(root, ".changelog/291-x.md"),
      fragment("added", "breadboard", "A changelog."),
    );
    assert.deepEqual(readFragments(root), [
      {
        file: ".changelog/291-x.md",
        type: "added",
        area: "breadboard",
        text: "A changelog.",
        issue: 291,
      },
    ]);
    fs.writeFileSync(
      path.join(root, ".changelog/292-y.md"),
      fragment("new", "breadboard", "x"),
    );
    assert.throws(() => readFragments(root), /292-y\.md: type "new"/);
  });
});
