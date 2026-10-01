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
  draftLine,
  draftWaits,
  isDraft,
  answeredOf,
  answersOf,
  claimAnswer,
  doneAction,
  releaseAnswer,
  takeAnswers,
} from "./actions.mjs";
import { decideProblem, optionsOf, splitOption } from "./actions.mjs";
import { optionsOfArgs } from "./commands/action.mjs";

const fileOf = () =>
  actionsFile(fs.mkdtempSync(path.join(os.tmpdir(), "actions-")));

describe("actions", () => {
  it("has none without a file", () => {
    assert.deepEqual(actionsOf(fileOf()), []);
  });

  it("keeps the MR of the orchestrator's merge", () => {
    const file = fileOf();
    addAction(file, { kind: "merge", text: "#242 rows", mr: 256 });
    addAction(file, { kind: "look", text: "the logo" });
    assert.deepEqual(
      actionsOf(file).map(action => action.mr),
      [256, undefined],
    );
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

  it("leaves a broken or different to the overview, as an unsent draft", () => {
    const file = fileOf();
    addAction(file, { kind: "try", text: "the chip" });
    addAction(file, { kind: "plan", text: "the plan" });
    addAction(file, { kind: "decide", text: "A or B", options: ["broken"] });
    answerAction(file, 1, "broken");
    answerAction(file, 2, "different");
    answerAction(file, 3, "broken");
    const [first, second, third] = actionsOf(file);
    assert.equal(isDraft(first), true);
    assert.equal(isDraft(second), true);
    // A decide's option of that name is sent as it is.
    assert.equal(isDraft(third), false);
    assert.equal(draftLine(first), '[overview] #1 try "the chip" → broken: ');
    // Not by the Stop hook: it would send the bare word.
    assert.deepEqual(
      takeAnswers(file).map(action => action.id),
      [3],
    );
    draftWaits(file, 1, "at a question");
    assert.equal(actionsOf(file)[0].waits, "at a question");
    draftWaits(file, 1, null);
    assert.equal(actionsOf(file)[0].waits, undefined);
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

describe("decideProblem", () => {
  const options = ["keep both lines", "drop the old line"].map(label =>
    splitOption(label),
  );

  it("takes a question, options saying what happens and a recommendation", () => {
    assert.equal(
      decideProblem(
        "Which line stays? The old one is shorter.",
        options,
        options[1].label,
      ),
      null,
    );
    // A short label is fine with what happens next to it.
    assert.equal(
      decideProblem(
        "Skia?",
        [splitOption("Go", "phases 2-4"), splitOption("No-go: pixi stays")],
        "Go",
      ),
      null,
    );
  });

  it("names what is missing", () => {
    assert.match(
      decideProblem("A or B", options, "keep both lines"),
      /question/,
    );
    assert.match(decideProblem("Which?", [options[0]], null), /two options/);
    assert.match(
      decideProblem("Which?", [splitOption("A"), splitOption("B")], "A"),
      /"A", "B"/,
    );
    assert.match(decideProblem("Which?", options, null), /--recommend/);
    assert.match(decideProblem("Which?", options, "C"), /none of the options/);
  });
});

describe("a decide's options", () => {
  it("pairs each --option with the --why after it, else splits it at its first colon", () => {
    assert.deepEqual(
      optionsOfArgs([
        "decide",
        "Skia?",
        "--option",
        "Go",
        "--why",
        "phases 2-4 build the painter",
        "--option=No-go: pixi stays, see wt:plan",
        "--option",
        "later",
        "--recommend",
        "1",
      ]),
      [
        { label: "Go", description: "phases 2-4 build the painter" },
        { label: "No-go", description: "pixi stays, see wt:plan" },
        { label: "later", description: "" },
      ],
    );
  });

  it("shows a stored option split, its answer the whole stored text", () => {
    assert.deepEqual(
      optionsOf({
        options: ["Go", "No-go: pixi stays"],
        descriptions: ["phases 2-4", ""],
      }),
      [
        { answer: "Go", label: "Go", description: "phases 2-4" },
        {
          answer: "No-go: pixi stays",
          label: "No-go",
          description: "pixi stays",
        },
      ],
    );
  });
});
