import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  DEPLOY_GRACE_MS,
  decidingDeployOf,
  deployStateOf,
  span,
  tryStateOf,
} from "./try-state.mjs";

const NOW = Date.parse("2026-09-30T12:00:00Z");
const at = ms => new Date(NOW - ms).toISOString();
const MIN = 60_000;
const job = (name, status, finishedAt = null) => ({
  id: name.length,
  name,
  status,
  url: `https://gitlab/jobs/${name}`,
  finishedAt,
});
/** The develop pipeline of a merge commit, started 3 minutes ago. */
const deploy = (jobs, status = "running") => ({
  pipeline: {
    id: 1,
    status,
    url: "https://gitlab/p/1",
    createdAt: at(3 * MIN),
  },
  jobs,
});
const TEST = { kind: "try", text: "boxes", url: "/atlas", on: "test" };
const ORIGIN = "https://test.example";
const merged = d => ({ iid: 7, state: "merged", deploy: d });
const stateOf = (mr, over = {}) =>
  tryStateOf(TEST, { mr, origin: ORIGIN, now: NOW, ...over });

describe("span", () => {
  it("says how long in the largest unit", () => {
    assert.equal(span(40_000), "40 s");
    assert.equal(span(3 * MIN + 5000), "3 min");
    assert.equal(span(125 * MIN), "2 h");
    assert.equal(span(-5), "0 s");
  });
});

describe("deployStateOf", () => {
  it("deploys while GitLab has no pipeline yet", () => {
    assert.equal(deployStateOf(null, NOW).state, "deploying");
    assert.equal(
      deployStateOf({ pipeline: null, jobs: [] }, NOW).state,
      "deploying",
    );
  });

  it("is at the running job, else the first to come; scans don't count", () => {
    const running = deployStateOf(
      deploy([
        job("scan:deps:develop", "running"),
        job("build:develop", "success"),
        job("publish:develop", "running"),
        job("deploy:develop", "created"),
      ]),
      NOW,
    );
    assert.equal(running.state, "deploying");
    assert.equal(running.job.name, "publish:develop");
    assert.equal(running.since, NOW - 3 * MIN);
    assert.equal(
      deployStateOf(
        deploy([
          job("build:develop", "success"),
          job("publish:develop", "pending"),
        ]),
        NOW,
      ).job.name,
      "publish:develop",
    );
  });

  it("is deployed once deploy:develop passed the grace period ago", () => {
    const passed = jobs => deployStateOf(deploy(jobs), NOW);
    const young = passed([
      job("build:develop", "success"),
      job("deploy:develop", "success", at(DEPLOY_GRACE_MS - 1000)),
    ]);
    assert.equal(young.state, "deploying");
    assert.equal(young.isFinal, false);
    const old = passed([
      job("build:develop", "success"),
      job("deploy:develop", "success", at(DEPLOY_GRACE_MS)),
    ]);
    assert.equal(old.state, "deployed");
    assert.equal(old.isFinal, true);
  });

  it("takes publish as the last step of an ended pipeline without a deploy job", () => {
    const published = deploy(
      [
        job("build:develop", "success"),
        job("publish:develop", "success", at(5 * MIN)),
      ],
      "success",
    );
    assert.equal(deployStateOf(published, NOW).state, "deployed");
    assert.equal(deployStateOf(published, NOW).job.name, "publish:develop");
    // Still running: the deploy job may come.
    assert.equal(
      deployStateOf(
        {
          ...published,
          pipeline: { ...published.pipeline, status: "running" },
        },
        NOW,
      ).state,
      "deploying",
    );
  });

  it("fails with the job that failed or was cancelled", () => {
    for (const status of ["failed", "canceled"]) {
      const state = deployStateOf(
        deploy([
          job("build:develop", "success"),
          job("publish:develop", status),
        ]),
        NOW,
      );
      assert.equal(state.state, "failed");
      assert.equal(state.job.url, "https://gitlab/jobs/publish:develop");
      assert.equal(state.isFinal, true);
    }
    // A failed scan doesn't stop the deployment.
    assert.equal(
      deployStateOf(
        deploy([
          job("scan:deps:develop", "failed"),
          job("build:develop", "running"),
        ]),
        NOW,
      ).state,
      "deploying",
    );
  });
});

describe("decidingDeployOf", () => {
  // #191: the merge's own pipeline failed in check:vita, so deploy:develop was skipped.
  const own = deploy(
    [
      job("check:vita", "failed"),
      job("build:develop", "skipped"),
      job("deploy:develop", "skipped"),
    ],
    "failed",
  );
  const later = deploy(
    [job("deploy:develop", "success", at(5 * MIN))],
    "success",
  );

  it("names the check that stopped a pipeline before it deployed", () => {
    assert.equal(deployStateOf(own, NOW).job.name, "check:vita");
  });

  it("lets a later develop pipeline deploy a merge whose own one failed", () => {
    assert.equal(decidingDeployOf(own, later, NOW), later);
    assert.equal(
      stateOf(merged(decidingDeployOf(own, later, NOW))).label,
      "ready (deploy:develop passed 5 min ago)",
    );
    // Without a later one the failure stays.
    assert.equal(decidingDeployOf(own, null, NOW), own);
  });

  it("keeps the merge's own pipeline unless it failed", () => {
    const running = deploy([job("build:develop", "running")]);
    assert.equal(decidingDeployOf(running, later, NOW), running);
  });
});

describe("tryStateOf", () => {
  it("readies a worktree's try once its commits are pushed", () => {
    const worktree = { kind: "try", text: "x", url: "/atlas" };
    assert.deepEqual(tryStateOf(worktree, { tree: { unpushed: 2 } }), {
      isReady: false,
      label: "not pushed",
      tone: "waits",
    });
    assert.equal(tryStateOf(worktree, { tree: { unpushed: 0 } }).isReady, true);
    // The orchestrator's own has no worktree.
    assert.equal(tryStateOf(worktree).label, "ready");
  });

  it("walks a test try from the merge through the deployment to ready", () => {
    assert.equal(stateOf(null).label, "waits for its MR");
    assert.equal(
      stateOf({ iid: 7, state: "opened" }).label,
      "waits for merge !7",
    );
    assert.equal(stateOf(merged(null)).label, "deploying (pipeline)");
    assert.equal(
      stateOf(
        merged(
          deploy([
            job("build:develop", "success"),
            job("publish:develop", "running"),
          ]),
        ),
      ).label,
      "deploying (publish, 3 min)",
    );
    assert.equal(
      stateOf(merged(deploy([job("deploy:develop", "success", at(30_000))])))
        .label,
      "deploying (deploy:develop passed, Coolify starts it, 3 min)",
    );
    const ready = stateOf(
      merged(deploy([job("deploy:develop", "success", at(2 * MIN))])),
    );
    assert.equal(ready.isReady, true);
    assert.equal(ready.label, "ready (deploy:develop passed 2 min ago)");
  });

  it("names the failed job, a click opens it", () => {
    const failed = stateOf(merged(deploy([job("build:develop", "failed")])));
    assert.deepEqual(failed, {
      isReady: false,
      label: "deploy failed (build:develop)",
      tone: "failed",
      url: "https://gitlab/jobs/build:develop",
    });
  });

  it("isn't ready without the test instance's origin or with a closed MR", () => {
    assert.equal(stateOf(merged(null), { origin: null }).tone, "failed");
    assert.equal(stateOf({ iid: 7, state: "closed" }).label, "!7 closed");
  });
});
