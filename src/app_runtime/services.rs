use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::core::compare::CompareService;
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::git::GitService;
use crate::core::vcs::github::{
    DeviceFlowState, GitHubApi, PullRequestInfo, parse_pr_url, poll_for_token, start_device_flow,
};
use crate::platform::persistence::{Settings, SettingsStore};
use crate::ui::effects::CompareRequest;
use crate::ui::events::{CompareFinished, RepositoryLoaded};

#[derive(Debug, Clone)]
pub struct AppServices {
    settings_store: SettingsStore,
}

impl AppServices {
    pub fn new(settings_store: SettingsStore) -> Self {
        Self { settings_store }
    }

    pub fn load_repository(&self, path: PathBuf) -> Result<RepositoryLoaded> {
        let mut git = GitService::new();
        git.open(path.to_string_lossy().as_ref())?;

        Ok(RepositoryLoaded {
            path,
            branches: git.branches()?,
            tags: git.tags()?,
            commits: git.commits("HEAD", 200).unwrap_or_default(),
        })
    }

    pub fn run_compare(&self, generation: u64, request: CompareRequest) -> Result<CompareFinished> {
        let mut git = GitService::new();
        git.open(request.repo_path.to_string_lossy().as_ref())?;
        if let Some(token) = request.github_token.clone() {
            git.set_github_token(token);
        }
        let (resolved_left, resolved_right) = git.resolve_comparison(
            &request.spec.left_ref,
            &request.spec.right_ref,
            request.spec.mode,
        )?;
        let output = CompareService::default().compare(&request.spec, &git)?;

        Ok(CompareFinished {
            generation,
            spec: request.spec,
            resolved_left,
            resolved_right,
            output,
        })
    }

    pub fn load_pull_request(
        &self,
        url: &str,
        repo_path: &Path,
        github_token: Option<String>,
    ) -> Result<(PullRequestInfo, String, String)> {
        let parsed = parse_pr_url(url)
            .ok_or_else(|| DiffyError::Parse("not a valid GitHub pull request URL".to_owned()))?;
        let token = github_token.unwrap_or_default();
        let info = GitHubApi::with_token(token.clone()).fetch_pull_request(
            &parsed.owner,
            &parsed.repo,
            parsed.number,
        )?;

        let mut git = GitService::new();
        git.set_github_token(token);
        git.open(repo_path.to_string_lossy().as_ref())?;
        let (left_ref, right_ref) = git.resolve_pull_request_comparison(url)?;
        Ok((info, left_ref, right_ref))
    }

    pub fn start_device_flow(&self, client_id: &str) -> Result<DeviceFlowState> {
        if client_id.trim().is_empty() {
            return Err(DiffyError::General(
                "GitHub client id is empty. Set DIFFY_GITHUB_CLIENT_ID.".to_owned(),
            ));
        }
        start_device_flow(client_id)
    }

    pub fn poll_device_flow(
        &self,
        client_id: &str,
        device_code: &str,
        interval_seconds: u32,
    ) -> Result<String> {
        loop {
            match poll_for_token(client_id, device_code)? {
                Some(token) => return Ok(token),
                None => thread::sleep(Duration::from_secs(u64::from(interval_seconds.max(5)))),
            }
        }
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.settings_store.save(settings)
    }

    pub fn open_browser(&self, url: &str) -> Result<()> {
        webbrowser::open(url)
            .map(|_| ())
            .map_err(|error| DiffyError::General(format!("failed to open browser: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{Repository, Signature};
    use tempfile::TempDir;

    use super::AppServices;
    use crate::core::compare::{CompareMode, CompareSpec, LayoutMode, RendererKind};
    use crate::platform::persistence::SettingsStore;
    use crate::ui::effects::CompareRequest;

    fn commit_file(repo: &Repository, relative_path: &str, content: &str, message: &str) -> String {
        let workdir = repo.workdir().expect("repo workdir");
        let full_path = workdir.join(relative_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full_path, content).unwrap();

        let mut index = repo.index().unwrap();
        index.add_path(Path::new(relative_path)).unwrap();
        index.write().unwrap();

        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = Signature::now("Diffy", "diffy@example.com").unwrap();
        let parents = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .map(|oid| repo.find_commit(oid).unwrap())
            .into_iter()
            .collect::<Vec<_>>();
        let parent_refs = parents.iter().collect::<Vec<_>>();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parent_refs,
        )
        .unwrap()
        .to_string()
    }

    #[test]
    fn services_can_load_repository_and_run_compare() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let left = commit_file(&repo, "src/lib.rs", "fn a() -> i32 { 1 }\n", "initial");
        let right = commit_file(&repo, "src/lib.rs", "fn a() -> i32 { 2 }\n", "second");

        let store_dir = TempDir::new().unwrap();
        let services = AppServices::new(SettingsStore::new_in(store_dir.path()));

        let loaded = services
            .load_repository(repo_dir.path().to_path_buf())
            .unwrap();
        assert!(!loaded.branches.is_empty());

        let finished = services
            .run_compare(
                1,
                CompareRequest {
                    repo_path: repo_dir.path().to_path_buf(),
                    spec: CompareSpec {
                        mode: CompareMode::TwoDot,
                        left_ref: left,
                        right_ref: right,
                        renderer: RendererKind::Builtin,
                        layout: LayoutMode::Unified,
                    },
                    github_token: None,
                },
            )
            .unwrap();

        assert_eq!(finished.generation, 1);
        assert_eq!(finished.output.files.len(), 1);
        assert_eq!(finished.output.files[0].path, "src/lib.rs");
    }
}
