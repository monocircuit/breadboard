import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  feedFile,
  feedLineOf,
  firstLine,
  lastAnswerOf,
  noteOf,
  setNote,
  setStopped,
  stoppedOf,
} from "./feed.mjs";

const fileOf = () => feedFile(fs.mkdtempSync(path.join(os.tmpdir(), "feed-")));

describe("feed", () => {
  it("has no note without a file", () => {
    assert.equal(noteOf(fileOf()), null);
  });

  it("keeps the latest note only", () => {
    const file = fileOf();
    setNote(file, "tests green");
    setNote(file, "  MR !12 opened  ");
    assert.equal(noteOf(file).text, "MR !12 opened");
  });

  it("takes the first line with something on it", () => {
    assert.equal(
      firstLine("\n  \n  Pushed stage 1.\nMore."),
      "Pushed stage 1.",
    );
    assert.equal(firstLine(null), "");
  });

  it("shows the newer of note and answer", () => {
    const note = { text: "at the FEED", at: "2026-09-29T10:00:00Z" };
    assert.deepEqual(
      feedLineOf(note, "Done.\nDetails", "2026-09-29T11:00:00Z"),
      { text: "Done.", at: new Date("2026-09-29T11:00:00Z"), source: "answer" },
    );
    assert.equal(
      feedLineOf(note, "Done.", "2026-09-29T09:00:00Z").source,
      "note",
    );
  });

  it("falls back to either, and to nothing", () => {
    const note = { text: "at it", at: "2026-09-29T10:00:00Z" };
    assert.equal(feedLineOf(note, null, null).text, "at it");
    assert.equal(feedLineOf(null, "Hello", null).text, "Hello");
    assert.equal(feedLineOf(null, "  ", null), null);
  });

  it("keeps the note when the Stop hook writes its line", () => {
    const file = fileOf();
    setNote(file, "MR !12 opened");
    setStopped(file, "\nPushed stage 3.\nDetails");
    setStopped(file, "  ");
    assert.equal(noteOf(file).text, "MR !12 opened");
    assert.equal(stoppedOf(file).text, "Pushed stage 3.");
  });

  it("takes the newer of the transcript's answer and the hook's", () => {
    const claude = {
      answer: "Read",
      answerAt: new Date("2026-09-29T10:00:00Z"),
    };
    const stopped = { text: "Stopped", at: "2026-09-29T11:00:00Z" };
    assert.equal(lastAnswerOf(claude, stopped).text, "Stopped");
    assert.equal(
      lastAnswerOf(
        { ...claude, answerAt: new Date("2026-09-29T12:00:00Z") },
        stopped,
      ).text,
      "Read",
    );
    assert.equal(lastAnswerOf(null, stopped).text, "Stopped");
    assert.equal(lastAnswerOf(claude, null).text, "Read");
    assert.equal(lastAnswerOf(null, null), null);
  });
});
