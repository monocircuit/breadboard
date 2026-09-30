import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { reader } from "./reader.mjs";

/** A worker as a data: URL, `body` handling each message `id`. */
const workerOf = body =>
  new URL(
    `data:text/javascript,${encodeURIComponent(`import { parentPort } from "node:worker_threads";
parentPort.on("message", id => { ${body} });`)}`,
  );

describe("reader", () => {
  it("answers each read with what the worker read, Maps and Sets intact", async () => {
    const read = reader(
      workerOf(
        "parentPort.postMessage({ id, local: { id, agents: new Map([[1, 2]]), files: new Set([3]) } });",
      ),
    );
    const [first, second] = await Promise.all([read(), read()]);
    assert.deepEqual([first.id, second.id], [0, 1]);
    assert.equal(first.agents.get(1), 2);
    assert.ok(first.files.has(3));
  });

  it("fails a read whose worker fails, and starts it again on the next", async () => {
    const read = reader(
      workerOf(
        'if (id === 0) process.exit(1); else parentPort.postMessage({ id, local: "read" });',
      ),
    );
    await assert.rejects(read(), /ended/);
    assert.equal(await read(), "read");
  });

  it("passes on the error of a read", async () => {
    const read = reader(
      workerOf('parentPort.postMessage({ id, error: "git failed" });'),
    );
    await assert.rejects(read(), /git failed/);
  });
});
