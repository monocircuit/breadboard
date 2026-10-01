import assert from "node:assert/strict";
import { after, describe, it } from "node:test";

import {
  configure,
  glabApi,
  GitlabError,
  progress,
  remoteOf,
  stats,
} from "./glab.mjs";
import {
  errorText,
  freshText,
  waitingText,
} from "../overview/gitlab-status.mjs";

/** A fetch that answers `responses` in turn, and the urls it was asked. */
const fake = responses => {
  const asked = [];
  const fetch = async (url, { headers }) => {
    asked.push({ url, headers });
    const next = responses.shift();
    if (next instanceof Error) throw next;
    return new Response(JSON.stringify(next.body ?? {}), {
      status: next.status ?? 200,
      headers: next.headers ?? {},
    });
  };
  return { asked, fetch };
};

describe("GitLab over HTTP (#305)", () => {
  after(() => configure(null));

  it("finds host and project in origin's url", () => {
    const project = { host: "gitlab.com", project: "monocircuit/platform" };
    assert.deepEqual(
      remoteOf("https://gitlab.com/monocircuit/platform.git"),
      project,
    );
    assert.deepEqual(
      remoteOf(["git", "gitlab.com:monocircuit/platform.git\n"].join("@")),
      project,
    );
  });

  it("asks the API with the token, the project in the path", async () => {
    const { asked, fetch } = fake([{ body: { iid: 7 } }]);
    configure({ fetch, project: "group/project" });
    assert.deepEqual(await glabApi("projects/:fullpath/merge_requests/7"), {
      iid: 7,
    });
    assert.equal(
      asked[0].url,
      "https://gitlab.com/api/v4/projects/group%2Fproject/merge_requests/7",
    );
    assert.equal(asked[0].headers["PRIVATE-TOKEN"], "token");
  });

  it("sends glab's OAuth token as a bearer", async () => {
    const { asked, fetch } = fake([{ body: [] }]);
    configure({ fetch, isOauth: true });
    await glabApi("projects/:fullpath/issues");
    assert.equal(asked[0].headers.Authorization, "Bearer token");
  });

  it("tries a rate limit, a server error and the network again", async () => {
    const { asked, fetch } = fake([
      { status: 429 },
      Object.assign(new TypeError("fetch failed"), {
        cause: { code: "ECONNRESET" },
      }),
      { body: { ok: true } },
    ]);
    configure({ fetch, backoff: [0, 0] });
    assert.deepEqual(await glabApi("projects/:fullpath/issues/1"), {
      ok: true,
    });
    assert.equal(asked.length, 3);
  });

  it("says why when it still fails, and doesn't try a 404 again", async () => {
    const gone = fake([{ status: 404 }]);
    configure({ fetch: gone.fetch, backoff: [0, 0] });
    await assert.rejects(glabApi("projects/:fullpath/issues/9"), error => {
      assert.ok(error instanceof GitlabError);
      assert.equal(error.reason, "HTTP 404");
      return true;
    });
    assert.equal(gone.asked.length, 1);
    const limited = fake([{ status: 429 }, { status: 429 }, { status: 429 }]);
    configure({ fetch: limited.fetch, backoff: [0, 0] });
    await assert.rejects(glabApi("projects/:fullpath/issues/9"), {
      reason: "HTTP 429, rate limit",
    });
    assert.equal(limited.asked.length, 3);
  });

  it("keeps GitLab's word on the user's minute", async () => {
    const { fetch } = fake([
      {
        body: [],
        headers: {
          "RateLimit-Limit": "2000",
          "RateLimit-Observed": "122",
          "RateLimit-Remaining": "1878",
        },
      },
    ]);
    configure({ fetch });
    await glabApi("projects/:fullpath/issues");
    assert.equal(stats.rate.observed, 122);
    assert.equal(stats.rate.limit, 2000);
  });
});

describe("what a view says about its GitLab data (#305)", () => {
  it("counts the read's calls while it waits for them", () => {
    assert.equal(
      waitingText({ asked: 0, answered: 0, phase: null }),
      "GitLab …",
    );
    assert.equal(
      waitingText({ asked: 7, answered: 4, phase: "pipelines" }),
      "GitLab: 4/7 · pipelines",
    );
    assert.equal(typeof progress.onChange, "function");
  });

  it("says when and why a read failed, and how old the data shown is", () => {
    const at = new Date(2026, 9, 1, 14, 58);
    const remote = { at: new Date(2026, 9, 1, 14, 57) };
    assert.equal(
      errorText({ reason: "HTTP 429, rate limit", at }, remote),
      "GitLab didn't answer 14:58 (HTTP 429, rate limit), data from 14:57, tried again",
    );
    assert.equal(errorText(null), null);
  });

  it("names the age of the data and the minute's calls of all clients", () => {
    const remote = { at: new Date(2026, 9, 1, 14, 57) };
    assert.equal(
      freshText(remote, { observed: 122, limit: 2000 }),
      "GitLab 14:57 · 122/2000 a minute",
    );
    assert.equal(freshText(null), null);
  });
});
