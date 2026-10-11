# A tab group per herdr space

Every worktree has its herdr space (`../herdr`); this puts its browser tabs
together too. What `pnpm wt:new` and `pnpm overview` open goes into a
Chromium tab group named like the space:

| Opened by | Group |
| --------- | ----- |
| `pnpm wt:new` / `pnpm wt:stack` with an own stack, inside herdr: the app (`http://localhost:3000+n`) | the worktree's, `<feature>` |
| a click in the overview of a worktree (app, keycloak, s3, studio, storybook, issue, MR) | that worktree's |
| a click in the main overview: a worktree's app, an MR or issue of a branch that has a worktree | that worktree's |
| any other click in the main overview | `Monocircuit/Platform` |

A group of that name is reused, also from another window, and a tab of it
that shows the url already comes to the front instead of a second one. A new
group gets a colour from its name. An app on localhost that doesn't answer yet
(`wt:new` opens it while `pnpm dev` still starts) gets a page that waits for it.

Chromium has no command line for tab groups, only extensions reach them: this
directory is one, unpacked. breadboard (`crates/breadboard/src/cli/setup/browser.rs`) opens its page
`chrome-extension://<id>/open.html?url=…&group=…`, which moves its tab into
the group and turns into the url. The id is fixed by the `key` in
`manifest.json`: `aikcnbeljldocaafkokjbngkboecmoli`.

## Install

```sh
pnpm browser
```

copies the extension to `~/.local/share/monocircuit/chromium-extension` and
says how your browser loads it. The browser loads it from there, never from a
checkout: a worktree goes away, and the main checkout only has a change of the
extension after its merge. A directory missing at the browser's start gives a
"Manifest file is missing" dialog. Then, once:

- on a system whose Chromium reads `~/.config/chromium-flags.conf` (Arch,
  Omarchy): add that directory to its `--load-extension=` list and restart
  Chromium, or
- `chrome://extensions` → *Developer mode* → *Load unpacked* → that directory.

breadboard looks for `chromium`, `chromium-browser`,
`google-chrome-stable` or `google-chrome` that loads the copy (the flags file)
or knows the id (a profile's preferences), and every time it opens something
it brings the copy up to `packages/breadboard/chromium` of the checkout it runs from.
Without such a browser, a url opens with `xdg-open` (`open` on macOS) as
before, without a group, and `wt:new` opens no app: it would show a
connection error.

## Change it

Edit `open.js`; the next open from that checkout (or `pnpm browser`) updates
the copy, then *Reload* on `chrome://extensions` or restart the browser. Two
checkouts with different versions of the extension take turns writing the
copy. Never change the `key`: it is the id breadboard opens.
