import { herdr, isInHerdr } from "../../lib/herdr.mjs";
import { fail } from "../../lib/proc.mjs";
import { SPACE } from "../constants.mjs";
import { layout } from "../layout.mjs";

/**
 * The layout of herdr, built from the worktrees that exist: the main space
 * with `root` and `overview`, a space per worktree, the tabs of every stack
 * that runs. Idempotent, so it is also the way back after a herdr restart
 * or after tabs were closed by hand. The herdr plugin runs it at startup
 * with `--no-focus`: the space in front stays.
 */
export default async function herdrCommand(_feature, flags) {
  if (!isInHerdr()) fail("this runs inside herdr (HERDR_ENV is not set)");
  const built = await layout({ shouldResume: true });
  if (!built)
    fail("herdr did not answer, is its server running? (herdr status)");
  for (const line of built.report) console.log(`  ${line}`);
  if (!flags.has("--no-focus")) herdr(["workspace", "focus", built.main]);
  console.log(`\n✓ ${SPACE} and a space per worktree`);
}
