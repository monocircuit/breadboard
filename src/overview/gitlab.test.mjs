import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import { gitlabCache } from "./gitlab-cache.mjs";
import { finalGet, readBranches, readMemory } from "./gitlab.mjs";

/** Long ago: a deployment that passed then is final (try-state.mjs). */
const PAST = "2026-01-01T10:00:00Z";

/**
 * A recorded orchestration (#305): `merged` branches whose MR is merged and
 * whose issue is closed, `open` ones with an open MR and issue, its pipeline
 * running. GitLab as a fake that answers each endpoint and writes it down.
 */
const recorded = ({ merged = 40, open = 2 } = {}) => {
  const branches = [];
  const mrs = [];
  const issues = [];
  for (let n = 1; n <= merged + open; n++) {
    const branch = `feature/${n}-thing`;
    const isMerged = n <= merged;
    branches.push(branch);
    mrs.push({
      iid: 1000 + n,
      source_branch: branch,
      state: isMerged ? "merged" : "opened",
      draft: false,
      title: `MR ${n}`,
      sha: `head${n}`,
      web_url: `https://gitlab/mr/${1000 + n}`,
      merge_commit_sha: isMerged ? `merge${n}` : null,
      merged_at: isMerged ? PAST : null,
      head_pipeline: {
        id: 5000 + n,
        status: isMerged ? "success" : "running",
        sha: `head${n}`,
        updated_at: PAST,
      },
    });
    issues.push({
      iid: n,
      title: `Issue ${n}`,
      web_url: `https://gitlab/issues/${n}`,
      state: isMerged ? "closed" : "opened",
      description: "- [ ] a box",
    });
  }
  const pipelines = mrs
    .filter(mr => mr.state === "opened")
    .map(mr => ({ ...mr.head_pipeline, ref: mr.source_branch }));
  const calls = [];
  const json = async endpoint => {
    calls.push(endpoint);
    const url = new URL(`https://gitlab/${endpoint}`);
    const route = url.pathname.replace("/projects/:fullpath/", "");
    const query = url.searchParams;
    if (route === "merge_requests") {
      if (query.get("source_branch"))
        return mrs.filter(
          mr => mr.source_branch === query.get("source_branch"),
        );
      return query.get("state") === "opened"
        ? mrs.filter(mr => mr.state === "opened")
        : // The latest 100 updated: here none of the merged ones.
          mrs.filter(mr => mr.state === "opened");
    }
    if (/^merge_requests\/\d+$/.test(route))
      return mrs.find(mr => `merge_requests/${mr.iid}` === route);
    if (route === "pipelines" && query.get("sha"))
      return [
        {
          id: 9000,
          status: "success",
          web_url: "p",
          created_at: PAST,
          updated_at: PAST,
        },
      ];
    if (route === "pipelines") return pipelines;
    if (/^pipelines\/\d+\/jobs$/.test(route))
      return route === "pipelines/9000/jobs"
        ? [
            {
              id: 1,
              name: "deploy:develop",
              status: "success",
              web_url: "j",
              finished_at: PAST,
            },
          ]
        : [
            {
              id: 2,
              name: "test",
              status: "running",
              web_url: "j",
              finished_at: null,
            },
          ];
    if (route === "issues") {
      const iids = query.getAll("iids[]").map(Number);
      return issues.filter(issue => iids.includes(issue.iid));
    }
    if (/^issues\/\d+\/links$/.test(route)) return [];
    throw new Error(`unexpected ${endpoint}`);
  };
  return { branches, calls, json, text: async () => "" };
};
/** The calls that ask about one of the merged MRs or closed issues of `merged`. */
const aboutFinal = (calls, merged) =>
  calls.filter(call =>
    Array.from({ length: merged }, (_, i) => i + 1).some(
      n =>
        call.includes(`source_branch=feature%2F${n}-thing`) ||
        call.includes(`merge_requests/${1000 + n}`) ||
        call.includes(`sha=merge${n}&`) ||
        call.includes(`pipelines/${5000 + n}/`) ||
        new RegExp(`iids\\[\\]=${n}(&|$)`).test(call) ||
        call.includes(`issues/${n}/`),
    ),
  );

