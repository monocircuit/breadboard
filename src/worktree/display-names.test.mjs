import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { FIRST, LAST, displayNamesOf, pickName } from "./display-names.mjs";

const ids = Array.from(
  { length: 40 },
  (_, i) => `${i.toString(16).padStart(8, "0")}-6c1e-4b7a-9f00-0d2c1a${i}`,
);

describe("the display name of an agent", () => {
  it("is a first and a last name, the same for the same conversation", () => {
    const name = pickName(ids[0]);
    assert.match(name, /^\S+ \S+$/);
    assert.ok(FIRST.includes(name.split(" ")[0]));
    assert.ok(LAST.includes(name.split(" ")[1]));
    assert.equal(pickName(ids[0]), name);
  });

  it("is no one else's in the orchestration, nor their first name while there are others", () => {
    const names = [...displayNamesOf({}, ids).values()];
    assert.equal(new Set(names).size, ids.length);
    assert.equal(new Set(names.map(name => name.split(" ")[0])).size, 40);
  });

  it("takes a full name not taken once every first name is", () => {
    const taken = FIRST.map(first => `${first} ${LAST[0]}`);
    const name = pickName(ids[1], taken);
    assert.ok(!taken.includes(name));
  });

  it("keeps the one the record has, and picks around it", () => {
    const own = pickName(ids[2]);
    const known = { other: { displayName: own }, [ids[2]]: {} };
    const names = displayNamesOf(known);
    assert.equal(names.get("other"), own);
    assert.notEqual(names.get(ids[2]), own);
  });

  it("stays the same across a handover: the record keeps it", () => {
    const known = { [ids[3]]: { displayName: "Mira Albrecht", issues: [1] } };
    assert.equal(
      displayNamesOf(known, [ids[3], ids[4]]).get(ids[3]),
      "Mira Albrecht",
    );
  });
});
