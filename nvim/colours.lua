-- The colours breadboard's nvim tabs mark changes in (changes.lua, ide.lua):
-- strong enough to see changed lines and characters, since a theme's diff
-- colours can lie close to its background (the matte black one: #202327 on
-- near black). Set again after every theme switch. Built-ins only.
--
--   BreadboardDiffAdd / Change / Text / Delete   for 'winhighlight' over Diff*
--   BreadboardDeleteText                          the removed characters of a changed line

local dark = {
  BreadboardDiffAdd = { bg = "#1f3d27" },
  BreadboardDiffChange = { bg = "#1f2c40" },
  BreadboardDiffText = { bg = "#3a6aa6", bold = true },
  BreadboardDiffDelete = { bg = "#46202a", fg = "#c0838c" },
  BreadboardDeleteText = { bg = "#7a2e3a", fg = "#f0c4ca", bold = true },
}
local light = {
  BreadboardDiffAdd = { bg = "#d4f2d9" },
  BreadboardDiffChange = { bg = "#dde8f7" },
  BreadboardDiffText = { bg = "#9fc0ec", bold = true },
  BreadboardDiffDelete = { bg = "#f6d5d8", fg = "#8a3a46" },
  BreadboardDeleteText = { bg = "#eba3ad", fg = "#5a1a24", bold = true },
}

local function set()
  for name, hl in pairs(vim.o.background == "light" and light or dark) do
    vim.api.nvim_set_hl(0, name, hl)
  end
end

set()
vim.api.nvim_create_autocmd("ColorScheme", {
  group = vim.api.nvim_create_augroup("BreadboardColours", { clear = true }),
  callback = set,
})

return {
  -- For 'winhighlight': the theme's Diff* groups shown in these colours.
  winhighlight = table.concat({
    "DiffAdd:BreadboardDiffAdd",
    "DiffChange:BreadboardDiffChange",
    "DiffText:BreadboardDiffText",
    "DiffDelete:BreadboardDiffDelete",
  }, ","),
}
