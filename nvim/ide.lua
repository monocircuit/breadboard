-- Claude Code's IDE integration in a worktree's nvim (#258). The herdr tab
-- `nvim` of a worktree's space runs it with the plugin's directory
-- (packages/breadboard/src/worktree/ide.mjs): :lua dofile("…/ide.lua")("<plugin>").
--
-- coder/claudecode.nvim listens for the worktree's claude (`claude --ide`) and
-- writes ~/.claude/ide/<port>.lock with this nvim's directory, the worktree.
-- claude then sees the open file and the selection, and an edit it asks
-- about opens here as a diff:
--
--   :w  or <leader>aa   accept the edit (your changes to it included)
--   :q  or <leader>ad   reject it
--   <leader>as          the selection (visual) as context for the next question
--
-- In auto mode claude asks about no edit, so no diff opens. claude runs in its
-- own herdr tab, not in nvim: no terminal of the plugin.
--
-- The diff's two windows get colours of their own (BreadboardDiff*), strong
-- enough to see the changed lines and, inside them, the changed characters:
-- a theme's diff colours can lie close to its background. Other windows keep
-- the theme's.

local dark = {
  BreadboardDiffAdd = { bg = "#1f3d27" },
  BreadboardDiffChange = { bg = "#1f2c40" },
  BreadboardDiffText = { bg = "#3a6aa6", bold = true },
  BreadboardDiffDelete = { bg = "#46202a", fg = "#9a5a64" },
}
local light = {
  BreadboardDiffAdd = { bg = "#d4f2d9" },
  BreadboardDiffChange = { bg = "#dde8f7" },
  BreadboardDiffText = { bg = "#9fc0ec", bold = true },
  BreadboardDiffDelete = { bg = "#f6d5d8", fg = "#a05560" },
}

local function colours()
  for name, hl in pairs(vim.o.background == "light" and light or dark) do
    vim.api.nvim_set_hl(0, name, hl)
  end
end

local WINHIGHLIGHT = table.concat({
  "DiffAdd:BreadboardDiffAdd",
  "DiffChange:BreadboardDiffChange",
  "DiffText:BreadboardDiffText",
  "DiffDelete:BreadboardDiffDelete",
}, ",")

-- The plugin shows its proposal in a buffer `<file> (proposed)` next to the
-- file, both in diff mode: once it is in its window, both windows get them.
local function mark_diff()
  vim.schedule(function()
    for _, win in ipairs(vim.api.nvim_tabpage_list_wins(0)) do
      if vim.wo[win].diff then
        vim.wo[win].winhighlight = WINHIGHLIGHT
      end
    end
    vim.notify("claude's edit: :w accepts, :q rejects", vim.log.levels.INFO)
  end)
end

return function(plugin)
  vim.opt.runtimepath:prepend(plugin)
  local ok, claudecode = pcall(require, "claudecode")
  if not ok then
    vim.notify("claudecode.nvim not found in " .. plugin, vim.log.levels.WARN)
    return
  end
  claudecode.setup({
    auto_start = true,
    log_level = "warn",
    track_selection = true,
    terminal = { provider = "none" },
    diff_opts = { layout = "vertical", open_in_new_tab = true },
  })
  colours()
  local group = vim.api.nvim_create_augroup("BreadboardIde", { clear = true })
  vim.api.nvim_create_autocmd("ColorScheme", { group = group, callback = colours })
  vim.api.nvim_create_autocmd("BufWinEnter", { group = group, pattern = "* (proposed)", callback = mark_diff })
  vim.keymap.set("n", "<leader>aa", "<cmd>ClaudeCodeDiffAccept<cr>", { desc = "Accept claude's edit" })
  vim.keymap.set("n", "<leader>ad", "<cmd>ClaudeCodeDiffDeny<cr>", { desc = "Reject claude's edit" })
  vim.keymap.set("v", "<leader>as", "<cmd>ClaudeCodeSend<cr>", { desc = "Selection to claude" })
end
