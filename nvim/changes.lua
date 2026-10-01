-- What a worktree changes against develop, in nvim (#293). The herdr tab `nvim` of a
-- worktree's space loads it (packages/breadboard/src/worktree/nvim.mjs, editorCommand);
-- any nvim can: :luafile packages/breadboard/nvim/changes.lua.
--
-- The view is the user's own neo-tree and gitsigns (LazyVim has both), only with the
-- branch's base instead of HEAD; nothing is installed, a missing plugin is named:
--
--   left           neo-tree, every file the branch changed marked (git status vs. the base)
--   middle         the file, its changes: signs in the gutter, added lines green, changed ones
--                  blue with the changed words stronger, removed lines in red where they were
--                  (colours.lua: readable on any theme)
--   below          :Changes, the changed files in the quickfix list (+added -removed each),
--                  kept up to date while nvim has the focus; ]q / :cnext walks them
--
--   :ChangesDiff   the file of this buffer side by side with its version in the base
--   :ChangesInline the marked lines and the removed ones on or off (the signs stay)
--   ]h / [h        next / previous change in the file, <leader>ghp one change inline
--                  (LazyVim's keys of gitsigns)
--
-- "Changed" is everything since the branch left develop: commits, uncommitted
-- and untracked files, what the MR will hold. A file new since then is marked in the
-- tree and the list; inside it gitsigns has no old version to mark against.

local root = vim.fn.getcwd()
local TITLE = "changes against develop"

local function git(args)
  local out = vim.fn.systemlist(vim.list_extend({ "git", "-C", root }, args))
  return vim.v.shell_error == 0 and out or nil
end

-- Where the branch left develop; origin's develop, since a worktree's local one lags.
local function base()
  local found = git({ "merge-base", "HEAD", "origin/develop" })
  return found and found[1] or nil
end

local function items()
  local from = base()
  if not from then
    return {}
  end
  local list = {}
  for _, line in ipairs(git({ "diff", "--numstat", from }) or {}) do
    local added, removed, file = line:match("^(%S+)\t(%S+)\t(.+)$")
    if file then
      local isGone = vim.fn.filereadable(root .. "/" .. file) == 0
      local text = added == "-" and "binary" or ("+" .. added .. " -" .. removed)
      table.insert(list, { filename = root .. "/" .. file, lnum = 1, text = isGone and "deleted" or text })
    end
  end
  for _, file in ipairs(git({ "ls-files", "--others", "--exclude-standard" }) or {}) do
    table.insert(list, { filename = root .. "/" .. file, lnum = 1, text = "new, untracked" })
  end
  return list
end

-- The list replaced in place, so an open quickfix window keeps its line.
local function refresh()
  local current = vim.fn.getqflist({ title = 0, idx = 0 })
  local isOurs = current.title == TITLE
  vim.fn.setqflist({}, isOurs and "r" or " ", { title = TITLE, items = items(), idx = isOurs and current.idx or 1 })
end

local here = debug.getinfo(1, "S").source:sub(2):match("(.*/)")
dofile(here .. "colours.lua")

local function plugin(name, module)
  local ok, found = pcall(require, module)
  if not ok then
    vim.notify(("breadboard: %s is not installed, so the changes against develop show without it"):format(name), vim.log.levels.WARN)
  end
  return ok and found or nil
end

vim.api.nvim_create_user_command("Changes", function()
  refresh()
  vim.cmd("botright copen 8")
end, { desc = "The files this worktree changes against develop" })

vim.api.nvim_create_user_command("ChangesDiff", function()
  local gitsigns, from = plugin("gitsigns.nvim", "gitsigns"), base()
  if gitsigns and from then
    gitsigns.diffthis(from)
  end
end, { desc = "This file side by side with its version in develop" })

local isInline = true
local function inline(gitsigns)
  gitsigns.toggle_linehl(isInline)
  gitsigns.toggle_word_diff(isInline)
  gitsigns.toggle_deleted(isInline)
end

vim.api.nvim_create_user_command("ChangesInline", function()
  local gitsigns = plugin("gitsigns.nvim", "gitsigns")
  if gitsigns then
    isInline = not isInline
    inline(gitsigns)
  end
end, { desc = "The changed and removed lines marked in every file, on or off" })

local function hasTree()
  for _, win in ipairs(vim.api.nvim_tabpage_list_wins(0)) do
    if vim.bo[vim.api.nvim_win_get_buf(win)].filetype == "neo-tree" then
      return true
    end
  end
  return false
end

-- The tree on the left, its changed files marked against the base. neo-tree debounces
-- every navigate of the filesystem (100 ms, first and last), so a later one can replace
-- this "show" without a word: once it had time, a missing tree is asked for again, then named.
local function showTree(neotree, from, isRetry)
  local ok, err = pcall(neotree.execute, { action = "show", source = "filesystem", position = "left", dir = root, git_base = from })
  if not ok then
    vim.notify("breadboard: neo-tree didn't open: " .. tostring(err), vim.log.levels.WARN)
    return
  end
  vim.defer_fn(function()
    if hasTree() then
      return
    end
    if isRetry then
      vim.notify("breadboard: neo-tree didn't open, :Neotree show opens it", vim.log.levels.WARN)
    else
      showTree(neotree, from, true)
    end
  end, 1500)
end

-- The tree, the editor and the list, everything against the base.
local function start()
  local from = base()
  if not from then
    vim.notify("breadboard: no merge-base with origin/develop, the changes can't be shown", vim.log.levels.WARN)
  end
  local gitsigns = plugin("gitsigns.nvim", "gitsigns")
  if gitsigns and from then
    gitsigns.change_base(from, true)
    inline(gitsigns)
  end
  local editor = vim.api.nvim_get_current_win()
  vim.cmd("Changes")
  vim.api.nvim_set_current_win(editor)
  -- "show" comes back to the window it was called from, so the editor keeps the focus.
  local neotree = plugin("neo-tree.nvim", "neo-tree.command")
  if neotree then
    showTree(neotree, from, false)
  end
  -- Nothing to read in the empty buffer nvim starts with: the first changed file instead.
  if vim.fn.argc() == 0 and #vim.fn.getqflist() > 0 then
    vim.cmd("silent! cfirst")
  end
end

if vim.v.vim_did_enter == 1 then
  start()
else
  -- LazyVim loads its plugins after VimEnter (VeryLazy); another config has them by VimEnter.
  vim.api.nvim_create_autocmd(package.loaded.lazy and "User" or "VimEnter", {
    pattern = package.loaded.lazy and "VeryLazy" or "*",
    once = true,
    -- Out of the autocmd, so the file opened gets its own events (filetype, gitsigns).
    callback = function()
      vim.schedule(start)
    end,
  })
end

-- This file is read once, from the main checkout (#297): a tab started before an update of
-- it keeps the old view until it restarts. When one of its files changes, the tab says so, once.
local function loadedAt()
  local stamps = {}
  for _, name in ipairs({ "changes.lua", "colours.lua" }) do
    local stat = vim.uv.fs_stat(here .. name)
    table.insert(stamps, stat and stat.mtime.sec or 0)
  end
  return table.concat(stamps, ",")
end
local loaded = loadedAt()
local isStale = false
local function noticeUpdate()
  if not isStale and loadedAt() ~= loaded then
    isStale = true
    vim.notify("breadboard: changes.lua was updated since this tab started; quit nvim (:qa), a click on a file in the overview or the next layout starts it again", vim.log.levels.WARN)
  end
end

vim.o.autoread = true
-- An agent changes files while this runs: the list follows when you come back, and every few seconds.
vim.api.nvim_create_autocmd({ "FocusGained", "BufWritePost" }, { callback = refresh })
vim.api.nvim_create_autocmd("FocusGained", { callback = noticeUpdate })
-- A file it wrote comes back into its buffer (autoread, checktime), and gitsigns marks it again.
vim.fn.timer_start(5000, function()
  if vim.fn.getqflist({ title = 0 }).title == TITLE then
    refresh()
  end
  if vim.fn.getcmdwintype() == "" then
    pcall(vim.cmd, "silent! checktime")
  end
  noticeUpdate()
end, { ["repeat"] = -1 })
