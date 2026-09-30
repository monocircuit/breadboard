import { fail, flagValue, git } from "../../lib/proc.mjs";
import {
  ACTION_KINDS,
  KIND_ALIASES,
  actionsFile,
  actionsOf,
  addAction,
  answerLine,
  answersOf,
  decideProblem,
  doneAction,
  takeAnswers,
} from "../actions.mjs";
import { pushStateOf } from "../change.mjs";
import { feedFile, setNote } from "../feed.mjs";
import { originOf, recordFile, recordOf } from "../origins.mjs";
import { mainDir } from "../worktrees.mjs";
import changeCommand from "./change.mjs";

const USAGE = `usage: pnpm wt:action <kind> <text> [--url <url>]   kind: ${ACTION_KINDS.join(", ")}
       pnpm wt:action decide "<what is decided>?" --option "<what happens>" --option "<what happens>"… --recommend <option>
       pnpm wt:action try <what to check> --run <command> | --url <path or url>
       pnpm wt:action note <one line>   what you do, for the FEED
       pnpm wt:action merge <what is in it> --orchestration <name> --mr !N   the orchestrator's merge of an MR
       pnpm wt:action change <what>     a change for this MR: draft at once, merge withdrawn (--orchestration <name> --mr !N: the orchestrator's)
       pnpm wt:action --list
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
  "--recommend",
  "--mr",
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

/** Every value of a flag given more than once: `--option A --option B`. */
const valuesOf = name => {
  const argv = process.argv.slice(2);
  return argv
    .flatMap((arg, i) =>
      arg === name
        ? [argv[i + 1]]
        : arg.startsWith(`${name}=`)
          ? [arg.slice(name.length + 1)]
          : [],
    )
    .filter(value => value && !value.startsWith("--"));
};

/**
 * The option `--recommend` names, by its text or its number (from 1); null
 * without the flag.
 */
const recommendOf = options => {
  const given = flagValue("--recommend");
  if (!given) return null;
  return options.includes(given)
    ? given
    : (options[Number(given) - 1] ?? given);
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
    const actions = actionsOf(file);
    console.log(
      actions.length
        ? actions
            .map(
              a =>
                `  ${a.id}  ${a.kind.padEnd(13)} ${a.text}${a.url ? `  ${a.url}` : ""}${a.run ? `  $ ${a.run}` : ""}${a.options?.length ? `  [${a.options.map(option => (option === a.recommend ? `★ ${option}` : option)).join(" | ")}]` : ""}${a.answer ? `  → ${a.answer}` : ""}`,
            )
            .join("\n")
        : "  no open actions",
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

  if (kind === "note") {
    const dir = git(["rev-parse", "--show-toplevel"]).out;
    if (!dir || dir === mainDir()) fail("a note is for a worktree's agent");
    const text = textOf();
    if (!text) fail(USAGE);
    setNote(feedFile(dir), text);
    console.log(
      `\n✓ ${name ? `in FEED of ⇶ ${name}` : "noted (no orchestration shows it)"}: ${text}\n`,
    );
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
  const options = valuesOf("--option");
  const recommend = recommendOf(options);
  if (kind === "decide") checkDecide(text, options, recommend);
  else if (options.length || flagValue("--recommend"))
    fail("only a decide has options: --option <answer> --recommend <option>");
  const action = addAction(file, {
    kind,
    text,
    url,
    run,
    options,
    recommend,
    pane: process.env.HERDR_PANE_ID ?? null,
    mr,
  });
  const spot = run
    ? `  the click opens a terminal in this worktree: ${run}\n`
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
