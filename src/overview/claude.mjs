// The claude conversations of the worktrees, read from their transcripts: the
// last prompt and answer, the context used, what herdr says the agent does.
import fs from "node:fs";

import {
  boundSession,
  isInHerdr,
  transcriptOf,
  usageOf,
} from "../worktree.mjs";
import { sh } from "./io.mjs";

// ---------------------------------------------------------------------------
// What is there
// ---------------------------------------------------------------------------

/** Compose project → `running(3)` / `exited(3)`, one docker call for every stack. */
export const composeProjects = () => {
  const result = sh("docker", ["compose", "ls", "--all", "--format", "json"]);
  if (result.status !== 0) return null;
  try {
    return new Map(JSON.parse(result.stdout).map(p => [p.Name, p.Status]));
  } catch {
    return null;
  }
};
/** How much of a transcript is read, from its end: the last answer is in there, the megabytes before it are not needed. */
const TAIL_BYTES = 1 << 19;
const transcripts = new Map();
/** The text of a message: a string, or its `text` parts; tool results and images are none. */
const textOf = content =>
  typeof content === "string"
    ? content
    : (content ?? [])
        .filter(part => part.type === "text")
        .map(part => part.text)
        .join("\n");
/**
 * The last text of `type` (`user`: a prompt someone typed; `assistant`: an
 * answer) in `lines` of a transcript, newest first. A prompt of the harness
 * (`<command-…>`, `<system-reminder>`, …) starts with a tag and is none. Only
 * lines that can be one are parsed: tool output fills most of a transcript.
 */
const lastEntryOf = (lines, type) => {
  for (const line of lines.toReversed()) {
    if (
      !line.includes(`"type":"${type}"`) ||
      (type === "user" && line.includes('"tool_result"'))
    )
      continue;
    let entry;
    try {
      entry = JSON.parse(line);
    } catch {
      continue;
    }
    if (entry.type !== type || entry.isSidechain || entry.isMeta) continue;
    const text = textOf(entry.message?.content).trim();
    if (text && !(type === "user" && text.startsWith("<")))
      return { text, at: entry.timestamp ? new Date(entry.timestamp) : null };
  }
  return null;
};
const lastTextOf = (lines, type) => lastEntryOf(lines, type)?.text ?? null;
/** How far back the last prompt is looked for: an agent at work writes megabytes of tool output after it. */
const PROMPT_BYTES = 1 << 23;
/**
 * The last prompt of a transcript: in its tail, else further back, a chunk at
 * a time up to PROMPT_BYTES.
 */
const lastPromptOf = (transcript, size, tail) => {
  const found = lastTextOf(tail, "user");
  if (found || size <= TAIL_BYTES) return found;
  const length = Math.min(size, PROMPT_BYTES);
  const buffer = Buffer.alloc(length);
  const fd = fs.openSync(transcript, "r");
  fs.readSync(fd, buffer, 0, length, size - length);
  fs.closeSync(fd);
  return lastTextOf(buffer.toString("utf-8").split("\n"), "user");
};
/** What herdr says the conversation `id` is doing: `working`, `blocked` (it asks you), `idle`, … null outside herdr. */
export const agentStatusOf = id => {
  if (!isInHerdr()) return null;
  try {
    const { panes } = JSON.parse(sh("herdr", ["pane", "list"]).stdout).result;
    return (
      panes.find(pane => pane.agent_session?.value === id)?.agent_status ?? null
    );
  } catch {
    return null;
  }
};
/**
 * The claude conversation bound to a worktree's space: its model, when it last
 * answered and the context that answer had (the prompt it read, cached or not,
 * plus what it wrote). Read again only when the transcript changed.
 */
export const readClaude = dir => {
  const id = boundSession(dir);
  const transcript = id && transcriptOf(id);
  if (!transcript) return null;
  const { size, mtimeMs } = fs.statSync(transcript);
  const cached = transcripts.get(transcript);
  if (cached?.size === size && cached.mtimeMs === mtimeMs) return cached.claude;

  const length = Math.min(size, TAIL_BYTES);
  const buffer = Buffer.alloc(length);
  const fd = fs.openSync(transcript, "r");
  fs.readSync(fd, buffer, 0, length, size - length);
  fs.closeSync(fd);
  const lines = buffer.toString("utf-8").split("\n");
  if (length < size) lines.shift(); // cut in the middle

  const { last, used, window } = usageOf(lines);
  // Where `claude --resume` finds it: the directory it last worked in, else the worktree.
  const cwd = buffer
    .toString("utf-8")
    .match(/"cwd":"[^"]*"/g)
    ?.at(-1)
    ?.slice(7, -1);
  const answer = lastEntryOf(lines, "assistant");
  const claude = {
    prompt: lastPromptOf(transcript, size, lines),
    answer: answer?.text ?? null,
    id,
    // The file itself, not the link a worktree's project is (worktree.mjs, shareConversations).
    transcript: fs.realpathSync(transcript),
    cwd: cwd && fs.existsSync(cwd) ? cwd : dir,
    model: last?.message.model ?? null,
    at: last ? new Date(last.timestamp) : null,
    used,
    window,
  };
  transcripts.set(transcript, { size, mtimeMs, claude });
  return claude;
};
