// One change of the herdr layout at a time, across every process of the user.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { isInHerdr } from "../lib/herdr.mjs";

const LOCK_STALE_MS = 60_000;
let lockDepth = 0;
/**
 * `fn` as one change of the layout at a time. Two at once —
 * `restore`, and the overview it starts again, which builds the layout right
 * away — both see a tab missing and both create it. The lock is a directory
 * (mkdir is atomic); a call inside a locked one (layout → syncStackTabs) just
 * runs.
 */
export const exclusive =
  fn =>
  async (...args) => {
    if (lockDepth > 0 || !isInHerdr()) return fn(...args);
    // One lock for all sessions of the user: the socket path isn't spelled the same everywhere.
    const lock = path.join(
      os.tmpdir(),
      `platform-herdr-layout-${os.userInfo().uid}.lock`,
    );
    for (let waited = 0; ; waited += 100) {
      try {
        fs.mkdirSync(lock);
        break;
      } catch {
        const age =
          Date.now() -
          (fs.statSync(lock, { throwIfNoEntry: false })?.mtimeMs ?? Date.now());
        if (age > LOCK_STALE_MS || waited > LOCK_STALE_MS)
          fs.rmSync(lock, { recursive: true, force: true });
        await new Promise(resolve => setTimeout(resolve, 100));
      }
    }
    lockDepth++;
    try {
      return await fn(...args);
    } finally {
      lockDepth--;
      fs.rmSync(lock, { recursive: true, force: true });
    }
  };

/** The pid in the pid file `file`, 0 when there is none. */
const pidIn = file => {
  try {
    return Number(fs.readFileSync(file, "utf-8")) || 0;
  } catch {
    return 0;
  }
};
/** The pid files this process holds (`holds`), dropped when it exits. */
const held = new Set();
process.once("exit", () => {
  for (const file of held)
    if (pidIn(file) === process.pid) fs.rmSync(file, { force: true });
});
const isAlive = pid => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error.code === "EPERM";
  }
};
/**
 * True when this process holds `file`, a pid file: the first to ask gets it
 * (a create that fails when the file exists is atomic) and keeps it while it
 * runs; one whose process is gone is taken over.
 */
export const holds = file => {
  if (held.has(file)) return true;
  for (let tries = 0; tries < 2; tries++) {
    try {
      fs.writeFileSync(file, String(process.pid), { flag: "wx" });
      held.add(file);
      return true;
    } catch {
      const pid = pidIn(file);
      if (pid && isAlive(pid)) return false;
      fs.rmSync(file, { force: true });
    }
  }
  return false;
};