describe("a GitLab read of an orchestration (#305)", () => {
  it("asks about merged MRs and closed issues on the first read only", async () => {
    const gitlab = recorded({ merged: 40, open: 2 });
    const context = {
      json: gitlab.json,
      text: gitlab.text,
      cache: gitlabCache(null),
    };
    const kept = readMemory();
    const first = await readBranches(gitlab.branches, { context, kept });
    assert.ok(aboutFinal(gitlab.calls, 40).length > 0);
    assert.equal(first.length, 42);

    for (const isFull of [false, true]) {
      gitlab.calls.length = 0;
      const again = await readBranches(gitlab.branches, {
        isFull,
        context,
        kept,
      });
      assert.deepEqual(aboutFinal(gitlab.calls, 40), [], `isFull ${isFull}`);
      assert.deepEqual(
        again,
        first.map(([branch, found]) => [
          branch,
          { ...found, at: again[0][1].at },
        ]),
      );
    }
  });

  it("costs a handful of calls a refresh, however many MRs merged", async () => {
    const counts = [];
    for (const merged of [5, 40]) {
      const gitlab = recorded({ merged, open: 2 });
      const context = {
        json: gitlab.json,
        text: gitlab.text,
        cache: gitlabCache(null),
      };
      const kept = readMemory();
      await readBranches(gitlab.branches, { context, kept });
      gitlab.calls.length = 0;
      await readBranches(gitlab.branches, { isFull: false, context, kept });
      counts.push(gitlab.calls.length);
    }
    // Open MRs, pipelines, the open issues, the jobs of the two running pipelines.
    assert.deepEqual(counts, [5, 5]);
  });

  it("reads each MR as the views know it, with its deployment and issue", async () => {
    const gitlab = recorded({ merged: 1, open: 1 });
    const context = {
      json: gitlab.json,
      text: gitlab.text,
      cache: gitlabCache(null),
    };
    const found = new Map(
      await readBranches(gitlab.branches, { context, kept: readMemory() }),
    );
    const merged = found.get("feature/1-thing");
    assert.equal(merged.mr.state, "merged");
    assert.equal(merged.mr.pipeline.status, "success");
    assert.equal(merged.mr.deploy.pipeline.status, "success");
    assert.equal(merged.issue.state, "closed");
    const open = found.get("feature/2-thing");
    assert.equal(open.mr.state, "opened");
    assert.equal(open.mr.pipeline.status, "running");
    assert.deepEqual(
      open.jobs.map(job => job.name),
      ["test"],
    );
    assert.equal(open.issue.boxes.length, 1);
  });

  it("keeps what can't change on disk for the next overview", async () => {
    const file = path.join(
      fs.mkdtempSync(path.join(os.tmpdir(), "gitlab-cache-")),
      "cache.json",
    );
    const gitlab = recorded({ merged: 3, open: 1 });
    await readBranches(gitlab.branches, {
      context: {
        json: gitlab.json,
        text: gitlab.text,
        cache: gitlabCache(file),
      },
      kept: readMemory(),
    });
    gitlab.calls.length = 0;
    // Another process: a new cache from the same file, nothing in memory.
    await readBranches(gitlab.branches, {
      context: {
        json: gitlab.json,
        text: gitlab.text,
        cache: gitlabCache(file),
      },
      kept: readMemory(),
    });
    assert.deepEqual(aboutFinal(gitlab.calls, 3), []);
  });
});

describe("what a rewrite of the changelog asks (#305)", () => {
  it("asks once for a merged MR or a closed issue, every time for an open one", async () => {
    const answers = {
      "mrs?source_branch=a": [{ iid: 1, state: "merged" }],
      "mrs?source_branch=b": [],
      "issues/1": { iid: 1, state: "closed" },
      "issues/2": { iid: 2, state: "opened" },
    };
    const calls = [];
    const get = finalGet({
      json: async endpoint => {
        calls.push(endpoint);
        return answers[endpoint];
      },
      cache: gitlabCache(null),
    });
    for (let round = 0; round < 2; round++)
      for (const endpoint of Object.keys(answers))
        assert.deepEqual(await get(endpoint), answers[endpoint]);
    assert.deepEqual(calls, [
      ...Object.keys(answers),
      "mrs?source_branch=b",
      "issues/2",
    ]);
  });
});
