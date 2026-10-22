use serde_json::{Value, json};

use super::api::{
    GitHubApi, PullRequestCheckSummary, PullRequestLabel, bool_field, int_field,
    optional_string_field, parse_check_summary, parse_labels, string_field,
};
use crate::core::error::{DiffyError, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullRequestActor {
    pub login: String,
    pub avatar_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullRequestReviewer {
    pub actor: PullRequestActor,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullRequestTimelineKind {
    Comment,
    Review { state: String, comments: i32 },
    Commit { short_oid: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestTimelineItem {
    pub id: String,
    pub kind: PullRequestTimelineKind,
    pub author: PullRequestActor,
    pub body: String,
    pub created_at: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullRequestOverview {
    pub title: String,
    pub body: String,
    pub url: String,
    pub state: String,
    pub is_draft: bool,
    pub author: PullRequestActor,
    pub created_at: String,
    pub updated_at: String,
    pub base_branch: String,
    pub head_branch: String,
    pub additions: i32,
    pub deletions: i32,
    pub changed_files: i32,
    pub commit_count: i32,
    pub mergeable: String,
    pub merge_state_status: String,
    pub review_decision: Option<String>,
    pub labels: Vec<PullRequestLabel>,
    pub reviewers: Vec<PullRequestReviewer>,
    pub team_reviewers: usize,
    pub checks: PullRequestCheckSummary,
    pub timeline: Vec<PullRequestTimelineItem>,
    pub timeline_total: usize,
    pub detailed: bool,
}

const PULL_REQUEST_OVERVIEW_QUERY: &str = r#"
query DiffyPullRequestOverview($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      title
      body
      url
      state
      isDraft
      createdAt
      updatedAt
      author { login avatarUrl }
      baseRefName
      headRefName
      additions
      deletions
      changedFiles
      mergeable
      mergeStateStatus
      reviewDecision
      commits(last: 1) {
        totalCount
        nodes {
          commit {
            statusCheckRollup {
              state
              contexts(first: 100) {
                totalCount
                nodes {
                  __typename
                  ... on CheckRun {
                    name
                    status
                    conclusion
                    detailsUrl
                  }
                  ... on StatusContext {
                    context
                    state
                    targetUrl
                  }
                }
              }
            }
          }
        }
      }
      labels(first: 50) {
        nodes {
          name
          color
        }
      }
      reviewRequests(first: 50) {
        nodes {
          requestedReviewer {
            __typename
            ... on User {
              login
              avatarUrl
            }
          }
        }
      }
      latestOpinionatedReviews(first: 50) {
        nodes {
          state
          author { login avatarUrl }
        }
      }
      latestReviews(first: 50) {
        nodes {
          state
          author { login avatarUrl }
        }
      }
      timelineItems(last: 100, itemTypes: [ISSUE_COMMENT, PULL_REQUEST_REVIEW, PULL_REQUEST_COMMIT]) {
        filteredCount
        nodes {
          __typename
          ... on IssueComment {
            id
            body
            createdAt
            url
            author { login avatarUrl }
          }
          ... on PullRequestReview {
            id
            state
            body
            createdAt
            submittedAt
            url
            author { login avatarUrl }
            comments { totalCount }
          }
          ... on PullRequestCommit {
            id
            url
            commit {
              abbreviatedOid
              messageHeadline
              committedDate
              author {
                name
                avatarUrl
                user { login }
              }
            }
          }
        }
      }
    }
  }
}
"#;

impl GitHubApi {
    pub fn fetch_pull_request_overview(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
    ) -> Result<PullRequestOverview> {
        if !self.has_token() {
            return self.fetch_public_pull_request_overview(owner, repo, number);
        }
        let value = self.graphql_request(
            PULL_REQUEST_OVERVIEW_QUERY,
            json!({
                "owner": owner,
                "repo": repo,
                "number": number,
            }),
        )?;
        let pull_request = value
            .pointer("/data/repository/pullRequest")
            .filter(|value| !value.is_null())
            .ok_or_else(|| {
                DiffyError::Parse("missing pull request in GraphQL response".to_owned())
            })?;
        Ok(parse_overview(pull_request))
    }

    pub fn add_pull_request_comment(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
        body: &str,
    ) -> Result<()> {
        if !self.has_token() {
            return Err(DiffyError::auth(
                "GitHub authentication is required to comment on pull requests".to_owned(),
            ));
        }
        self.post_json(
            &format!("https://api.github.com/repos/{owner}/{repo}/issues/{number}/comments"),
            &json!({ "body": body }),
            "GitHub pull request comment",
        )?;
        Ok(())
    }

    fn fetch_public_pull_request_overview(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
    ) -> Result<PullRequestOverview> {
        let base = format!("https://api.github.com/repos/{owner}/{repo}");
        let pull = self.get_json(
            &format!("{base}/pulls/{number}"),
            "GitHub pull request fetch",
        )?;
        let comments = self.get_json(
            &format!("{base}/issues/{number}/comments?per_page=100"),
            "GitHub pull request comments fetch",
        )?;
        Ok(parse_public_overview(&pull, &comments))
    }
}

fn actor(value: Option<&Value>) -> PullRequestActor {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return PullRequestActor::default();
    };
    PullRequestActor {
        login: string_field(value, "login"),
        avatar_url: optional_string_field(value, "avatarUrl")
            .or_else(|| optional_string_field(value, "avatar_url"))
            .unwrap_or_default(),
    }
}

fn nodes<'a>(value: &'a Value, pointer: &str) -> impl Iterator<Item = &'a Value> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn parse_overview(value: &Value) -> PullRequestOverview {
    let author = actor(value.get("author"));
    let check_rollup = value
        .pointer("/commits/nodes/0/commit/statusCheckRollup")
        .unwrap_or(&Value::Null);
    let mut reviewers: Vec<PullRequestReviewer> = Vec::new();
    let mut team_reviewers = 0;
    for reviewer in nodes(value, "/reviewRequests/nodes").filter_map(|n| n.get("requestedReviewer"))
    {
        match string_field(reviewer, "__typename").as_str() {
            "Team" => team_reviewers += 1,
            _ => {
                let actor = actor(Some(reviewer));
                if !actor.login.is_empty() {
                    reviewers.push(PullRequestReviewer {
                        actor,
                        state: "REQUESTED".to_owned(),
                    });
                }
            }
        }
    }
    let reviews =
        nodes(value, "/latestOpinionatedReviews/nodes").chain(nodes(value, "/latestReviews/nodes"));
    for review in reviews {
        let actor = actor(review.get("author"));
        let state = string_field(review, "state");
        if actor.login.is_empty()
            || state == "PENDING"
            || actor.login == author.login
            || reviewers.iter().any(|r| r.actor.login == actor.login)
        {
            continue;
        }
        reviewers.push(PullRequestReviewer { actor, state });
    }
    let timeline = nodes(value, "/timelineItems/nodes")
        .filter_map(parse_timeline_item)
        .collect::<Vec<_>>();
    let timeline_total = value
        .pointer("/timelineItems/filteredCount")
        .and_then(Value::as_u64)
        .map_or(timeline.len(), |count| count as usize);
    PullRequestOverview {
        title: string_field(value, "title"),
        body: string_field(value, "body"),
        url: string_field(value, "url"),
        state: string_field(value, "state"),
        is_draft: bool_field(value, "isDraft"),
        author,
        created_at: string_field(value, "createdAt"),
        updated_at: string_field(value, "updatedAt"),
        base_branch: string_field(value, "baseRefName"),
        head_branch: string_field(value, "headRefName"),
        additions: int_field(value, "additions") as i32,
        deletions: int_field(value, "deletions") as i32,
        changed_files: int_field(value, "changedFiles") as i32,
        commit_count: value
            .pointer("/commits/totalCount")
            .and_then(Value::as_i64)
            .unwrap_or_default() as i32,
        mergeable: string_field(value, "mergeable"),
        merge_state_status: string_field(value, "mergeStateStatus"),
        review_decision: optional_string_field(value, "reviewDecision"),
        labels: parse_labels(value.pointer("/labels/nodes")),
        reviewers,
        team_reviewers,
        checks: parse_check_summary(check_rollup),
        timeline_total: timeline_total.max(timeline.len()),
        timeline,
        detailed: true,
    }
}

fn parse_timeline_item(value: &Value) -> Option<PullRequestTimelineItem> {
    let id = string_field(value, "id");
    let url = string_field(value, "url");
    match string_field(value, "__typename").as_str() {
        "IssueComment" => Some(PullRequestTimelineItem {
            id,
            kind: PullRequestTimelineKind::Comment,
            author: actor(value.get("author")),
            body: string_field(value, "body"),
            created_at: string_field(value, "createdAt"),
            url,
        }),
        "PullRequestReview" => {
            let state = string_field(value, "state");
            if state == "PENDING" {
                return None;
            }
            Some(PullRequestTimelineItem {
                id,
                kind: PullRequestTimelineKind::Review {
                    state,
                    comments: value
                        .pointer("/comments/totalCount")
                        .and_then(Value::as_i64)
                        .unwrap_or_default() as i32,
                },
                author: actor(value.get("author")),
                body: string_field(value, "body"),
                created_at: optional_string_field(value, "submittedAt")
                    .unwrap_or_else(|| string_field(value, "createdAt")),
                url,
            })
        }
        "PullRequestCommit" => {
            let commit = value.get("commit")?;
            let git_author = commit.get("author").unwrap_or(&Value::Null);
            let login = git_author
                .pointer("/user/login")
                .and_then(Value::as_str)
                .filter(|login| !login.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| string_field(git_author, "name"));
            Some(PullRequestTimelineItem {
                id,
                kind: PullRequestTimelineKind::Commit {
                    short_oid: string_field(commit, "abbreviatedOid"),
                },
                author: PullRequestActor {
                    login,
                    avatar_url: string_field(git_author, "avatarUrl"),
                },
                body: string_field(commit, "messageHeadline"),
                created_at: string_field(commit, "committedDate"),
                url,
            })
        }
        _ => None,
    }
}

fn parse_public_overview(pull: &Value, comments: &Value) -> PullRequestOverview {
    let state = if bool_field(pull, "merged") || !string_field(pull, "merged_at").is_empty() {
        "MERGED".to_owned()
    } else {
        string_field(pull, "state").to_ascii_uppercase()
    };
    let mergeable = match pull.get("mergeable").and_then(Value::as_bool) {
        Some(true) => "MERGEABLE",
        Some(false) => "CONFLICTING",
        None => "UNKNOWN",
    };
    let reviewers = nodes(pull, "/requested_reviewers")
        .map(|user| PullRequestReviewer {
            actor: actor(Some(user)),
            state: "REQUESTED".to_owned(),
        })
        .filter(|reviewer| !reviewer.actor.login.is_empty())
        .collect();
    let timeline: Vec<PullRequestTimelineItem> = comments
        .as_array()
        .into_iter()
        .flatten()
        .map(|comment| PullRequestTimelineItem {
            id: optional_string_field(comment, "node_id")
                .unwrap_or_else(|| int_field(comment, "id").to_string()),
            kind: PullRequestTimelineKind::Comment,
            author: actor(comment.get("user")),
            body: string_field(comment, "body"),
            created_at: string_field(comment, "created_at"),
            url: string_field(comment, "html_url"),
        })
        .collect();
    PullRequestOverview {
        title: string_field(pull, "title"),
        body: string_field(pull, "body"),
        url: string_field(pull, "html_url"),
        state,
        is_draft: bool_field(pull, "draft"),
        author: actor(pull.get("user")),
        created_at: string_field(pull, "created_at"),
        updated_at: string_field(pull, "updated_at"),
        base_branch: pull
            .pointer("/base/ref")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        head_branch: pull
            .pointer("/head/ref")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        additions: int_field(pull, "additions") as i32,
        deletions: int_field(pull, "deletions") as i32,
        changed_files: int_field(pull, "changed_files") as i32,
        commit_count: int_field(pull, "commits") as i32,
        mergeable: mergeable.to_owned(),
        merge_state_status: string_field(pull, "mergeable_state").to_ascii_uppercase(),
        review_decision: None,
        labels: nodes(pull, "/labels")
            .map(|label| PullRequestLabel {
                name: string_field(label, "name"),
                color: string_field(label, "color"),
            })
            .filter(|label| !label.name.is_empty())
            .collect(),
        reviewers,
        team_reviewers: nodes(pull, "/requested_teams").count(),
        checks: PullRequestCheckSummary::default(),
        timeline_total: timeline.len(),
        timeline,
        detailed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphql_overview_merges_reviewers_and_orders_the_timeline() {
        let value = json!({
            "title": "Tidy the widget",
            "body": "Body text",
            "state": "OPEN",
            "author": { "login": "author", "avatarUrl": "https://avatars.test/a" },
            "baseRefName": "main",
            "headRefName": "feature",
            "commits": { "totalCount": 2, "nodes": [] },
            "reviewRequests": { "nodes": [
                { "requestedReviewer": { "__typename": "User", "login": "alpha", "avatarUrl": "" } },
                { "requestedReviewer": { "__typename": "Team" } }
            ] },
            "latestOpinionatedReviews": { "nodes": [
                { "state": "APPROVED", "author": { "login": "bravo" } },
                { "state": "CHANGES_REQUESTED", "author": { "login": "alpha" } }
            ] },
            "latestReviews": { "nodes": [
                { "state": "COMMENTED", "author": { "login": "charlie" } },
                { "state": "COMMENTED", "author": { "login": "author" } },
                { "state": "COMMENTED", "author": { "login": "bravo" } }
            ] },
            "timelineItems": { "filteredCount": 5, "nodes": [
                { "__typename": "PullRequestCommit", "id": "c1", "commit": {
                    "abbreviatedOid": "abc1234", "messageHeadline": "Add widget",
                    "committedDate": "2026-01-01T00:00:00Z",
                    "author": { "name": "Author Name", "user": null } } },
                { "__typename": "IssueComment", "id": "i1", "body": "Looks fine",
                  "createdAt": "2026-01-02T00:00:00Z", "author": { "login": "charlie" } },
                { "__typename": "PullRequestReview", "id": "r1", "state": "PENDING", "body": "" },
                { "__typename": "PullRequestReview", "id": "r2", "state": "APPROVED", "body": "",
                  "submittedAt": "2026-01-03T00:00:00Z", "comments": { "totalCount": 2 },
                  "author": { "login": "bravo" } }
            ] }
        });

        let overview = parse_overview(&value);

        let reviewers: Vec<(&str, &str)> = overview
            .reviewers
            .iter()
            .map(|r| (r.actor.login.as_str(), r.state.as_str()))
            .collect();
        assert_eq!(
            reviewers,
            [
                ("alpha", "REQUESTED"),
                ("bravo", "APPROVED"),
                ("charlie", "COMMENTED")
            ]
        );
        assert_eq!(overview.team_reviewers, 1);
        assert_eq!(overview.commit_count, 2);
        assert_eq!(overview.timeline.len(), 3);
        assert_eq!(overview.timeline_total, 5);
        assert_eq!(overview.timeline[0].author.login, "Author Name");
        assert_eq!(
            overview.timeline[2].kind,
            PullRequestTimelineKind::Review {
                state: "APPROVED".to_owned(),
                comments: 2
            }
        );
        assert!(overview.detailed);
    }

    #[test]
    fn public_overview_maps_rest_fields() {
        let pull = json!({
            "title": "Tidy the widget",
            "body": null,
            "state": "closed",
            "merged": true,
            "draft": false,
            "user": { "login": "author", "avatar_url": "https://avatars.test/a" },
            "base": { "ref": "main" },
            "head": { "ref": "feature" },
            "mergeable": null,
            "commits": 3,
            "labels": [{ "name": "bug", "color": "d73a4a" }],
            "requested_reviewers": [{ "login": "alpha", "avatar_url": "" }],
            "requested_teams": [{ "slug": "core" }]
        });
        let comments = json!([
            { "node_id": "n1", "body": "First", "user": { "login": "bravo" },
              "created_at": "2026-01-02T00:00:00Z" }
        ]);

        let overview = parse_public_overview(&pull, &comments);

        assert_eq!(overview.state, "MERGED");
        assert_eq!(overview.mergeable, "UNKNOWN");
        assert_eq!(overview.body, "");
        assert_eq!(overview.author.avatar_url, "https://avatars.test/a");
        assert_eq!(overview.labels[0].name, "bug");
        assert_eq!(overview.reviewers[0].state, "REQUESTED");
        assert_eq!(overview.team_reviewers, 1);
        assert_eq!(overview.timeline[0].id, "n1");
        assert!(!overview.detailed);
    }
}
