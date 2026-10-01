import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { CHANGES_LUA, editorCommand, openExpr, socketOf } from "./nvim.mjs";

describe("the socket of an nvim tab (#257)", () => {
  it("lies in .claude of the checkout, relative to it", () => {
    assert.equal(socketOf(), ".claude/nvim.sock");
  });

  it("is one per orchestration in the main checkout", () => {
    assert.equal(socketOf("breadboard"), ".claude/nvim-breadboard.sock");
  });
});

describe("the command line of an nvim tab (#257)", () => {
  const dir = "/repos/platform-257-x";

  it("starts nvim in the checkout, listening on its socket", () => {
    const command = editorCommand(dir);
    assert.ok(command.startsWith(`cd '${dir}'; `));
    assert.ok(
      command.endsWith(
        `nvim --listen '.claude/nvim.sock' -c 'luafile ${CHANGES_LUA}'`,
      ),
    );
  });

  it("removes a socket nobody answers on before it listens", () => {
    assert.ok(
      editorCommand(dir).includes(
        "nvim --server '.claude/nvim.sock' --remote-expr 1 >/dev/null 2>&1 || rm -f '.claude/nvim.sock'; ",
      ),
    );
  });

  it("listens on the orchestration's socket for its tab", () => {
    assert.ok(
      editorCommand(dir, { name: "breadboard" }).includes(
        "nvim --listen '.claude/nvim-breadboard.sock'",
      ),
    );
  });

  it("opens the clicked file at its line when it starts for a click", () => {
    assert.ok(
      editorCommand(dir, { file: "it's here.mjs", line: 12 }).endsWith(
        ` +12 'it'\\''s here.mjs'`,
      ),
    );
    assert.ok(editorCommand(dir, { file: "a.mjs" }).endsWith(" +1 'a.mjs'"));
  });

  it("uses no syntax only one shell knows", () => {
    assert.doesNotMatch(editorCommand(dir), /[{}]|&&|\$\(/);
  });
});

describe("opening a file in a running nvim (#257)", () => {
  it("drops to the file at the line, escaped for Vim script", () => {
    assert.equal(
      openExpr("/r/it's.mjs", 7),
      "execute('drop +7 ' .. fnameescape('/r/it''s.mjs') .. ' | normal! zz')",
    );
  });

  it("takes the first line without one", () => {
    assert.match(openExpr("/r/a.mjs"), /drop \+1 /);
  });
});
