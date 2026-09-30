import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import { ESCAPES } from "../lib/tty.mjs";
import { titleHeight } from "./boxes.mjs";
import { frameUpdate, painter } from "./frame.mjs";
import { renderOrchestration } from "./render-orchestration.mjs";
import { screen } from "./screen.mjs";
import { scrollBy } from "./scroll.mjs";

const plain = text => text.replace(ESCAPES, "");

/** A terminal that knows only what frameUpdate writes: rows by their position. */
const terminal = () => {
  const rows = [];
  return {
    rows,
    write: text => {
      for (const part of text.split(/(?=\x1b\[\d+;1H)/)) {
        const at = part.match(/^\x1b\[(\d+);1H(\x1b\[2K|\x1b\[J)([\s\S]*)$/);
        if (!at) continue;
        const row = Number(at[1]) - 1;
        if (at[2] === "\x1b[J") rows.length = row;
        else rows[row] = at[3];
      }
    },
  };
};

/** A painter whose frames come when `flush` says, with a count of renders. */
const setup = (render, rows = 20) => {
  const screenOf = terminal();
  let pending = null;
  let renders = 0;
  const frame = painter({
    render: () => {
      renders++;
      return render();
    },
    rows: () => rows,
    write: screenOf.write,
    now: () => 0,
    later: paint => {
      pending = paint;
      return 1;
    },
  });
  return {
    frame,
    screen: screenOf.rows,
    renders: () => renders,
    isPending: () => pending !== null,
    flush: () => {
      const paint = pending;
      pending = null;
      paint?.();
    },
  };
};

describe("frameUpdate", () => {
  it("writes only the rows that changed, each at its row", () => {
    const out = frameUpdate(["a", "b", "c"], ["a", "B", "c"]);
    assert.equal(out, "\x1b[2;1H\x1b[2KB");
  });

  it("clears what is left below a shorter frame", () => {
    assert.equal(frameUpdate(["a", "b"], ["a"]), "\x1b[2;1H\x1b[J");
  });

  it("keeps every row in place, even after a line too wide for the pane", () => {
    const { rows, write } = terminal();
    write(frameUpdate([], ["x".repeat(500), "second", "third"]));
    assert.deepEqual(rows.slice(1), ["second", "third"]);
  });
});

describe("painter", () => {
  beforeEach(() => {
    screen.scroll = 0;
  });
  const view = count => () => {
    screen.clicks = [];
    screen.sticky = 1;
    const lines = Array.from({ length: count }, (_, i) => {
      screen.clicks[i] = () => i;
      return `line ${i}`;
    });
    return lines.join("\n");
  };

  it("keeps the render while only the window moves", () => {
    const { frame, flush, renders, screen: shown } = setup(view(100), 10);
    frame.changed();
    flush();
    assert.equal(renders(), 1);
    for (let i = 0; i < 5; i++) {
      scrollBy(3);
      frame.scrolled();
      flush();
    }
    assert.equal(renders(), 1);
    assert.equal(shown[1], "line 16");
    frame.changed();
    flush();
    assert.equal(renders(), 2);
  });

  it("adds up the steps of a burst and draws them once", () => {
    const writes = [];
    let pending = [];
    const frame = painter({
      render: view(100),
      rows: () => 10,
      write: text => writes.push(text),
      now: () => 0,
      later: paint => pending.push(paint),
    });
    frame.changed();
    for (let i = 0; i < 10; i++) {
      scrollBy(3);
      frame.scrolled();
    }
    assert.equal(pending.length, 1);
    pending[0]();
    pending = [];
    assert.equal(writes.length, 1);
    assert.equal(screen.scroll, 30);
  });

  it("resolves a click against the frame on screen, not a scroll still to draw", () => {
    const { frame, flush } = setup(view(100), 10);
    frame.changed();
    flush();
    scrollBy(9);
    frame.scrolled();
    // Not drawn yet: row 3 still shows line 3.
    assert.equal(frame.actionAt(3)(), 3);
    flush();
    assert.equal(frame.actionAt(3)(), 12);
  });
});

describe("clicks in a scrolled orchestration view", () => {
  const long =
    "a title long enough to wrap over more than one line of the worktree box, and then some more words";
  const tree = number => ({
    dir: `/tmp/no-such-worktree/${number}`,
    name: `${number}-feature`,
    branch: `feature/${number}-feature`,
    ahead: 0,
    changed: 0,
    actions: [],
    claude: null,
    files: new Set(),
  });
  const issue = (number, isLive) => ({
    number,
    feature: `${number}-feature`,
    branch: `feature/${number}-feature`,
    title: `${long} (#${number})`,
    tree: isLive ? tree(number) : null,
    removedAt: isLive ? undefined : `2026-09-2${number % 10}T10:00:00Z`,
  });
  const issues = [
    ...[301, 302, 303, 304].map(number => issue(number, true)),
    ...[201, 202, 203, 204, 205].map(number => issue(number, false)),
  ];
  const local = {
    issues,
    trees: issues.map(entry => entry.tree).filter(Boolean),
    plan: { work: [301, 302, 303, 304], merge: [], why: long },
    ownActions: [],
    agents: new Map(),
  };
  const remote = { of: new Map() };
  let text = "";
  const render = () => {
    screen.clicks = [];
    screen.upTarget = null;
    screen.sticky = titleHeight();
    text = renderOrchestration(local, remote, null);
    return text;
  };

  it("hits the action of the line under the pointer on every row, at every scroll position", () => {
    screen.scroll = 0;
    const { frame, flush, screen: shown } = setup(render, 18);
    frame.changed();
    flush();
    const lines = text.split("\n");
    const clicks = screen.clicks;
    assert.ok(screen.sticky > titleHeight(), "NEXT stays on top too");
    assert.ok(lines.length > 30, "the view is taller than the pane");
    for (let offset = 0; offset < lines.length; offset++) {
      screen.scroll = offset;
      frame.scrolled();
      flush();
      shown.forEach((row, at) => {
        // The band: its time line carries the mark, and it has no clicks.
        if (at < titleHeight()) return;
        const index = frame.viewRowAt(at);
        const where = `row ${at} at offset ${offset}`;
        // The row shows that line of the view…
        assert.equal(row, lines[index], where);
        // …and a click on it does what a click on that line does.
        assert.equal(frame.actionAt(at), clicks[index], where);
      });
    }
  });

  it("unfolds done and removed with a click on its line, scrolled down", () => {
    const { frame, flush, screen: shown } = setup(render, 18);
    screen.scroll = Infinity;
    frame.changed();
    flush();
    const row = shown.findIndex(line =>
      plain(line).includes("▸ done and removed (5)"),
    );
    assert.ok(row > 0, "the fold is on screen");
    frame.clickAt(row, 5);
    frame.changed();
    flush();
    assert.match(plain(text), /▾ done and removed \(5\)/);
    assert.match(plain(text), /201-feature/);
  });
});
