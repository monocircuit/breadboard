// The full view: every worktree, the stacks without one, the open MRs and
// issues.
import {
  ESCAPES,
  bold,
  columns,
  cyan,
  dim,
  fit,
  green,
  link,
  red,
  table,
  yellow,
} from "../lib/tty.mjs";
import { ORCHESTRATOR, focusSpace, mainDir } from "../worktree.mjs";
import { isStarting, pushStackButton } from "./actions.mjs";
import { checkFact } from "./check.mjs";
import { isOnce } from "./args.mjs";
import {
  closeBox,
  openBox,
  pipelineMark,
  stackCell,
  titleLines,
} from "./boxes.mjs";
import { errorText, waitingText } from "./gitlab-status.mjs";
import { open, show } from "./io.mjs";
import { KEYS, pushRows } from "./screen.mjs";

export const render = (local, remote, error) => {
  const width = columns();
  const lines = [];
  const now = new Date().toLocaleTimeString();
  const develop = remote ? `develop ${pipelineMark(remote.develop)}` : "";
  lines.push(
    ...titleLines(
      `platform  ${develop.replace(ESCAPES, "")}`,
      now,
      isOnce ? "" : KEYS,
    ),
  );

  const mrOf = branch => remote?.mrs.find(mr => mr.branch === branch);
  const appOf = tree => tree.isUp && `http://localhost:${3000 + tree.offset}`;
  const treesBox = openBox(lines);
  pushRows(
    lines,
    local.trees.map(tree => {
      const mr = mrOf(tree.branch);
      const app = !tree.hasEnv
        ? dim("—")
        : tree.hasOwnPort
          ? tree.isUp
            ? link(appOf(tree), green(`● :${3000 + tree.offset}`))
            : dim(`○ :${3000 + tree.offset}`)
          : dim("⇒ :3000");
      const work = [
        tree.ahead ? `${tree.ahead} ahead` : "",
        tree.changed ? yellow(`${tree.changed} changed`) : "",
        checkFact(tree.checked),
      ]
        .filter(Boolean)
        .join(" · ");
      return [
        tree.isHere ? cyan("→") : " ",
        // feature/<name> says nothing the name doesn't; a branch that differs does.
        `${tree.branch.split("/").at(-1) === tree.name ? bold(tree.name) : `${bold(tree.name)} ${dim(tree.branch)}`}${tree.origin ? ` ${cyan(`${ORCHESTRATOR} ${tree.origin.name}`)}` : ""}`,
        app,
        stackCell(tree),
        work || dim("clean"),
        mr
          ? `${link(mr.url, `!${mr.iid}`)}${mr.isDraft ? dim(" draft") : ""} ${pipelineMark(mr.pipeline)}`
          : "",
      ];
    }),
    // Its space in herdr; outside herdr its app, when that runs.
    local.trees.map(
      tree => () => focusSpace(tree.dir) || open(appOf(tree), tree.dir),
    ),
  );

  const main = local.trees.find(tree => tree.dir === mainDir());
  if (
    local.hasDocker &&
    main?.hasEnv &&
    (!main.stack?.includes("running") || isStarting(main.dir))
  ) {
    lines.push("");
    pushStackButton(lines, main.dir, "the default stack");
  }
  if (!local.hasDocker)
    lines.push("", yellow("docker does not answer: the stacks are unknown"));
  // The rows start with → or a space, not with an empty cell: nothing to take off.
  closeBox(
    lines,
    treesBox,
    `${bold("WORKTREES")} ${dim("click one to go to its space")}`,
    { isIndented: false },
  );
  if (local.orphans.length) {
    const orphansBox = openBox(lines);
    lines.push(
      ...table(
        local.orphans.map(([name, state]) => [
          "",
          name,
          yellow(state),
          dim(`docker compose -p ${name} down -v`),
        ]),
      ),
    );
    closeBox(lines, orphansBox, bold("STACKS WITHOUT A WORKTREE"));
  }

  if (error) lines.push("", red(errorText(error, remote)));
  if (remote) {
    const mrsBox = openBox(lines);
    pushRows(
      lines,
      remote.mrs.map(mr => [
        "",
        link(mr.url, `!${mr.iid}`),
        mr.isDraft ? dim("draft") : green("ready"),
        pipelineMark(mr.pipeline),
        mr.title,
        dim(`${mr.branch} · ${mr.who ?? ""}`),
      ]),
      remote.mrs.map(
        mr => () =>
          show(
            mr.url,
            local.trees.find(tree => tree.branch === mr.branch)?.dir,
          ),
      ),
    );
    closeBox(
      lines,
      mrsBox,
      `${bold("MERGE REQUESTS")} ${dim(`${remote.mrs.length} open`)}`,
    );

    // The issues take what the screen has left; --once shows all of them.
    // Below them: the gap, the box's two borders, the footer.
    const used = lines.length + 7;
    const room = isOnce
      ? Infinity
      : Math.max(3, (process.stdout.rows || 40) - used);
    const shown = remote.issues.slice(0, room);
    const issuesBox = openBox(lines);
    pushRows(
      lines,
      shown.map(issue => [
        "",
        link(issue.url, `#${issue.iid}`),
        issue.branch
          ? cyan(`← ${issue.branch}${issue.mr ? ` !${issue.mr}` : ""}`)
          : dim("unclaimed"),
        issue.title,
        dim(
          [issue.labels.join(","), issue.who && `@${issue.who}`]
            .filter(Boolean)
            .join(" "),
        ),
      ]),
      // Claimed here: the space it is worked on in. Otherwise the issue itself.
      shown.map(issue => () => {
        const tree =
          issue.branch && local.trees.find(t => t.branch === issue.branch);
        return (tree && focusSpace(tree.dir)) || show(issue.url, tree?.dir);
      }),
    );
    if (shown.length < remote.issues.length) {
      lines.push(
        dim(`  … ${remote.issues.length - shown.length} more: glab issue list`),
      );
    }
    closeBox(
      lines,
      issuesBox,
      `${bold("ISSUES")} ${dim(`${remote.issues.length} open, claimed first`)}`,
    );
  } else if (!error) {
    lines.push("", dim(waitingText()));
  }

  lines.push(
    "",
    dim(
      " pnpm stack:up|down|reset --port <p> · pnpm wt:new <n-feature> · pnpm commands",
    ),
  );
  return lines.map(line => fit(line, width)).join("\n");
};
