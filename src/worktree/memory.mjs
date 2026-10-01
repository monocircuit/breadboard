// The machine is the only limit on an orchestration's agents (#229): before a
// fresh agent starts, wt:orchestrate checks the free memory. Below the
// threshold it starts anyway, on the shared stack, and says so; the overview
// names the threshold (the head line of WORKTREES).
import fs from "node:fs";
import os from "node:os";

const GIB = 1024 ** 3;
/** What a worktree's own dev stack takes, and what a claude does. */
export const MEMORY = { stack: 1 * GIB, agent: 0.5 * GIB };
/** Below this much free memory a fresh agent gets the shared stack: its own would not fit next to it. */
export const THRESHOLD = MEMORY.stack + MEMORY.agent;

/** `3.2 GB`. */
export const gb = bytes => `${(bytes / GIB).toFixed(1)} GB`;

/**
 * The memory free for a new process, in bytes: `MemAvailable` of Linux (it
 * counts the caches that give way), else what node says is free.
 */
export const freeMemory = (meminfo = "/proc/meminfo") => {
  try {
    const kb = fs
      .readFileSync(meminfo, "utf-8")
      .match(/^MemAvailable:\s+(\d+) kB/m)?.[1];
    if (kb) return Number(kb) * 1024;
  } catch {
    // Not Linux.
  }
  return os.freemem();
};

/**
 * The stack of the next fresh agent: `{ isShared, why }`. `free` the memory
 * free now, `reserved` what the agents started just before take once their
 * stacks are up. Asked for shared (`--shared`) it is shared anyway, `why` null.
 */
export const stackFor = ({ isShared = false, free, reserved = 0 }) => {
  if (isShared) return { isShared: true, why: null };
  const left = free - reserved;
  return left < THRESHOLD
    ? {
        isShared: true,
        why: `${gb(Math.max(0, left))} free, below ${gb(THRESHOLD)} (own stack ~${gb(MEMORY.stack)} + agent ~${gb(MEMORY.agent)}): on the shared stack`,
      }
    : { isShared: false, why: null };
};
/** What a fresh agent takes, with its stack or without. */
export const costOf = ({ isShared }) =>
  MEMORY.agent + (isShared ? 0 : MEMORY.stack);
