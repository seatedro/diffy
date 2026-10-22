use std::collections::HashMap;

use serde_json::{Value, json};

use super::api::{
    GitHubApi, PullRequestCheckSummary, bool_field, int_field, optional_string_field,
    parse_check_summary, string_field,
};
use super::overview::{PullRequestReviewer, actor, nodes};
use crate::core::error::{DiffyError, Result};

const SEARCH: &str = "is:pr is:open author:@me archived:false sort:updated-desc";
const EXCERPT_CHARS: usize = 280;

// One search with every field times out on GitHub's side for large
// monorepo PRs, so the diff stats and the check rollup are separate
// queries over the same search, run in parallel and joined by node id.
const CORE_QUERY: &str = r#"
query DiffyMyPullRequests($search: String!) {
  viewer { login }
  search(query: $search, type: ISSUE, first: 50) {
    nodes {
      ... on PullRequest {
        id
        number
        title
        url
        isDraft
        updatedAt
        bodyText
        repository { nameWithOwner }
        headRefName
        baseRefName
        reviewDecision
        latestReviews(first: 10) {
          nodes { state author { login avatarUrl } }
        }
        reviewRequests(first: 10) {
          nodes {
            requestedReviewer {
              __typename
              ... on User { login avatarUrl }
            }
          }
        }
        reviewThreads(first: 100) {
          nodes { isResolved }
        }
      }
    }
  }
}
"#;

const STATS_QUERY: &str = r#"
query DiffyMyPullRequestStats($search: String!) {
  search(query: $search, type: ISSUE, first: 50) {
    nodes {
      ... on PullRequest { id mergeable additions deletions changedFiles }
    }
  }
}
"#;

