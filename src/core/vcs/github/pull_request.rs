#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubPullRequest {
    pub owner: String,
    pub repo: String,
    pub number: i32,
}

pub fn parse_pr_url(url: &str) -> Option<GitHubPullRequest> {
    let trimmed = url.trim().trim_end_matches('/');
    let path = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .or_else(|| trimmed.strip_prefix("github.com/"))?;
    let path = path.split(['?', '#']).next()?;
    let mut parts = path.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim().trim_end_matches(".git");
    let pull = parts.next()?;
    let number = parts.next()?;
    if pull != "pull" || owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(GitHubPullRequest {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        number: number.parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_pr_url;

    #[test]
    fn parses_standard_url() {
        let parsed = parse_pr_url("https://github.com/owner/repo/pull/123").unwrap();
        assert_eq!(parsed.owner, "owner");
        assert_eq!(parsed.repo, "repo");
        assert_eq!(parsed.number, 123);
    }

    #[test]
    fn parses_query_and_trailing_slash() {
        let parsed = parse_pr_url("https://github.com/owner/repo/pull/456/?foo=bar").unwrap();
        assert_eq!(parsed.number, 456);
    }

    #[test]
    fn rejects_invalid_urls() {
        assert!(parse_pr_url("https://github.com/owner/repo/issues/1").is_none());
        assert!(parse_pr_url("git@github.com:owner/repo.git").is_none());
        assert!(parse_pr_url("https://example.com/owner/repo/pull/1").is_none());
    }
}
