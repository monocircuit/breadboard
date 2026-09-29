// The local read of the live overview in a worker thread (#225): its git, fs
// and herdr calls are synchronous and take ~300–700 ms, which would hold up
// the wheel and the keys if they ran in the thread that draws.
import { Worker } from "node:worker_threads";

import { here } from "./args.mjs";

/**
 * A `read()` that runs `readView` (view.mjs) in a worker and resolves to its
 * result. A worker that dies is started again on the next read; the reads
 * waiting on it fail, and the loop keeps the last state.
 */
export const reader = (url = new URL("./read-worker.mjs", import.meta.url)) => {
  let worker = null;
  let next = 0;
  const waiting = new Map();
  const failAll = error => {
    for (const { reject } of waiting.values()) reject(error);
    waiting.clear();
  };
  const start = () => {
    const started = new Worker(url, {
      argv: process.argv.slice(2),
      workerData: { isTTY: Boolean(process.stdout.isTTY), here },
    });
    started.on("message", ({ id, local, error }) => {
      const waiter = waiting.get(id);
      waiting.delete(id);
      if (waiting.size === 0) started.unref();
      if (error) waiter?.reject(new Error(error));
      else waiter?.resolve(local);
    });
    started.on("error", failAll);
    started.on("exit", () => {
      if (worker === started) worker = null;
      failAll(new Error("the read worker ended"));
    });
    return started;
  };
  return () =>
    new Promise((resolve, reject) => {
      worker ??= start();
      // A read keeps the process alive until it is answered, an idle worker
      // never does: the overview ends with process.exit, a test by itself.
      worker.ref();
      const id = next++;
      waiting.set(id, { resolve, reject });
      worker.postMessage(id);
    });
};
