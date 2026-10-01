// The view: a status line and a scroll box of sections. Every block and
// button is a renderable with its own mouse handlers, so a click hits what
// is drawn under it (#303) and hover lights the whole block (#296); flexbox
// wraps titles and jobs (#304); each section builds inside its own try (#306).

import {
  BoxRenderable,
  type CliRenderer,
  type Renderable,
  ScrollBoxRenderable,
  TextRenderable,
  bold,
  dim,
  fg,
  t,
  underline,
} from "@opentui/core";
import type { Job, Mr } from "./gitlab";

export type Action =
  | { kind: "open"; url: string }
  | { kind: "pipeline"; url: string }
  | { kind: "toggleJobs"; iid: string }
  | { kind: "select"; iid: string };

export interface State {
  mrs: Mr[] | null;
  error: string | null;
  readAt: number | null;
  readTook: number | null;
  isReading: boolean;
  folded: Set<string>;
  selected: string | null;
  /** Every action a click ran, for the tests and the status line. */
  ran: Action[];
}

export const newState = (): State => ({
  mrs: null,
  error: null,
  readAt: null,
  readTook: null,
  isReading: false,
  folded: new Set(),
  selected: null,
  ran: [],
});

const HOVER = "#282c34";
const HOVER_BUTTON = "#464e60";
const TITLE_COLUMN = 10;

const MARKS: Record<string, [string, string]> = {
  SUCCESS: ["✓", "#98c379"],
  FAILED: ["✗", "#e06c75"],
  RUNNING: ["●", "#e5c07b"],
};
const WAITING = new Set(["PENDING", "CREATED", "WAITING_FOR_RESOURCE", "PREPARING"]);
const mark = (status: string) => {
  const [symbol, color] = MARKS[status] ?? [WAITING.has(status) ? "○" : "·", "#5c6370"];
  return fg(color)(symbol);
};
/** Failed jobs first, then running and waiting ones, then the rest (#304). */
const rank = (job: Job) =>
  job.status === "FAILED" ? 0 : job.status === "RUNNING" ? 1 : WAITING.has(job.status) ? 2 : 3;

export class View {
  readonly status: TextRenderable;
  readonly body: ScrollBoxRenderable;

  constructor(
    readonly renderer: CliRenderer,
    readonly state: State,
    /** Runs an action's effect outside the view (a browser), after the state took it. */
    readonly effect: (action: Action) => void = () => {},
  ) {
    const root = new BoxRenderable(renderer, { flexDirection: "column", width: "100%", height: "100%" });
    this.status = new TextRenderable(renderer, { content: "", height: 1 });
    root.add(this.status);
    root.add(new BoxRenderable(renderer, { height: 1 }));
    this.body = new ScrollBoxRenderable(renderer, { flexGrow: 1, scrollY: true });
    root.add(this.body);
    renderer.root.add(root);
    this.update();
  }

  run(action: Action) {
    const { state } = this;
    if (action.kind === "toggleJobs") {
      if (!state.folded.delete(action.iid)) state.folded.add(action.iid);
    } else if (action.kind === "select") state.selected = action.iid;
    state.ran.push(action);
    this.effect(action);
    this.update();
  }

  /** Builds the sections again from the state. */
  update() {
    this.status.content = this.statusLine();
    const top = this.body.scrollTop;
    for (const child of this.body.content.getChildren()) child.destroyRecursively();
    const sections: [string, () => Renderable[]][] = [
      ["MERGE REQUESTS", () => this.mergeRequests()],
      ["BROKEN", () => this.broken()],
    ];
    for (const [name, build] of sections) {
      let parts: Renderable[];
      try {
        parts = build();
      } catch (error) {
        parts = this.failed(name, error instanceof Error ? error.message : String(error));
      }
      for (const part of parts) this.body.add(part);
    }
    this.body.scrollTop = top;
    this.renderer.requestRender();
  }

  statusLine() {
    const { state } = this;
    const count = state.mrs ? String(state.mrs.length) : "…";
    const gitlab = state.isReading
      ? "GitLab: reading…"
      : state.error
        ? `GitLab failed: ${state.error}`
        : state.readAt
          ? `GitLab: read in ${state.readTook} ms, ${Math.round((Date.now() - state.readAt) / 1000)} s ago`
          : "GitLab: not read yet";
    const last = state.ran.at(-1);
    return t`${bold(" breadboard spike · opentui")}${dim(`  ${count} open  ${gitlab}`)}${
      last ? fg("#e5c07b")(`  last click: ${JSON.stringify(last)}`) : ""
    }`;
  }

