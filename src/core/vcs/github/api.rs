use serde::Deserialize;
use serde_json::Value;

use crate::core::error::{DiffyError, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
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

    pub fn fetch_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
    ) -> Result<PullRequestInfo> {
        let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}");
        let mut request = ureq::get(&url)
            .header("Accept", "application/vnd.github.v3+json")
            .header("User-Agent", "diffy/0.1");
        if !self.token.is_empty() {
            request = request.header("Authorization", &format!("Bearer {}", self.token));
        }

        let body = request
            .call()?
            .into_body()
            .read_to_string()
            .map_err(|error| DiffyError::Http(error.to_string()))?;
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
