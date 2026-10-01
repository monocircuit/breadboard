-- What an orchestration changed in the code (#256), read only, in nvim. The tab
-- `changes` of the space ⇶ <title> loads it (packages/breadboard/src/worktree/orchestrator.mjs);
-- any nvim can: :luafile packages/breadboard/nvim/orchestration.lua, then :OrchestrationChanges <name>.
-- Built-ins only.
--
--   :OrchestrationChanges <name>  the files its MRs added, changed, deleted, merged and open,
--                                 grouped per MR (!N #issue title) or as one list (<Tab>)
--                                 with +/- counts; <CR> opens the file at its first change,
--                                 its new lines marked; R reads again, else every minute
--   :OrchestrationDiff            in such a file: side by side with its state before the
--                                 MR (combined: before the orchestration); a new file is
--                                 shown in full; <BS> back to the list
--
-- The list comes from `wt:orchestrate <name> --changes` (changeset.mjs): merged MRs as their
-- merge commit against its first parent, open ones as their branch against where it left
-- develop, so not what develop got meanwhile. In the combined list a file spans the first
-- MR's base to the last one's head; with several open MRs on it, that is the last one's branch.

local here = debug.getinfo(1, "S").source:sub(2)
local ENTRY = vim.fn.fnamemodify(here, ":p:h:h") .. "/src/worktree.mjs"
local REFRESH_MS = 60000

local state = { name = nil, set = nil, isCombined = false, error = nil, isReading = false, at = nil, lines = {} }
local ns = vim.api.nvim_create_namespace("orchestration")

local function hl(name, fallback)
  return vim.fn.hlexists(name) == 1 and name or fallback
end
local STATUS_HL = {
  new = hl("Added", "DiffAdd"),
  changed = hl("Changed", "DiffChange"),
  deleted = hl("Removed", "DiffDelete"),
}

local function git(args)
  local root = state.set and state.set.root or vim.fn.getcwd()
  local out = vim.fn.systemlist(vim.list_extend({ "git", "-C", root, "-c", "core.quotePath=false" }, args))
  return vim.v.shell_error == 0 and out or nil
end

local function counts(file)
  if file.added == vim.NIL or file.added == nil then
    return "binary"
  end
  return ("+%d -%d"):format(file.added, file.removed)
end

local function listBuffer()
  for _, buf in ipairs(vim.api.nvim_list_bufs()) do
    if vim.api.nvim_buf_is_valid(buf) and vim.b[buf].isOrchestrationList then
      return buf
    end
  end
  local buf = vim.api.nvim_create_buf(true, true)
  vim.b[buf].isOrchestrationList = true
  vim.bo[buf].bufhidden = "hide"
  vim.bo[buf].filetype = "orchestration"
  vim.api.nvim_buf_set_name(buf, "changes")
  return buf
end

-- The list drawn into its buffer; `state.lines[n]` is what line n opens.
local function draw()
  local buf = listBuffer()
  local text, marks, targets = {}, {}, {}
  local function add(line, target, mark)
    table.insert(text, line)
    targets[#text] = target
    if mark then
      for _, m in ipairs(mark) do
        table.insert(marks, { #text - 1, m[1], m[2], m[3] })
      end
    end
  end
  local set = state.set
  local title = set and set.title or state.name or "?"
  local view = state.isCombined and "combined (<Tab>: per MR)" or "per MR (<Tab>: combined)"
  local status = state.isReading and "reading…" or (state.at and ("read " .. state.at) or "")
  add(("⇶ %s: what it changed · %s · R reads again · %s"):format(title, view, status), nil, {
    { 0, -1, "Title" },
  })
  if state.error then
    add("")
    for _, line in ipairs(vim.split(state.error, "\n", { trimempty = true })) do
      add(line, nil, { { 0, -1, "ErrorMsg" } })
    end
  end
  local function fileLine(file, group, suffix)
    local status_ = ("%-8s"):format(file.status)
    local count = ("%-12s"):format(counts(file))
    local line = "  " .. status_ .. " " .. count .. " " .. file.path .. (suffix or "")
    local countStart = 2 + #status_ + 1
    add(line, { file = file, group = group }, {
      { 2, 2 + #file.status, STATUS_HL[file.status] },
      { countStart, countStart + #counts(file), "Comment" },
    })
  end
  if set then
    if #set.groups == 0 then
      add("")
      add("No merge request of it changed anything yet.", nil, { { 0, -1, "Comment" } })
    end
    if state.isCombined then
      add("")
      add(("%d files, from %d merge requests"):format(#set.combined, #set.groups), nil, { { 0, -1, "Comment" } })
      for _, file in ipairs(set.combined) do
        local mrs = vim.tbl_map(function(iid)
          return "!" .. iid
        end, file.mrs)
        fileLine(file, nil, "   " .. table.concat(mrs, " "))
      end
    else
      for _, group in ipairs(set.groups) do
        add("")
        local head = ("!%d%s %s"):format(group.iid, group.issue ~= vim.NIL and (" #" .. group.issue) or "", group.title)
        local where = group.state == "merged" and "merged" or "open"
        add(head .. " · " .. where, { group = group }, {
          { 0, #head, "Identifier" },
          { #head, -1, group.state == "merged" and "Comment" or "WarningMsg" },
        })
        for _, file in ipairs(group.files) do
          fileLine(file, group)
        end
      end
    end
    if #set.missing > 0 then
      add("")
      local missing = vim.tbl_map(function(iid)
        return "!" .. iid
      end, set.missing)
      add("Not in git (not fetched, branch gone): " .. table.concat(missing, " "), nil, { { 0, -1, "WarningMsg" } })
    end
  end
  vim.bo[buf].modifiable = true
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, text)
  vim.bo[buf].modifiable = false
  vim.api.nvim_buf_clear_namespace(buf, ns, 0, -1)
  for _, m in ipairs(marks) do
    local row, from, to, group = m[1], m[2], m[3], m[4]
    local length = #text[row + 1]
    vim.api.nvim_buf_set_extmark(buf, ns, row, math.min(from, length), {
      end_col = to < 0 and length or math.min(to, length),
      hl_group = group,
    })
  end
  state.lines = targets
end

local function read()
  if not state.name or state.isReading then
    return
  end
  state.isReading = true
  draw()
  vim.system({ "node", ENTRY, "orchestrate", state.name, "--changes" }, { text = true }, function(done)
    vim.schedule(function()
      state.isReading = false
      if done.code == 0 then
        local ok, set = pcall(vim.json.decode, done.stdout)
        if ok then
          state.set, state.error = set, nil
          state.at = os.date("%H:%M")
        else
          state.error = "could not read the change set: " .. tostring(set)
        end
      else
        state.error = (done.stderr ~= "" and done.stderr or done.stdout)
      end
      draw()
    end)
  end)
end

-- `@@ -a,b +c,d @@` of `git diff -U0`: where the new lines are, and where lines went.
local function hunks(base, head, path)
  local found = {}
  for _, line in ipairs(git({ "diff", "-U0", "--no-renames", base, head, "--", path }) or {}) do
    local start, count = line:match("^@@ %-%d+,?%d* %+(%d+),?(%d*) @@")
    if start then
      table.insert(found, { start = tonumber(start), count = count == "" and 1 or tonumber(count) })
    end
  end
  return found
end

local function scratch(name, lines, path)
  local existing = vim.fn.bufnr(name)
  local buf = existing ~= -1 and existing or vim.api.nvim_create_buf(true, true)
  vim.bo[buf].modifiable = true
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)
  vim.bo[buf].buftype, vim.bo[buf].bufhidden = "nofile", "hide"
  vim.bo[buf].modifiable, vim.bo[buf].readonly = false, true
  vim.bo[buf].filetype = vim.filetype.match({ filename = path, buf = buf }) or ""
  if existing == -1 then
    vim.api.nvim_buf_set_name(buf, name)
  end
  return buf
end

-- A file of the list as its MR (or the orchestration) left it, the new lines marked.
local function openFile(target)
  local file, group = target.file, target.group
  local base = group and group.base or file.base
  local head = group and group.head or file.head
  local isDeleted = file.status == "deleted"
  local lines = git({ "show", (isDeleted and base or head) .. ":" .. file.path }) or { "(binary or not in git)" }
  local where = group and ("!" .. group.iid) or "⇶"
  local buf = scratch(where .. ":" .. file.path, lines, file.path)
  vim.b[buf].orchestration = { base = base, head = head, path = file.path, status = file.status, where = where }
  vim.api.nvim_set_current_buf(buf)
  vim.api.nvim_buf_clear_namespace(buf, ns, 0, -1)
  local first = 1
  if isDeleted then
    for row = 0, #lines - 1 do
      vim.api.nvim_buf_set_extmark(buf, ns, row, 0, { line_hl_group = "DiffDelete", sign_text = "-" })
    end
    vim.notify(file.path .. ": deleted, shown as it was", vim.log.levels.INFO)
  else
    local found = hunks(base, head, file.path)
    for index, hunk in ipairs(found) do
      if index == 1 then
        first = math.max(hunk.start, 1)
      end
      if hunk.count == 0 then
        -- Lines removed after this one.
        local row = math.min(math.max(hunk.start - 1, 0), #lines - 1)
        vim.api.nvim_buf_set_extmark(buf, ns, row, 0, { sign_text = "-", sign_hl_group = "DiffDelete" })
      end
      for row = hunk.start - 1, hunk.start + hunk.count - 2 do
        if row >= 0 and row < #lines then
          vim.api.nvim_buf_set_extmark(buf, ns, row, 0, {
            line_hl_group = "DiffAdd",
            sign_text = "+",
            sign_hl_group = "DiffAdd",
          })
        end
      end
    end
  end
  vim.wo.signcolumn = "yes"
  vim.api.nvim_win_set_cursor(0, { math.min(first, math.max(#lines, 1)), 0 })
  vim.cmd("normal! zz")
  vim.keymap.set("n", "<BS>", function()
    vim.cmd("OrchestrationChanges")
  end, { buffer = buf, desc = "Back to the list of changes" })
end

local function onEnter()
  local target = state.lines[vim.api.nvim_win_get_cursor(0)[1]]
  if target and target.file then
    openFile(target)
  end
end

local function showList()
  local buf = listBuffer()
  vim.api.nvim_set_current_buf(buf)
  vim.keymap.set("n", "<CR>", onEnter, { buffer = buf, desc = "Open the file at its first change" })
  vim.keymap.set("n", "<Tab>", function()
    state.isCombined = not state.isCombined
    draw()
  end, { buffer = buf, desc = "Per MR / combined" })
  vim.keymap.set("n", "R", read, { buffer = buf, desc = "Read the changes again" })
end

vim.api.nvim_create_user_command("OrchestrationChanges", function(opts)
  local name = opts.args ~= "" and opts.args or state.name
  if not name then
    vim.notify("which orchestration? :OrchestrationChanges <name>", vim.log.levels.WARN)
    return
  end
  local isNew = name ~= state.name
  state.name = name
  showList()
  if isNew then
    state.set = nil
    read()
  else
    draw()
  end
end, { nargs = "?", desc = "What an orchestration changed in the code" })

vim.api.nvim_create_user_command("OrchestrationDiff", function()
  local this = vim.b.orchestration
  if not this then
    vim.notify("not a file of :OrchestrationChanges", vim.log.levels.WARN)
    return
  end
  if this.status == "new" then
    vim.notify(this.path .. ": new, shown in full", vim.log.levels.INFO)
    return
  end
  local old = git({ "show", this.base .. ":" .. this.path }) or {}
  vim.cmd("diffthis")
  vim.cmd("leftabove vnew")
  local buf = scratch("before " .. this.where .. ":" .. this.path, old, this.path)
  vim.api.nvim_set_current_buf(buf)
  vim.cmd("diffthis")
  vim.cmd("wincmd p")
end, { desc = "This file side by side with its state before the change" })

-- A merge or a push of one of its MRs: the list follows within a minute.
vim.fn.timer_start(REFRESH_MS, function()
  read()
end, { ["repeat"] = -1 })
