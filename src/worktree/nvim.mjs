// The nvim of a checkout, driven from outside (#257): every nvim tab breadboard
// starts listens on a socket in the checkout's `.claude/` (git ignores it),
// and `nvim --server <socket>` opens a file in it — the overview's clicks on
// files.
//
// The socket is named relative to the checkout, and everything that uses it
// runs there: a socket's path holds at most 107 bytes, and
// `<checkout>/.claude/nvim.sock` of a worktree is longer than that.
import { spawnSync } from "node:child_process";
import path from "node:path";

import { quote } from "../lib/proc.mjs";
import { IDE_LUA } from "./ide.mjs";

/** What an nvim tab of breadboard loads: `:Changes`, `:ChangesDiff`. */
export const CHANGES_LUA = path.join(
  import.meta.dirname,
  "..",
  "..",
  "nvim",
  "changes.lua",
);
/**
 * The socket of an nvim tab, relative to its checkout: `nvim.sock` for a
 * worktree's tab `nvim`, `nvim-<name>.sock` for the tab of the orchestration
 * `name`, whose space stands in the main checkout.
 */
export const socketOf = (name = null) =>
  path.join(".claude", name ? `nvim-${name}.sock` : "nvim.sock");
/**
 * The shell line of an nvim tab in `dir`, listening on the socket of `name`
 * (socketOf). A socket nobody answers on is left over (herdr restarted and
 * killed nvim) and would keep `--listen` from starting: it goes first.
 * `commands`: what nvim runs on start (`-c` each), by default `:Changes` and
 * `:ChangesDiff` loaded; the orchestration's tab loads its own.
 * `ide`: the directory of claudecode.nvim (ide.mjs), started too: the
 * worktree's claude connects to this nvim (#258).
 * `file`/`line`: opened at once, when the tab starts for a click. Plain `;`
 * and `||`: the line runs in bash, zsh and fish alike.
 */
export const editorCommand = (
  dir,
  {
    name = null,
    commands = [`luafile ${CHANGES_LUA}`],
    ide = null,
    file,
    line,
  } = {},
) => {
  const socket = quote(socketOf(name));
  const start = [...commands, ...(ide ? [ideCommand(ide)] : [])]
    .map(c => ` -c ${quote(c)}`)
    .join("");
  const open = file ? ` +${line || 1} ${quote(file)}` : "";
  return `cd ${quote(dir)}; nvim --server ${socket} --remote-expr 1 >/dev/null 2>&1 || rm -f ${socket}; nvim --listen ${socket}${start}${open}`;
};
/** The nvim command that starts nvim/ide.lua with the plugin in `plugin`: Lua strings as JSON writes them. */
export const ideCommand = plugin =>
  `lua dofile(${JSON.stringify(IDE_LUA)})(${JSON.stringify(plugin)})`;
const remote = (dir, name, args) =>
  spawnSync("nvim", ["--server", socketOf(name), ...args], {
    cwd: dir,
    encoding: "utf-8",
    stdio: "pipe",
    timeout: 2000,
  });
/** Whether an nvim answers on the socket of `name` in `dir`. */
export const isListening = (dir, name = null) =>
  remote(dir, name, ["--remote-expr", "1"]).status === 0;
/** A string literal of Vim script: `'` doubled inside single quotes. */
const vimString = text => `'${text.replaceAll("'", "''")}'`;
/**
 * The Vim script expression that opens `file` at `line` (its window if one
 * shows it, else in the current one) with the line in the middle.
 */
export const openExpr = (file, line) =>
  `execute('drop +${line || 1} ' .. fnameescape(${vimString(file)}) .. ' | normal! zz')`;
/**
 * `file` (relative to `dir`, or absolute) at `line` in the nvim listening in
 * `dir` on the socket of `name`, out of insert or command-line mode first.
 * True when it answered.
 */
export const openOnSocket = (dir, file, line, name = null) => {
  remote(dir, name, ["--remote-send", "<C-\\><C-N>"]);
  return (
    remote(dir, name, [
      "--remote-expr",
      openExpr(path.resolve(dir, file), line),
    ]).status === 0
  );
};
