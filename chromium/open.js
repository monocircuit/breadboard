// open.html?url=<url>&group=<herdr space>: breadboard (cli/setup/browser.rs) opens this page
// with Chromium; it moves its own tab into the tab group named like the space
// (the one there, else a new one) and then turns into <url>. A tab of that
// group already showing <url> is brought to the front instead.
//
// An app on localhost may not answer yet (wt:new opens it while `pnpm dev`
// still starts): the page waits for it instead of showing a connection error.

/** How long to wait for a local app, then open it anyway. */
const WAIT_MS = 10 * 60_000;

const params = new URLSearchParams(location.search);
const url = params.get("url");
const group = params.get("group");
const state = document.querySelector("#state");
const say = text => {
  state.textContent = text;
  document.title = text;
};

/** A colour per name, so a space's group looks the same every time it is made. */
const COLORS = [
  "blue",
  "cyan",
  "green",
  "yellow",
  "orange",
  "red",
  "pink",
  "purple",
  "grey",
];
const colorOf = title =>
  COLORS[
    [...title].reduce(
      (hash, char) => (hash * 31 + char.charCodeAt(0)) >>> 0,
      7,
    ) % COLORS.length
  ];

/** The group titled `title`, in any window; null when there is none. */
const groupNamed = async title =>
  (await chrome.tabGroups.query({ title })).find(g => g.title === title) ??
  null;

/** A tab of the group showing `href` already, not counting `self`. */
const openedIn = async (groupId, href, self) =>
  (await chrome.tabs.query({ groupId })).find(
    tab => tab.id !== self && tab.url && new URL(tab.url).href === href,
  );

/** Resolves once something answers on `href`; any answer counts, even an error page of the app. */
const answers = async href => {
  const until = Date.now() + WAIT_MS;
  while (Date.now() < until) {
    try {
      await fetch(href, { mode: "no-cors", cache: "no-store" });
      return;
    } catch {
      await new Promise(resolve => setTimeout(resolve, 1000));
    }
  }
};

const run = async () => {
  const target = url && new URL(url);
  if (!target || !["http:", "https:"].includes(target.protocol)) {
    say(`not a web address: ${url ?? "(none)"}`);
    return;
  }
  const self = await chrome.tabs.getCurrent();

  if (group) {
    const found = await groupNamed(group);
    const known = found && (await openedIn(found.id, target.href, self.id));
    if (known) {
      await chrome.tabs.update(known.id, { active: true });
      await chrome.windows.update(known.windowId, { focused: true });
      await chrome.tabs.remove(self.id);
      return;
    }
    // Into the group's window when it exists: a tab group can't span windows.
    const groupId = await chrome.tabs.group(
      found ? { groupId: found.id, tabIds: self.id } : { tabIds: self.id },
    );
    if (!found)
      await chrome.tabGroups.update(groupId, {
        title: group,
        color: colorOf(group),
        collapsed: false,
      });
    const moved = await chrome.tabs.update(self.id, { active: true });
    await chrome.windows.update(moved.windowId, { focused: true });
  }

  if (["localhost", "127.0.0.1"].includes(target.hostname)) {
    say(`waiting for ${target.host} …`);
    await answers(target.href);
  }
  location.replace(target.href);
};

run().catch(error => say(`could not open ${url}: ${error.message}`));
