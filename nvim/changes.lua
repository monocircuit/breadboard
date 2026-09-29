-- What a worktree changes against develop, in nvim. The herdr tab `nvim` of a
-- worktree's space loads it (packages/workflow/src/worktree.mjs, ensureEditor); any nvim can:
-- :luafile packages/workflow/nvim/changes.lua. Built-ins only, gitsigns used where there.
--
--   :Changes       the changed files in the quickfix list (+added -removed each),
--                  kept up to date while nvim has the focus; ]q / :cnext walks them
--   :ChangesDiff   the file of this buffer side by side with its version in develop
--
-- "Changed" is everything since the branch left develop: commits, uncommitted
-- and untracked files, what the MR will hold.

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

vim.api.nvim_create_user_command("Changes", function()
  refresh()
  vim.cmd("botright copen")
end, { desc = "The files this worktree changes against develop" })

vim.api.nvim_create_user_command("ChangesDiff", function()
  local from = base()
  local file = vim.fn.expand("%:p")
  if not from or file == "" then
    return
  end
  local relative = file:sub(#root + 2)
  local old = git({ "show", from .. ":" .. relative }) or {}
  vim.cmd("diffthis")
  vim.cmd("leftabove vnew")
  vim.api.nvim_buf_set_lines(0, 0, -1, false, old)
  vim.bo.buftype, vim.bo.bufhidden, vim.bo.modifiable = "nofile", "wipe", false
  vim.bo.filetype = vim.filetype.match({ filename = file }) or ""
  vim.api.nvim_buf_set_name(0, "develop:" .. relative)
  vim.cmd("diffthis")
end, { desc = "This file side by side with its version in develop" })

-- An agent changes files while this runs: the list follows when you come back, and every few seconds.
vim.api.nvim_create_autocmd({ "FocusGained", "BufWritePost" }, { callback = refresh })
vim.fn.timer_start(5000, function()
  if vim.fn.getqflist({ title = 0 }).title == TITLE then
    refresh()
  end
end, { ["repeat"] = -1 })

-- gitsigns, where installed: the signs mark what changed since develop, not only since the last commit.
vim.api.nvim_create_autocmd("User", {
  pattern = "VeryLazy",
  once = true,
  callback = function()
    local ok, gitsigns = pcall(require, "gitsigns")
    local from = base()
    if ok and from then
      gitsigns.change_base(from, true)
    end
  end,
})

vim.cmd("Changes")
