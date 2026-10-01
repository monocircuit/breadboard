// What a view says about its GitLab data (#305): while the first read runs,
// how far it is (`GitLab: 4/7 · pipelines`); once there is data, how old it
// is; when a read failed, when and why, with the data it still shows.
import { progress, stats } from "../lib/glab.mjs";

/** `14:57`, local time. */
const clock = date =>
  date.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });

/** In place of the data while the read runs: how many calls are answered, what it waits for. */
export const waitingText = (state = progress) =>
  state.asked === 0
    ? "GitLab …"
    : `GitLab: ${state.answered}/${state.asked}${state.phase ? ` · ${state.phase}` : ""}`;

/**
 * The error line of a read that failed (`error`: `{ reason, at }`, or a
 * text): when, why, and the age of the data shown in its place.
 */
export const errorText = (error, remote = null) => {
  if (!error) return null;
  const { reason, at } =
    typeof error === "string" ? { reason: error, at: null } : error;
  const when = at ? ` ${clock(new Date(at))}` : "";
  const kept = remote?.at ? `, data from ${clock(new Date(remote.at))}` : "";
  return `GitLab didn't answer${when} (${reason.split("\n")[0]})${kept}, tried again`;
};

/** The age of the data and GitLab's minute, all clients of the user: `GitLab 14:57 · 122/2000 a minute`. */
export const freshText = (remote, rate = stats.rate) =>
  remote?.at
    ? `GitLab ${clock(new Date(remote.at))}${rate ? ` · ${rate.observed}/${rate.limit} a minute` : ""}`
    : null;
