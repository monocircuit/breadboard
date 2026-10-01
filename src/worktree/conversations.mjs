// The claude conversation of a worktree: kept in the main checkout's project,
// bound to its space, resumed after a restart.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { herdr, panesOf, spaces } from "../lib/herdr.mjs";
import { quote } from "../lib/proc.mjs";
import { ideFlag } from "./ide.mjs";
import { featureOf, idlePaneOf } from "./tabs.mjs";
import { mainDir, worktrees } from "./worktrees.mjs";

/** Where the claude conversation bound to a worktree's space is written down. */
export const sessionFile = dir => path.join(dir, ".claude", "herdr-session");
/**
 * One claude conversation belongs to a worktree's space: the one running in
 * it, and while several do, the one already bound. Written down on a change,
 * so a restart of herdr can resume it. The overview of the main space calls
 * this on every read; one `pane list` for all spaces.
 */
export const rememberSessions = () => {
  const panes = herdr(["pane", "list"])?.panes ?? [];
  const byLabel = new Map(spaces().map(w => [w.label, w.workspace_id]));
  for (const { dir } of worktrees().slice(1)) {
    const workspace = byLabel.get(featureOf(dir));
    const ids = panes
      .filter(
        p =>
          p.workspace_id === workspace &&
          p.agent === "claude" &&
          p.agent_session?.value,
      )
      .map(p => p.agent_session.value);
    if (ids.length === 0) continue;
    const file = sessionFile(dir);
    const bound = fs.existsSync(file)
      ? fs.readFileSync(file, "utf-8").trim()
      : null;
    if (ids.includes(bound)) continue;
    // Never into a worktree being removed: `git worktree remove` deleted the file a moment
    // ago, a new one would leave the directory not empty and stop wt:remove halfway.
    if (
      !fs.existsSync(path.join(dir, ".git")) ||
      !fs.existsSync(path.dirname(file))
    )
      continue;
    fs.writeFileSync(file, `${ids[0]}\n`);
  }
};
/** The config directory of claude: its settings, and its transcripts under `projects/`. */
export const claudeDir = () =>
  process.env.CLAUDE_CONFIG_DIR ?? path.join(os.homedir(), ".claude");
/** The project directory claude keeps the transcripts of a directory in: its path with every other character a `-`. */
export const projectOf = dir =>
  path.join(claudeDir(), "projects", dir.replace(/[^a-zA-Z0-9]/g, "-"));
/**
 * Everything of the directory `from` moved into `to`, a folder both have
 * (`memory/`, a session's `subagents/`) merged the same way. A file `to`
 * already has arrives as `<name>.from-<suffix>`, nothing is lost. True when
 * `from` ended up empty and is gone.
 */
const mergeInto = (from, to, suffix) => {
  for (const entry of fs.readdirSync(from, { withFileTypes: true })) {
    const source = path.join(from, entry.name);
    const target = path.join(to, entry.name);
    const existing = fs.lstatSync(target, { throwIfNoEntry: false });
    if (!existing) fs.renameSync(source, target);
    else if (entry.isDirectory() && existing.isDirectory())
      mergeInto(source, target, suffix);
    else if (!fs.existsSync(`${target}.from-${suffix}`))
      fs.renameSync(source, `${target}.from-${suffix}`);
  }
  if (fs.readdirSync(from).length > 0) return false;
  fs.rmdirSync(from);
  return true;
};
/** A transcript written to within this time belongs to a conversation that may still run: it is not moved. */
const LIVE_MS = 10 * 60_000;
/**
 * Every conversation of the repo is kept in the project of the main checkout,
 * so `/resume` there lists them all, also those of a worktree long removed.
 * The project of a worktree (and of a directory below one, `…-products-vita`)
 * is a link to it. claude follows the link, and EnterWorktree sees both as
 * the same place. A project that is a directory still is merged in and
 * replaced by the link, once none of its transcripts was written to lately;
 * what the main project already has stays where it is. Returns what moved.
 */
