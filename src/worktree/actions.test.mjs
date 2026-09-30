import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  actionsFile,
  actionsOf,
  addAction,
  answerAction,
  answerLine,
  answeredOf,
  answersOf,
  claimAnswer,
  doneAction,
  releaseAnswer,
  takeAnswers,
} from "./actions.mjs";

const fileOf = () =>
  actionsFile(fs.mkdtempSync(path.join(os.tmpdir(), "actions-")));

describe("actions", () => {
  it("has none without a file", () => {
    assert.deepEqual(actionsOf(fileOf()), []);
  });

  it("numbers new actions after the highest", () => {
    const file = fileOf();
    addAction(file, { kind: "merge", text: "!1 ready" });
    const second = addAction(file, {
      kind: "browser-check",
      text: "pinch-zoom",
      url: "http://localhost:3001",
    });
    assert.equal(second.id, 2);
    assert.deepEqual(
      actionsOf(file).map(action => [action.id, action.kind, action.url]),
      [
        [1, "merge", null],
        [2, "browser-check", "http://localhost:3001"],
      ],
    );
  });

  it("replaces the same kind and text announced again", () => {
    const file = fileOf();
    addAction(file, { kind: "decide", text: "A or B" });
    addAction(file, { kind: "decide", text: "A or B" });
    assert.deepEqual(
      actionsOf(file).map(action => action.id),
      [2],
    );
  });

  it("drops an action that is done, and says when there is none", () => {
    const file = fileOf();
    addAction(file, { kind: "look", text: "the log" });
    assert.equal(doneAction(file, 1), true);
    assert.equal(doneAction(file, 1), false);
    assert.deepEqual(actionsOf(file), []);
  });

  it("keeps what else the file holds: the record of an orchestration", () => {
    const file = fileOf();
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, JSON.stringify({ issues: [{ number: 1 }] }));
    addAction(file, { kind: "decide", text: "merge order" });
    const record = JSON.parse(fs.readFileSync(file, "utf-8"));
    assert.deepEqual(record.issues, [{ number: 1 }]);
    assert.equal(record.actions.length, 1);
  });

  it("offers the options of a decision, fixed answers for the other kinds", () => {
    assert.deepEqual(answersOf({ kind: "decide", options: ["A", "B"] }), [
      "A",
      "B",
    ]);
    assert.deepEqual(answersOf({ kind: "decide" }), []);
    assert.deepEqual(answersOf({ kind: "plan" }), ["ok", "different"]);
    assert.deepEqual(answersOf({ kind: "merge" }), []);
  });

  it("hands an answer over once, as one line", () => {
    const file = fileOf();
    addAction(file, { kind: "decide", text: "A or B", options: ["A", "B"] });
    addAction(file, { kind: "look", text: "the log" });
    assert.deepEqual(takeAnswers(file), []);
    assert.equal(answerAction(file, 1, "B"), true);
    const [taken] = takeAnswers(file);
    assert.equal(answerLine(taken), '[overview] #1 decide "A or B" → B');
    assert.deepEqual(takeAnswers(file), []);
    assert.deepEqual(
      actionsOf(file).map(action => action.id),
      [2],
    );
  });

  it("takes only the answers asked from a pane", () => {
    const file = fileOf();
    addAction(file, { kind: "plan", text: "mine", pane: "w1:p1" });
    addAction(file, { kind: "plan", text: "theirs", pane: "w2:p1" });
    answerAction(file, 1, "ok");
    answerAction(file, 2, "ok");
    assert.deepEqual(
      takeAnswers(file, { pane: "w1:p1" }).map(action => action.id),
      [1],
    );
    assert.equal(actionsOf(file).length, 1);
  });

  it("gives a claimed answer to no one else until it is released", () => {
    const file = fileOf();
    addAction(file, { kind: "decide", text: "A or B", options: ["A", "B"] });
    answerAction(file, 1, "A");
    assert.equal(claimAnswer(file, 1).answer, "A");
    // A second overview and the Stop hook skip it meanwhile.
    assert.equal(claimAnswer(file, 1), null);
    assert.deepEqual(takeAnswers(file), []);
    releaseAnswer(file, 1);
    assert.equal(actionsOf(file)[0].deliveringAt, undefined);
    assert.deepEqual(
      takeAnswers(file).map(action => action.id),
      [1],
    );
  });

  it("frees the claim of a deliverer that died", () => {
    const file = fileOf();
    addAction(file, { kind: "look", text: "the log" });
    answerAction(file, 1, "done");
    const record = JSON.parse(fs.readFileSync(file, "utf-8"));
    record.actions[0].deliveringAt = new Date(
      Date.now() - 60_000,
    ).toISOString();
    fs.writeFileSync(file, JSON.stringify(record));
    assert.notEqual(claimAnswer(file, 1), null);
  });
  it("keeps a delivered answer readable, so it is not asked again", () => {
    const file = fileOf();
    addAction(file, { kind: "decide", text: "A or B", options: ["A", "B"] });
    addAction(file, { kind: "plan", text: "the plan" });
    addAction(file, { kind: "look", text: "the log" });
    answerAction(file, 1, "B");
    answerAction(file, 2, "ok");
    // Typed into the pane by the overview, taken by the Stop hook.
    claimAnswer(file, 1);
    doneAction(file, 1);
    takeAnswers(file);
    // Ticked off without an answer: nothing to remember.
    doneAction(file, 3);
    assert.deepEqual(actionsOf(file), []);
    assert.deepEqual(
      answeredOf(file).map(
        ({ id, kind, answer, answeredAt, deliveredAt, deliveringAt }) => [
          id,
          kind,
          answer,
          typeof answeredAt,
          typeof deliveredAt,
          deliveringAt,
        ],
      ),
      [
        [1, "decide", "B", "string", "string", undefined],
        [2, "plan", "ok", "string", "string", undefined],
      ],
    );
    // Numbered after the delivered ones: #1 and #2 stay theirs.
    assert.equal(addAction(file, { kind: "look", text: "more" }).id, 3);
  });
});
