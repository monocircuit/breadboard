// The herdr CLI of the session this runs in, as JSON results. Outside herdr
// (or when herdr is old or unhappy) every call answers null, never throws: the
// layout is comfort, the work goes on without it.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";

export const isInHerdr = () => process.env.HERDR_ENV === "1";
/** Claude Code sets it. An agent stays in its tab (EnterWorktree), a human gets a new one. */
export const isAgent = () => Boolean(process.env.CLAUDECODE);
/** `herdr <args>` → its `result`, or null if herdr is missing, old or unhappy. */
export const herdr = args => {
  const result = spawnSync("herdr", args, { encoding: "utf-8", stdio: "pipe" });
  if (result.status !== 0) return null;
  try {
    return JSON.parse(result.stdout).result ?? null;
  } catch {
    return null;
  }
};
/** `herdr <args>` for its effect: true when herdr did it (some answer with no result). */
export const herdrDid = args =>
  spawnSync("herdr", args, { encoding: "utf-8", stdio: "pipe" }).status === 0;
/** The last `lines` lines on the screen of `pane`, as plain text; null when herdr can't read it. */
export const paneText = (pane, lines) => {
  const result = spawnSync(
    "herdr",
    ["pane", "read", pane, "--source", "visible", "--lines", String(lines)],
    { encoding: "utf-8", stdio: "pipe" },
  );
  return result.status === 0 ? result.stdout : null;
};
export const spaces = () => herdr(["workspace", "list"])?.workspaces ?? [];
export const tabsOf = workspace =>
  herdr(["tab", "list", "--workspace", workspace])?.tabs ?? [];
export const panesOf = workspace =>
  herdr(["pane", "list", "--workspace", workspace])?.panes ?? [];
/**
 * The pids from `pid` up to init, `pid` first, from Linux's /proc; only
 * `pid` where there is none. `readStat`: the text of `/proc/<pid>/stat`.
 */
export const ancestorsOf = (
  pid = process.pid,
  readStat = id => fs.readFileSync(`/proc/${id}/stat`, "utf-8"),
) => {
  const chain = [];
  for (let id = pid; id > 1 && chain.length < 64; ) {
    chain.push(id);
    try {
      // `<pid> (<comm>) <state> <ppid> …`: the name may hold spaces and parentheses.
      const stat = readStat(id);
      id = Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
    } catch {
      break;
    }
  }
  return chain;
};
/** The shell of a pane, its pid; null when herdr can't say. */
const shellOf = paneId =>
  herdr(["pane", "process-info", "--pane", paneId])?.process_info?.shell_pid ??
  null;
/**
 * The pane whose shell is an ancestor of this process: `current` (what herdr
 * answers for the inherited HERDR_PANE_ID) when it is one, else the first of
 * `panes()` that is, else null. herdr answers a pane id that is gone with the
 * focused pane, which may be the user's or another agent's (#287).
 */
export const paneOfProcess = ({
  ancestors,
  current,
  panes,
  shellOf: shell,
}) => {
  const chain = new Set(ancestors);
  if (current && chain.has(shell(current.pane_id))) return current;
  return panes().find(pane => chain.has(shell(pane.pane_id))) ?? null;
};
/**
 * The pane running this. Not trusting HERDR_PANE_ID alone: a pane that moved
 * got a new id, and a claude started from its shell later carries the old one.
 * Without /proc (no chain to check) herdr's answer as it is.
 */
export const currentPane = () => {
  const current = herdr(["pane", "current", "--current"])?.pane ?? null;
  const ancestors = ancestorsOf();
  if (ancestors.length < 2) return current;
  return paneOfProcess({
    ancestors,
    current,
    panes: () => herdr(["pane", "list"])?.panes ?? [],
    shellOf,
  });
};
/**
 * A method of herdr's socket API the CLI has no command for (`workspace.move_block`)
 * → its `result`, or null: outside herdr, on an error, or after `timeout` ms.
 */
export const herdrApi = (method, params, { timeout = 2000 } = {}) =>
  new Promise(resolve => {
    const socket = process.env.HERDR_SOCKET_PATH;
    if (!isInHerdr() || !socket) return resolve(null);
    const client = net.createConnection(socket);
    let answer = "";
    const done = result => {
      clearTimeout(timer);
      client.destroy();
      resolve(result);
    };
    const timer = setTimeout(() => done(null), timeout);
    client.on("connect", () =>
      client.write(`${JSON.stringify({ id: "breadboard", method, params })}\n`),
    );
    client.on("data", data => {
      answer += data;
      if (!answer.includes("\n")) return;
      try {
        done(JSON.parse(answer.slice(0, answer.indexOf("\n"))).result ?? null);
      } catch {
        done(null);
      }
    });
    client.on("error", () => done(null));
  });
