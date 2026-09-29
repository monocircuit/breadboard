// Names and ports every part of the worktree workflow agrees on.
import path from "node:path";

export const VITA = "products/vita";
/**
 * Default port → the variable it is moved with, written into the .env.local of
 * the worktree. docker compose reads them, except VISUALIZER_PORT, which
 * `pnpm pg:visualizer` reads, and the app port, which comes from APP_ORIGIN.
 * A base below 50 apart from the next one would make worktreeOnPort ambiguous.
 */
export const PORTS = {
  3000: null,
  // drizzle studio, not in .env.local: the herdr tab `studio` passes the port.
  4983: null,
  5432: "POSTGRES_PORT",
  // monolithium's Storybook, not in .env.local: the herdr tab `storybook` passes the port.
  6006: null,
  8080: "KEYCLOAK_PORT",
  8333: "S3_PORT",
  64738: "VISUALIZER_PORT",
};
export const NAME = /^[a-z0-9]+(-[a-z0-9]+)*$/;
/** The main herdr space of the repo; every worktree gets a space of its own next to it. */
export const SPACE = "Monocircuit/Platform";
/** The tab of the live overview, in the main space and in each worktree's space. */
export const OVERVIEW = "overview";
/** A shell in the checkout of its space, the leftmost tab of every space: the main one's is where the next `claude` starts. */
export const ROOT = "root";
/** The tab of a worktree's space with nvim open in the worktree, after the conversation's tab. */
export const EDITOR = "nvim";
/** drizzle studio's default port; a worktree's studio (`pnpm pg:studio --port`) moves it by its offset. */
export const STUDIO_PORT = 4983;
/** Storybook's default port (`pnpm storybook`); a worktree's moves by its offset like studio's. */
export const STORYBOOK_PORT = 6006;

/** The command line entry: what a command runs to call another one in a child process. */
export const ENTRY = path.join(import.meta.dirname, "..", "worktree.mjs");