export const shareConversations = () => {
  try {
    return linkProjects();
  } catch {
    // Comfort like the spaces: a claude directory it can't arrange never fails wt:new or the layout
    // (and the overview, which runs the layout, has no line to say it on).
    return [];
  }
};
const linkProjects = () => {
  const home = projectOf(mainDir());
  const projects = path.dirname(home);
  fs.mkdirSync(home, { recursive: true });
  const link = project => {
    if (fs.lstatSync(project, { throwIfNoEntry: false })) return;
    fs.symlinkSync(path.basename(home), project);
  };
  // A worktree just made has no project yet: its first conversation lands in the main one right away.
  for (const { dir } of worktrees().slice(1)) link(projectOf(dir));

  const merged = [];
  const prefix = `${path.basename(home)}-`;
  for (const entry of fs.readdirSync(projects, { withFileTypes: true })) {
    if (!entry.isDirectory() || !entry.name.startsWith(prefix)) continue;
    const project = path.join(projects, entry.name);
    const names = fs.readdirSync(project);
    const isLive = names.some(
      name =>
        name.endsWith(".jsonl") &&
        Date.now() - fs.statSync(path.join(project, name)).mtimeMs < LIVE_MS,
    );
    if (isLive || !mergeInto(project, home, entry.name.slice(prefix.length)))
      continue;
    link(project);
    merged.push(entry.name.slice(prefix.length));
  }
  return merged;
};
/**
 * The context window of the conversations: 1M with a `[1m]` model in the
 * settings of claude, 200k otherwise. The transcript names the model without it.
 */
const contextWindow = () => {
  try {
    const { model } = JSON.parse(
      fs.readFileSync(path.join(claudeDir(), "settings.json"), "utf-8"),
    );
    return String(model ?? "").includes("[1m]") ? 1_000_000 : 200_000;
  } catch {
    return 200_000;
  }
};
/**
 * The context of the last answer in `lines` of a transcript (oldest first):
 * `{ last, used, window }`, `last` that entry, `used` the prompt it read
 * (cached or not) plus what it wrote.
 */
export const usageOf = lines => {
  let last = null;
  for (const line of lines.toReversed()) {
    if (!line.includes('"usage"')) continue;
    try {
      const entry = JSON.parse(line);
      if (
        entry.type !== "assistant" ||
        entry.isSidechain ||
        !entry.message?.usage ||
        entry.message.model === "<synthetic>"
      )
        continue;
      last = entry;
      break;
    } catch {
      // a line still being written
    }
  }
  const usage = last?.message.usage;
  const used = usage
    ? usage.input_tokens +
      (usage.cache_creation_input_tokens ?? 0) +
      (usage.cache_read_input_tokens ?? 0) +
      usage.output_tokens
    : 0;
  // Past 200k it can only be the 1M window, whatever the settings say now.
  return { last, used, window: used > 200_000 ? 1_000_000 : contextWindow() };
};
/** How much of a transcript's end contextShareOf reads: the last answer is in there. */
const USAGE_BYTES = 1 << 19;
/** The share of its window (0 to 1) the conversation `id` had used at its last answer; null without a transcript. */
export const contextShareOf = id => {
  const transcript = transcriptOf(id);
  if (!transcript) return null;
  const { size } = fs.statSync(transcript);
  const length = Math.min(size, USAGE_BYTES);
  const buffer = Buffer.alloc(length);
  const fd = fs.openSync(transcript, "r");
  fs.readSync(fd, buffer, 0, length, size - length);
  fs.closeSync(fd);
  const { used, window } = usageOf(buffer.toString("utf-8").split("\n"));
  return Math.min(used / window, 1);
};
/** The conversation bound to a worktree's space, its id or null. */
export const boundSession = dir => {
  const file = sessionFile(dir);
  const id = fs.existsSync(file) ? fs.readFileSync(file, "utf-8").trim() : "";
  return /^[0-9a-f-]{36}$/.test(id) ? id : null;
};
/** The transcript of a claude conversation, in whichever project it was started; null if there is none. */
export const transcriptOf = id => {
  const projects = path.join(claudeDir(), "projects");
  const project = fs.existsSync(projects)
    ? fs
        .readdirSync(projects)
        .find(name => fs.existsSync(path.join(projects, name, `${id}.jsonl`)))
    : null;
  return project ? path.join(projects, project, `${id}.jsonl`) : null;
};
/**
 * The directory `claude --resume` finds a conversation from: the last one it
 * worked in (EnterWorktree moves the transcript along), else `dir`.
 */
const sessionCwd = (id, dir) => {
  const transcript = transcriptOf(id);
  if (!transcript) return dir;
  const cwds =
    fs.readFileSync(transcript, "utf-8").match(/"cwd":"[^"]*"/g) ?? [];
  const last = cwds.at(-1)?.slice(7, -1);
  return last && fs.existsSync(last) ? last : dir;
};
/**
 * Whether a claude runs the conversation already, anywhere on this machine —
 * in another herdr session, say, when the same repo lives in two (`pnpm herdr`
 * and pnpm herdr:install). Claude Code keeps a file per running process in
 * ~/.claude/sessions; a second resume would write into the same transcript.
 */
