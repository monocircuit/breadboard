import { fail, flagValue, git } from "../../lib/proc.mjs";
import {
  ACTION_KINDS,
  KIND_ALIASES,
  actionsFile,
  actionsOf,
  addAction,
  answerLine,
  answeredOf,
  answersOf,
  decideProblem,
  doneAction,
  splitOption,
  takeAnswers,
} from "../actions.mjs";
import { pushStateOf } from "../change.mjs";
import { originOf, recordFile, recordOf } from "../origins.mjs";
import { mainDir } from "../worktrees.mjs";
import changeCommand from "./change.mjs";

const USAGE = `usage: pnpm wt:action <kind> <text> [--url <url>]   kind: ${ACTION_KINDS.join(", ")}
       pnpm wt:action decide "<what is decided>?" --option "<label>" --why "<what happens>" --option …… --recommend <label>
                (or --option "<label>: <what happens>", split at the first colon)
       pnpm wt:action try <what to check> --run <command> | --url <path or url>
       pnpm wt:action try <what to check> --on test --url <path>   on the test instance, ready once the MR is merged and deployed
       pnpm wt:action merge <what is in it> --orchestration <name> --mr !N   the orchestrator's merge of an MR
       pnpm wt:action change <what>     a change for this MR: draft at once, merge withdrawn (--orchestration <name> --mr !N: the orchestrator's)
       pnpm wt:action --list   the open actions, then the answered ones (a decision answered is taken)
       pnpm wt:action --done <id>
       pnpm wt:action --answers   the answers not yet delivered, handed over (they go)
       … --orchestration <name>   the orchestrator's own, from any checkout`;
/** The flags that take a value. */
const VALUED = [
  "--url",
  "--run",
  "--done",
  "--orchestration",
  "--option",
  "--why",
  "--recommend",
  "--mr",
  "--on",
];

/** The words of the command line after `<kind>` that are no flag and no flag's value. */
const textOf = () => {
  // node, worktree.mjs, action, <kind>, then the text.
  const argv = process.argv.slice(4);
  const words = [];
  for (let i = 0; i < argv.length; i++) {
    if (VALUED.includes(argv[i])) i++;
    else if (!argv[i].startsWith("--")) words.push(argv[i]);
  }
  return words.join(" ").trim();
};

/**
 * The options of a decide, `{ label, description }`, in their order:
 * each `--option`, with the `--why` right after it as what happens, or
 * split at its first `: ` without one.
 */
export const optionsOfArgs = argv => {
  const options = [];
  const valueAt = (arg, i, name) =>
    arg === name
      ? argv[i + 1]
      : arg.startsWith(`${name}=`)
        ? arg.slice(name.length + 1)
        : undefined;
  argv.forEach((arg, i) => {
    const option = valueAt(arg, i, "--option");
    if (option && !option.startsWith("--")) options.push({ given: option });
    const why = valueAt(arg, i, "--why");
    if (why && !why.startsWith("--") && options.length)
      options.at(-1).why = why;
  });
  return options.map(({ given, why }) => splitOption(given, why ?? ""));
};

/**
 * The label of the option `--recommend` names, by its label or its number
 * (from 1); null without the flag.
 */
const recommendOf = options => {
  const given = flagValue("--recommend");
  if (!given) return null;
  const labels = options.map(({ label }) => label);
  return labels.includes(given) ? given : (labels[Number(given) - 1] ?? given);
};
const checkDecide = (text, options, recommend) => {
  const problem = decideProblem(text, options, recommend);
  if (problem) fail(problem);
};

/**
 * Where the actions go: the orchestrator's into the record of its
 * orchestration, an agent's into its worktree (so they go with it).
 */
const whereOf = () => {
  const name = flagValue("--orchestration");
  if (name) {
    if (!recordOf(name)) fail(`no orchestration ${name} is recorded`);
    return { file: recordFile(name), name };
  }
  const dir = git(["rev-parse", "--show-toplevel"]).out;
  if (!dir || dir === mainDir())
    fail(
      "run it inside a worktree, or name the orchestration: --orchestration <name>",
    );
  return { file: actionsFile(dir), name: originOf(dir)?.name ?? null };
};

/**
 * Someone asks the human for something concrete: the row lands in ACTIONS
 * of the orchestration's overview (`pnpm overview --orchestration <name>`),
 * the first one in its NEXT line, where a click does it and ✓ ticks it off.
 */
