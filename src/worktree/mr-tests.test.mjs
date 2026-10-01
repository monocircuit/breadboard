import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { describe, it } from "node:test";
import { fileURLToPath } from "node:url";

import { unfilled } from "./issues.mjs";
import { inapplicable, packagesOf, testLines, withTests } from "./mr-tests.mjs";

const template = fs.readFileSync(
  path.join(
    path.dirname(fileURLToPath(import.meta.url)),
    "../../../../.gitlab/merge_request_templates/Default.md",
  ),
  "utf-8",
);

describe("testLines", () => {
  it("has a lint/typecheck/test line per changed package", () => {
    assert.deepEqual(
      testLines({
        packages: [
          { name: "@monocircuit/breadboard", scripts: ["lint", "test"] },
        ],
        files: ["packages/breadboard/src/worktree/mr-tests.mjs"],
      }),
      ["- [ ] `pnpm --filter @monocircuit/breadboard lint` / `test`"],
    );
  });

  it("has no browser box and no test:int box without such changes", () => {
    const lines = testLines({
      packages: [{ name: "@monocircuit/vita-core" }],
      files: ["products/vita/core/src/chronicle.ts"],
    }).join("\n");
    assert.doesNotMatch(lines, /browser/);
    assert.doesNotMatch(lines, /test:int/);
  });

  it("asks for test:int with a change under a src/server of vita", () => {
    assert.match(
      testLines({
        packages: [{ name: "@monocircuit/vita" }],
        files: ["products/vita/web/src/server/operations/share.ts"],
      }).join("\n"),
      /pnpm test:int/,
    );
  });

  it("asks for the browser with a page, a component, a screen or monolithium", () => {
    for (const file of [
      "products/vita/web/src/app/atlas/page.tsx",
      "products/vita/web/src/components/Header.tsx",
      "products/vita/ui/src/screens/Atlas.tsx",
      "packages/monolithium/src/components/Button.tsx",
    ])
      assert.match(
        testLines({ packages: [], files: [file] }).join("\n"),
        /clicked through in the browser/,
        file,
      );
  });

  it("asks for no browser with a server or a docs change", () => {
    const lines = testLines({
      packages: [{ name: "@monocircuit/vita" }],
      files: [
        "products/vita/web/src/server/db/schema.ts",
        "products/vita/docs/backend.md",
      ],
    }).join("\n");
    assert.doesNotMatch(lines, /browser/);
  });
});

describe("packagesOf", () => {
  const names = {
    "packages/breadboard": "@monocircuit/breadboard",
    "products/vita/web": "@monocircuit/vita",
  };
  const packageAt = dir =>
    names[dir] ? { name: names[dir], scripts: ["test"] } : null;

  it("names the nearest package of each file once, the root's left out", () => {
    assert.deepEqual(
      packagesOf(
        [
          "products/vita/web/src/app/page.tsx",
          "products/vita/web/src/server/a.ts",
          "packages/breadboard/src/worktree/mr.mjs",
          "turbo.json",
          ".gitlab/merge_request_templates/Default.md",
        ],
        packageAt,
      ).map(pkg => pkg.name),
      ["@monocircuit/breadboard", "@monocircuit/vita"],
    );
  });
});

describe("withTests", () => {
  it("replaces the template's boxes with the ones that apply, the rest stays", () => {
    const text = withTests(template, [
      "- [ ] `pnpm --filter @monocircuit/breadboard lint` / `typecheck` / `test`",
    ]);
    const tests = text.slice(text.indexOf("## Tests"));
    assert.doesNotMatch(tests, /browser|test:int/);
    assert.match(tests, /@monocircuit\/breadboard lint/);
    assert.match(text, /## Changes/);
    assert.match(text, /\nRefs #\s*$/);
  });

  it("leaves an empty bullet when nothing applies, so wt:mr asks why", () => {
    const text = withTests(template, []);
    assert.ok(unfilled(text).some(line => line.includes("(empty bullet)")));
  });
});

describe("inapplicable", () => {
  it("finds an unticked Tests box that says it doesn't apply", () => {
    const text = [
      "## Tests",
      "",
      "- [x] `pnpm --filter @monocircuit/breadboard test`: 412 pass",
      "- [ ] clicked through in the browser: no browser change",
      "- [ ] `pnpm test:int` (no `src/server` change)",
      "- [ ] `pnpm test:int`: n/a",
    ].join("\n");
    assert.deepEqual(inapplicable(text), [
      "     4  - [ ] clicked through in the browser: no browser change",
      "     5  - [ ] `pnpm test:int` (no `src/server` change)",
      "     6  - [ ] `pnpm test:int`: n/a",
    ]);
  });

  it("keeps a box that applies and is still open, and boxes outside Tests", () => {
    const text = [
      "## Changes",
      "- [ ] follow-up: no browser change needed here",
      "## Tests",
      "- [ ] clicked through in the browser",
      "- [ ] `pnpm test:int` (`src/server` changed)",
      "- [x] clicked through: no browser change",
    ].join("\n");
    assert.deepEqual(inapplicable(text), []);
  });

  it("finds none in the template", () => {
    assert.deepEqual(inapplicable(template), []);
  });
});
