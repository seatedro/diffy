use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::ai::{self, GenerateRequest, StreamMessage};
use crate::apprt::ProgressReporter;
use crate::apprt::runtime::RuntimeEventSender;
use crate::core::compare::backends::{DifftasticBackend, GitDiffBackend};
use crate::core::compare::{CompareOutput, ComparePhase, ProgressSink, RendererKind};
use crate::core::error::{DiffyError, Result};
use crate::core::forge::github::{
    CreatePullRequestReviewComment, DeviceFlowState, GitHubApi, GitHubUser, PullRequestInfo,
    PullRequestReviewComment, parse_pr_url, poll_for_token, start_device_flow,
};
use crate::core::http;
use crate::core::syntax::annotator::FullFileSyntax;
use crate::core::vcs::discovery;
use crate::core::vcs::git::GitService;
use crate::core::vcs::model::RevisionId;
use crate::effects::{
    CompareFileRequest, CompareFileStatsRequest, CompareHistoryRequest, CompareRequest,
    CompareStatsRequest, GenerateCommitMessageRequest, LoadFileSyntaxRequest, StatusDiffRequest,
};
use crate::events::{
    AiEvent, CompareFileFinished, CompareFileStat, CompareFileStatsReady, CompareFinished,
    CompareHistoryReady, CompareStatsReady, StatusDiffFinished,
};
use crate::platform::persistence::{Settings, SettingsStore};
use crate::platform::secrets::{self, AiKeyKind};
use crate::ui::state::prepare_active_file;

#[derive(Debug, Clone)]
pub struct AppServices {
    settings_store: SettingsStore,
    syntax_cache: Arc<Mutex<FileSyntaxCache>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FileSyntaxCacheKey {
    repo_path: String,
    reference: String,
    path: String,
    generation: u64,
}

#[derive(Debug, Default)]
struct FileSyntaxCache {
    entries: HashMap<FileSyntaxCacheKey, Arc<FullFileSyntax>>,
}

impl FileSyntaxCache {
    fn insert(&mut self, key: FileSyntaxCacheKey, syntax: Arc<FullFileSyntax>) {
        const MAX_ENTRIES: usize = 8;
        if self.entries.len() >= MAX_ENTRIES
            && !self.entries.contains_key(&key)
            && let Some(first_key) = self.entries.keys().next().cloned()
        {
            self.entries.remove(&first_key);
        }
        self.entries.insert(key, syntax);
    }
}

impl AppServices {
    pub fn new(settings_store: SettingsStore) -> Self {
        Self {
            settings_store,
            syntax_cache: Arc::new(Mutex::new(FileSyntaxCache::default())),
        }
    }

    pub fn run_compare(
        &self,
        generation: u64,
        request: CompareRequest,
        reporter: Option<&ProgressReporter>,
    ) -> Result<CompareFinished> {
        if let Some(r) = reporter {
            r.phase(ComparePhase::OpeningRepo);
        }
        let mut repo = discovery::open_repository(&request.repo_path)?;

        if let Some(r) = reporter {
            r.phase(ComparePhase::ResolvingRefs);
        }
        let (resolved_left, resolved_right, backend_spec) =
            repo.resolve_compare_spec(&request.spec)?;

        // Phase labels are now driven from inside the backend (see
        // `EnumeratingChanges` + per-file `LoadingFiles`), so we just pass
        // the reporter through and let it speak for itself.
        let output = repo.compare(&backend_spec, reporter.map(|r| r as &dyn ProgressSink))?;

        Ok(CompareFinished {
            generation,
            spec: request.spec,
            resolved_left,
            resolved_right,
            output,
            range_commits: Vec::new(),
        })
    }

