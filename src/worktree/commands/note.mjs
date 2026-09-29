import fs from "node:fs";

import { fail, flagValue, git, run } from "../../lib/proc.mjs";
import { shown } from "../issues.mjs";

/**
 * A comment on an issue, from a file. `glab issue note` only takes
 * `--message`, and `--message "$(cat f)"` is not available to every session,
 * which is what sends people to `glab api --raw-field body=@f` — and that
 * one posts the path instead of the text. So: read the file, hand it over,
 * read the comment back.
 */
export default function noteCommand(issue) {
  const top = git(["rev-parse", "--show-toplevel"]).out;
  const number = issue?.replace(/^#/, "");
  const file = flagValue("--message-file");
  if (!number || !/^\d+$/.test(number) || !file)
    fail("usage: pnpm wt:note <issue> --message-file <file>");
  if (!fs.existsSync(file))
    fail(`${shown(top, file)} does not exist — write the comment there first`);
  const message = fs.readFileSync(file, "utf-8").trim();
  if (!message) fail(`${shown(top, file)} is empty`);

  const posted = run("glab", ["issue", "note", number, "--message", message], {
    cwd: top,
    quiet: true,
    allowFail: true,
  });
  if (!posted.ok) fail(`glab could not comment on #${number}`);
  const url = posted.out.split("\n").find(l => l.startsWith("http")) ?? "";

  // Read it back: a comment that holds a path instead of the text looks like
  // a success on the way out and is only visible on the way in.
  // From the API, as stored: `glab issue view --comments` renders the
  // markdown and leaves the newest comments of a long issue out.
  const back = run(
    "glab",
    [
      "api",
      `projects/:fullpath/issues/${number}/notes?sort=desc&order_by=created_at&per_page=10`,
    ],
    { cwd: top, quiet: true, allowFail: true },
  );
  const isThere =
    back.ok && JSON.parse(back.out).some(note => note.body.trim() === message);
  console.log(
    `\n${isThere ? "✓" : "⚠"} comment on #${number}${isThere ? "" : " — GitLab does not show it back, check it"}\n  ${url}\n`,
  );
}
