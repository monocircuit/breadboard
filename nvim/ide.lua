-- Claude Code's IDE integration in a worktree's nvim (#258). The herdr tab
-- `nvim` of a worktree's space runs it with the plugin's directory
-- (crates/breadboard/src/cli/lifecycle/layout/nvim.rs): :lua dofile("…/ide.lua")("<plugin>").
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
-- The diff's two windows get the colours of colours.lua: a theme's diff
-- colours can lie close to its background. Other windows keep the theme's.

local colours = dofile(debug.getinfo(1, "S").source:sub(2):match("(.*/)") .. "colours.lua")

-- The plugin shows its proposal in a buffer `<file> (proposed)` next to the
-- file, both in diff mode: once it is in its window, both windows get them.
local function mark_diff()
  vim.schedule(function()
    for _, win in ipairs(vim.api.nvim_tabpage_list_wins(0)) do
      if vim.wo[win].diff then
        vim.wo[win].winhighlight = colours.winhighlight
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
  local group = vim.api.nvim_create_augroup("BreadboardIde", { clear = true })
  vim.api.nvim_create_autocmd("BufWinEnter", { group = group, pattern = "* (proposed)", callback = mark_diff })
  vim.keymap.set("n", "<leader>aa", "<cmd>ClaudeCodeDiffAccept<cr>", { desc = "Accept claude's edit" })
  vim.keymap.set("n", "<leader>ad", "<cmd>ClaudeCodeDiffDeny<cr>", { desc = "Reject claude's edit" })
  vim.keymap.set("v", "<leader>as", "<cmd>ClaudeCodeSend<cr>", { desc = "Selection to claude" })
end