export default function actionCommand(given, flags) {
  const kind = KIND_ALIASES[given] ?? given;
  if (kind === "change") {
    const text = textOf();
    if (!text) fail(USAGE);
    changeCommand(text, flagValue("--orchestration"));
    return;
  }
  const { file, name } = whereOf();

  if (flags.has("--list")) {
    const row = a =>
      `  ${a.id}  ${a.kind.padEnd(13)} ${a.text}${a.url ? `  ${a.url}` : ""}${a.on ? `  on ${a.on}` : ""}${a.run ? `  $ ${a.run}` : ""}${a.options?.length ? `  [${a.options.map(option => (option === a.recommend ? `${option} (Recommended)` : option)).join(" | ")}]` : ""}${a.answer ? `  → ${a.answer}` : ""}`;
    const actions = actionsOf(file);
    // The newest first: what was just decided is what gets relayed late.
    const answered = answeredOf(file).toReversed().slice(0, 10);
    console.log(
      `${actions.length ? actions.map(row).join("\n") : "  no open actions"}${
        answered.length
          ? `\n\n  answered, delivered:\n${answered.map(a => `${row(a)}  (${a.answeredAt})`).join("\n")}`
          : ""
      }`,
    );
    return;
  }
  if (flags.has("--answers")) {
    const answers = takeAnswers(file);
    console.log(
      answers.length
        ? answers.map(answerLine).join("\n")
        : "  no answers waiting",
    );
    return;
  }
  if (flags.has("--done")) {
    const id = Number(flagValue("--done"));
    if (!Number.isInteger(id)) fail(USAGE);
    if (!doneAction(file, id))
      fail(`no open action ${id} (pnpm wt:action --list)`);
    console.log(`\n✓ action ${id} done\n`);
    return;
  }

  if (!ACTION_KINDS.includes(kind)) fail(USAGE);
  const text = textOf();
  if (!text) fail(USAGE);
  const url = flagValue("--url") ?? null;
  const run = flagValue("--run") ?? null;
  if (given === "browser-check" && !url)
    fail("a browser-check needs the page: --url <url>");
  if (kind === "try" && Boolean(url) === Boolean(run))
    fail(
      "a try opens one spot: --run <command> (a terminal in the worktree) or --url <path or url> (the worktree's app)",
    );
  if (run && (kind !== "try" || flagValue("--orchestration")))
    fail("only an agent's try runs a command: --run <command>");
  const on = flagValue("--on") ?? null;
  if (on && on !== "test")
    fail(
      "a try is on the worktree's app (the default) or on the test instance: --on test",
    );
  if (on && (kind !== "try" || flagValue("--orchestration")))
    fail(
      "only an agent's try is on the test instance: its MR says when it is deployed",
    );
  if (on && !url?.startsWith("/"))
    fail(
      "a try on the test instance opens a path there: --url /atlas/…, never its hostname",
    );
  if (kind === "merge" && !flagValue("--orchestration")) {
    const dir = git(["rev-parse", "--show-toplevel"]).out;
    const branch = git(["branch", "--show-current"], dir).out;
    const { unpushed, hasRemoteBranch } = pushStateOf(dir, branch);
    if (unpushed)
      fail(
        `${unpushed} commit${unpushed === 1 ? "" : "s"} not pushed${hasRemoteBranch ? "" : " (no remote branch)"}: push first, the merge takes only what GitLab has`,
      );
  }
  // The orchestrator's merge names its MR: the overview merges that one.
  const mr = Number(flagValue("--mr")?.replace(/^!/, "")) || null;
  if (kind === "merge" && flagValue("--orchestration") && !mr)
    fail("which MR does it merge? --mr !N");
  if (mr && kind !== "merge")
    fail("only a merge or a change names its MR: --mr !N");
  const options = optionsOfArgs(process.argv.slice(2));
  const recommend = recommendOf(options);
  if (kind === "decide") checkDecide(text, options, recommend);
  else if (options.length || flagValue("--recommend") || flagValue("--why"))
    fail(
      "only a decide has options: --option <label> --why <what happens> --recommend <label>",
    );
  const action = addAction(file, {
    kind,
    text,
    url,
    run,
    on,
    options: options.map(({ label }) => label),
    descriptions: options.map(({ description }) => description),
    recommend,
    pane: process.env.HERDR_PANE_ID ?? null,
    mr,
  });
  const spot = run
    ? `  the click opens a terminal in this worktree: ${run}\n`
    : on
      ? `  the click opens ${url} on the test instance (VITA_TEST_ORIGIN); the row waits for the merge and the deployment\n`
      : kind === "try"
        ? `  the click opens ${url}${url.startsWith("/") ? " on this worktree's app (its stack started first when it is down)" : ""}\n`
        : "";
  console.log(
    `\n✓ action ${action.id} (${kind}): ${text}\n${spot}  ${
      name
        ? `in ACTIONS of ⇶ ${name}${kind === "decide" && !flagValue("--orchestration") ? "; send the orchestrator the options too (SendMessage)" : ""}`
        : "this worktree belongs to no orchestration: no overview shows it, tell the user"
    }\n${answersOf(action).length ? `  the answer comes typed into this pane: [overview] #${action.id} ${kind} "…" → <answer>\n` : ""}  done by hand: pnpm wt:action --done ${action.id}${flagValue("--orchestration") ? ` --orchestration ${name}` : ""}\n`,
  );
}
