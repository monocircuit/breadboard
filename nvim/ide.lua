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
  vim.keymap.set("n", "<leader>aa", "<cmd>ClaudeCodeDiffAccept<cr>", { desc = "Accept claude's edit" })
  vim.keymap.set("n", "<leader>ad", "<cmd>ClaudeCodeDiffDeny<cr>", { desc = "Reject claude's edit" })
  vim.keymap.set("v", "<leader>as", "<cmd>ClaudeCodeSend<cr>", { desc = "Selection to claude" })
end