  heading(name: string, note: string) {
    return new TextRenderable(this.renderer, { content: t` ${bold(name)}${dim(`  ${note}`)}`, height: 1 });
  }

  failed(name: string, why: string) {
    const box = new BoxRenderable(this.renderer, { paddingLeft: 3 });
    box.add(new TextRenderable(this.renderer, { content: t`${fg("#e06c75")(`⚠ ${why}`)}`, wrapMode: "word" }));
    return [this.heading(name, "failed to draw"), box];
  }

  button(label: string, action: Action) {
    const button = new BoxRenderable(this.renderer, {
      id: `button:${label}:${JSON.stringify(action)}`,
      flexShrink: 0,
      onMouseDown: event => {
        event.stopPropagation();
        this.run(action);
      },
      onMouseOver() {
        this.backgroundColor = HOVER_BUTTON;
      },
      onMouseOut() {
        this.backgroundColor = "transparent";
      },
    });
    button.add(new TextRenderable(this.renderer, { content: t`${fg("#61afef")(`[${label}]`)}` }));
    return button;
  }

  mergeRequests(): Renderable[] {
    const { renderer, state } = this;
    if (!state.mrs) return [this.heading("MERGE REQUESTS", "reading GitLab…")];
    const blocks = state.mrs.map(mr => {
      const block = new BoxRenderable(renderer, {
        id: `block:${mr.iid}`,
        flexDirection: "column",
        width: "100%",
        onMouseDown: () => this.run({ kind: "select", iid: mr.iid }),
        onMouseOver() {
          this.backgroundColor = HOVER;
        },
        onMouseOut() {
          this.backgroundColor = "transparent";
        },
      });
      const status = mr.headPipeline?.status ?? "NONE";
      const head = new BoxRenderable(renderer, { flexDirection: "row" });
      head.add(new TextRenderable(renderer, { content: t`  ${mark(status)} ${fg("#56b6c2")(`!${mr.iid}`)}`, width: TITLE_COLUMN, flexShrink: 0 }));
      const isSelected = state.selected === mr.iid;
      head.add(
        new TextRenderable(renderer, {
          content: t`${isSelected ? underline(bold(mr.title)) : bold(mr.title)}`,
          wrapMode: "word",
          flexGrow: 1,
          flexShrink: 1,
        }),
      );
      block.add(head);

      const jobs = mr.headPipeline?.jobs.nodes ?? [];
      const done = jobs.filter(job => job.status === "SUCCESS").length;
      const meta = new BoxRenderable(renderer, { flexDirection: "row", flexWrap: "wrap", columnGap: 1, paddingLeft: TITLE_COLUMN });
      const draft = mr.draft ? "draft · " : "";
      meta.add(
        new TextRenderable(renderer, {
          content: t`${dim(`${draft}${status.toLowerCase()} · ${done}/${jobs.length} jobs · ${mr.sourceBranch}`)}`,
          wrapMode: "word",
          flexShrink: 1,
        }),
      );
      meta.add(this.button("open", { kind: "open", url: mr.webUrl }));
      if (mr.headPipeline) meta.add(this.button("pipeline", { kind: "pipeline", url: `https://gitlab.com${mr.headPipeline.path}` }));
      const isFolded = state.folded.has(mr.iid);
      if (jobs.length) meta.add(this.button(isFolded ? "jobs ▸" : "jobs ▾", { kind: "toggleJobs", iid: mr.iid }));
      block.add(meta);

      if (!isFolded && jobs.length) {
        const list = new BoxRenderable(renderer, { flexDirection: "row", flexWrap: "wrap", columnGap: 2, paddingLeft: TITLE_COLUMN });
        for (const job of jobs.toSorted((a, b) => rank(a) - rank(b)))
          list.add(new TextRenderable(renderer, { content: t`${mark(job.status)} ${job.name}`, flexShrink: 0 }));
        block.add(list);
      }
      return block;
    });
    return [this.heading("MERGE REQUESTS", `${state.mrs.length} open`), ...blocks];
  }

  /** Throws on every draw, the way #306 did: `undefined.branch`. */
  broken(): Renderable[] {
    const issue = undefined as unknown as { branch: string };
    return [this.heading("BROKEN", issue.branch)];
  }
}
