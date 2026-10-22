#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRepo {
    pub owner: String,
    pub repo: String,
}

impl GitHubRepo {
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    pub fn clone_url(&self) -> String {
        format!("https://github.com/{}/{}.git", self.owner, self.repo)
    }

    pub fn matches(&self, owner: &str, repo: &str) -> bool {
        self.owner.eq_ignore_ascii_case(owner) && self.repo.eq_ignore_ascii_case(repo)
    }
}

pub fn parse_remote(url: &str) -> Option<GitHubRepo> {
    let url = url.trim();
    let (scheme, rest) = match url.split_once("://") {
        Some((_, rest)) => (true, rest),
        None => (false, url),
    };
    let rest = rest.split_once('@').map_or(rest, |(_, rest)| rest);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let path = rest.strip_prefix("github.com")?;
    let path = match path.strip_prefix(':') {
        Some(port) if scheme => port.trim_start_matches(|c: char| c.is_ascii_digit()),
        Some(path) => return slug(path),
        None => path,
    };
    slug(path.strip_prefix('/')?)
}

pub fn parse_repo_reference(input: &str) -> Option<GitHubRepo> {
    let input = input.trim();
    if input.contains("github.com") {
        return parse_remote(input);
    }
    slug(input)
}

fn slug(path: &str) -> Option<GitHubRepo> {
    let path = path.trim_end_matches('/');
    let (owner, repo) = path.split_once('/')?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (valid(owner) && valid(repo)).then(|| GitHubRepo {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(repo: Option<GitHubRepo>) -> Option<(String, String)> {
        repo.map(|repo| (repo.owner, repo.repo))
    }

    #[test]
    fn parses_remote_urls() {
        let expected = Some(("owner".to_owned(), "repo".to_owned()));
        for url in [
            "git@github.com:owner/repo.git",
            "https://github.com/owner/repo",
            "https://github.com/owner/repo.git/",
            "https://user@github.com/owner/repo.git",
            "ssh://git@github.com/owner/repo.git",
            "ssh://git@github.com:22/owner/repo.git",
            "git://github.com/owner/repo.git",
        ] {
            assert_eq!(pair(parse_remote(url)), expected, "{url}");
        }
        assert_eq!(parse_remote("git@example.com:owner/repo.git"), None);
        assert_eq!(parse_remote("https://github.com/owner"), None);
    }

    #[test]
    fn parses_typed_repo_references() {
        let expected = Some(("owner".to_owned(), "repo".to_owned()));
        assert_eq!(pair(parse_repo_reference("owner/repo")), expected);
        assert_eq!(
            pair(parse_repo_reference("github.com/owner/repo")),
            expected
        );
        assert_eq!(parse_repo_reference("owner/repo#4"), None);
        assert_eq!(parse_repo_reference("/abs/path"), None);
        assert_eq!(parse_repo_reference("owner/repo/extra"), None);
        assert!(
            parse_repo_reference("Owner/Repo")
                .unwrap()
                .matches("owner", "repo")
        );
    }
}
