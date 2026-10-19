//! What a GraphQL query costs on gitlab.com (#454): GitLab refuses one above
//! LIMIT before it reads anything, so a query that grew past it leaves every
//! overview without its MRs. Measured against gitlab.com's `queryComplexity`
//! (2026-10-19): a field costs 1 and its fields; a connection (a field with
//! `nodes`) `(fields + 2) × (1 + first/100)`, `first` at most and by default
//! 100, rounded down; a few fields cost more (COSTLY). An estimate a little
//! above GitLab's, never below.

/// gitlab.com's limit for a signed-in user.
pub const LIMIT: u32 = 250;
/// What the tests allow a query: room for a field or two before LIMIT.
pub const ALLOWED: u32 = 200;

/// Fields GitLab prices above 1, as measured.
const COSTLY: [(&str, u32); 3] = [("mergedAt", 5), ("detailedMergeStatus", 2), ("diffHeadSha", 2)];

/// The complexity of `query` as GitLab counts it, its own `query(…)` header aside.
pub fn complexity(query: &str) -> u32 {
    let tokens = tokens(query);
    let Some(start) = tokens.iter().position(|token| token == "{") else {
        return 0;
    };
    let mut at = start + 1;
    selection(&tokens, &mut at).0
}

/// Names, numbers, strings (quotes kept) and punctuation.
fn tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = query.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() || c == ',' {
            continue;
        }
        if c == '"' {
            let mut text = String::from('"');
            while let Some(next) = chars.next() {
                text.push(next);
                match next {
                    '\\' => text.extend(chars.next()),
                    '"' => break,
                    _ => {}
                }
            }
            tokens.push(text);
        } else if c.is_alphanumeric() || c == '_' || c == '$' {
            let mut word = String::from(c);
            while let Some(next) = chars.peek().filter(|next| next.is_alphanumeric() || **next == '_') {
                word.push(*next);
                chars.next();
            }
            tokens.push(word);
        } else {
            tokens.push(c.to_string());
        }
    }
    tokens
}

/// The cost of the fields up to the `}` that closes them, and their names; `at` past it.
fn selection(tokens: &[String], at: &mut usize) -> (u32, Vec<String>) {
    let (mut total, mut names) = (0, Vec::new());
    while let Some(token) = tokens.get(*at) {
        *at += 1;
        if token == "}" {
            break;
        }
        // `alias: name`: the name prices it.
        let mut name = token.clone();
        if tokens.get(*at).is_some_and(|next| next == ":") {
            name = tokens.get(*at + 1).cloned().unwrap_or_default();
            *at += 2;
        }
        let mut first = None;
        if tokens.get(*at).is_some_and(|next| next == "(") {
            let mut depth = 0;
            while let Some(token) = tokens.get(*at) {
                *at += 1;
                match token.as_str() {
                    "(" | "[" => depth += 1,
                    ")" | "]" => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    "first" if depth == 1 => first = tokens.get(*at + 1).and_then(|value| value.parse().ok()),
                    _ => {}
                }
            }
        }
        let children = tokens.get(*at).is_some_and(|next| next == "{").then(|| {
            *at += 1;
            selection(tokens, at)
        });
        total += cost(&name, first, children);
        names.push(name);
    }
    (total, names)
}

fn cost(name: &str, first: Option<u32>, children: Option<(u32, Vec<String>)>) -> u32 {
    let Some((inner, names)) = children else {
        return COSTLY
            .iter()
            .find(|(costly, _)| *costly == name)
            .map_or(1, |(_, cost)| *cost);
    };
    if !names.iter().any(|child| child == "nodes") {
        return 1 + inner;
    }
    let first = first.unwrap_or(100).min(100);
    (inner + 2) * (100 + first) / 100
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gitlab.com's `queryComplexity` of each, its own two fields taken off.
    #[test]
    fn prices_queries_as_gitlab_did() {
        for (query, gitlab) in [
            ("{ project(fullPath: $p) { id } }", 2),
            (
                "{ project(fullPath: $p) { a: pipelines(first: 1) { nodes { id } } } }",
                5,
            ),
            (
                "{ project(fullPath: $p) { pipelines(first: 50) { nodes { id iid } } } }",
                8,
            ),
            (
                "{ project(fullPath: $p) { mergeRequests(first: 100) { nodes { iid } pageInfo { hasNextPage endCursor } } } }",
                15,
            ),
            (
                "{ project(fullPath: $p) { mergeRequests(first: 100) { nodes { iid headPipeline { id } } } } }",
                13,
            ),
            (
                "{ project(fullPath: $p) { mergeRequests(first: 100) { nodes { iid detailedMergeStatus } } } }",
                13,
            ),
        ] {
            assert_eq!(complexity(query), gitlab, "{query}");
        }
    }

    /// Nested connections and fields GitLab prices lower: the estimate may be above, never below.
    #[test]
    fn never_prices_below_gitlab() {
        for (query, gitlab) in [
            (
                "{ project(fullPath: $p) { pipelines(first: 1) { nodes { id jobs(first: 100) { nodes { id name status webPath finishedAt } } } } } }",
                19,
            ),
            (
                "{ project(fullPath: $p) { mergeRequests(first: 100) { nodes { iid labels { nodes { title } } } } } }",
                23,
            ),
            (
                "{ project(fullPath: $p) { issues(first: 100) { nodes { iid blockedByIssues { nodes { iid } } } } } }",
                23,
            ),
            (
                "{ project(fullPath: $p) { mergeRequests(first: 100) { nodes { iid pipelines(first: 20) { nodes { id } } } } } }",
                17,
            ),
        ] {
            let estimate = complexity(query);
            assert!(
                estimate >= gitlab && estimate <= gitlab + gitlab / 5,
                "{estimate} for {gitlab}: {query}"
            );
        }
    }

    /// The query of the outage (#454): GitLab answered `complexity of 281`.
    #[test]
    fn would_have_refused_the_query_of_the_outage() {
        let pipeline = "id iid status path sha ref createdAt updatedAt finishedAt jobs(first: 100) { nodes { id name status webPath finishedAt } }";
        let mr = format!(
            "iid title description state draft webUrl sourceBranch targetBranch mergedAt conflicts detailedMergeStatus diffHeadSha mergeCommitSha headPipeline {{ {pipeline} }}"
        );
        let query = format!(
            "query($project: ID!) {{ project(fullPath: $project) {{ develop: pipelines(ref: \"develop\", first: 5) {{ nodes {{ {pipeline} }} }} \
             epicPipelines: pipelines(ref: $epic, first: 5) {{ nodes {{ {pipeline} }} }} \
             epicMrs: mergeRequests(sourceBranches: $epics, state: opened, first: 10) {{ nodes {{ {mr} }} }} \
             epicTasks: mergeRequests(targetBranches: $t, state: merged, first: 100, sort: MERGED_AT_DESC) {{ nodes {{ iid title webUrl sourceBranch mergedAt labels {{ nodes {{ title }} }} }} }} \
             mrs: mergeRequests(sourceBranches: $branches, first: 100, sort: UPDATED_DESC) {{ nodes {{ {mr} }} pageInfo {{ hasNextPage endCursor }} }} \
             issues(iids: $iids, state: all, first: 100) {{ nodes {{ iid title description state webUrl }} }} \
             awaited: mergeRequests(iids: $awaited, first: 100) {{ nodes {{ iid state }} }} }} }}"
        );
        assert!(complexity(&query) > LIMIT, "{}", complexity(&query));
    }
}
