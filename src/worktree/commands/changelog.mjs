import { flagValue } from "../../lib/proc.mjs";
import { overallChangelogCommand } from "../changelog-view.mjs";

/**
 * `pnpm changelog [--since 30d|2w|<date>]`: the changelog of the repo (#261),
 * every MR merged into develop by product and package, rewritten from GitLab
 * and printed; `--watch` shows it live (the tab `changelog` of the main space).
 */
export default function changelogCommand(_, flags) {
  return overallChangelogCommand({
    since: flagValue("--since") ?? null,
    isLive: flags.has("--watch"),
  });
}
