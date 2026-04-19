use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::ai::{self, GenerateRequest, StreamMessage};
use crate::apprt::runtime::RuntimeEventSender;
use crate::core::compare::{CompareOutput, CompareService, RendererKind};
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::git::{GitService, WORKDIR_REF};
use crate::core::vcs::github::{
    DeviceFlowState, GitHubApi, GitHubUser, PullRequestInfo, parse_pr_url, poll_for_token,
    start_device_flow,
};
use crate::effects::{CompareRequest, GenerateCommitMessageRequest, StatusDiffRequest};
use crate::events::{AppEvent, CompareFinished, StatusDiffFinished};
use crate::platform::persistence::{Settings, SettingsStore};
use crate::platform::secrets::{self, AiKeyKind};

#[derive(Debug, Clone)]
pub struct AppServices {
    settings_store: SettingsStore,
}

impl AppServices {
    pub fn new(settings_store: SettingsStore) -> Self {
        Self { settings_store }
    }

    pub fn run_compare(&self, generation: u64, request: CompareRequest) -> Result<CompareFinished> {
        let mut git = GitService::new();
        git.open(request.repo_path.to_string_lossy().as_ref())?;
        let (resolved_left, resolved_right) = git.resolve_comparison(
            &request.spec.left_ref,
            &request.spec.right_ref,
            request.spec.mode,
        )?;
        let output = CompareService::default().compare(&request.spec, &git)?;

        let range_right = if resolved_right == WORKDIR_REF {
            "HEAD"
        } else {
            &resolved_right
        };
        let range_commits = git
            .commits_in_range(&resolved_left, range_right, 500)
            .unwrap_or_default();

        Ok(CompareFinished {
            generation,
            spec: request.spec,
            resolved_left,
            resolved_right,
            output,
            range_commits,
        })
    }

