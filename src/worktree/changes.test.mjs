// nvim/changes.lua in a headless nvim (#297): no user config, a neo-tree of
// our own on the runtimepath that opens its window as the real one would, or
// loses the first call, or never opens, or fails.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import { CHANGES_LUA } from "./nvim.mjs";

const hasNvim = !spawnSync("nvim", ["--version"]).error;

const git = (args, cwd) => {
  const result = spawnSync("git", args, { cwd, encoding: "utf-8" });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
};
/** A worktree whose branch left origin/develop and changed one file since. */
const worktree = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "changes-"));
  const commit = message =>
    git(
      ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", message],
      dir,
    );
  git(["init", "--quiet"], dir);
  fs.writeFileSync(path.join(dir, "a.txt"), "a\n");
  git(["add", "."], dir);
  commit("a");
  git(["update-ref", "refs/remotes/origin/develop", "HEAD"], dir);
  fs.writeFileSync(path.join(dir, "b.txt"), "b\n");
  git(["add", "."], dir);
  commit("b");
  return dir;
};

// `open`: which calls of execute open the window (1-based), `fail`: it throws.
const neotree = ({ open = [1], fail = false }) => `
local M, calls = {}, 0
function M.execute(args)
  calls = calls + 1
  _G.treeArgs = args
  ${fail ? 'error("broken")' : ""}
  if not vim.tbl_contains(${JSON.stringify(open).replace("[", "{").replace("]", "}")}, calls) then return end
  local back = vim.api.nvim_get_current_win()
  vim.cmd("topleft vnew")
  vim.bo.filetype = "neo-tree"
  vim.api.nvim_set_current_win(back)
end
return M
`;

// Waits for the tree or a word about it (up to 4 s), then writes what the tab ends with.
// `touch`: a file changed on disk after that, nvim in front again.
const harness = (out, touch) => `
local notes = {}
vim.notify = function(text) table.insert(notes, text) end
vim.api.nvim_create_autocmd("VimEnter", { once = true, callback = function()
  vim.defer_fn(function()
    local function windows()
      return vim.tbl_map(function(w) return vim.bo[vim.api.nvim_win_get_buf(w)].filetype end, vim.api.nvim_list_wins())
    end
    local function isSettled()
      return vim.tbl_contains(windows(), "neo-tree") or #vim.tbl_filter(function(n) return n:find("neo-tree", 1, true) end, notes) > 0
    end
    vim.wait(4000, isSettled, 50)
    local touch = ${JSON.stringify(touch)}
    if touch then
      local later = os.time() + 60
      vim.uv.fs_utime(touch, later, later)
      vim.cmd("doautocmd FocusGained")
    end
    vim.fn.writefile({ vim.json.encode({ windows = windows(), notes = notes, args = _G.treeArgs or vim.NIL }) }, ${JSON.stringify(out)})
    vim.cmd("qa!")
  end, 0)
end })
`;

/**
 * The tab started with changes.lua (a copy) in a worktree, with `stub` as
 * neo-tree; `isUpdated`: colours.lua changes on disk while it runs.
 */
const start = (stub, { isUpdated = false } = {}) => {
  const dir = worktree();
  const plugins = fs.mkdtempSync(path.join(os.tmpdir(), "neo-tree-"));
  fs.mkdirSync(path.join(plugins, "lua", "neo-tree"), { recursive: true });
  fs.writeFileSync(
    path.join(plugins, "lua", "neo-tree", "command.lua"),
    neotree(stub),
  );
  const out = path.join(plugins, "out.json");
  const run = path.join(plugins, "harness.lua");
  const lua = path.join(plugins, "nvim");
  fs.cpSync(path.dirname(CHANGES_LUA), lua, { recursive: true });
  const touch = isUpdated ? path.join(lua, "colours.lua") : null;
  fs.writeFileSync(run, harness(out, touch));
  const result = spawnSync(
    "nvim",
    [
      "--headless",
      "-u",
      "NONE",
      "--cmd",
      `set rtp^=${plugins}`,
      "-c",
      `luafile ${path.join(lua, "changes.lua")}`,
      "-c",
      `luafile ${run}`,
    ],
    { cwd: dir, encoding: "utf-8", timeout: 20000 },
  );
  assert.equal(result.status, 0, result.stderr);
  return { dir, ...JSON.parse(fs.readFileSync(out, "utf-8")) };
};
const treeNotes = notes => notes.filter(note => note.includes("neo-tree"));

describe(
  "the tree of a worktree's nvim tab (#297)",
  { skip: !hasNvim && "no nvim on the PATH" },
  () => {
    it("ends with a neo-tree window, marked against the merge-base", () => {
      const { dir, windows, notes, args } = start({});
      assert.ok(windows.includes("neo-tree"), windows.join());
      assert.equal(args.action, "show");
      assert.equal(args.dir, fs.realpathSync(dir));
      assert.equal(
        args.git_base,
        git(["merge-base", "HEAD", "origin/develop"], dir),
      );
      assert.deepEqual(treeNotes(notes), []);
    });

    it("asks again when the first show is lost", () => {
      const { windows, notes } = start({ open: [2] });
      assert.ok(windows.includes("neo-tree"), windows.join());
      assert.deepEqual(treeNotes(notes), []);
    });

    it("says so when the tree never opens", () => {
      const { windows, notes } = start({ open: [] });
      assert.ok(!windows.includes("neo-tree"));
      assert.match(
        treeNotes(notes).join(),
        /neo-tree didn't open, :Neotree show opens it/,
      );
    });

    it("says why when neo-tree fails", () => {
      const { notes } = start({ fail: true });
      assert.match(treeNotes(notes).join(), /neo-tree didn't open: .*broken/);
    });

    it("says when changes.lua changed since the tab started", () => {
      assert.deepEqual(
        start({}).notes.filter(note => note.includes("updated")),
        [],
      );
      assert.match(
        start({}, { isUpdated: true }).notes.join(),
        /changes\.lua was updated since this tab started; quit nvim \(:qa\)/,
      );
    });
  },
);