export const isConversationRunning = id => {
  const sessions = path.join(claudeDir(), "sessions");
  const files = fs.existsSync(sessions)
    ? fs.readdirSync(sessions).filter(file => file.endsWith(".json"))
    : [];
  return files.some(file => {
    try {
      const { pid, sessionId } = JSON.parse(
        fs.readFileSync(path.join(sessions, file), "utf-8"),
      );
      if (sessionId !== id) return false;
      process.kill(pid, 0);
      return true;
    } catch (error) {
      return error.code === "EPERM";
    }
  });
};
/** Where a process stands: Linux's /proc, else the cwd claude wrote into its session file. */
const cwdOf = (pid, written) => {
  try {
    return fs.readlinkSync(`/proc/${pid}/cwd`);
  } catch {
    return written ?? "";
  }
};
/**
 * The claudes running on this machine, from their files in ~/.claude/sessions:
 * `[{ pid, id, name, cwd }]`, `cwd` where the process stands now (it follows
 * EnterWorktree, the shell of its pane doesn't).
 */
export const runningClaudes = () => {
  const sessions = path.join(claudeDir(), "sessions");
  const files = fs.existsSync(sessions)
    ? fs.readdirSync(sessions).filter(file => file.endsWith(".json"))
    : [];
  return files.flatMap(file => {
    try {
      const { pid, sessionId, name, cwd } = JSON.parse(
        fs.readFileSync(path.join(sessions, file), "utf-8"),
      );
      process.kill(pid, 0);
      return [{ pid, id: sessionId, name: name ?? null, cwd: cwdOf(pid, cwd) }];
    } catch {
      return [];
    }
  });
};
/**
 * The claudes that stand in the worktree `dir` without being its agent: a
 * handover bound their conversation to the next worktree (moveWorker), and
 * they haven't entered it yet (#252). Removing `dir` now would leave them in
 * a directory that is gone, where EnterWorktree fails. `bound`: the
 * conversation bound to `dir`, which closes with its space.
 */
export const leftIn = (
  dir,
  claudes = runningClaudes(),
  bound = boundSession(dir),
) =>
  claudes.filter(
    ({ id, cwd }) =>
      id !== bound && (cwd === dir || cwd.startsWith(`${dir}${path.sep}`)),
  );
/** `244-x`, `244-x, claude 1234`: the claudes leftIn() found. */
export const leftText = left =>
  left.map(({ name, pid }) => name ?? `claude ${pid}`).join(", ");
/**
 * After a restart: the conversation bound to the space runs again in its
 * `<feature>` tab, if that tab idles, no claude runs anywhere in the space and
 * the conversation doesn't run elsewhere.
 */
export const resumeSession = (dir, workspace) => {
  const id = boundSession(dir);
  if (!id || panesOf(workspace).some(p => p.agent) || isConversationRunning(id))
    return false;
  const idle = idlePaneOf(workspace, featureOf(dir));
  if (!idle) return false;
  const cwd = sessionCwd(id, dir);
  return Boolean(
    herdr([
      "pane",
      "run",
      idle,
      `${cwd === dir ? "" : `cd ${quote(cwd)} && `}claude --resume ${id}${ideFlag(dir)}`,
    ]),
  );
};
/**
 * A tab that must not be closed from a script: an agent works in it, it holds
 * more than one pane (someone arranged it), or it runs the very command.
 */
export const isBusy = (tab, panes, current) =>
  tab.tab_id === current?.tab_id ||
  tab.pane_count > 1 ||
  panes.some(p => p.tab_id === tab.tab_id && p.agent);
/** What an agent in each of herdr's states holds its tab for; `idle` and `done` hold nothing. */
const AGENT_BUSY = {
  working: "an agent works there",
  blocked: "an agent waits for an answer there",
  unknown: "herdr can't tell whether an agent works there",
};
/**
 * The same, once the worktree is gone: why the tab has to stay, or null. An
 * agent only holds its tab while it works. A ready one closes — `idle`, or
 * `done`: a turn finished nobody looked at yet (#142) — its conversation stays
 * resumable from the main checkout (shareConversations). A tab left over would
 * sit in a deleted directory.
 */
export const busyReason = (tab, panes, current) => {
  if (tab.tab_id === current?.tab_id) return "it runs this command";
  if (tab.pane_count > 1) return "it has several panes";
  const agent = panes.find(
    p =>
      p.tab_id === tab.tab_id &&
      p.agent &&
      AGENT_BUSY[p.agent_status ?? "unknown"],
  );
  return agent ? AGENT_BUSY[agent.agent_status ?? "unknown"] : null;
};
