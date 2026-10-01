// An action as a block of its own (#273), in ACTIONS and in NEXT: a head
// line (its kind as a chip, the agent, the age, a try's state), its text
// wrapped at the full width, then how it is answered: a decide's options
// numbered one under the other like Claude Code's question dialog, the
// buttons of the other kinds, or the answer on its way. No line is wider
// than the width it is given.
import { ESCAPES, bold, dim, green, wrap } from "../lib/tty.mjs";
import { optionsOf } from "../worktree.mjs";
import { answerCell } from "./answers.mjs";
import { chip } from "./boxes.mjs";
import { ago } from "./render-worktree.mjs";

/** The colour of a kind's chip. */
const TONE = {
  merge: "green",
  try: "cyan",
  decide: "yellow",
  plan: "cyan",
  end: "red",
  order: "cyan",
  "hand over": "cyan",
  start: "cyan",
  pipeline: "red",
  "clean up": "cyan",
  waits: "yellow",
  push: "red",
};
/** Kinds shown by another name: `browser-check` is stored before `try` came. */
const SHOWN = { "browser-check": "try" };
/** The chip of a kind, an automatic action's marked in it: `merge · auto`. */
export const kindChip = (kind, isAuto = false) => {
  const name = SHOWN[kind] ?? kind;
  return chip(`${name}${isAuto ? " · auto" : ""}`, TONE[name] ?? "plain");
};
/** How many lines a text, an option's label or its description may take; past them `…`. */
const MAX_LINES = 12;
/** Grey, its own colours and links dropped: a try that can't be tried yet. */
const grey = text => dim(String(text).replace(ESCAPES, ""));
/**
 * `text` wrapped at `width`, after `first` on its first line and indented
 * as far on the others.
 */
const hanging = (first, text, width) => {
  const hang = first.length;
  return wrap(text, Math.max(1, width - hang), MAX_LINES).map(
    (line, i) => `${i === 0 ? first : " ".repeat(hang)}${line}`,
  );
};
/**
 * A decide's text: the question, up to its `?`, bold; what follows (why the
 * recommendation) dim on the lines below.
 */
const questionLines = (text, width) => {
  const at = text.indexOf("?");
  const [question, why] =
    at === -1 ? [text, ""] : [text.slice(0, at + 1), text.slice(at + 1).trim()];
  return [
    ...wrap(bold(question), width, MAX_LINES),
    ...(why ? wrap(dim(why), width, MAX_LINES) : []),
  ];
};
/**
 * The options of a decide as lines: `1. <label>` bold, the recommended one
 * marked `(Recommended)` like Claude Code does, its description dim on the
 * lines below, under the label. Each line answers its option (`on: i`).
 */
const optionLines = (action, width) =>
  optionsOf(action).flatMap(({ answer, label, description }, i) => {
    const number = `${i + 1}. `;
    const mark =
      action.recommend === answer ? ` ${green("(Recommended)")}` : "";
    return [
      ...hanging(number, `${bold(label)}${mark}`, width),
      ...(description
        ? hanging(" ".repeat(number.length), dim(description), width)
        : []),
    ].map(line => ({ line, on: i }));
  });

/**
 * The lines of the action of `item` (`{ action, who, isAuto }`) as a block
 * `width` wide, each `{ line, on }`, `on` what a click on it does: `"go"`
 * the action, `"buttons"` a button of its buttons line answers
 * (answers.mjs clickAction), a number the option it answers.
 * `text`: its text as the view shows it (a merge's with its MR); `state`: a
 * try's state, painted; `isWaiting`: a try that can't be tried yet, grey but
 * for its state and without buttons. An automatic action has no buttons:
 * its chip says `auto`.
 */
export const actionBlock = (
  item,
  { text, width, state = "", isWaiting = false },
) => {
  const { action } = item;
  const tone = isWaiting ? grey : cell => cell;
  const head = [
    tone(kindChip(action.kind, item.isAuto)),
    tone(bold(item.who)),
    action.at ? dim(ago(new Date(action.at))) : "",
    state,
  ]
    .filter(Boolean)
    .join("  ");
  const go = line => ({ line, on: "go" });
  const lines = [
    ...wrap(head, width, 2).map(go),
    ...(action.kind === "decide"
      ? questionLines(text, width)
      : wrap(tone(text), width, MAX_LINES)
    ).map(go),
  ];
  if (item.isAuto || isWaiting) return lines;
  if (action.answer) return [...lines, go(answerCell(action))];
  if (action.kind === "decide" && optionsOf(action).length)
    return [...lines, ...optionLines(action, width)];
  return [...lines, { line: answerCell(action), on: "buttons" }];
};
