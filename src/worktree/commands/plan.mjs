import { fail, flagValue } from "../../lib/proc.mjs";
import { recordOf, writePlan } from "../origins.mjs";

const USAGE =
  "usage: pnpm wt:plan <orchestration> [--work <issue>…] [--merge <issue>…] [--why <text>]";

/** The issue numbers after `flag`, up to the next flag; undefined when it is not given. */
const numbersAfter = flag => {
  const argv = process.argv.slice(2);
  const at = argv.indexOf(flag);
  if (at === -1) return undefined;
  const values = [];
  for (const arg of argv.slice(at + 1)) {
    if (arg.startsWith("--")) break;
    values.push(Number(arg.replace(/^#/, "")));
  }
  if (values.some(n => !Number.isInteger(n) || n <= 0)) fail(USAGE);
  return values;
};

/**
 * The orchestrator sets the order of its orchestration: the issues in the
 * order they are worked on, the order their MRs are merged in, and why. The
 * overview sorts WORKTREES and MERGE REQUESTS by it, and its NEXT line names the
 * MR to merge next. Without options it prints the plan there is.
 */
export default function planCommand(name) {
  const record = name ? recordOf(name) : null;
  if (!name) fail(USAGE);
  if (!record) fail(`no orchestration ${name} is recorded`);
  const known = new Set(record.issues.map(issue => issue.number));
  const work = numbersAfter("--work");
  const merge = numbersAfter("--merge");
  const why = flagValue("--why");
  const strangers = [...(work ?? []), ...(merge ?? [])].filter(
    n => !known.has(n),
  );
  if (strangers.length)
    fail(
      `not in ⇶ ${name}: ${strangers.map(n => `#${n}`).join(", ")} (its issues: ${[...known].map(n => `#${n}`).join(", ")})`,
    );
  const plan =
    work || merge || why !== undefined
      ? writePlan(name, { work, merge, why })
      : record.plan;
  const list = numbers =>
    numbers?.length ? numbers.map(n => `#${n}`).join(" → ") : "not set";
  console.log(
    `\n⇶ ${name}\n  work:  ${list(plan?.work)}\n  merge: ${list(plan?.merge)}${plan?.why ? `\n  why:   ${plan.why}` : ""}\n`,
  );
}
