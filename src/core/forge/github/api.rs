use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::core::error::{DiffyError, Result};
use crate::core::http;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GitHubUser {
    pub login: String,
    pub name: String,
    pub avatar_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PullRequestInfo {
    pub title: String,
    pub state: String,
    pub author_login: String,
    pub number: i32,
    pub additions: i32,
    pub deletions: i32,
    pub changed_files: i32,
    pub base_branch: String,
    pub head_branch: String,
    pub base_sha: String,
    pub head_sha: String,
    pub base_repo_url: String,
    pub head_repo_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum GitHubReviewSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GitHubReviewCommentUser {
    pub login: String,
    #[serde(default)]
    pub avatar_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GitHubReactionGroup {
    pub content: String,
    pub count: u32,
    pub viewer_has_reacted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PullRequestReviewComment {
    pub id: i64,
    #[serde(default)]
    pub in_reply_to_id: Option<i64>,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub commit_id: String,
    #[serde(default)]
    pub original_commit_id: String,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub original_line: Option<u32>,
    #[serde(default)]
    pub side: Option<GitHubReviewSide>,
    #[serde(default)]
    pub start_line: Option<u32>,
    #[serde(default)]
    pub original_start_line: Option<u32>,
    #[serde(default)]
    pub start_side: Option<GitHubReviewSide>,
    #[serde(default)]
    pub subject_type: Option<String>,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub user: Option<GitHubReviewCommentUser>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PullRequestReviewMetadata {
    pub node_id: String,
    pub url: String,
    pub review_decision: Option<String>,
    pub mergeable: String,
    pub merge_state_status: String,
    pub is_draft: bool,
    pub is_read_by_viewer: Option<bool>,
    pub viewer_latest_review_state: Option<String>,
    pub latest_head_oid: String,
    pub commit_count: i32,
    pub labels: Vec<PullRequestLabel>,
    pub review_requests: Vec<PullRequestReviewRequest>,
    pub checks: PullRequestCheckSummary,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PullRequestLabel {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PullRequestReviewRequest {
    pub reviewer_type: String,
    pub login: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PullRequestCheckSummary {
    pub state: Option<String>,
    pub total_count: i32,
    pub success_count: i32,
    pub failure_count: i32,
    pub pending_count: i32,
    pub contexts: Vec<PullRequestCheckContext>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PullRequestCheckContext {
    pub name: String,
    pub state: String,
    pub details_url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PullRequestReviewSummary {
    pub node_id: String,
    pub database_id: Option<i64>,
    pub state: String,
    pub body: String,
    pub author_login: String,
    pub submitted_at: Option<String>,
    pub commit_oid: String,
    pub viewer_did_author: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GitHubPullRequestReviewThreadComment {
    pub node_id: String,
    pub database_id: Option<i64>,
    pub reply_to_node_id: Option<String>,
    pub reply_to_database_id: Option<i64>,
    pub author_login: String,
    pub author_avatar_url: String,
    pub body: String,
    pub path: String,
    pub line: Option<u32>,
    pub original_line: Option<u32>,
    pub start_line: Option<u32>,
    pub original_start_line: Option<u32>,
    pub subject_type: String,
    pub url: String,
    pub created_at: String,
    pub updated_at: String,
    pub outdated: bool,
    pub state: String,
    pub viewer_can_update: bool,
    pub viewer_can_delete: bool,
    pub reactions: Vec<GitHubReactionGroup>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GitHubPullRequestReviewThread {
    pub node_id: String,
    pub path: String,
    pub line: Option<u32>,
    pub original_line: Option<u32>,
    pub start_line: Option<u32>,
    pub original_start_line: Option<u32>,
    pub diff_side: Option<GitHubReviewSide>,
    pub start_diff_side: Option<GitHubReviewSide>,
    pub subject_type: String,
    pub is_collapsed: bool,
    pub is_outdated: bool,
    pub is_resolved: bool,
    pub viewer_can_reply: bool,
    pub viewer_can_resolve: bool,
    pub viewer_can_unresolve: bool,
    pub comments: Vec<GitHubPullRequestReviewThreadComment>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GitHubPullRequestReviewData {
    pub metadata: PullRequestReviewMetadata,
    pub reviews: Vec<PullRequestReviewSummary>,
    pub threads: Vec<GitHubPullRequestReviewThread>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GitHubReviewThreadResolution {
    pub thread_node_id: String,
    pub is_resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CreatePullRequestReviewComment {
    pub body: String,
    pub commit_id: String,
    pub path: String,
    pub line: u32,
    pub side: GitHubReviewSide,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_side: Option<GitHubReviewSide>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PullRequestReview {
    pub id: i64,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub pull_request_url: String,
    #[serde(default)]
    pub submitted_at: Option<String>,
    #[serde(default)]
    pub commit_id: String,
    #[serde(default)]
    pub user: Option<GitHubReviewCommentUser>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GitHubPullRequestReviewEvent {
    Approve,
    RequestChanges,
    Comment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CreatePullRequestReviewDraftComment {
    pub path: String,
    pub body: String,
    pub line: u32,
    pub side: GitHubReviewSide,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_side: Option<GitHubReviewSide>,
}

impl From<CreatePullRequestReviewComment> for CreatePullRequestReviewDraftComment {
    fn from(comment: CreatePullRequestReviewComment) -> Self {
        Self {
            path: comment.path,
            body: comment.body,
            line: comment.line,
            side: comment.side,
            start_line: comment.start_line,
            start_side: comment.start_side,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CreatePullRequestReview {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<GitHubPullRequestReviewEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<CreatePullRequestReviewDraftComment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubmitPullRequestReview {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub event: GitHubPullRequestReviewEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CreatePullRequestReviewReply {
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdatePullRequestReviewComment {
    pub body: String,
}

#[derive(Debug, Clone, Default)]
pub struct GitHubApi {
    token: String,
}

impl GitHubApi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_token(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }

    pub fn set_token(&mut self, token: impl Into<String>) {
        self.token = token.into();
    }

    pub fn fetch_current_user(&self) -> Result<GitHubUser> {
        let body = http::block_on(async {
            let mut request = reqwest::Client::new()
                .get("https://api.github.com/user")
                .header("Accept", "application/vnd.github.v3+json")
                .header("User-Agent", "diffy/0.1");
            if !self.token.is_empty() {
                request = request.header("Authorization", &format!("Bearer {}", self.token));
            }
            let response = request
                .send()
                .await
                .map_err(|error| DiffyError::Http(format!("GitHub user fetch failed: {error}")))?;
            http::response_text(response, "GitHub user fetch").await
        })?;
        let json: Value = serde_json::from_str(&body)?;

        let login = string_field(&json, "login");
        if login.is_empty() {
            return Err(DiffyError::Parse(
                "missing login in GitHub user response".to_owned(),
            ));
        }
        let name = string_field(&json, "name");
        let avatar_url = string_field(&json, "avatar_url");
        let display_name = if name.is_empty() { login.clone() } else { name };

        Ok(GitHubUser {
            login,
            name: display_name,
            avatar_url,
        })
    }

    pub fn fetch_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
    ) -> Result<PullRequestInfo> {
        let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}");
        let body = http::block_on(async {
            let mut request = reqwest::Client::new()
                .get(&url)
                .header("Accept", "application/vnd.github.v3+json")
                .header("User-Agent", "diffy/0.1");
            if !self.token.is_empty() {
                request = request.header("Authorization", &format!("Bearer {}", self.token));
            }
            let response = request.send().await.map_err(|error| {
                DiffyError::Http(format!("GitHub pull request fetch failed: {error}"))
            })?;
            http::response_text(response, "GitHub pull request fetch").await
        })?;
        let json: Value = serde_json::from_str(&body)?;
        let base = json.get("base").cloned().unwrap_or(Value::Null);
        let head = json.get("head").cloned().unwrap_or(Value::Null);
        let base_repo = base.get("repo").cloned().unwrap_or(Value::Null);
        let head_repo = head.get("repo").cloned().unwrap_or(Value::Null);
        let user = json.get("user").cloned().unwrap_or(Value::Null);

        let result = PullRequestInfo {
            title: string_field(&json, "title"),
            state: string_field(&json, "state"),
            author_login: string_field(&user, "login"),
            number: int_field(&json, "number") as i32,
            additions: int_field(&json, "additions") as i32,
            deletions: int_field(&json, "deletions") as i32,
            changed_files: int_field(&json, "changed_files") as i32,
            base_branch: string_field(&base, "ref"),
            head_branch: string_field(&head, "ref"),
            base_sha: string_field(&base, "sha"),
            head_sha: string_field(&head, "sha"),
            base_repo_url: string_field(&base_repo, "clone_url"),
            head_repo_url: string_field(&head_repo, "clone_url"),
        };

        if result.base_branch.is_empty() || result.head_branch.is_empty() {
            return Err(DiffyError::Parse(
                "failed to parse GitHub pull request response".to_owned(),
            ));
        }

        Ok(result)
    }

    pub fn fetch_pull_request_review_comments(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
    ) -> Result<Vec<PullRequestReviewComment>> {
        http::block_on(async {
            let client = reqwest::Client::new();
            let mut comments = Vec::new();
            let mut page = 1_u32;

            loop {
                let url = format!(
                    "https://api.github.com/repos/{owner}/{repo}/pulls/{number}/comments?per_page=100&page={page}"
                );
                let mut request = client
                    .get(&url)
                    .header("Accept", "application/vnd.github+json")
                    .header("X-GitHub-Api-Version", "2026-03-10")
                    .header("User-Agent", "diffy/0.1");
                if !self.token.is_empty() {
                    request = request.header("Authorization", &format!("Bearer {}", self.token));
                }
                let response = request.send().await.map_err(|error| {
                    DiffyError::Http(format!("GitHub review comments fetch failed: {error}"))
                })?;
                let body = http::response_text(response, "GitHub review comments fetch").await?;
                let mut page_comments: Vec<PullRequestReviewComment> = serde_json::from_str(&body)?;
                let count = page_comments.len();
                comments.append(&mut page_comments);
                if count < 100 {
                    break;
                }
                page = page.saturating_add(1);
            }

            Ok(comments)
        })
    }

    pub fn fetch_pull_request_review_data(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
    ) -> Result<GitHubPullRequestReviewData> {
        if self.token.trim().is_empty() {
            return Err(DiffyError::General(
                "GitHub authentication is required to fetch pull request review data".to_owned(),
            ));
        }

        let mut result = GitHubPullRequestReviewData::default();
        let mut after: Option<String> = None;
        let query = review_graphql_query(PULL_REQUEST_REVIEW_DATA_QUERY);
        loop {
            let value = self.graphql_request(
                &query,
                json!({
                    "owner": owner,
                    "repo": repo,
                    "number": number,
                    "after": after,
                }),
            )?;
            let pull_request = value
                .pointer("/data/repository/pullRequest")
                .ok_or_else(|| {
                    DiffyError::Parse("missing pull request in GraphQL response".to_owned())
                })?;
            if result.metadata.node_id.is_empty() {
                result.metadata = parse_pull_request_review_metadata(pull_request);
                result.reviews =
                    parse_pull_request_reviews(pull_request.pointer("/latestReviews/nodes"));
            }
            result.threads.extend(parse_review_threads(
                pull_request.pointer("/reviewThreads/nodes"),
            ));

            let page_info = pull_request.pointer("/reviewThreads/pageInfo");
            let has_next = page_info
                .and_then(|v| v.get("hasNextPage"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !has_next {
                break;
            }
            after = page_info
                .and_then(|v| v.get("endCursor"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            if after.is_none() {
                break;
            }
        }

        Ok(result)
    }

    pub fn create_pull_request_review_comment(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
        comment: &CreatePullRequestReviewComment,
    ) -> Result<PullRequestReviewComment> {
        if self.token.trim().is_empty() {
            return Err(DiffyError::General(
                "GitHub authentication is required to add review comments".to_owned(),
            ));
        }
        let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}/comments");
        let request_body = serde_json::to_string(comment)?;
        let body = http::block_on(async {
            let response = reqwest::Client::new()
                .post(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2026-03-10")
                .header("User-Agent", "diffy/0.1")
                .header("Authorization", &format!("Bearer {}", self.token))
                .header("Content-Type", "application/json")
                .body(request_body)
                .send()
                .await
                .map_err(|error| {
                    DiffyError::Http(format!("GitHub review comment create failed: {error}"))
                })?;
            http::response_text(response, "GitHub review comment create").await
        })?;
        serde_json::from_str(&body).map_err(Into::into)
    }

    pub fn create_pull_request_review_reply(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
        comment_id: i64,
        reply: &CreatePullRequestReviewReply,
    ) -> Result<PullRequestReviewComment> {
        if self.token.trim().is_empty() {
            return Err(DiffyError::General(
                "GitHub authentication is required to reply to review comments".to_owned(),
            ));
        }
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/pulls/{number}/comments/{comment_id}/replies"
        );
        let request_body = serde_json::to_string(reply)?;
        let body = http::block_on(async {
            let response = reqwest::Client::new()
                .post(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2026-03-10")
                .header("User-Agent", "diffy/0.1")
                .header("Authorization", &format!("Bearer {}", self.token))
                .header("Content-Type", "application/json")
                .body(request_body)
                .send()
                .await
                .map_err(|error| {
                    DiffyError::Http(format!("GitHub review comment reply failed: {error}"))
                })?;
            http::response_text(response, "GitHub review comment reply").await
        })?;
        serde_json::from_str(&body).map_err(Into::into)
    }

    pub fn update_pull_request_review_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: i64,
        update: &UpdatePullRequestReviewComment,
    ) -> Result<PullRequestReviewComment> {
        if self.token.trim().is_empty() {
            return Err(DiffyError::General(
                "GitHub authentication is required to update review comments".to_owned(),
            ));
        }
        let url =
            format!("https://api.github.com/repos/{owner}/{repo}/pulls/comments/{comment_id}");
        let request_body = serde_json::to_string(update)?;
        let body = http::block_on(async {
            let response = reqwest::Client::new()
                .patch(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2026-03-10")
                .header("User-Agent", "diffy/0.1")
                .header("Authorization", &format!("Bearer {}", self.token))
                .header("Content-Type", "application/json")
                .body(request_body)
                .send()
                .await
                .map_err(|error| {
                    DiffyError::Http(format!("GitHub review comment update failed: {error}"))
                })?;
            http::response_text(response, "GitHub review comment update").await
        })?;
        serde_json::from_str(&body).map_err(Into::into)
    }

    pub fn delete_pull_request_review_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: i64,
    ) -> Result<()> {
        if self.token.trim().is_empty() {
            return Err(DiffyError::General(
                "GitHub authentication is required to delete review comments".to_owned(),
            ));
        }
        let url =
            format!("https://api.github.com/repos/{owner}/{repo}/pulls/comments/{comment_id}");
        http::block_on(async {
            let response = reqwest::Client::new()
                .delete(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2026-03-10")
                .header("User-Agent", "diffy/0.1")
                .header("Authorization", &format!("Bearer {}", self.token))
                .send()
                .await
                .map_err(|error| {
                    DiffyError::Http(format!("GitHub review comment delete failed: {error}"))
                })?;
            http::response_text(response, "GitHub review comment delete").await
        })?;
        Ok(())
    }

    pub fn create_pull_request_review(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
        review: &CreatePullRequestReview,
    ) -> Result<PullRequestReview> {
        if self.token.trim().is_empty() {
            return Err(DiffyError::General(
                "GitHub authentication is required to create pull request reviews".to_owned(),
            ));
        }
        let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}/reviews");
        let request_body = serde_json::to_string(review)?;
        let body = http::block_on(async {
            let response = reqwest::Client::new()
                .post(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2026-03-10")
                .header("User-Agent", "diffy/0.1")
                .header("Authorization", &format!("Bearer {}", self.token))
                .header("Content-Type", "application/json")
                .body(request_body)
                .send()
                .await
                .map_err(|error| {
                    DiffyError::Http(format!("GitHub pull request review create failed: {error}"))
                })?;
            http::response_text(response, "GitHub pull request review create").await
        })?;
        serde_json::from_str(&body).map_err(Into::into)
    }

    pub fn submit_pull_request_review(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
        review_id: i64,
        submit: &SubmitPullRequestReview,
    ) -> Result<PullRequestReview> {
        if self.token.trim().is_empty() {
            return Err(DiffyError::General(
                "GitHub authentication is required to submit pull request reviews".to_owned(),
            ));
        }
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/pulls/{number}/reviews/{review_id}/events"
        );
        let request_body = serde_json::to_string(submit)?;
        let body = http::block_on(async {
            let response = reqwest::Client::new()
                .post(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2026-03-10")
                .header("User-Agent", "diffy/0.1")
                .header("Authorization", &format!("Bearer {}", self.token))
                .header("Content-Type", "application/json")
                .body(request_body)
                .send()
                .await
                .map_err(|error| {
                    DiffyError::Http(format!("GitHub pull request review submit failed: {error}"))
                })?;
            http::response_text(response, "GitHub pull request review submit").await
        })?;
        serde_json::from_str(&body).map_err(Into::into)
    }

    pub fn add_pull_request_review_thread_reply(
        &self,
        thread_node_id: &str,
        review_node_id: Option<&str>,
        body: &str,
    ) -> Result<GitHubPullRequestReviewThreadComment> {
        if self.token.trim().is_empty() {
            return Err(DiffyError::General(
                "GitHub authentication is required to reply to review threads".to_owned(),
            ));
        }
        let value = self.graphql_request(
            &review_graphql_query(ADD_REVIEW_THREAD_REPLY_MUTATION),
            json!({
                "threadId": thread_node_id,
                "reviewId": review_node_id,
                "body": body,
            }),
        )?;
        let comment = value
            .pointer("/data/addPullRequestReviewThreadReply/comment")
            .ok_or_else(|| {
                DiffyError::Parse("missing review thread reply in GraphQL response".to_owned())
            })?;
        Ok(parse_thread_comment(comment))
    }

    pub fn update_pull_request_review_comment_graphql(
        &self,
        comment_node_id: &str,
        body: &str,
    ) -> Result<GitHubPullRequestReviewThreadComment> {
        if self.token.trim().is_empty() {
            return Err(DiffyError::General(
                "GitHub authentication is required to update review comments".to_owned(),
            ));
        }
        let value = self.graphql_request(
            &review_graphql_query(UPDATE_REVIEW_COMMENT_MUTATION),
            json!({
                "commentId": comment_node_id,
                "body": body,
            }),
        )?;
        let comment = value
            .pointer("/data/updatePullRequestReviewComment/pullRequestReviewComment")
            .ok_or_else(|| {
                DiffyError::Parse("missing updated review comment in GraphQL response".to_owned())
            })?;
        Ok(parse_thread_comment(comment))
    }

    pub fn delete_pull_request_review_comment_graphql(
        &self,
        comment_node_id: &str,
    ) -> Result<Option<GitHubPullRequestReviewThreadComment>> {
        if self.token.trim().is_empty() {
            return Err(DiffyError::General(
                "GitHub authentication is required to delete review comments".to_owned(),
            ));
        }
        let value = self.graphql_request(
            &review_graphql_query(DELETE_REVIEW_COMMENT_MUTATION),
            json!({
                "commentId": comment_node_id,
            }),
        )?;
        Ok(value
            .pointer("/data/deletePullRequestReviewComment/pullRequestReviewComment")
            .filter(|v| !v.is_null())
            .map(parse_thread_comment))
    }

    pub fn set_pull_request_review_thread_resolution(
        &self,
        thread_node_id: &str,
        resolved: bool,
    ) -> Result<GitHubReviewThreadResolution> {
        if self.token.trim().is_empty() {
            return Err(DiffyError::General(
                "GitHub authentication is required to update review thread resolution".to_owned(),
            ));
        }
        let mutation = if resolved {
            RESOLVE_REVIEW_THREAD_MUTATION
        } else {
            UNRESOLVE_REVIEW_THREAD_MUTATION
        };
        let value = self.graphql_request(
            mutation,
            json!({
                "threadId": thread_node_id,
            }),
        )?;
        let thread = value
            .pointer(if resolved {
                "/data/resolveReviewThread/thread"
            } else {
                "/data/unresolveReviewThread/thread"
            })
            .ok_or_else(|| {
                DiffyError::Parse("missing review thread in GraphQL response".to_owned())
            })?;
        Ok(GitHubReviewThreadResolution {
            thread_node_id: string_field(thread, "id"),
            is_resolved: bool_field(thread, "isResolved"),
        })
    }

    fn graphql_request(&self, query: &str, variables: Value) -> Result<Value> {
        let request_body = json!({
            "query": query,
            "variables": variables,
        })
        .to_string();
        let body = http::block_on(async {
            let response = reqwest::Client::new()
                .post("https://api.github.com/graphql")
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2026-03-10")
                .header("User-Agent", "diffy/0.1")
                .header("Authorization", &format!("Bearer {}", self.token))
                .header("Content-Type", "application/json")
                .body(request_body)
                .send()
                .await
                .map_err(|error| {
                    DiffyError::Http(format!("GitHub GraphQL request failed: {error}"))
                })?;
            http::response_text(response, "GitHub GraphQL request").await
        })?;
        let value: Value = serde_json::from_str(&body)?;
        if let Some(errors) = value.get("errors").and_then(Value::as_array)
            && !errors.is_empty()
        {
            return Err(DiffyError::Http(format!(
                "GitHub GraphQL request failed: {}",
                graphql_error_message(errors)
            )));
        }
        Ok(value)
    }
}

const REVIEW_COMMENT_FIELDS: &str = r#"
    id
    fullDatabaseId
    body
    path
    line
    originalLine
    startLine
    originalStartLine
    subjectType
    url
    createdAt
    updatedAt
    outdated
    state
    viewerCanUpdate
    viewerCanDelete
    author { login avatarUrl }
    replyTo {
      id
      fullDatabaseId
    }
    reactionGroups {
      content
      viewerHasReacted
      users { totalCount }
    }
"#;

const PULL_REQUEST_REVIEW_DATA_QUERY: &str = r#"
query DiffyPullRequestReviewData($owner: String!, $repo: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      id
      url
      reviewDecision
      mergeable
      mergeStateStatus
      isDraft
      isReadByViewer
      headRefOid
      commits(last: 1) {
        totalCount
        nodes {
          commit {
            oid
            statusCheckRollup {
              state
              contexts(first: 50) {
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
              name
            }
            ... on Team {
              name
              slug
            }
          }
        }
      }
      viewerLatestReview {
        state
      }
      latestReviews(first: 50) {
        nodes {
          id
          fullDatabaseId
          state
          body
          submittedAt
          viewerDidAuthor
          author { login }
          commit { oid }
        }
      }
      reviewThreads(first: 100, after: $after) {
        pageInfo {
          hasNextPage
          endCursor
        }
        nodes {
          id
          path
          line
          originalLine
          startLine
          originalStartLine
          diffSide
          startDiffSide
          subjectType
          isCollapsed
          isOutdated
          isResolved
          viewerCanReply
          viewerCanResolve
          viewerCanUnresolve
          comments(first: 100) {
            nodes {
              __REVIEW_COMMENT_FIELDS__
            }
          }
        }
      }
    }
  }
}
"#;

const ADD_REVIEW_THREAD_REPLY_MUTATION: &str = r#"
mutation DiffyAddReviewThreadReply($threadId: ID!, $reviewId: ID, $body: String!) {
  addPullRequestReviewThreadReply(input: {
    pullRequestReviewThreadId: $threadId,
    pullRequestReviewId: $reviewId,
    body: $body
  }) {
    comment {
      __REVIEW_COMMENT_FIELDS__
    }
  }
}
"#;

const UPDATE_REVIEW_COMMENT_MUTATION: &str = r#"
mutation DiffyUpdateReviewComment($commentId: ID!, $body: String!) {
  updatePullRequestReviewComment(input: {
    pullRequestReviewCommentId: $commentId,
    body: $body
  }) {
    pullRequestReviewComment {
      __REVIEW_COMMENT_FIELDS__
    }
  }
}
"#;

const DELETE_REVIEW_COMMENT_MUTATION: &str = r#"
mutation DiffyDeleteReviewComment($commentId: ID!) {
  deletePullRequestReviewComment(input: { id: $commentId }) {
    pullRequestReviewComment {
      __REVIEW_COMMENT_FIELDS__
    }
  }
}
"#;

const RESOLVE_REVIEW_THREAD_MUTATION: &str = r#"
mutation DiffyResolveReviewThread($threadId: ID!) {
  resolveReviewThread(input: { threadId: $threadId }) {
    thread {
      id
      isResolved
    }
  }
}
"#;

const UNRESOLVE_REVIEW_THREAD_MUTATION: &str = r#"
mutation DiffyUnresolveReviewThread($threadId: ID!) {
  unresolveReviewThread(input: { threadId: $threadId }) {
    thread {
      id
      isResolved
    }
  }
}
"#;

fn review_graphql_query(template: &str) -> String {
    template.replace("__REVIEW_COMMENT_FIELDS__", REVIEW_COMMENT_FIELDS)
}

fn parse_pull_request_review_metadata(value: &Value) -> PullRequestReviewMetadata {
    let check_rollup = value
        .pointer("/commits/nodes/0/commit/statusCheckRollup")
        .unwrap_or(&Value::Null);
    PullRequestReviewMetadata {
        node_id: string_field(value, "id"),
        url: string_field(value, "url"),
        review_decision: optional_string_field(value, "reviewDecision"),
        mergeable: string_field(value, "mergeable"),
        merge_state_status: string_field(value, "mergeStateStatus"),
        is_draft: bool_field(value, "isDraft"),
        is_read_by_viewer: value.get("isReadByViewer").and_then(Value::as_bool),
        viewer_latest_review_state: value
            .pointer("/viewerLatestReview/state")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        latest_head_oid: string_field(value, "headRefOid"),
        commit_count: int_field(
            value.pointer("/commits").unwrap_or(&Value::Null),
            "totalCount",
        ) as i32,
        labels: parse_labels(value.pointer("/labels/nodes")),
        review_requests: parse_review_requests(value.pointer("/reviewRequests/nodes")),
        checks: parse_check_summary(check_rollup),
    }
}

fn parse_labels(value: Option<&Value>) -> Vec<PullRequestLabel> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|node| PullRequestLabel {
            name: string_field(node, "name"),
            color: string_field(node, "color"),
        })
        .filter(|label| !label.name.is_empty())
        .collect()
}

fn parse_review_requests(value: Option<&Value>) -> Vec<PullRequestReviewRequest> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|node| node.get("requestedReviewer"))
        .map(|reviewer| {
            let reviewer_type = string_field(reviewer, "__typename");
            let login = optional_string_field(reviewer, "login")
                .or_else(|| optional_string_field(reviewer, "slug"))
                .unwrap_or_default();
            let name = optional_string_field(reviewer, "name").unwrap_or_else(|| login.clone());
            PullRequestReviewRequest {
                reviewer_type,
                login,
                name,
            }
        })
        .filter(|request| !request.login.is_empty() || !request.name.is_empty())
        .collect()
}

fn parse_check_summary(value: &Value) -> PullRequestCheckSummary {
    let contexts = value.pointer("/contexts/nodes");
    let mut summary = PullRequestCheckSummary {
        state: optional_string_field(value, "state"),
        total_count: int_field(
            value.pointer("/contexts").unwrap_or(&Value::Null),
            "totalCount",
        ) as i32,
        ..PullRequestCheckSummary::default()
    };
    summary.contexts = contexts
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(parse_check_context)
        .filter(|context| !context.name.is_empty())
        .collect();
    for context in &summary.contexts {
        match context.state.as_str() {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => summary.success_count += 1,
            "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED"
            | "STARTUP_FAILURE" => summary.failure_count += 1,
            _ => summary.pending_count += 1,
        }
    }
    summary
}

fn parse_check_context(value: &Value) -> PullRequestCheckContext {
    match string_field(value, "__typename").as_str() {
        "CheckRun" => PullRequestCheckContext {
            name: string_field(value, "name"),
            state: optional_string_field(value, "conclusion")
                .filter(|s| !s.is_empty())
                .or_else(|| optional_string_field(value, "status"))
                .unwrap_or_default(),
            details_url: string_field(value, "detailsUrl"),
        },
        "StatusContext" => PullRequestCheckContext {
            name: string_field(value, "context"),
            state: string_field(value, "state"),
            details_url: string_field(value, "targetUrl"),
        },
        _ => PullRequestCheckContext::default(),
    }
}

fn parse_pull_request_reviews(value: Option<&Value>) -> Vec<PullRequestReviewSummary> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|node| PullRequestReviewSummary {
            node_id: string_field(node, "id"),
            database_id: optional_i64_field(node, "fullDatabaseId"),
            state: string_field(node, "state"),
            body: string_field(node, "body"),
            author_login: node
                .get("author")
                .map(|author| string_field(author, "login"))
                .unwrap_or_default(),
            submitted_at: optional_string_field(node, "submittedAt"),
            commit_oid: node
                .get("commit")
                .map(|commit| string_field(commit, "oid"))
                .unwrap_or_default(),
            viewer_did_author: bool_field(node, "viewerDidAuthor"),
        })
        .collect()
}

fn parse_review_threads(value: Option<&Value>) -> Vec<GitHubPullRequestReviewThread> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|node| GitHubPullRequestReviewThread {
            node_id: string_field(node, "id"),
            path: string_field(node, "path"),
            line: optional_u32_field(node, "line"),
            original_line: optional_u32_field(node, "originalLine"),
            start_line: optional_u32_field(node, "startLine"),
            original_start_line: optional_u32_field(node, "originalStartLine"),
            diff_side: optional_review_side(node, "diffSide"),
            start_diff_side: optional_review_side(node, "startDiffSide"),
            subject_type: string_field(node, "subjectType"),
            is_collapsed: bool_field(node, "isCollapsed"),
            is_outdated: bool_field(node, "isOutdated"),
            is_resolved: bool_field(node, "isResolved"),
            viewer_can_reply: bool_field(node, "viewerCanReply"),
            viewer_can_resolve: bool_field(node, "viewerCanResolve"),
            viewer_can_unresolve: bool_field(node, "viewerCanUnresolve"),
            comments: node
                .pointer("/comments/nodes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(parse_thread_comment)
                .collect(),
        })
        .collect()
}

fn parse_thread_comment(value: &Value) -> GitHubPullRequestReviewThreadComment {
    GitHubPullRequestReviewThreadComment {
        node_id: string_field(value, "id"),
        database_id: optional_i64_field(value, "fullDatabaseId"),
        reply_to_node_id: value
            .pointer("/replyTo/id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        reply_to_database_id: value
            .pointer("/replyTo/fullDatabaseId")
            .and_then(value_to_i64),
        author_login: value
            .get("author")
            .map(|author| string_field(author, "login"))
            .unwrap_or_default(),
        author_avatar_url: value
            .get("author")
            .map(|author| string_field(author, "avatarUrl"))
            .unwrap_or_default(),
        body: string_field(value, "body"),
        path: string_field(value, "path"),
        line: optional_u32_field(value, "line"),
        original_line: optional_u32_field(value, "originalLine"),
        start_line: optional_u32_field(value, "startLine"),
        original_start_line: optional_u32_field(value, "originalStartLine"),
        subject_type: string_field(value, "subjectType"),
        url: string_field(value, "url"),
        created_at: string_field(value, "createdAt"),
        updated_at: string_field(value, "updatedAt"),
        outdated: bool_field(value, "outdated"),
        state: string_field(value, "state"),
        viewer_can_update: bool_field(value, "viewerCanUpdate"),
        viewer_can_delete: bool_field(value, "viewerCanDelete"),
        reactions: value
            .get("reactionGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|group| GitHubReactionGroup {
                content: string_field(group, "content"),
                count: group
                    .pointer("/users/totalCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .min(u64::from(u32::MAX)) as u32,
                viewer_has_reacted: bool_field(group, "viewerHasReacted"),
            })
            .filter(|group| group.count > 0)
            .collect(),
    }
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn optional_string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn int_field(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or_default()
}

fn optional_i64_field(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(value_to_i64)
}

fn optional_u32_field(value: &Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(value_to_i64)
        .and_then(|value| u32::try_from(value).ok())
}

fn value_to_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))
}

fn bool_field(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn optional_review_side(value: &Value, key: &str) -> Option<GitHubReviewSide> {
    match value.get(key).and_then(Value::as_str) {
        Some("LEFT") => Some(GitHubReviewSide::Left),
        Some("RIGHT") => Some(GitHubReviewSide::Right),
        _ => None,
    }
}

fn graphql_error_message(errors: &[Value]) -> String {
    errors
        .iter()
        .filter_map(|error| error.get("message").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("; ")
}
