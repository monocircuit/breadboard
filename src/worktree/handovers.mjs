// An announced handover (#227): "the agent of #224 takes #226 once !232 is
// merged", in the record of the orchestration instead of the orchestrator's
// head. `wt:orchestrate <name> <issue> --to <agent> --after !N…` writes it,
// the overview does it once the MRs are merged, the handover drops it.
import fs from "node:fs";

import { recordFile, recordOf } from "./origins.mjs";

/**
 * The MRs after `--after`: `!232`, `232`, also `!232,!233`, each once, in
 * order. `{ after }`, or `{ error }` for one that is no MR.
 */
export const parseAfter = values => {
  const after = [];
  for (const value of values.flatMap(arg => arg.split(","))) {
    if (!/^!?\d+$/.test(value.trim()))
      return { error: `${value} is no MR (!232)` };
    const iid = Number(value.trim().replace("!", ""));
    if (!after.includes(iid)) after.push(iid);
  }
  return after.length
    ? { after }
    : { error: "--after names the MRs to wait for: --after !232" };
};
/**
 * The values of `--after` in `argv` and where they stand: `--after=!232,!233`
 * one, `--after !232 !233` the `!` arguments behind it (an issue may follow).
 * `{ values, at }`, `at` their indexes; null without `--after`.
 */
export const afterArgs = argv => {
  const at = argv.findIndex(
    arg => arg === "--after" || arg.startsWith("--after="),
  );
  if (at === -1) return null;
  const inline = argv[at].split(/=(.*)/s)[1];
  if (inline !== undefined) return { values: [inline], at: [at] };
  const indexes = [];
  for (let i = at + 1; i < argv.length && argv[i].startsWith("!"); i++)
    indexes.push(i);
  return { values: indexes.map(i => argv[i]), at: [at, ...indexes] };
};

/** The announced handovers of a record: `[{ number, to, after, at }]`. */
export const handoversOf = record =>
  Array.isArray(record?.handovers) ? record.handovers : [];
/** `record` with `handover` in it, one per issue: a new one for the same issue replaces the old. */
export const withHandover = (record, handover) => ({
  ...record,
  handovers: [
    ...handoversOf(record).filter(known => known.number !== handover.number),
    handover,
  ],
});
/** `record` without the handover of the issue `number`. */
export const withoutHandover = (record, number) => ({
  ...record,
  handovers: handoversOf(record).filter(known => known.number !== number),
});

const write = (name, record) =>
  fs.writeFileSync(recordFile(name), `${JSON.stringify(record, null, 2)}\n`);
/** Records in `name` that the agent `to` takes the issue `number` once the MRs `after` are merged. */
export const announceHandover = (name, { number, to, after }) =>
  write(
    name,
    withHandover(recordOf(name), {
      number,
      to,
      after,
      at: new Date().toISOString(),
    }),
  );
/** The handover of the issue `number` is done (or overtaken): out of the record of `name`. */
export const dropHandover = (name, number) => {
  const record = recordOf(name);
  if (handoversOf(record).some(known => known.number === number))
    write(name, withoutHandover(record, number));
};
