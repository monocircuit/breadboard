// The orchestrator's chat (#287): the claude conversation in the tab
// `orchestrator` of `⇶ <name>`. One such tab per space; its conversation is
// written into the record (`orchestrator: { sessionId, cwd }`), so a click in
// the overview (`open orchestrator chat`) and the layout after a herdr
// restart bring the same one back, as .claude/herdr-session does for an agent.
import {
  herdr,
  herdrDid,
  isInHerdr,
  panesOf,
  spaces,
  tabsOf,
} from "../lib/herdr.mjs";
import { quote } from "../lib/proc.mjs";
import { isConversationRunning, runningClaudes } from "./conversations.mjs";
import { ideFlag } from "./ide.mjs";
import { orchestratorOrder, orchestratorSpace } from "./orchestrator.mjs";
import {
  ORCHESTRATOR_TAB,
  orchestrationOf,
  orchestratorLabel,
  recordOf,
  updateRecord,
} from "./origins.mjs";
import { ensureTab, featureOf, isIdle, keepOrder } from "./tabs.mjs";
import { mainDir, worktrees } from "./worktrees.mjs";

const SESSION = /^[0-9a-f-]{36}$/;
/** The conversation recorded for the orchestration `name`, `{ sessionId, cwd }`, or null. */
export const chatOf = (name, record = recordOf(name)) => {
  const chat = record?.orchestrator;
  return chat && SESSION.test(chat.sessionId ?? "")
    ? { sessionId: chat.sessionId, cwd: chat.cwd || mainDir() }
    : null;
};
/** `chat` into the record of `name`, only on a change. True when it was written. */
export const recordChat = (name, chat) => {
  if (!chat) return false;
  const known = chatOf(name);
  if (known?.sessionId === chat.sessionId && known.cwd === chat.cwd)
    return false;
  updateRecord(name, {
    orchestrator: { sessionId: chat.sessionId, cwd: chat.cwd },
  });
  return true;
};
/**
 * The conversation a pane of `pane list` runs, `{ sessionId, cwd }`: `cwd`
 * where its claude stands now (runningClaudes), else herdr's; null without a claude.
 */
export const chatOfPane = (pane, claudes = runningClaudes()) => {
  const sessionId = pane?.agent === "claude" ? pane.agent_session?.value : null;
  if (!sessionId || !SESSION.test(sessionId)) return null;
  return {
    sessionId,
    cwd:
      claudes.find(claude => claude.id === sessionId)?.cwd ||
      pane.foreground_cwd ||
      pane.cwd ||
      mainDir(),
  };
};
/** The tab `orchestrator` of a space's `tabs`, or null. */
const chatTabOf = tabs => tabs.find(tab => tab.label === ORCHESTRATOR_TAB);
/**
 * A tab dropDuplicates would close: one pane, no agent, its shell idle.
 * `isIdle`: of a pane id (tabs.mjs), passed in for the tests.
 */
const isSpare = (tab, panes, idle) => {
  const own = panes.filter(pane => pane.tab_id === tab.tab_id);
  return own.length === 1 && !own[0].agent && idle(own[0].pane_id);
};
/**
 * Whether `⇶ <name>` has an orchestrator already: a tab `orchestrator` that
 * holds more than a spare shell (a claude, or anything else running). Such a
 * tab is never closed or replaced from a script.
 */
export const hasChat = ({ tabs, panes, isIdle: idle = isIdle }) => {
  const tab = chatTabOf(tabs);
  return Boolean(tab) && !isSpare(tab, panes, idle);
};
/**
 * Why wt:orchestrate leaves the tab running it where it is, or null when it
 * moves into `⇶ <name>` as `orchestrator` (#287). Only a session that is
 * nobody's orchestrator or agent moves, and only while the space has no
 * orchestrator: one `orchestrator` tab per space, and an orchestrator that
 * starts an issue of another orchestration stays its own.
 * `here`: the orchestration whose space the pane stands in, or null;
 * `isWorktree`: it stands in a worktree's space (an agent's); `hasLive`: hasChat.
 */
export const stayReason = ({ name, here, isWorktree, hasLive }) => {
  if (here === name) return null;
  if (here) return `it is the orchestrator of ${here}`;
  if (isWorktree) return "it is a worktree's";
  if (hasLive) return `${orchestratorLabel(name)} has its ${ORCHESTRATOR_TAB}`;
  return null;
};
/** Whether `label` is the space of a worktree (featureOf), where its agent works. */
export const isWorktreeSpace = label =>
  worktrees()
    .slice(1)
    .some(({ dir }) => featureOf(dir) === label);
/**
 * Where the chat of a space stands, from herdr's `tabs` and `panes` of it,
 * `all` the panes of every space and the recorded `sessionId`:
 * `{ state, pane, tab }`, `state` one of
 * - `alive`: the recorded conversation runs in a pane (anywhere), or a claude
 *   runs in the tab `orchestrator` (a fresh chat);
 * - `idle`: the tab is there, a shell alone idles in it (its claude died);
 * - `busy`: the tab is there, something else runs in it;
 * - `missing`: no tab `orchestrator`.
 */
