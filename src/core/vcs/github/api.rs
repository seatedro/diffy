use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn int_field(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or_default()
}
