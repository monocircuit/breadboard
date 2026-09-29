// Running commands and talking to the user: `run`, `git`, `exec`, `step`,
// `fail`.
import { execFile, spawnSync } from "node:child_process";
import { promisify } from "node:util";

/** The value of `--flag <value>` or `--flag=<value>` on the command line. */
export function flagValue(name) {
  const argv = process.argv.slice(2);
  const at = argv.indexOf(name);
  if (at !== -1)
    return argv[at + 1]?.startsWith("--") ? undefined : argv[at + 1];
  return argv.find(a => a.startsWith(`${name}=`))?.slice(name.length + 1);
}
/** Shown after a failure, e.g. how to clean up a half set up worktree. */
export let hint = "";
export const fail = message => {
  console.error(`\n✗ ${message}${hint ? `\n  ${hint}` : ""}`);
  process.exit(1);
};
export const step = message => console.log(`\n▸ ${message}`);
export const run = (
  command,
  args,
  { cwd, quiet = false, allowFail = false, input } = {},
) => {
  const result = spawnSync(command, args, {
    cwd,
    input,
    encoding: "utf-8",
    stdio: quiet ? "pipe" : "inherit",
  });
  if (result.status !== 0 && !allowFail) {
    fail(
      `${command} ${args.join(" ")} failed${quiet ? `:\n${result.stderr}` : ""}`,
    );
  }
  return { ok: result.status === 0, out: (result.stdout ?? "").trim() };
};
export const git = (args, cwd, input) =>
  run("git", args, { cwd, quiet: true, allowFail: true, input });
export const exec = promisify(execFile);
/** A command's whole result (`status`, `stdout`, `stderr`), quiet, never failing: for reading. */
export const sh = (command, args, cwd) =>
  spawnSync(command, args, { cwd, encoding: "utf-8", stdio: "pipe" });
/** The non-empty lines of a command's output; none for null. */
export const lines = text => (text ?? "").split("\n").filter(Boolean);
export const quote = text => `'${text.replaceAll("'", "'\\''")}'`;

/** What `fail` adds under its message from here on. */
export const setHint = text => {
  hint = text;
};