const CHECKS_QUERY: &str = r#"
query DiffyMyPullRequestChecks($search: String!) {
  search(query: $search, type: ISSUE, first: 50) {
    nodes {
      ... on PullRequest {
        id
        commits(last: 1) {
          nodes {
            commit {
              statusCheckRollup {
                state
                contexts(first: 10) {
                  totalCount
                  checkRunCountsByState { state count }
                  statusContextCountsByState { state count }
                  nodes {
                    __typename
                    ... on CheckRun { name status conclusion detailsUrl }
                    ... on StatusContext { context state targetUrl }
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}
"#;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullRequestStats {
    pub mergeable: String,
    pub additions: i32,
    pub deletions: i32,
    pub changed_files: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MyPullRequest {
    pub id: String,
    pub number: i32,
    pub title: String,
    pub url: String,
    pub repository: String,
    pub is_draft: bool,
    pub updated_at: String,
    pub head_branch: String,
    pub base_branch: String,
    pub review_decision: Option<String>,
    pub reviewers: Vec<PullRequestReviewer>,
    pub requested_reviewers: usize,
    pub unresolved_threads: usize,
    pub excerpt: String,
    pub stats: Option<PullRequestStats>,
    pub checks: Option<PullRequestCheckSummary>,
}

impl MyPullRequest {
    pub fn repo_key(&self) -> String {
        self.repository.to_ascii_lowercase()
    }
}

impl GitHubApi {
    pub fn fetch_my_pull_requests(&self) -> Result<Vec<MyPullRequest>> {
        if !self.has_token() {
            return Err(DiffyError::auth(
                "GitHub authentication is required to list your pull requests".to_owned(),
            ));
        }
        let variables = json!({ "search": SEARCH });
        let (core, stats, checks) = std::thread::scope(|scope| {
            let stats = scope.spawn(|| self.graphql_query(STATS_QUERY, variables.clone()));
            let checks = scope.spawn(|| self.graphql_query(CHECKS_QUERY, variables.clone()));
            let core = self.graphql_query(CORE_QUERY, variables.clone());
            (
                core,
                optional(stats.join(), "stats"),
                optional(checks.join(), "checks"),
            )
        });
        let mut pulls = parse_core(&core?);
        let stats = stats.as_ref().map(parse_stats).unwrap_or_default();
        let checks = checks.as_ref().map(parse_checks).unwrap_or_default();
        for pull in &mut pulls {
            pull.stats = stats.get(&pull.id).cloned();
            pull.checks = checks.get(&pull.id).cloned();
        }
        Ok(pulls)
    }
}

fn optional(joined: std::thread::Result<Result<Value>>, part: &str) -> Option<Value> {
    match joined {
        Ok(Ok(value)) => Some(value),
        Ok(Err(error)) => {
            tracing::warn!("your pull requests: {part} query failed: {error}");
            None
        }
        Err(_) => None,
    }
}

fn search_nodes(value: &Value) -> impl Iterator<Item = &Value> {
    nodes(value, "/data/search/nodes").filter(|node| !string_field(node, "id").is_empty())
}

fn parse_core(value: &Value) -> Vec<MyPullRequest> {
    let viewer = value
        .pointer("/data/viewer/login")
        .and_then(Value::as_str)
        .unwrap_or_default();
    search_nodes(value)
        .map(|node| parse_pull(node, viewer))
        .collect()
}

fn parse_pull(node: &Value, viewer: &str) -> MyPullRequest {
    let mut reviewers: Vec<PullRequestReviewer> = nodes(node, "/reviewRequests/nodes")
        .filter_map(|request| request.get("requestedReviewer"))
        .filter(|reviewer| string_field(reviewer, "__typename") == "User")
        .map(|reviewer| PullRequestReviewer {
            actor: actor(Some(reviewer)),
            state: "REQUESTED".to_owned(),
        })
        .filter(|reviewer| !reviewer.actor.login.is_empty())
        .collect();
    let requested_reviewers = reviewers.len();
    for review in nodes(node, "/latestReviews/nodes") {
        let actor = actor(review.get("author"));
        let state = string_field(review, "state");
        if actor.login.is_empty()
            || state == "PENDING"
            || actor.login.eq_ignore_ascii_case(viewer)
            || reviewers.iter().any(|r| r.actor.login == actor.login)
        {
            continue;
        }
        reviewers.push(PullRequestReviewer { actor, state });
    }
    MyPullRequest {
        id: string_field(node, "id"),
        number: int_field(node, "number") as i32,
        title: string_field(node, "title"),
        url: string_field(node, "url"),
        repository: node
            .pointer("/repository/nameWithOwner")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        is_draft: bool_field(node, "isDraft"),
        updated_at: string_field(node, "updatedAt"),
        head_branch: string_field(node, "headRefName"),
        base_branch: string_field(node, "baseRefName"),
        review_decision: optional_string_field(node, "reviewDecision"),
        reviewers,
        requested_reviewers,
        unresolved_threads: nodes(node, "/reviewThreads/nodes")
            .filter(|thread| !bool_field(thread, "isResolved"))
            .count(),
        excerpt: excerpt(&string_field(node, "bodyText")),
        stats: None,
        checks: None,
    }
}

fn parse_stats(value: &Value) -> HashMap<String, PullRequestStats> {
    search_nodes(value)
        .map(|node| {
            (
                string_field(node, "id"),
                PullRequestStats {
                    mergeable: string_field(node, "mergeable"),
                    additions: int_field(node, "additions") as i32,
                    deletions: int_field(node, "deletions") as i32,
                    changed_files: int_field(node, "changedFiles") as i32,
                },
            )
        })
        .collect()
}

fn parse_checks(value: &Value) -> HashMap<String, PullRequestCheckSummary> {
    search_nodes(value)
        .filter_map(|node| {
            let rollup = node
                .pointer("/commits/nodes/0/commit/statusCheckRollup")
                .filter(|rollup| !rollup.is_null())?;
            Some((string_field(node, "id"), parse_rollup(rollup)))
        })
        .collect()
}

fn parse_rollup(rollup: &Value) -> PullRequestCheckSummary {
    let mut summary = parse_check_summary(rollup);
    let counts = nodes(rollup, "/contexts/checkRunCountsByState")
        .chain(nodes(rollup, "/contexts/statusContextCountsByState"))
        .collect::<Vec<_>>();
    if !counts.is_empty() {
        summary.success_count = 0;
        summary.failure_count = 0;
        summary.pending_count = 0;
        for count in counts {
            summary.add_state(
                &string_field(count, "state"),
                int_field(count, "count") as i32,
            );
        }
    }
    summary
}

fn excerpt(body: &str) -> String {
    let mut out = String::new();
    for (index, word) in body.split_whitespace().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(word);
        if out.chars().count() > EXCERPT_CHARS {
            let cut: String = out.chars().take(EXCERPT_CHARS).collect();
            return format!("{}\u{2026}", cut.trim_end());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_stats_and_checks_merge_by_node_id() {
        let core = json!({ "data": {
            "viewer": { "login": "me" },
            "search": { "nodes": [
                {
                    "id": "PR_1", "number": 12, "title": "Tidy the widget",
                    "url": "https://github.com/owner/repo/pull/12", "isDraft": false,
                    "updatedAt": "2026-01-02T00:00:00Z", "bodyText": "Line one\n\nline two",
                    "repository": { "nameWithOwner": "Owner/Repo" },
                    "headRefName": "feature", "baseRefName": "main",
                    "reviewDecision": "CHANGES_REQUESTED",
                    "latestReviews": { "nodes": [
                        { "state": "CHANGES_REQUESTED", "author": { "login": "alpha" } },
                        { "state": "COMMENTED", "author": { "login": "me" } }
                    ] },
                    "reviewRequests": { "nodes": [
                        { "requestedReviewer": { "__typename": "User", "login": "bravo" } },
                        { "requestedReviewer": { "__typename": "Team" } }
                    ] },
                    "reviewThreads": { "nodes": [
                        { "isResolved": false }, { "isResolved": true }, { "isResolved": false }
                    ] }
                },
                {}
            ] }
        } });
        let stats = json!({ "data": { "search": { "nodes": [
            { "id": "PR_1", "mergeable": "CONFLICTING", "additions": 5, "deletions": 2, "changedFiles": 1 }
        ] } } });
        let checks = json!({ "data": { "search": { "nodes": [
            { "id": "PR_1", "commits": { "nodes": [ { "commit": { "statusCheckRollup": {
                "state": "FAILURE",
                "contexts": {
                    "totalCount": 30,
                    "checkRunCountsByState": [
                        { "state": "SUCCESS", "count": 20 },
                        { "state": "FAILURE", "count": 2 },
                        { "state": "IN_PROGRESS", "count": 7 }
                    ],
                    "statusContextCountsByState": [ { "state": "SUCCESS", "count": 1 } ],
                    "nodes": [ { "__typename": "CheckRun", "name": "lint", "conclusion": "FAILURE" } ]
                }
            } } } ] } }
        ] } } });

        let mut pulls = parse_core(&core);
        assert_eq!(pulls.len(), 1);
        let pull = &mut pulls[0];
        pull.stats = parse_stats(&stats).remove(&pull.id);
        pull.checks = parse_checks(&checks).remove(&pull.id);

        assert_eq!(pull.repo_key(), "owner/repo");
        assert_eq!(pull.excerpt, "Line one line two");
        assert_eq!(pull.unresolved_threads, 2);
        assert_eq!(pull.requested_reviewers, 1);
        let reviewers: Vec<(&str, &str)> = pull
            .reviewers
            .iter()
            .map(|r| (r.actor.login.as_str(), r.state.as_str()))
            .collect();
        assert_eq!(
            reviewers,
            [("bravo", "REQUESTED"), ("alpha", "CHANGES_REQUESTED")]
        );
        assert_eq!(pull.stats.as_ref().unwrap().mergeable, "CONFLICTING");
        let checks = pull.checks.as_ref().unwrap();
        assert_eq!(
            (
                checks.total_count,
                checks.success_count,
                checks.failure_count,
                checks.pending_count
            ),
            (30, 21, 2, 7)
        );
        assert_eq!(checks.contexts[0].name, "lint");
    }

    #[test]
    fn excerpt_collapses_whitespace_and_truncates() {
        assert_eq!(excerpt("  a\n\n b  "), "a b");
        let long = "word ".repeat(100);
        let cut = excerpt(&long);
        assert!(cut.ends_with('\u{2026}'));
        assert!(cut.chars().count() <= EXCERPT_CHARS + 1);
    }
}
