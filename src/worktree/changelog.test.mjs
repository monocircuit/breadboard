import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { after, before, describe, it } from "node:test";

// Days and times of the changelog are local: the same everywhere in this test.
process.env.TZ = "UTC";

const {
  changelogFile,
  changelogText,
  groupOf,
  isChangelogBehind,
  summaryOf,
  writeChangelog,
} = await import("./changelog.mjs");
const { renderMarkdown } = await import("./changelog-view.mjs");
const { endOrchestration } = await import("./end.mjs");
const { recordFile } = await import("./origins.mjs");

const URL = "https://gitlab.test/monocircuit/platform/-";
const mr = (iid, title, mergedAt, number = null) => ({
  iid,
  url: `${URL}/merge_requests/${iid}`,
  title,
  issue: number ? { number, url: `${URL}/issues/${number}` } : null,
  mergedAt,
  summary: `What !${iid} changed.`,
});

describe("groupOf", () => {
  it("takes the Conventional Commit type as the group, the rest as the text", () => {
    assert.deepEqual(groupOf("feat(breadboard): a changelog tab"), {
      group: "Features",
      text: "a changelog tab",
    });
    assert.deepEqual(groupOf("fix!: no more double asks"), {
      group: "Fixes",
      text: "no more double asks",
    });
  });

  it("puts a title without a known type under Other", () => {
    assert.equal(groupOf("Update the readme").group, "Other");
    assert.equal(groupOf("wip(x): something").group, "Other");
  });
});

describe("summaryOf", () => {
  it("takes the first two sentences of the first paragraph after the title", () => {
    const description =
      "feat(x): title\n\nOne thing changed, e.g. `a.md` moved. Another did. A third one too.\n\n## Changes\n\n- more";
    assert.equal(
      summaryOf(description, "feat(x): title"),
      "One thing changed, e.g. `a.md` moved. Another did.",
    );
  });

  it("prefers a Summary section, links as their text, Closes lines left out", () => {
    const description =
      "Closes #12\n\n## Summary\n\nSee [the issue](https://x.test) for it.\n\n## Test plan\n\n- [ ] run";
    assert.equal(summaryOf(description), "See the issue for it.");
  });

  it("is empty without a description", () => {
    assert.equal(summaryOf(null), "");
  });
});

