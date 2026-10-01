-- What a worktree changes against develop, in nvim. The herdr tab `nvim` of a
-- worktree's space loads it (packages/breadboard/src/worktree.mjs, ensureEditor); any nvim can:
-- :luafile packages/breadboard/nvim/changes.lua. Built-ins only, gitsigns used where there.
--
--   :Changes       the changed files in the quickfix list (+added -removed each),
--                  kept up to date while nvim has the focus; ]q / :cnext walks them
--   :ChangesDiff   the file of this buffer side by side with its version in develop
--   :ChangesInline the changes inline in every file, on (the default) or off: added
--                  lines green, a changed line's old version in red above it, the
--                  changed characters marked in both, removed lines in red where
--                  they were; kept up to date while you type and when an agent writes
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

-- The changes inline, as extmarks of the namespace below; colours from colours.lua.
dofile(debug.getinfo(1, "S").source:sub(2):match("(.*/)") .. "colours.lua")
local ns = vim.api.nvim_create_namespace("breadboard_changes")
local isInline = true
local diff = vim.text and vim.text.diff or vim.diff
local olds = {} -- file -> its lines in the base, per base: { from, lines }

local function old_lines(from, relative)
  local cached = olds[relative]
  if cached and cached.from == from then
    return cached.lines
  end
  -- A file new since develop has no old version: every line is added.
  local lines = git({ "show", from .. ":" .. relative }) or {}
  olds[relative] = { from = from, lines = lines }
  return lines
end

-- Where two lines differ: the bytes between their common start and end.
local function changed_range(old, new)
  local prefix = 0
  local most = math.min(#old, #new)
  while prefix < most and old:byte(prefix + 1) == new:byte(prefix + 1) do
    prefix = prefix + 1
  end
  local suffix = 0
  while suffix < most - prefix and old:byte(#old - suffix) == new:byte(#new - suffix) do
    suffix = suffix + 1
  end
  return prefix, #old - suffix, #new - suffix
end

-- A removed line as a virtual line: red, its removed characters (from..to) stronger.
local function removed(line, from, to)
  if not from or from >= to then
    return { { line, "BreadboardDiffDelete" } }
  end
  return {
    { line:sub(1, from), "BreadboardDiffDelete" },
    { line:sub(from + 1, to), "BreadboardDeleteText" },
    { line:sub(to + 1), "BreadboardDiffDelete" },
  }
end

local function mark(buf)
  if not vim.api.nvim_buf_is_valid(buf) then
    return
  end
  vim.api.nvim_buf_clear_namespace(buf, ns, 0, -1)
  local file = vim.api.nvim_buf_get_name(buf)
  local from = isInline and base()
  if not from or vim.bo[buf].buftype ~= "" or file:sub(1, #root + 1) ~= root .. "/" then
    return
  end
  local old = old_lines(from, file:sub(#root + 2))
  local new = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
  local hunks = diff(table.concat(old, "\n") .. "\n", table.concat(new, "\n") .. "\n", {
    result_type = "indices",
    algorithm = "histogram",
    linematch = 60,
  })
  for _, hunk in ipairs(hunks or {}) do
    local old_start, old_count, new_start, new_count = unpack(hunk)
    local virtual = {}
    for i = 0, old_count - 1 do
      local line = old[old_start + i]
      local paired = i < new_count and new[new_start + i]
      local from_byte, old_to
      if paired then
        from_byte, old_to = changed_range(line, paired)
      end
      table.insert(virtual, removed(line, from_byte, old_to))
    end
    for i = 0, new_count - 1 do
      local row = new_start + i - 1
      local isChanged = i < old_count
      vim.api.nvim_buf_set_extmark(buf, ns, row, 0, {
        line_hl_group = isChanged and "BreadboardDiffChange" or "BreadboardDiffAdd",
        sign_text = isChanged and "~" or "+",
        sign_hl_group = isChanged and "BreadboardDiffText" or "BreadboardDiffAdd",
        priority = 50,
      })
      if isChanged then
        local line = new[new_start + i]
        local from_byte, _, new_to = changed_range(old[old_start + i], line)
        if from_byte < new_to then
          vim.api.nvim_buf_set_extmark(buf, ns, row, from_byte, { end_col = new_to, hl_group = "BreadboardDiffText", priority = 60 })
        end
      end
    end
    if #virtual > 0 then
      -- Above the hunk's first new line; lines only removed go below the line before them
      -- (above the first line when they were at the top).
      local isAtTop = new_count == 0 and new_start == 0
      vim.api.nvim_buf_set_extmark(buf, ns, math.max(new_start - 1, 0), 0, {
        virt_lines = virtual,
        virt_lines_above = new_count > 0 or isAtTop,
      })
    end
  end
end

local timers = {}
local function mark_soon(buf)
  if timers[buf] then
    timers[buf]:stop()
  end
  timers[buf] = vim.defer_fn(function()
    timers[buf] = nil
    mark(buf)
  end, 150)
end

vim.api.nvim_create_user_command("ChangesInline", function()
  isInline = not isInline
  for _, buf in ipairs(vim.api.nvim_list_bufs()) do
    if vim.api.nvim_buf_is_loaded(buf) then
      mark(buf)
    end
  end
end, { desc = "The changes against develop inline in every file, on or off" })

vim.o.autoread = true
vim.api.nvim_create_autocmd({ "BufReadPost", "BufWritePost", "FileChangedShellPost" }, {
  callback = function(args)
    mark(args.buf)
  end,
})
vim.api.nvim_create_autocmd({ "TextChanged", "TextChangedI" }, {
  callback = function(args)
    mark_soon(args.buf)
  end,
})

-- An agent changes files while this runs: the list follows when you come back, and every few seconds.
vim.api.nvim_create_autocmd({ "FocusGained", "BufWritePost" }, { callback = refresh })
-- A file it wrote comes back into its buffer (autoread, checktime), and with it the marks.
vim.fn.timer_start(5000, function()
  if vim.fn.getqflist({ title = 0 }).title == TITLE then
    refresh()
  end
  if vim.fn.getcmdwintype() == "" then
    pcall(vim.cmd, "silent! checktime")
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
