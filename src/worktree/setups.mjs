// An issue's start while it runs (#272): wt:new takes minutes, and until the
// brief and the agent are in its worktree, that one looks like it has no
// agent. A handover or a fresh start claims its issue here first, as a file
// per issue next to the record, gone with its process: a second start of the
// same issue is refused, the queue gives the agent no second issue, the
// clean-up keeps the agent's old worktree, WORKTREES says what is set up
// and wt:remove leaves both worktrees alone meanwhile.
import fs from "node:fs";
import path from "node:path";

import { mainDir } from "./worktrees.mjs";

/** The folder of the setups of the orchestration `name`, next to its record. */
export const setupsDir = name =>
  path.join(mainDir(), ".claude", "orchestrations", `${name}.setups`);
const fileOf = (name, number) => path.join(setupsDir(name), `${number}.json`);

/** Whether the process `pid` still runs. */
export const isAlive = pid => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error.code === "EPERM";
  }
};

const readSetup = file => {
  try {
    return JSON.parse(fs.readFileSync(file, "utf-8"));
  } catch {
    return null;
  }
};

/**
 * The setups of `name` whose process still runs: `[{ number, dir, to, from,
 * pid, at }]`, `dir` the worktree it makes, `to` the agent it is for (null:
 * a fresh one), `from` that agent's worktree now. One whose process ended
 * without its release (killed) is dropped.
 */
export const setupsOf = (name, { alive = isAlive } = {}) => {
  let names;
  try {
    names = fs.readdirSync(setupsDir(name));
  } catch {
    return [];
  }
  return names.flatMap(file => {
    const at = path.join(setupsDir(name), file);
    const setup = readSetup(at);
    if (setup && alive(setup.pid)) return [setup];
    fs.rmSync(at, { force: true });
    return [];
  });
};

/**
 * Claims the start of the issue `number` for this process: `{ release }`,
 * or `{ holder }`, the setup running already. Released on exit, too: fail()
 * exits. `setup`: `{ dir, to, from }` (setupsOf).
 */
export const claimSetup = (name, number, setup) => {
  const file = fileOf(name, number);
  fs.mkdirSync(setupsDir(name), { recursive: true });
  const entry = {
    number: Number(number),
    dir: setup.dir,
    to: setup.to ?? null,
    from: setup.from ?? null,
    pid: process.pid,
    at: new Date().toISOString(),
  };
  for (let tries = 0; tries < 2; tries++) {
    try {
      fs.writeFileSync(file, `${JSON.stringify(entry, null, 2)}\n`, {
        flag: "wx",
      });
      const release = () => {
        if (readSetup(file)?.pid === process.pid)
          fs.rmSync(file, { force: true });
        process.off("exit", release);
      };
      process.on("exit", release);
      return { release };
    } catch (error) {
      if (error.code !== "EEXIST") throw error;
      const holder = readSetup(file);
      if (holder && isAlive(holder.pid)) return { holder };
      // Its process ended without a release: the claim is free again.
      fs.rmSync(file, { force: true });
    }
  }
  return { holder: readSetup(file) };
};

/** `a handover to 3-x` / `a fresh start`: what a setup is, for a message. */
export const setupText = setup =>
  `${setup.to ? `a handover of #${setup.number} to ${setup.to}` : `the fresh start of #${setup.number}`} (pid ${setup.pid}, since ${setup.at.slice(11, 16)} UTC)`;

/**
 * The setup that involves the worktree `dir` among those of every
 * orchestration (`setups`): it makes it, or moves its agent out. Undefined
 * for none.
 */
export const setupAt = (setups, dir) =>
  setups.find(setup => setup.dir === dir || setup.from === dir);
