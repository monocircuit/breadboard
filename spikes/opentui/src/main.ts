// The MERGE REQUESTS slice of the orchestration overview on OpenTUI (#307).

import { createCliRenderer } from "@opentui/core";
import { FIXTURE, read, type Source, token } from "./gitlab";
import { View, newState } from "./view";

const isFixture = process.argv.includes("--fixture");
const glabToken = isFixture ? null : token();
const source: Source = glabToken ? { token: glabToken } : { fixture: FIXTURE };

const renderer = await createCliRenderer({ exitOnCtrlC: true, useMouse: true, enableMouseMovement: true });
const state = newState();
const view = new View(renderer, state, action => {
  if (action.kind === "open" || action.kind === "pipeline")
    Bun.spawn(["xdg-open", action.url], { stdout: "ignore", stderr: "ignore" });
});

const refresh = async () => {
  if (state.isReading) return;
  state.isReading = true;
  view.update();
  const start = performance.now();
  try {
    state.mrs = await read(source);
    state.error = null;
  } catch (error) {
    state.error = error instanceof Error ? error.message : String(error);
  }
  state.isReading = false;
  state.readTook = Math.round(performance.now() - start);
  state.readAt = Date.now();
  view.update();
};

renderer.keyInput.on("keypress", key => {
  if (key.name === "q" || key.name === "escape") {
    renderer.destroy();
    process.exit(0);
  }
  if (key.name === "r") void refresh();
  if (key.name === "j" || key.name === "down") view.body.scrollBy(1);
  if (key.name === "k" || key.name === "up") view.body.scrollBy(-1);
});

void refresh();
setInterval(refresh, 30_000);
// Only the age in the status line changes.
setInterval(() => {
  view.status.content = view.statusLine();
  renderer.requestRender();
}, 1000);
