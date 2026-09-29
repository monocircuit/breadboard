// The one line an agent says about what it does (`pnpm wt:action note`), for
// the FEED of its orchestration's overview: the human follows the agents
// there instead of in their chats.
import fs from "node:fs";
import path from "node:path";

/**
 * Where a worktree keeps its lines, in its ignored `.claude/`: `{ note,
 * stopped }`, each `{ text, at }`. `note` is what the agent chose to say,
 * `stopped` the first line of its last answer, written by the `Stop` hook
 * (`.claude/hooks/stop-answers.mjs`): current without a token spent.
 */
export const feedFile = dir => path.join(dir, ".claude", "feed.json");
const readFeed = file => {
  try {
    const data = JSON.parse(fs.readFileSync(file, "utf-8"));
    return data && typeof data === "object" ? data : {};
  } catch {
    return {};
  }
};
const lineOf = line =>
  typeof line?.text === "string" && line.text && line.at
    ? { text: line.text, at: line.at }
    : null;
/** `key` as `text` in `file`, the one before it replaced, the other line kept. */
const setLine = (file, key, text) => {
  const line = { text: text.trim(), at: new Date().toISOString() };
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(
    file,
    `${JSON.stringify({ ...readFeed(file), [key]: line }, null, 2)}\n`,
  );
  return line;
};
/** The agent's note in `file`, or null when there is none. */
export const noteOf = file => lineOf(readFeed(file).note);
/** `text` as the agent's note: the feed shows the latest only. */
export const setNote = (file, text) => setLine(file, "note", text);
/** What the `Stop` hook saw last, or null. */
export const stoppedOf = file => lineOf(readFeed(file).stopped);
/** The first line of `answer` as what the agent said when it stopped; nothing for an empty one. */
export const setStopped = (file, answer) => {
  const text = firstLine(answer);
  return text ? setLine(file, "stopped", text) : null;
};
/** The first line of a text with something on it, trimmed. */
export const firstLine = text =>
  text
    ?.split("\n")
    .find(line => line.trim())
    ?.trim() ?? "";
/**
 * The agent's last answer, `{ text, at }`: the newer of the transcript's
 * (`claude`, from the overview) and the Stop hook's. The transcript may be
 * gone or unbound; the hook's line is there all the same.
 */
export const lastAnswerOf = (claude, stopped) => {
  const read = claude?.answer
    ? { text: claude.answer, at: claude.answerAt }
    : null;
  if (!stopped) return read;
  if (!read?.at || new Date(stopped.at) > new Date(read.at)) return stopped;
  return read;
};
/**
 * What the FEED says of an agent: its note or the first line of its last
 * answer, whichever is newer; a note without a time of the answer wins.
 * `{ text, at, source: "note" | "answer" }`, or null when there is neither.
 */
export const feedLineOf = (note, answer, answerAt) => {
  const said = firstLine(answer);
  const isNoteNewer =
    note && (!said || !answerAt || new Date(note.at) >= new Date(answerAt));
  if (isNoteNewer)
    return { text: note.text, at: new Date(note.at), source: "note" };
  if (!said) return null;
  return {
    text: said,
    at: answerAt ? new Date(answerAt) : null,
    source: "answer",
  };
};