    pub fn load_status_diff(
        &self,
        generation: u64,
        index: usize,
        request: StatusDiffRequest,
    ) -> Result<StatusDiffFinished> {
        if let Some(location) = discovery::discover_repository(&request.repo_path)?
            && location.kind == crate::core::vcs::model::VcsKind::Jj
        {
            let mut repo = discovery::open_location(location)?;
            let output = repo.compare_working_file(&request.item.path)?;
            return Ok(StatusDiffFinished {
                generation,
                index,
                item: request.item,
                output,
            });
        }

        let mut git = GitService::new();
        git.open(request.repo_path.to_string_lossy().as_ref())?;
        let mut output: CompareOutput = match request.renderer {
            RendererKind::Builtin => git.diff_status_item(&request.item)?,
            RendererKind::Difftastic if DifftasticBackend::is_available() => {
                compare_status_item_with_difftastic(&request.item, &git)?
            }
            RendererKind::Difftastic => git.diff_status_item(&request.item)?,
        };
        if request.renderer == RendererKind::Difftastic && !DifftasticBackend::is_available() {
            mark_difftastic_fallback(&mut output);
        }

        Ok(StatusDiffFinished {
            generation,
            index,
            item: request.item,
            output,
        })
    }

    pub fn load_compare_file(
        &self,
        generation: u64,
        request: CompareFileRequest,
    ) -> Result<CompareFileFinished> {
        let mut repo = discovery::open_repository(&request.repo_path)?;
        let (_, _, backend_spec) = repo.resolve_compare_spec(&request.spec)?;
        let mut output =
            repo.compare_path(&backend_spec, &request.path, request.deferred_file.as_ref())?;

        let carbon_file = output.carbon.files.pop().ok_or_else(|| {
            DiffyError::General("compare file returned no Carbon file".to_owned())
        })?;

        Ok(CompareFileFinished {
            generation,
            index: request.index,
            path: request.path,
            prepared: prepare_active_file(request.index, &carbon_file),
        })
    }

    pub fn load_compare_stats(
        &self,
        generation: u64,
        request: CompareStatsRequest,
    ) -> Result<CompareStatsReady> {
        let mut repo = discovery::open_repository(&request.repo_path)?;
        let (_, _, backend_spec) = repo.resolve_compare_spec(&request.spec)?;
        let (additions, deletions) = repo.compare_stats(&backend_spec)?;

        Ok(CompareStatsReady {
            generation,
            additions,
            deletions,
        })
    }

    pub fn load_compare_history(
        &self,
        generation: u64,
        request: CompareHistoryRequest,
    ) -> Result<CompareHistoryReady> {
        if let Some(location) = discovery::discover_repository(&request.repo_path)?
            && location.kind == crate::core::vcs::model::VcsKind::Jj
        {
            return Ok(CompareHistoryReady {
                generation,
                range_commits: Vec::new(),
            });
        }

        let mut git = GitService::new();
        git.open(request.repo_path.to_string_lossy().as_ref())?;
        let range_commits = git
            .commits_in_range(&request.left_ref, &request.right_ref, 500)
            .unwrap_or_default();

        Ok(CompareHistoryReady {
            generation,
            range_commits,
        })
    }

    pub fn load_compare_file_stats(
        &self,
        generation: u64,
        request: CompareFileStatsRequest,
    ) -> Result<CompareFileStatsReady> {
        if let Some(location) = discovery::discover_repository(&request.repo_path)?
            && location.kind == crate::core::vcs::model::VcsKind::Jj
        {
            let stats = request
                .files
                .into_iter()
                .map(|item| CompareFileStat {
                    index: item.index,
                    path: item.file.path().to_owned(),
                    additions: u32_to_i32_saturating(item.file.additions),
                    deletions: u32_to_i32_saturating(item.file.deletions),
                })
                .collect();
            return Ok(CompareFileStatsReady {
                generation,
                stats,
                request_complete: true,
            });
        }

        let files: Vec<&carbon::FileDiff> = request.files.iter().map(|item| &item.file).collect();
        let repo_path = request.repo_path.to_string_lossy();
        let file_stats =
            GitDiffBackend.deferred_file_line_stats_batch_for_repo_path(&files, &repo_path);
        let mut stats = Vec::with_capacity(request.files.len());
        for (item, stat) in request.files.into_iter().zip(file_stats) {
            let (additions, deletions) = stat.unwrap_or((
                u32_to_i32_saturating(item.file.additions),
                u32_to_i32_saturating(item.file.deletions),
            ));
            stats.push(CompareFileStat {
                index: item.index,
                path: item.file.path().to_owned(),
                additions,
                deletions,
            });
        }

        Ok(CompareFileStatsReady {
            generation,
            stats,
            request_complete: true,
        })
    }

