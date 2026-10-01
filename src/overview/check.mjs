// Whether a worktree's HEAD is tested (#298): wt:check's state as a fact of
// its node in WORKTREES and a row of its own view.
import { cyan, dim, green, red, yellow } from "../lib/tty.mjs";
import { failuresText } from "../worktree.mjs";

/**
 * `✓ tested 1a2b3c4d` / `untested: 2 commits` / `checking…` / `✗ tests
 * failed: <package> (<tests>)`; nothing for a branch without commits of its
 * own. `checked`: checkOf's state.
 */
export const checkFact = checked => {
  switch (checked?.key) {
    case "tested":
      return green(`✓ tested ${checked.sha.slice(0, 8)}`);
    case "checking":
      return cyan("checking…");
    case "failed": {
      const isTest = checked.failures.some(({ task }) => task === "test");
      return red(
        `✗ ${isTest ? "tests" : "check"} failed: ${failuresText(checked.failures) || "see pnpm wt:check"}`,
      );
    }
    case "untested":
      return yellow(
        checked.count
          ? `untested: ${checked.count} commit${checked.count === 1 ? "" : "s"}`
          : "untested",
      );
    default:
      return "";
  }
};

/** The same, with what to do about it: the row `tests` of a worktree's view. */
export const checkRow = checked => {
  const fact = checkFact(checked);
  if (!fact) return "";
  if (checked.key === "tested")
    return `${fact} ${dim(`· ${checked.packages.length ? checked.packages.join(", ") : "no package changed"}`)}`;
  if (checked.key === "checking") return fact;
  return `${fact}  ${dim("pnpm wt:check before the push")}`;
};
