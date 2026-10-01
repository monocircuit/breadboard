// `open orchestrator chat` at the top of an orchestration's view (#287): one
// click brings the orchestrator's conversation back in the tab `orchestrator`
// of its space, or focuses it while it runs; never a second tab
// (worktree/orchestrator-chat.mjs).
import { dim, red } from "../lib/tty.mjs";
import { restoreChat } from "../worktree.mjs";
import { isOnce } from "./args.mjs";
import { button } from "./boxes.mjs";
import { onButton, pushRows, screen } from "./screen.mjs";

/** How long the row shows what a click did, before it says again what one would do. */
const SAID_MS = 8000;
/** What the last click did, shown next to the button for SAID_MS. */
let last = null;
/** The click: restoreChat, its answer shown on the row for a while. */
export const clickChat = (name, restore = restoreChat) => {
  const said = restore(name, { isClick: true }) ?? "herdr did nothing";
  last = said;
  setTimeout(() => {
    if (last !== said) return;
    last = null;
    screen.redraw();
  }, SAID_MS).unref?.();
  screen.redraw({ isReread: true });
  return said;
};
/** `00000000-…`: a conversation's id, short. */
const shortId = id => `${id.slice(0, 8)}…`;
/** What the row says next to the button, for `status` of chatStatus. */
export const chatText = status => {
  const { state, sessionId } = status;
  const what = sessionId
    ? `resumes ${shortId(sessionId)}`
    : "starts a fresh claude, none is recorded";
  if (state === "alive") return "the orchestrator runs · a click focuses it";
  if (state === "busy")
    return "something else runs in its tab · a click focuses it";
  if (state === "idle") return `its claude is gone · a click ${what}`;
  if (state === "missing") return `its tab is gone · a click ${what}`;
  return `its space is gone · a click opens it and ${what}`;
};
/**
 * The row: the button and what a click does now, or what the last one did.
 * Left out in a --once print and outside herdr (`status` null).
 */
export const pushChatButton = (lines, name, status) => {
  if (isOnce || !status) return;
  const isAlive = status.state === "alive";
  const label = isAlive ? "◉ orchestrator chat" : "▶ open orchestrator chat";
  const chip = button(label, isAlive ? "green" : "cyan");
  const said = last
    ? last.startsWith("the conversation")
      ? red(last)
      : dim(last)
    : dim(chatText(status));
  pushRows(
    lines,
    [["", `${chip} ${said}`]],
    [onButton(label, () => clickChat(name))],
  );
};
