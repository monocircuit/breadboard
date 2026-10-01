// For the tests (#303): every button drawn in a frame on screen, clicked at
// each of its cells, must run its own click, not the line's and not nothing.
import { ESCAPES } from "../lib/tty.mjs";
import { buttonsOn } from "./boxes.mjs";

/**
 * The buttons of the frame on screen (`rows`, as drawn by the painter
 * `frame`) whose click misses them at some cell: `{ row, column, label, ran
 * }` each, `ran` what runs there instead (frame.mjs targetAt); none when
 * every cell of every button runs it. `clicked`: each button's cells are
 * counted in it, `{ count }`, so a test can tell it found any.
 */
export const missedButtons = (frame, rows, clicked = { count: 0 }) => {
  const missed = [];
  rows.forEach((line, row) => {
    for (const { from, to, label } of buttonsOn(line).buttons)
      for (let column = from; column <= to; column++) {
        clicked.count++;
        const { ran } = frame.targetAt(row, column);
        if (ran !== `button ${label}`)
          missed.push({
            row,
            column,
            label,
            ran,
            line: line.replace(ESCAPES, "").trim(),
          });
      }
  });
  return missed;
};
