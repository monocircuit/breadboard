// The worker of reader.mjs: a read per message, answered with its id.
import { parentPort } from "node:worker_threads";

import { readView } from "./view.mjs";

parentPort.on("message", async id => {
  try {
    parentPort.postMessage({ id, local: await readView() });
  } catch (error) {
    parentPort.postMessage({ id, error: String(error?.stack ?? error) });
  }
});
