// GitLab in one request (#305): the open MRs with their head pipeline and
// every job of it, through GraphQL with glab's token. No process per call.

const PROJECT = "monocircuit/platform";
const QUERY =
  "query($p:ID!){project(fullPath:$p){mergeRequests(state:opened,first:50,sort:CREATED_DESC){nodes{iid title webUrl draft sourceBranch headPipeline{status path jobs(first:100){nodes{name status}}}}}}}";

export interface Job {
  name: string;
  status: string;
}
export interface Mr {
  iid: string;
  title: string;
  webUrl: string;
  draft: boolean;
  sourceBranch: string;
  headPipeline: { status: string; path: string; jobs: { nodes: Job[] } } | null;
}

export const FIXTURE = new URL("../../fixtures/merge-requests.json", import.meta.url).pathname;

/** The MRs of a GraphQL answer (live or recorded). */
export const parse = (json: string): Mr[] => {
  const answer = JSON.parse(json);
  if (answer.errors) throw new Error(`GitLab: ${JSON.stringify(answer.errors)}`);
  return answer.data.project.mergeRequests.nodes;
};

/** glab's token, asked once per run. */
export const token = (): string | null => {
  if (process.env.GITLAB_TOKEN) return process.env.GITLAB_TOKEN;
  const out = Bun.spawnSync(["glab", "config", "get", "token", "--host", "gitlab.com"]);
  return out.stdout.toString().trim() || null;
};

export type Source = { token: string } | { fixture: string };

export const read = async (source: Source): Promise<Mr[]> => {
  if ("fixture" in source) return parse(await Bun.file(source.fixture).text());
  const response = await fetch("https://gitlab.com/api/graphql", {
    method: "POST",
    headers: { authorization: `Bearer ${source.token}`, "content-type": "application/json" },
    body: JSON.stringify({ query: QUERY, variables: { p: PROJECT } }),
    signal: AbortSignal.timeout(20_000),
  });
  if (!response.ok) throw new Error(`GitLab answered ${response.status}`);
  return parse(await response.text());
};
