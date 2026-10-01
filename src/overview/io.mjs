// Commands, GitLab and the browser, as the overview uses them.
import { openInGroup, openPopup } from "../browser.mjs";
import { mainDir, spaceLabel } from "../worktree.mjs";
import { only } from "./args.mjs";

export { glabApi as glab, glabText, phase } from "../lib/glab.mjs";
export { exec, sh } from "../lib/proc.mjs";

/**
 * Hand a url to the browser, without waiting for it: into the tab group of the
 * space of `dir` (the checkout it is about), else of the view's own space.
 */
export const open = (url, dir = only ?? mainDir()) =>
  openInGroup(url, spaceLabel(dir));
/**
 * An issue or MR in a browser popup: a window of its own without tabs, next
 * to the terminal (browser.mjs). Ctrl-click on `#n` or `!n` still
 * opens it as a tab, in the group of its space.
 */
export const show = (url, dir = only ?? mainDir()) =>
  openPopup(url, spaceLabel(dir));