    pub fn load_status_diff(
        &self,
        generation: u64,
        index: usize,
        request: StatusDiffRequest,
    ) -> Result<StatusDiffFinished> {
        let mut git = GitService::new();
        git.open(request.repo_path.to_string_lossy().as_ref())?;
        let output: CompareOutput = match request.renderer {
            RendererKind::Builtin => git.diff_status_item(&request.item)?,
            RendererKind::Difftastic => crate::core::compare::backends::DifftasticBackend
                .compare_status_item(&request.item, &git)?,
        };

        Ok(StatusDiffFinished {
            generation,
            index,
            item: request.item,
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
        git.open(repo_path.to_string_lossy().as_ref())?;
        let (left_ref, right_ref) = git.resolve_pull_request_comparison(url, &token)?;
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

    pub fn load_github_token(&self) -> Result<Option<String>> {
        secrets::load_github_token()
    }

    pub fn save_github_token(&self, token: &str) -> Result<()> {
        secrets::save_github_token(token)
    }

    pub fn clear_github_token(&self) -> Result<()> {
        secrets::clear_github_token()
    }

    pub fn peek_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
        token: Option<String>,
    ) -> Result<PullRequestInfo> {
        let api = match token {
            Some(t) if !t.is_empty() => GitHubApi::with_token(t),
            _ => GitHubApi::new(),
        };
        api.fetch_pull_request(owner, repo, number)
    }

    pub fn fetch_github_user(&self, token: &str) -> Result<GitHubUser> {
        if token.trim().is_empty() {
            return Err(DiffyError::General(
                "cannot fetch GitHub user without a token".to_owned(),
            ));
        }
        GitHubApi::with_token(token).fetch_current_user()
    }

    pub fn fetch_avatar(&self, url: &str) -> Result<(Vec<u8>, u32, u32)> {
        let bytes = ureq::get(url)
            .header("User-Agent", "diffy/0.1")
            .call()?
            .into_body()
            .read_to_vec()
            .map_err(|error| DiffyError::Http(error.to_string()))?;
        let img = image::load_from_memory(&bytes)
            .map_err(|error| DiffyError::Parse(format!("avatar decode failed: {error}")))?
            .to_rgba8();
        let (w, h) = img.dimensions();
        let mut rgba = img.into_raw();
        apply_circular_mask(&mut rgba, w, h);
        Ok((rgba, w, h))
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.settings_store.save(settings)
    }

    pub fn open_repository_dialog(&self) -> Option<PathBuf> {
        rfd::FileDialog::new().pick_folder()
    }

    pub fn resolve_ref(&self, repo_path: &Path, reference: &str) -> Result<(String, String)> {
        let mut git = GitService::new();
        git.open(repo_path.to_string_lossy().as_ref())?;
        // libgit2 may not support bare `@` as HEAD alias; normalize it.
        let normalized;
        let reference =
            if reference == "@" || reference.starts_with("@~") || reference.starts_with("@^") {
                normalized = format!("HEAD{}", &reference[1..]);
                &normalized
            } else {
                reference
            };
        let oid = git.resolve_commit_oid(reference)?;
        let repo = git.repo()?;
        let commit = repo.find_commit(oid)?;
        let short_oid = git
            .abbreviate_oid(&oid.to_string())
            .unwrap_or_else(|_| oid.to_string()[..7].to_owned());
        let summary = commit.summary().unwrap_or_default().to_owned();
        Ok((short_oid, summary))
    }

    pub fn fetch_context_lines(
        &self,
        request: &crate::effects::FetchContextLinesRequest,
    ) -> Result<Vec<String>> {
        let mut git = GitService::new();
        git.open(request.repo_path.to_string_lossy().as_ref())?;
        git.read_file_lines_at(&request.reference, &request.path)
    }

    pub fn open_browser(&self, url: &str) -> Result<()> {
        webbrowser::open(url)
            .map(|_| ())
            .map_err(|error| DiffyError::General(format!("failed to open browser: {error}")))
    }

    pub(crate) fn run_commit_message_generation(
        &self,
        request: GenerateCommitMessageRequest,
        event_sender: RuntimeEventSender,
    ) {
        let GenerateCommitMessageRequest {
            repo_path,
            has_staged,
            provider,
            api_key,
            steering_prompt,
            subject_override,
            generation,
        } = request;

        let started = std::time::Instant::now();
        tracing::info!(
            generation,
            repo = %repo_path.display(),
            has_staged,
            provider = provider.label(),
            "ai: dispatch"
        );

        let diff_text = match read_commit_diff(&repo_path, has_staged) {
            Ok(text) => text,
            Err(error) => {
                tracing::error!(generation, %error, "ai: diff read failed");
                event_sender.send(AppEvent::CommitMessageGenerationFailed {
                    generation,
                    message: format!("failed to read diff: {error}"),
                });
                return;
            }
        };

        let raw_bytes = diff_text.len();
        let compressed = ai::diff_compress::compress_commit_diff(&diff_text, ai::MAX_DIFF_BYTES);
        tracing::debug!(
            generation,
            raw_bytes,
            compressed_bytes = compressed.len(),
            max_bytes = ai::MAX_DIFF_BYTES,
            "ai: diff compressed"
        );

        let user_message = ai::build_user_message(
            &steering_prompt,
            subject_override.as_deref(),
            &compressed,
        );
        tracing::debug!(
            generation,
            user_message_bytes = user_message.len(),
            "ai: prompt built"
        );

        let rx = ai::run_streaming(GenerateRequest {
            provider,
            api_key,
            user_message,
        });

        let mut chunk_count: usize = 0;
        let mut byte_count: usize = 0;
        loop {
            match rx.recv() {
                Ok(StreamMessage::Chunk(chunk)) => {
                    chunk_count += 1;
                    byte_count += chunk.len();
                    event_sender.send(AppEvent::CommitMessageChunk {
                        generation,
                        chunk,
                    });
                }
                Ok(StreamMessage::Finished) => {
                    tracing::info!(
                        generation,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        chunks = chunk_count,
                        bytes = byte_count,
                        "ai: stream finished"
                    );
                    event_sender.send(AppEvent::CommitMessageGenerationFinished { generation });
                    return;
                }
                Ok(StreamMessage::Failed(message)) => {
                    tracing::error!(
                        generation,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        chunks = chunk_count,
                        %message,
                        "ai: stream failed"
                    );
                    event_sender.send(AppEvent::CommitMessageGenerationFailed {
                        generation,
                        message,
                    });
                    return;
                }
                Err(_) => {
                    tracing::error!(generation, "ai: worker channel disconnected");
                    event_sender.send(AppEvent::CommitMessageGenerationFailed {
                        generation,
                        message: "llm worker exited unexpectedly".to_owned(),
                    });
                    return;
                }
            }
        }
    }
}

pub fn load_ai_keys() -> Result<(Option<String>, Option<String>)> {
    let openai = secrets::load_ai_key(AiKeyKind::OpenAi)?;
    let anthropic = secrets::load_ai_key(AiKeyKind::Anthropic)?;
    Ok((openai, anthropic))
}

fn read_commit_diff(repo_path: &Path, has_staged: bool) -> Result<String> {
    let mut git = GitService::new();
    git.open(repo_path.to_string_lossy().as_ref())?;
    git.diff_for_commit(has_staged)
}

/// Apply an anti-aliased circular alpha mask to a square-ish RGBA buffer in-place.
/// Pixels outside the inscribed circle are transparent; a 1-pixel band at the edge
/// is feathered by coverage so the circle renders smoothly.
fn apply_circular_mask(rgba: &mut [u8], width: u32, height: u32) {
    let cx = (width as f32 - 1.0) / 2.0;
    let cy = (height as f32 - 1.0) / 2.0;
    let radius = width.min(height) as f32 / 2.0 - 0.5;
    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let coverage = (radius - dist + 0.5).clamp(0.0, 1.0);
            if coverage < 1.0 {
                let idx = ((y * width + x) * 4 + 3) as usize;
                let a = rgba[idx] as f32 * coverage;
                rgba[idx] = a.round().clamp(0.0, 255.0) as u8;
            }
        }
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
    use crate::effects::CompareRequest;
    use crate::platform::persistence::SettingsStore;

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
    fn services_can_run_compare() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let left = commit_file(&repo, "src/lib.rs", "fn a() -> i32 { 1 }\n", "initial");
        let right = commit_file(&repo, "src/lib.rs", "fn a() -> i32 { 2 }\n", "second");

        let store_dir = TempDir::new().unwrap();
        let services = AppServices::new(SettingsStore::new_in(store_dir.path()));

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