describe("changelogText", () => {
  const text = changelogText({
    title: "@monocircuit/breadboard",
    merged: [
      mr(249, "feat(breadboard): an own title", "2026-09-29T15:27:00Z", 234),
      mr(254, "fix(breadboard): no double asks", "2026-09-30T20:03:00Z", 238),
      mr(253, "feat(breadboard): ACTIONS text", "2026-09-30T19:08:00Z", 235),
      mr(260, "feat(breadboard): a later one", "2026-09-30T21:00:00Z"),
    ],
    closed: [
      {
        number: 250,
        url: `${URL}/issues/250`,
        title: "a duplicate",
        closedAt: "2026-09-30T10:00:00Z",
      },
    ],
  });

  it("lists every merged MR with its issue as links, its time and summary", () => {
    assert.match(
      text,
      /^- \[!253\]\(.*\/merge_requests\/253\) \[#235\]\(.*\/issues\/235\) ACTIONS text · 19:08\n {2}What !253 changed\.$/m,
    );
    assert.match(text, /^- \[!260\]\(\S+\) a later one · 21:00$/m);
  });

  it("groups by day, newest first, and by type in each day", () => {
    const headings = text.match(/^#{2,3} .*$/gm);
    assert.deepEqual(headings, [
      "## 2026-09-30",
      "### Features",
      "### Fixes",
      "## 2026-09-29",
      "### Features",
      "## Closed without a merge request",
    ]);
    // In a group the newest merge first.
    assert.ok(text.indexOf("[!260]") < text.indexOf("[!253]"));
  });

  it("lists the issues closed without an MR separately", () => {
    assert.match(
      text,
      /## Closed without a merge request\n\n- \[#250\]\(\S+\) a duplicate · 2026-09-30 10:00/,
    );
  });

  it("says so when nothing is merged yet", () => {
    assert.match(changelogText({ title: "x" }), /Nothing merged yet/);
  });

  it("renders for the terminal: headings, items, links as their text", () => {
    const lines = renderMarkdown(text, 60);
    assert.equal(lines[0], "⇶ @monocircuit/breadboard: changelog");
    assert.ok(lines.includes("2026-09-30"));
    assert.ok(
      lines.some(line => line.startsWith("• !254 #238 no double asks")),
    );
    assert.ok(lines.every(line => [...line].length <= 60));
  });
});

describe("writeChangelog", () => {
  const name = `changelog-test-${process.pid}`;
  const saved = process.env.HERDR_ENV;
  const merged = {
    "feature/1-one": {
      iid: 11,
      title: "feat(x): the first one",
      merged_at: "2026-09-30T08:00:00Z",
      web_url: `${URL}/merge_requests/11`,
      description: "It works now.",
    },
  };
  const issues = {
    1: { iid: 1, state: "closed", web_url: `${URL}/issues/1`, title: "one" },
    2: { iid: 2, state: "opened", web_url: `${URL}/issues/2`, title: "two" },
    3: {
      iid: 3,
      state: "closed",
      web_url: `${URL}/issues/3`,
      title: "three, a duplicate",
      closed_at: "2026-09-30T09:00:00Z",
    },
  };
  const gitlab = async endpoint => {
    const branch = decodeURIComponent(
      endpoint.match(/source_branch=([^&]+)/)?.[1] ?? "",
    );
    if (branch) return merged[branch] ? [merged[branch]] : [];
    return issues[endpoint.match(/issues\/(\d+)$/)[1]];
  };
  before(() => {
    // The end must not look for a herdr space.
    delete process.env.HERDR_ENV;
    fs.mkdirSync(path.dirname(recordFile(name)), { recursive: true });
    fs.writeFileSync(
      recordFile(name),
      JSON.stringify({
        title: "A test",
        issues: [1, 2, 3].map(number => ({
          number,
          feature: `${number}-x`,
          branch: `feature/${number}-${["one", "two", "three"][number - 1]}`,
          why: null,
        })),
      }),
    );
  });
  after(() => {
    if (saved !== undefined) process.env.HERDR_ENV = saved;
    fs.rmSync(recordFile(name), { force: true });
    fs.rmSync(changelogFile(name), { force: true });
  });

  it("rebuilds it from the record and GitLab", async () => {
    fs.writeFileSync(changelogFile(name), "stale\n");
    const text = await writeChangelog(name, gitlab);
    assert.equal(fs.readFileSync(changelogFile(name), "utf-8"), text);
    assert.match(text, /^# ⇶ A test: changelog/);
    assert.match(text, /\[!11\]\(\S+\) \[#1\]\(\S+\) the first one · 08:00/);
    assert.match(text, /It works now\./);
    assert.match(text, /Closed without a merge request\n\n- \[#3\]/);
    assert.doesNotMatch(text, /#2\]/);
    assert.doesNotMatch(text, /stale/);
  });

  it("knows when a merge or a closed issue is missing from it", () => {
    assert.equal(isChangelogBehind(name, { mrs: [11], closed: [3] }), false);
    assert.equal(isChangelogBehind(name, { mrs: [11, 12] }), true);
    assert.equal(isChangelogBehind(name, { closed: [4] }), true);
  });

  it("stays when the orchestration ends, and the end says where", async () => {
    const { done } = endOrchestration(name);
    assert.ok(!fs.existsSync(recordFile(name)));
    assert.ok(fs.existsSync(changelogFile(name)));
    assert.ok(done.includes(`kept the changelog ${changelogFile(name)}`));
    // Without its record a rewrite leaves it as it is.
    assert.equal(await writeChangelog(name, gitlab), null);
    assert.match(fs.readFileSync(changelogFile(name), "utf-8"), /!11/);
  });
});