    pub fn load_file_syntax(
        &self,
        request: &LoadFileSyntaxRequest,
    ) -> Vec<crate::core::syntax::annotator::SyntaxLineTokens> {
        let Ok(mut repo) = discovery::open_repository(&request.repo_path) else {
            return Vec::new();
        };

        let annotator = crate::core::syntax::DiffSyntaxAnnotator::new();
        let old_syntax =
            self.cached_file_syntax(&mut *repo, request, &request.left_ref, &annotator);
        let new_syntax =
            self.cached_file_syntax(&mut *repo, request, &request.right_ref, &annotator);

        annotator.annotate_carbon_full_file_window_from_cache(
            &request.carbon_file,
            request.file_index,
            old_syntax.as_deref(),
            new_syntax.as_deref(),
            request.window,
        )
    }

    fn cached_file_syntax(
        &self,
        repo: &mut dyn crate::core::vcs::backend::VcsRepository,
        request: &LoadFileSyntaxRequest,
        reference: &str,
        annotator: &crate::core::syntax::DiffSyntaxAnnotator,
    ) -> Option<Arc<FullFileSyntax>> {
        if reference.is_empty() {
            return None;
        }
        let key = FileSyntaxCacheKey {
            repo_path: request.repo_path.to_string_lossy().into_owned(),
            reference: reference.to_owned(),
            path: request.path.clone(),
            generation: request.cache_generation,
        };

        if let Some(cached) = self
            .syntax_cache
            .lock()
            .ok()
            .and_then(|cache| cache.entries.get(&key).cloned())
        {
            return Some(cached);
        }

        let revision = RevisionId {
            backend: repo.location().kind,
            id: reference.to_owned(),
        };
        let text = repo.read_file_text(&revision, &request.path).ok()?;
        let syntax = Arc::new(annotator.highlight_full_text_store(&request.path, &text));
        if syntax.has_tokens()
            && let Ok(mut cache) = self.syntax_cache.lock()
        {
            cache.insert(key, syntax.clone());
        }
        Some(syntax)
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

    pub fn check_for_updates(
        &self,
        current_version: &str,
    ) -> Result<crate::core::update::UpdateCheck> {
        crate::core::update::check_for_update(current_version)
    }

    pub fn stage_update(
        &self,
        update: &crate::core::update::AvailableUpdate,
    ) -> Result<crate::core::update::StagedUpdate> {
        crate::core::update::download_and_stage(update)
    }

    pub fn apply_staged_update(&self, staged: &crate::core::update::StagedUpdate) -> Result<()> {
        crate::core::update::apply_staged_update(staged)
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

    pub fn fetch_pull_request_review_comments(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
        token: Option<String>,
    ) -> Result<Vec<PullRequestReviewComment>> {
        let api = match token {
            Some(t) if !t.is_empty() => GitHubApi::with_token(t),
            _ => GitHubApi::new(),
        };
        api.fetch_pull_request_review_comments(owner, repo, number)
    }

    pub fn create_pull_request_review_comment(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
        token: Option<String>,
        comment: &CreatePullRequestReviewComment,
    ) -> Result<PullRequestReviewComment> {
        let token = token.unwrap_or_default();
        GitHubApi::with_token(token)
            .create_pull_request_review_comment(owner, repo, number, comment)
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
        let bytes = http::block_on(async {
            let response = reqwest::Client::new()
                .get(url)
                .header("User-Agent", "diffy/0.1")
                .send()
                .await
                .map_err(|error| DiffyError::Http(format!("avatar fetch failed: {error}")))?;
            http::response_bytes(response, "avatar fetch").await
        })?;
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
        let mut repo = discovery::open_repository(repo_path)?;
        repo.resolve_ref(reference)
    }

    pub fn fetch_context_lines(
        &self,
        request: &crate::effects::FetchContextLinesRequest,
    ) -> Result<(Vec<String>, Vec<String>)> {
        let mut repo = discovery::open_repository(&request.repo_path)?;
        let old_lines = if request.old_reference.is_empty() {
            Vec::new()
        } else {
            read_file_lines_from_repo(&mut *repo, &request.old_reference, &request.path)
                .unwrap_or_default()
        };
        let new_lines = if request.new_reference.is_empty() {
            Vec::new()
        } else {
            read_file_lines_from_repo(&mut *repo, &request.new_reference, &request.path)
                .unwrap_or_default()
        };
        Ok((old_lines, new_lines))
    }

    pub fn install_common_syntax_packs(&self) -> Result<Vec<String>> {
        let installer = phosphor::PackInstaller::new()
            .map_err(|error| DiffyError::General(error.to_string()))?;
        http::block_on(async {
            installer
                .install_common_packs()
                .await
                .map(|languages| {
                    languages
                        .into_iter()
                        .map(|language| language.name().to_owned())
                        .collect()
                })
                .map_err(|error| DiffyError::General(error.to_string()))
        })
    }

    pub fn ensure_syntax_pack_for_path(&self, path: &str) -> Result<Option<String>> {
        let installer = phosphor::PackInstaller::new()
            .map_err(|error| DiffyError::General(error.to_string()))?;
        http::block_on(async {
            installer
                .ensure_pack_for_path(Path::new(path))
                .await
                .map(|language| language.map(|language| language.name().to_owned()))
                .map_err(|error| DiffyError::General(error.to_string()))
        })
    }

    pub fn ensure_syntax_packs_for_paths(&self, paths: &[String]) -> Result<Vec<String>> {
        let installer = phosphor::PackInstaller::new()
            .map_err(|error| DiffyError::General(error.to_string()))?;
        http::block_on(async {
            installer
                .ensure_packs_for_paths(paths.iter().map(|path| Path::new(path.as_str())))
                .await
                .map(|languages| {
                    languages
                        .into_iter()
                        .map(|language| language.name().to_owned())
                        .collect()
                })
                .map_err(|error| DiffyError::General(error.to_string()))
        })
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
                event_sender.send(AiEvent::CommitMessageGenerationFailed {
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

        let user_message =
            ai::build_user_message(&steering_prompt, subject_override.as_deref(), &compressed);
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
                    event_sender.send(AiEvent::CommitMessageChunk { generation, chunk });
                }
                Ok(StreamMessage::Finished) => {
                    tracing::info!(
                        generation,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        chunks = chunk_count,
                        bytes = byte_count,
                        "ai: stream finished"
                    );
                    event_sender.send(AiEvent::CommitMessageGenerationFinished { generation });
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
                    event_sender.send(AiEvent::CommitMessageGenerationFailed {
                        generation,
                        message,
                    });
                    return;
                }
                Err(_) => {
                    tracing::error!(generation, "ai: worker channel disconnected");
                    event_sender.send(AiEvent::CommitMessageGenerationFailed {
                        generation,
                        message: "AI worker exited unexpectedly".to_owned(),
                    });
                    return;
                }
            }
        }
    }
}

#[cfg(feature = "difftastic")]
fn compare_status_item_with_difftastic(
    item: &crate::core::vcs::git::StatusItem,
    git: &GitService,
) -> Result<CompareOutput> {
    DifftasticBackend.compare_status_item(item, git)
}

#[cfg(not(feature = "difftastic"))]
fn compare_status_item_with_difftastic(
    _item: &crate::core::vcs::git::StatusItem,
    _git: &GitService,
) -> Result<CompareOutput> {
    unreachable!("difftastic status compare is gated by DifftasticBackend::is_available()")
}

fn mark_difftastic_fallback(output: &mut CompareOutput) {
    output.used_fallback = true;
    output.fallback_message = "difftastic not compiled in, used built-in backend".to_owned();
}

fn u32_to_i32_saturating(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn read_file_lines_from_repo(
    repo: &mut dyn crate::core::vcs::backend::VcsRepository,
    reference: &str,
    path: &str,
) -> Result<Vec<String>> {
    let revision = RevisionId {
        backend: repo.location().kind,
        id: reference.to_owned(),
    };
    let text = repo.read_file_text(&revision, path)?;
    Ok((0..text.line_count())
        .filter_map(|line| text.line_str(carbon::LineId(line)).map(str::to_owned))
        .collect())
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
                None,
            )
            .unwrap();

        assert_eq!(finished.generation, 1);
        assert_eq!(finished.output.carbon.files.len(), 1);
        assert_eq!(finished.output.carbon.files[0].path(), "src/lib.rs");
    }
}