export const chatStateOf = ({
  tabs,
  panes,
  all = panes,
  sessionId = null,
  isIdle: idle = isIdle,
}) => {
  const running = sessionId
    ? all.find(pane => pane.agent_session?.value === sessionId)
    : null;
  if (running) return { state: "alive", pane: running, tab: null };
  const tab = chatTabOf(tabs);
  if (!tab) return { state: "missing", pane: null, tab: null };
  const own = panes.filter(pane => pane.tab_id === tab.tab_id);
  const claude = own.find(pane => pane.agent);
  if (claude) return { state: "alive", pane: claude, tab };
  if (own.length === 1 && idle(own[0].pane_id))
    return { state: "idle", pane: own[0], tab };
  return { state: "busy", pane: own[0] ?? null, tab };
};
/**
 * The pane of the orchestrator's claude in its space: the claude in the tab
 * `orchestrator`; without that tab the one claude in the space, a fresh chat
 * started in a tab of its own (it becomes `orchestrator`, rememberChats).
 */
export const chatPaneOf = ({ tabs, panes }) => {
  const claudes = panes.filter(
    pane => pane.agent === "claude" && pane.agent_session?.value,
  );
  const tab = chatTabOf(tabs);
  if (tab) return claudes.find(pane => pane.tab_id === tab.tab_id) ?? null;
  return claudes.length === 1 ? claudes[0] : null;
};
/**
 * Every orchestration's chat written down, as rememberSessions does it for
 * the worktrees: the claude of its tab `orchestrator` — a fresh chat there
 * replaces the recorded one. A lone claude in a space without that tab gets
 * the tab's name. The overview of the main space calls it on every read.
 */
export const rememberChats = () => {
  const all = herdr(["pane", "list"])?.panes ?? [];
  let claudes = null;
  for (const space of spaces()) {
    const name = orchestrationOf(space.label);
    if (!name || !recordOf(name)) continue;
    const tabs = tabsOf(space.workspace_id);
    const pane = chatPaneOf({
      tabs,
      panes: all.filter(p => p.workspace_id === space.workspace_id),
    });
    if (!pane) continue;
    if (!chatTabOf(tabs))
      herdr(["tab", "rename", pane.tab_id, ORCHESTRATOR_TAB]);
    claudes ??= runningClaudes();
    recordChat(name, chatOfPane(pane, claudes));
  }
};
/** What the tab `orchestrator` runs: the recorded conversation, else a fresh claude in the main checkout. */
export const chatCommand = chat =>
  chat
    ? `cd ${quote(chat.cwd)} && claude --resume ${chat.sessionId}${ideFlag(chat.cwd)}`
    : `cd ${quote(mainDir())} && claude${ideFlag(mainDir())}`;
const focus = (workspace, tab) =>
  herdrDid(["workspace", "focus", workspace]) &&
  herdrDid(["tab", "focus", tab]);
/**
 * The orchestrator's chat back in `⇶ <name>`, never a second tab: a live
 * one is focused; the tab whose claude died gets the recorded conversation
 * resumed; a missing tab is made, with it running. `isClick` (the overview's
 * `open orchestrator chat`): the space is made if it is gone, without a
 * recorded conversation a fresh claude starts, and the tab gets the focus.
 * Without it (the layout after a herdr restart) only an idle tab with a
 * recorded conversation is acted on, like resumeSession. `chat`: the
 * recorded one (chatOf). Returns what happened.
 */
export const restoreChat = (
  name,
  { isClick = false, chat = chatOf(name) } = {},
) => {
  if (!isInHerdr()) return null;
  const label = orchestratorLabel(name);
  const workspace =
    spaces().find(space => space.label === label)?.workspace_id ??
    (isClick ? orchestratorSpace(name) : null);
  if (!workspace) return null;
  const tabs = tabsOf(workspace);
  const { state, pane } = chatStateOf({
    tabs,
    panes: panesOf(workspace),
    all: herdr(["pane", "list"])?.panes ?? [],
    sessionId: chat?.sessionId,
  });
  if (state === "alive" || state === "busy") {
    if (!isClick || !pane) return null;
    focus(pane.workspace_id ?? workspace, pane.tab_id);
    return state === "alive"
      ? "focused the orchestrator"
      : `focused the tab ${ORCHESTRATOR_TAB}, something else runs in it`;
  }
  if (!chat && !isClick) return null;
  // A second claude in the same conversation would write into the same transcript.
  if (chat && isConversationRunning(chat.sessionId))
    return `the conversation ${chat.sessionId} runs elsewhere, outside herdr's panes`;
  if (state === "missing" && !isClick) return null;
  const command = chatCommand(chat);
  if (state === "idle") {
    if (!herdrDid(["pane", "run", pane.pane_id, command])) return null;
  } else {
    if (
      !ensureTab(
        workspace,
        { label: ORCHESTRATOR_TAB, cwd: mainDir(), command },
        tabs,
      )
    )
      return null;
    keepOrder(workspace, orchestratorOrder());
  }
  const now = chatTabOf(tabsOf(workspace));
  if (isClick && now) focus(workspace, now.tab_id);
  const what = chat ? `resumed ${chat.sessionId}` : "started a fresh claude";
  return state === "idle"
    ? `${what} in its tab`
    : `${what} in a new tab ${ORCHESTRATOR_TAB}`;
};
/**
 * What the overview of `name` shows on its `open orchestrator chat`:
 * `{ state, sessionId }` (chatStateOf; `gone` for no space), null outside herdr.
 */
export const chatStatus = name => {
  if (!isInHerdr()) return null;
  const chat = chatOf(name);
  const label = orchestratorLabel(name);
  const workspace = spaces().find(space => space.label === label)?.workspace_id;
  if (!workspace) return { state: "gone", sessionId: chat?.sessionId ?? null };
  const { state } = chatStateOf({
    tabs: tabsOf(workspace),
    panes: panesOf(workspace),
    all: herdr(["pane", "list"])?.panes ?? [],
    sessionId: chat?.sessionId,
  });
  return { state, sessionId: chat?.sessionId ?? null };
};
