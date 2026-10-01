import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import { HOVER } from "../lib/mouse.mjs";
import { ESCAPES } from "../lib/tty.mjs";
import { titleHeight } from "./boxes.mjs";
import { EventEmitter } from "node:events";

import { frameUpdate, painter, watchSize } from "./frame.mjs";
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
      if (text.startsWith("\x1b[2J")) rows.length = 0;
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

describe("a pane that changes size (#253)", () => {
  beforeEach(() => {
    screen.scroll = 0;
  });
  /** A pane whose size the test sets, and a view as wide as it, one line per row of 100. */
  const pane = { columns: 40, rows: 10 };
  const wide = () => {
    screen.clicks = [];
    screen.sticky = 1;
    return Array.from({ length: 100 }, (_, i) =>
      `${i}`.padEnd(pane.columns, "-"),
    ).join("\n");
  };
  const sized = () => {
    const screenOf = terminal();
    let pending = null;
    const frame = painter({
      render: wide,
      rows: () => pane.rows,
      size: () => `${pane.columns}x${pane.rows}`,
      write: screenOf.write,
      now: () => 0,
      later: paint => {
        pending = paint;
        return 1;
      },
    });
    const flush = () => {
      const paint = pending;
      pending = null;
      paint?.();
    };
    return { frame, flush, shown: screenOf.rows };
  };

  it("lays the view out at the new width after a resize, wider and narrower", () => {
    pane.columns = 40;
    pane.rows = 10;
    const { frame, flush, shown } = sized();
    frame.changed();
    flush();
    assert.equal(shown[1].length, 40);
    pane.columns = 120;
    frame.repaint();
    flush();
    assert.equal(shown[1].length, 120);
    pane.columns = 30;
    frame.repaint();
    flush();
    assert.equal(shown[1].length, 30);
    assert.equal(shown.length, 10);
  });

  it("lays it out again when the size changed without a resize event", () => {
    pane.columns = 40;
    pane.rows = 10;
    const { frame, flush, shown } = sized();
    frame.changed();
    flush();
    pane.columns = 120;
    scrollBy(1);
    frame.scrolled();
    flush();
    assert.ok(shown.slice(0, 10).every(line => line.length === 120));
  });

  it("keeps the scroll position inside the view when the height changes", () => {
    pane.columns = 40;
    pane.rows = 10;
    const { frame, flush, shown } = sized();
    const lineOf = row => Number(shown[row].split("-")[0]);
    frame.changed();
    flush();
    scrollBy(50);
    frame.scrolled();
    flush();
    pane.rows = 4;
    frame.repaint();
    flush();
    assert.deepEqual([0, 1, 2, 3].map(lineOf), [0, 51, 52, 53]);
    scrollBy(Infinity);
    frame.scrolled();
    flush();
    assert.equal(lineOf(3), 99);
    pane.rows = 10;
    frame.repaint();
    flush();
    // Scrolled to the end, a taller pane moves the window up instead of showing empty rows.
    assert.equal(shown.length, 10);
    assert.equal(lineOf(9), 99);
    assert.equal(screen.scroll, 90);
  });
});

describe("watchSize", () => {
  /** A tty stream whose size the terminal changes behind Node's back. */
  const tty = () => {
    const stream = new EventEmitter();
    stream.isTTY = true;
    stream.columns = 40;
    stream.rows = 10;
    stream.actual = [40, 10];
    stream._refreshSize = () => {
      const [columns, rows] = stream.actual;
      if (columns === stream.columns && rows === stream.rows) return;
      stream.columns = columns;
      stream.rows = rows;
      stream.emit("resize");
    };
    return stream;
  };

  it("emits resize once the terminal's size differs from what Node knows", () => {
    const stream = tty();
    let tick = null;
    watchSize(stream, { repeat: run => (tick = run) });
    let resizes = 0;
    stream.on("resize", () => resizes++);
    tick();
    assert.equal(resizes, 0);
    stream.actual = [120, 30];
    tick();
    assert.equal(resizes, 1);
    assert.equal(stream.columns, 120);
    tick();
    assert.equal(resizes, 1);
  });

  it("asks nothing of a stream that is no terminal", () => {
    const stream = tty();
    stream.isTTY = false;
    assert.equal(
      watchSize(stream, {
        repeat: () => assert.fail("no timer for a pipe"),
      }),
      null,
    );
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

describe("the hover (#277)", () => {
  beforeEach(() => {
    screen.scroll = 0;
  });
  const ISLAND = "\x1b[48;2;38;38;38m";
  /** Island lines, every odd one clickable, line 4 with a button at column 2. */
  const view = () => {
    screen.clicks = [];
    screen.sticky = 1;
    return Array.from({ length: 40 }, (_, i) => {
      if (i % 2) screen.clicks[i] = () => i;
      const body =
        i === 3 ? `\x1b[1;30;46m go \x1b[0m${ISLAND} after` : `l${i}`;
      return `${ISLAND} ${body} \x1b[0m`;
    }).join("\n");
  };

  it("draws the line under the pointer lit, and again when the pointer leaves it", () => {
    const { frame, flush, screen: shown } = setup(view, 10);
    frame.changed();
    flush();
    frame.hoverAt(1, 10);
    flush();
    assert.ok(shown[1].startsWith(HOVER));
    frame.hoverAt(2, 10);
    flush();
    assert.ok(shown[1].startsWith(ISLAND));
    assert.ok(
      shown[2].startsWith(ISLAND),
      "a line a click does nothing on stays",
    );
  });

  it("draws nothing while the pointer moves over the same target", () => {
    const { frame, flush, isPending, renders } = setup(view, 10);
    frame.changed();
    flush();
    frame.hoverAt(3, 12);
    assert.ok(isPending());
    flush();
    for (const column of [13, 14, 20]) {
      frame.hoverAt(3, column);
      assert.ok(!isPending(), `column ${column}`);
    }
    frame.hoverAt(3, 3);
    assert.ok(isPending(), "onto the button");
    flush();
    frame.hoverAt(3, 4);
    assert.ok(!isPending(), "still on the button");
    assert.equal(renders(), 1, "the view itself is never rendered again");
  });

  it("lights only the button under the pointer", () => {
    const { frame, flush, screen: shown } = setup(view, 10);
    frame.changed();
    flush();
    frame.hoverAt(3, 3);
    flush();
    assert.ok(shown[3].includes("\x1b[1;30;106m go "));
    assert.ok(!shown[3].includes(HOVER));
  });

  it("clears on hoverAt(null), the focus gone", () => {
    const { frame, flush, isPending, screen: shown } = setup(view, 10);
    frame.changed();
    flush();
    frame.hoverAt(1, 5);
    flush();
    frame.hoverAt(null);
    assert.ok(isPending());
    flush();
    assert.ok(!shown.some(line => line.includes(HOVER)));
  });

  it("finds the target again after a scroll, under the pointer that stayed", () => {
    const { frame, flush, screen: shown } = setup(view, 10);
    frame.changed();
    flush();
    frame.hoverAt(2, 10);
    flush();
    assert.ok(!shown[2].includes(HOVER));
    scrollBy(1);
    frame.scrolled();
    flush();
    assert.ok(shown[2].startsWith(HOVER), "line 3 is under it now");
  });
});
