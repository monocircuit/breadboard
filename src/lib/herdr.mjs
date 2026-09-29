// The herdr CLI of the session this runs in, as JSON results. Outside herdr
// (or when herdr is old or unhappy) every call answers null, never throws: the
// layout is comfort, the work goes on without it.
import { spawnSync } from "node:child_process";

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
export const spaces = () => herdr(["workspace", "list"])?.workspaces ?? [];
export const tabsOf = workspace =>
  herdr(["tab", "list", "--workspace", workspace])?.tabs ?? [];
export const panesOf = workspace =>
  herdr(["pane", "list", "--workspace", workspace])?.panes ?? [];
export const currentPane = () =>
  herdr(["pane", "current", "--current"])?.pane ?? null;
