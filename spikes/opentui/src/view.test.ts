// Headless: frames on OpenTUI's test renderer, every button clicked on every
// one of its cells, hover, wrapping and the broken section.

import { expect, test } from "bun:test";
import { type Renderable } from "@opentui/core";
import { createTestRenderer } from "@opentui/core/testing";
import { FIXTURE, parse } from "./gitlab";
import { View, newState } from "./view";

const WIDTHS = [40, 80, 120, 200];
const LABELS = [
  ["[open]", "open"],
  ["[pipeline]", "pipeline"],
  ["[jobs ▾]", "toggleJobs"],
] as const;
const mrs = parse(await Bun.file(FIXTURE).text());

const setup = async (width: number, height: number) => {
  const testing = await createTestRenderer({ width, height });
  const state = newState();
  state.mrs = mrs;
  const view = new View(testing.renderer, state);
  await testing.renderOnce();
  return { ...testing, state, view };
};

const descendants = (node: Renderable): Renderable[] =>
  node.getChildren().flatMap(child => [child, ...descendants(child as Renderable)]);

test("every button cell runs its own action, scrolled through the whole view", async () => {
  let clicked = 0;
  for (const width of WIDTHS) {
    const height = 30;
    const { view, state, renderOnce, mockMouse, captureCharFrame, renderer } = await setup(width, height);
    const viewport = view.body.viewport;
    for (;;) {
      // Black box: the buttons are found in the drawn frame, each of their
      // cells clicked through the mouse, and the action that ran checked.
      const frame = captureCharFrame().split("\n");
      for (let y = viewport.screenY; y < viewport.screenY + viewport.height; y++) {
        const line = [...frame[y]!].join("");
        for (const [label, kind] of LABELS) {
          for (let at = line.indexOf(label); at >= 0; at = line.indexOf(label, at + 1)) {
            const x = [...line.slice(0, at)].length;
            for (let cell = x; cell < x + [...label].length; cell++) {
              const folded = new Set(state.folded);
              const top = view.body.scrollTop;
              state.ran.length = 0;
              await mockMouse.click(cell, y);
              expect({ label, cell, y, ran: state.ran.map(action => action.kind) }).toEqual({ label, cell, y, ran: [kind] });
              state.folded = folded;
              view.update();
              // Unfolding near the end clamped the scroll: put it back once laid out.
              await renderOnce();
              view.body.scrollTop = top;
              await renderOnce();
              clicked++;
            }
          }
        }
      }
      const last = view.body.scrollTop;
      view.body.scrollBy(viewport.height);
      await renderOnce();
      if (view.body.scrollTop === last) break;
    }
    renderer.destroy();
  }
  expect(clicked).toBeGreaterThan(500);
}, 120_000);

test("a job is never cut or split, and no line ends in …", async () => {
  for (const width of WIDTHS) {
    const { captureCharFrame, renderer } = await setup(width, 800);
    const frame = captureCharFrame();
    expect(frame).not.toContain("…");
    for (const mr of mrs)
      for (const job of mr.headPipeline?.jobs.nodes ?? [])
        expect(frame.split("\n").some(line => line.includes(job.name))).toBe(true);
    renderer.destroy();
  }
});

/** The background of the cell at column `x` of a captured line. */
const bgAt = (spans: { width: number; bg: { r: number; g: number; b: number; a: number } }[], x: number) => {
  let at = 0;
  for (const span of spans) {
    if (x < at + span.width) return span.bg;
    at += span.width;
  }
  return spans.at(-1)!.bg;
};

test("hover on any line of a block lights all of its lines and no other", async () => {
  const { view, mockMouse, renderOnce, captureSpans, renderer } = await setup(80, 200);
  const blocks = descendants(view.renderer.root).filter(node => node.id.startsWith("block:"));
  expect(blocks.length).toBeGreaterThan(3);
  for (const block of blocks.slice(0, 5)) {
    for (let dy = 0; dy < block.height; dy++) {
      await mockMouse.moveTo(70, block.screenY + dy);
      await renderOnce();
      const lines = captureSpans().lines;
      for (let y = 2; y < lines.length; y++) {
        const bg = bgAt(lines[y]!.spans, 70);
        const isLit = bg.a > 0 && !(bg.r === 0 && bg.g === 0 && bg.b === 0);
        const isInside = y >= block.screenY && y < block.screenY + block.height;
        expect({ y, isLit }).toEqual({ y, isLit: isInside });
      }
    }
  }
  renderer.destroy();
});

test("the broken section draws its error; MERGE REQUESTS draws all of its MRs", async () => {
  const { view, captureCharFrame, renderer } = await setup(120, 800);
  const frame = captureCharFrame();
  expect(frame).toContain("BROKEN  failed to draw");
  expect(frame).toMatch(/\.branch|'branch'/);
  expect(descendants(view.renderer.root).filter(node => node.id.startsWith("block:")).length).toBe(mrs.length);
  renderer.destroy();
});
