#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubPullRequest {
    pub owner: String,
    pub repo: String,
    pub number: i32,
}

impl GitHubPullRequest {
    pub fn url(&self) -> String {
        format!(
            "https://github.com/{}/{}/pull/{}",
            self.owner, self.repo, self.number
        )
    }
}

pub fn parse_pr_reference(input: &str) -> Option<GitHubPullRequest> {
    parse_pr_url(input).or_else(|| parse_short_reference(input))
}

fn parse_short_reference(input: &str) -> Option<GitHubPullRequest> {
    let (slug, number) = input.trim().split_once('#')?;
    let (owner, repo) = slug.split_once('/')?;
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    if !valid(owner) || !valid(repo) {
        return None;
    }
    Some(GitHubPullRequest {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        number: number.parse().ok().filter(|n: &i32| *n > 0)?,
    })
}

pub fn parse_pr_url(url: &str) -> Option<GitHubPullRequest> {
    let trimmed = url.trim().trim_end_matches('/');
    let path = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let path = path
        .strip_prefix("www.github.com/")
        .or_else(|| path.strip_prefix("github.com/"))?;
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
    use super::{parse_pr_reference, parse_pr_url};

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
    fn parses_short_references_and_file_tabs() {
        let short = parse_pr_reference("owner/repo#12").unwrap();
        assert_eq!(
            (short.owner.as_str(), short.repo.as_str(), short.number),
            ("owner", "repo", 12)
        );
        let tab = parse_pr_reference("www.github.com/owner/repo/pull/7/files").unwrap();
        assert_eq!(tab.url(), "https://github.com/owner/repo/pull/7");
        assert!(parse_pr_reference("owner/repo#0").is_none());
        assert!(parse_pr_reference("fix the #12 bug").is_none());
        assert!(parse_pr_reference("owner/repo").is_none());
    }

    #[test]
    fn rejects_invalid_urls() {
        assert!(parse_pr_url("https://github.com/owner/repo/issues/1").is_none());
        assert!(parse_pr_url("git@github.com:owner/repo.git").is_none());
        assert!(parse_pr_url("https://example.com/owner/repo/pull/1").is_none());
    }
}
