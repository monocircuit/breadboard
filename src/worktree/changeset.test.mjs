import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, before, describe, it } from "node:test";

const { combine, readChangeSet } = await import("./changeset.mjs");
const { recordFile } = await import("./origins.mjs");

/** git in the test's repo, its output trimmed; fails the test when git does. */
const gitIn =
  dir =>
  (...args) => {
    const done = spawnSync("git", args, { cwd: dir, encoding: "utf-8" });
    assert.equal(done.status, 0, done.stderr);
    return done.stdout.trim();
  };

describe("combine", () => {
  const group = (iid, files) => ({
    iid,
    base: `base-${iid}`,
    head: `head-${iid}`,
    files,
  });
  const file = (filePath, status, added = 1, removed = 0) => ({
    path: filePath,
    status,
    added,
    removed,
  });

  it("spans a file from the first MR's base to the last one's head, counts summed", () => {
    assert.deepEqual(
      combine([
        group(1, [file("a", "new", 10)]),
        group(2, [file("a", "changed", 2, 1), file("b", "changed")]),
      ]),
      [
        {
          path: "a",
          status: "new",
          added: 12,
          removed: 1,
          base: "base-1",
          head: "head-2",
          mrs: [1, 2],
        },
        {
          path: "b",
          status: "changed",
          added: 1,
          removed: 0,
          base: "base-2",
          head: "head-2",
          mrs: [2],
        },
      ],
    );
  });

  it("marks a file deleted by the last MR, leaves out one that came and went", () => {
    const combined = combine([
      group(1, [file("gone", "changed"), file("brief", "new")]),
      group(2, [file("gone", "deleted"), file("brief", "deleted")]),
    ]);
    assert.deepEqual(
      combined.map(entry => [entry.path, entry.status]),
      [["gone", "deleted"]],
    );
  });

  it("has no counts once a binary file is among them", () => {
    const [entry] = combine([
      group(1, [file("logo.png", "new", null, null)]),
      group(2, [file("logo.png", "changed")]),
    ]);
    assert.equal(entry.added, null);
  });
});

describe("readChangeSet", () => {
  const name = `changeset-test-${process.pid}`;
  let dir;
  let shas;
  before(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), "changeset-"));
    const git = gitIn(dir);
    const write = (file, text) => fs.writeFileSync(path.join(dir, file), text);
    const commit = message => {
      git("add", "-A");
      git("commit", "--quiet", "-m", message);
      return git("rev-parse", "HEAD");
    };
    git("init", "--quiet", "-b", "develop");
    git("config", "user.email", "test@test");
    git("config", "user.name", "test");
    write("a.txt", "a\n");
    write("keep.txt", "keep\n");
    commit("start");
    // MR !11: a new file, a.txt changed; develop gets someone else's change meanwhile.
    git("checkout", "--quiet", "-b", "feature/1-one");
    write("new.txt", "one\ntwo\n");
    write("a.txt", "a\nb\n");
    commit("one");
    git("checkout", "--quiet", "develop");
    write("other.txt", "not ours\n");
    commit("someone else");
    git("merge", "--quiet", "--no-ff", "-m", "merge !11", "feature/1-one");
    const merge = git("rev-parse", "HEAD");
    // MR !12, open: new.txt changed again, keep.txt deleted; develop goes on after it left.
    git("checkout", "--quiet", "-b", "feature/2-two");
    write("new.txt", "one\ntwo\nthree\n");
    fs.rmSync(path.join(dir, "keep.txt"));
    const open = commit("two");
    git("checkout", "--quiet", "develop");
    write("later.txt", "not ours either\n");
    commit("someone else again");
    git("update-ref", "refs/remotes/origin/develop", "develop");
    git("update-ref", "refs/remotes/origin/feature/2-two", "feature/2-two");
    shas = { merge, open };
    fs.mkdirSync(path.dirname(recordFile(name)), { recursive: true });
    fs.writeFileSync(
      recordFile(name),
      JSON.stringify({
        title: "A test",
        issues: [
          { number: 1, branch: "feature/1-one" },
          { number: 2, branch: "feature/2-two" },
          { number: 3, branch: "feature/3-three" },
        ],
      }),
    );
  });
  after(() => {
    fs.rmSync(recordFile(name), { force: true });
    fs.rmSync(dir, { recursive: true, force: true });
  });
  const gitlab = async endpoint => {
    const branch = decodeURIComponent(endpoint.match(/source_branch=(.+)$/)[1]);
    return {
      "feature/1-one": [
        {
          iid: 11,
          state: "merged",
          title: "feat(x): one",
          web_url: "https://gitlab.test/-/merge_requests/11",
          merge_commit_sha: shas.merge,
          merged_at: "2026-09-30T08:00:00Z",
          source_branch: branch,
        },
        // Closed without a merge: it changed nothing.
        { iid: 10, state: "closed", title: "feat(x): one, first try" },
      ],
      "feature/2-two": [
        {
          iid: 12,
          state: "opened",
          title: "Draft: feat(x): two",
          web_url: "https://gitlab.test/-/merge_requests/12",
          sha: shas.open,
          source_branch: branch,
          target_branch: "develop",
        },
      ],
      // Merged, but its commit isn't in this repo.
      "feature/3-three": [
        {
          iid: 13,
          state: "merged",
          title: "fix(x): three",
          merge_commit_sha: "0".repeat(40),
          merged_at: "2026-09-30T07:00:00Z",
        },
      ],
    }[branch];
  };

  it("lists the files of each MR, merged first, without what develop got meanwhile", async () => {
    const set = await readChangeSet(name, {
      gitlab,
      cwd: dir,
      shouldFetch: false,
    });
    assert.equal(set.title, "A test");
    assert.deepEqual(
      set.groups.map(group => [
        group.iid,
        group.state,
        group.issue,
        group.title,
      ]),
      [
        [11, "merged", 1, "feat(x): one"],
        [12, "opened", 2, "feat(x): two"],
      ],
    );
    const filesOf = iid =>
      set.groups
        .find(group => group.iid === iid)
        .files.map(file => [file.path, file.status, file.added, file.removed]);
    assert.deepEqual(filesOf(11), [
      ["a.txt", "changed", 1, 0],
      ["new.txt", "new", 2, 0],
    ]);
    assert.deepEqual(filesOf(12), [
      ["keep.txt", "deleted", 0, 1],
      ["new.txt", "changed", 1, 0],
    ]);
    assert.deepEqual(set.missing, [13]);
  });

  it("combines them: new.txt from before !11 to the head of !12", async () => {
    const set = await readChangeSet(name, {
      gitlab,
      cwd: dir,
      shouldFetch: false,
    });
    const git = gitIn(dir);
    assert.deepEqual(
      set.combined.map(file => [file.path, file.status, file.mrs]),
      [
        ["a.txt", "changed", [11]],
        ["keep.txt", "deleted", [12]],
        ["new.txt", "new", [11, 12]],
      ],
    );
    const added = set.combined.find(file => file.path === "new.txt");
    assert.equal(added.base, git("rev-parse", `${shas.merge}^1`));
    assert.equal(added.head, shas.open);
  });

  it("is null without a record", async () => {
    assert.equal(
      await readChangeSet(`${name}-none`, {
        gitlab,
        cwd: dir,
        shouldFetch: false,
      }),
      null,
    );
  });
});
