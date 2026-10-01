// A worktree's name in the overview is pink, the colour herdr's sidebar gives
// branches (#227): a worktree reads the same in both. The pink comes from the
// config herdr runs with, as the sidebar takes it.
import { hex } from "../lib/tty.mjs";
import { branchColourOf, herdrConfigText } from "../worktree.mjs";

let paint = null;
/** `text` in the sidebar's pink; the config is read once. */
export const pink = text => {
  if (!paint) paint = hex(branchColourOf(herdrConfigText()));
  return paint(text);
};
