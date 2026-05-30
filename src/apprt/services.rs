use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use crate::ai::{self, GenerateRequest, StreamMessage};
use crate::apprt::ProgressReporter;
use crate::apprt::runtime::RuntimeEventSender;
use crate::core::compare::{ComparePhase, ProgressSink};
use crate::core::error::{DiffyError, Result};
use crate::core::forge::github::{
    CreatePullRequestReview, CreatePullRequestReviewComment, CreatePullRequestReviewReply,
    DeviceFlowState, GitHubApi, GitHubPullRequestReviewData, GitHubPullRequestReviewThreadComment,
    GitHubReviewThreadResolution, GitHubUser, PullRequestInfo, PullRequestReview,
    PullRequestReviewComment, SubmitPullRequestReview, UpdatePullRequestReviewComment,
    parse_pr_url, poll_for_token, start_device_flow,
};
use crate::core::http;
use crate::core::review::{ReviewDecision, ReviewSession, ReviewSessionKey, ReviewTarget};
use crate::core::syntax::annotator::FullFileSyntax;
use crate::core::vcs::discovery;
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
use crate::platform::review_store::ReviewStore;
use crate::platform::secrets::{self, AiKeyKind};
use crate::ui::state::prepare_active_file;

const DEV_GITHUB_TOKEN_FILE_NAME: &str = "github-token.dev";

#[derive(Debug, Clone)]
pub struct AppServices {
    settings_store: SettingsStore,
    review_store: ReviewStore,
    syntax_cache: Arc<Mutex<FileSyntaxCache>>,
    syntax_cache_ready: Arc<Condvar>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FileSyntaxCacheKey {
    repo_path: String,
    reference: String,
    path: String,
    generation: u64,
    epoch: u64,
}

#[derive(Debug, Default)]
struct FileSyntaxCache {
    entries: HashMap<FileSyntaxCacheKey, FileSyntaxCacheEntry>,
    inflight: HashSet<FileSyntaxCacheKey>,
    bytes: usize,
    tick: u64,
    epoch: u64,
}

#[derive(Debug)]
struct FileSyntaxCacheEntry {
    syntax: Arc<FullFileSyntax>,
    bytes: usize,
    last_used: u64,
}

impl FileSyntaxCache {
    fn get(&mut self, key: &FileSyntaxCacheKey) -> Option<Arc<FullFileSyntax>> {
        let tick = self.next_tick();
        let entry = self.entries.get_mut(key)?;
        entry.last_used = tick;
        Some(entry.syntax.clone())
    }

    fn insert(&mut self, key: FileSyntaxCacheKey, syntax: Arc<FullFileSyntax>) {
        const MAX_ENTRIES: usize = 128;
        const BYTE_BUDGET: usize = 48 * 1024 * 1024;

        let bytes = syntax.estimated_bytes().max(1);
        let tick = self.next_tick();
        if let Some(previous) = self.entries.insert(
            key,
            FileSyntaxCacheEntry {
                syntax,
                bytes,
                last_used: tick,
            },
        ) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }
        self.bytes = self.bytes.saturating_add(bytes);

        while self.entries.len() > MAX_ENTRIES
            || (self.entries.len() > 1 && self.bytes > BYTE_BUDGET)
        {
            let Some(victim) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&victim) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
            }
        }
    }

    fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.saturating_add(1);
        self.tick
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.inflight.clear();
        self.bytes = 0;
        self.epoch = self.epoch.saturating_add(1);
    }
}

impl AppServices {
    pub fn new(settings_store: SettingsStore) -> Self {
        let review_store = ReviewStore::for_settings_store(&settings_store);
        Self {
            settings_store,
            review_store,
            syntax_cache: Arc::new(Mutex::new(FileSyntaxCache::default())),
            syntax_cache_ready: Arc::new(Condvar::new()),
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
        let (resolved_left, resolved_right) = repo.resolve_compare_request(&request.request)?;

        // Phase labels are now driven from inside the backend (see
        // `EnumeratingChanges` + per-file `LoadingFiles`), so we just pass
        // the reporter through and let it speak for itself.
        let output = repo.compare(&request.request, reporter.map(|r| r as &dyn ProgressSink))?;

        Ok(CompareFinished {
            generation,
            request: request.request,
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
        let mut repo = discovery::open_repository(&request.repo_path)?;
        let output = repo.file_change_diff(&request.file_change, request.renderer)?;

        Ok(StatusDiffFinished {
            generation,
            index,
            file_change: request.file_change,
            output,
        })
    }

    pub fn load_compare_file(
        &self,
        generation: u64,
        request: CompareFileRequest,
    ) -> Result<CompareFileFinished> {
        let mut repo = discovery::open_repository(&request.repo_path)?;
        let mut output = repo.compare_path(
            &request.request,
            &request.path,
            request.deferred_file.as_ref(),
        )?;

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
        let (additions, deletions) = repo.compare_stats(&request.request)?;

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
        let mut repo = discovery::open_repository(&request.repo_path)?;
        let range_commits = repo.compare_history(&request.left_ref, &request.right_ref, 500)?;

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
        let mut repo = discovery::open_repository(&request.repo_path)?;
        let files = request
            .files
            .iter()
            .map(|item| item.target.clone())
            .collect::<Vec<_>>();
        let file_stats = repo.compare_file_stats(&request.request, &files)?;
        let mut stats = Vec::with_capacity(request.files.len());
        for (item, stat) in request.files.into_iter().zip(file_stats) {
            let (additions, deletions) = stat;
            stats.push(CompareFileStat {
                index: item.index,
                path: item.target.path().into_owned(),
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

    pub fn load_file_syntax<F>(
        &self,
        request: &LoadFileSyntaxRequest,
        is_current: &F,
    ) -> Vec<crate::core::syntax::annotator::SyntaxLineTokens>
    where
        F: Fn() -> bool,
    {
        if !is_current() {
            return Vec::new();
        }
        let Ok(mut repo) = discovery::open_repository(&request.repo_path) else {
            return Vec::new();
        };

        let annotator = crate::core::syntax::DiffSyntaxAnnotator::new();
        let old_syntax = request
            .carbon_file
            .old_path
            .as_deref()
            .and_then(|old_path| {
                self.cached_file_syntax(
                    &mut *repo,
                    request,
                    &request.left_ref,
                    old_path,
                    &annotator,
                    is_current,
                )
            });
        if !is_current() {
            return Vec::new();
        }
        let new_syntax = request
            .carbon_file
            .new_path
            .as_deref()
            .and_then(|new_path| {
                self.cached_file_syntax(
                    &mut *repo,
                    request,
                    &request.right_ref,
                    new_path,
                    &annotator,
                    is_current,
                )
            });
        if !is_current() {
            return Vec::new();
        }

        annotator.annotate_carbon_full_file_window_from_cache(
            &request.carbon_file,
            &request.carbon_expansion,
            request.file_index,
            old_syntax.as_deref(),
            new_syntax.as_deref(),
            request.window,
        )
    }

    fn cached_file_syntax<F>(
        &self,
        repo: &mut dyn crate::core::vcs::backend::VcsRepository,
        request: &LoadFileSyntaxRequest,
        reference: &str,
        source_path: &str,
        annotator: &crate::core::syntax::DiffSyntaxAnnotator,
        is_current: &F,
    ) -> Option<Arc<FullFileSyntax>>
    where
        F: Fn() -> bool,
    {
        if reference.is_empty() || !is_current() {
            return None;
        }
        let mut cache = self.syntax_cache.lock().ok()?;
        let key = FileSyntaxCacheKey {
            repo_path: request.repo_path.to_string_lossy().into_owned(),
            reference: reference.to_owned(),
            path: source_path.to_owned(),
            generation: request.cache_generation,
            epoch: cache.epoch,
        };
        loop {
            if cache.epoch != key.epoch {
                return None;
            }
            if let Some(cached) = cache.get(&key) {
                return Some(cached);
            }
            if cache.inflight.insert(key.clone()) {
                break;
            }
            cache = self
                .syntax_cache_ready
                .wait_timeout(cache, Duration::from_millis(25))
                .ok()?
                .0;
            if cache.epoch != key.epoch || !is_current() {
                return None;
            }
        }
        if cache.epoch != key.epoch || !is_current() {
            cache.inflight.remove(&key);
            self.syntax_cache_ready.notify_all();
            return None;
        }
        drop(cache);

        let revision = RevisionId {
            backend: repo.location().kind,
            id: reference.to_owned(),
        };
        let text = match repo.read_file_text(&revision, source_path) {
            Ok(text) => text,
            Err(_) => {
                if let Ok(mut cache) = self.syntax_cache.lock() {
                    cache.inflight.remove(&key);
                    self.syntax_cache_ready.notify_all();
                }
                return None;
            }
        };
        if !is_current() {
            if let Ok(mut cache) = self.syntax_cache.lock() {
                cache.inflight.remove(&key);
                self.syntax_cache_ready.notify_all();
            }
            return None;
        }
        let syntax = Arc::new(annotator.highlight_full_text_store(source_path, &text));
        match self.syntax_cache.lock() {
            Ok(mut cache) => {
                cache.inflight.remove(&key);
                if cache.epoch != key.epoch || !is_current() {
                    self.syntax_cache_ready.notify_all();
                    return None;
                }
                cache.insert(key, syntax.clone());
                self.syntax_cache_ready.notify_all();
            }
            Err(_) => self.syntax_cache_ready.notify_all(),
        }
        Some(syntax)
    }

    pub fn clear_file_syntax_cache(&self) {
        if let Ok(mut cache) = self.syntax_cache.lock() {
            cache.clear();
            self.syntax_cache_ready.notify_all();
        }
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

        let mut repo = discovery::open_repository(repo_path)?;
        if !repo.capabilities().github_pull_requests {
            return Err(DiffyError::General(
                "this repository backend does not support GitHub pull request comparisons"
                    .to_owned(),
            ));
        }
        let (left_ref, right_ref) = repo.resolve_pull_request_comparison(url, &token)?;
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

    pub fn load_dev_github_token(&self) -> Result<Option<String>> {
        secrets::load_github_token_file(&self.dev_github_token_path()?)
    }

    pub fn save_dev_github_token(&self, token: &str) -> Result<()> {
        secrets::save_github_token_file(&self.dev_github_token_path()?, token)
    }

    pub fn clear_dev_github_token(&self) -> Result<()> {
        secrets::clear_github_token_file(&self.dev_github_token_path()?)
    }

    fn dev_github_token_path(&self) -> Result<PathBuf> {
        let parent = self.settings_store.path().parent().ok_or_else(|| {
            DiffyError::General(format!(
                "settings path has no parent directory: {}",
                self.settings_store.path().display()
            ))
        })?;
        Ok(parent.join(DEV_GITHUB_TOKEN_FILE_NAME))
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

    pub fn fetch_pull_request_review_data(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
        token: Option<String>,
    ) -> Result<GitHubPullRequestReviewData> {
        let token = token.unwrap_or_default();
        GitHubApi::with_token(token).fetch_pull_request_review_data(owner, repo, number)
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

    pub fn create_pull_request_review_reply(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
        comment_id: i64,
        token: Option<String>,
        reply: &CreatePullRequestReviewReply,
    ) -> Result<PullRequestReviewComment> {
        let token = token.unwrap_or_default();
        GitHubApi::with_token(token)
            .create_pull_request_review_reply(owner, repo, number, comment_id, reply)
    }

    pub fn update_pull_request_review_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: i64,
        token: Option<String>,
        update: &UpdatePullRequestReviewComment,
    ) -> Result<PullRequestReviewComment> {
        let token = token.unwrap_or_default();
        GitHubApi::with_token(token)
            .update_pull_request_review_comment(owner, repo, comment_id, update)
    }

    pub fn delete_pull_request_review_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: i64,
        token: Option<String>,
    ) -> Result<()> {
        let token = token.unwrap_or_default();
        GitHubApi::with_token(token).delete_pull_request_review_comment(owner, repo, comment_id)
    }

    pub fn create_pull_request_review(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
        token: Option<String>,
        review: &CreatePullRequestReview,
    ) -> Result<PullRequestReview> {
        let token = token.unwrap_or_default();
        GitHubApi::with_token(token).create_pull_request_review(owner, repo, number, review)
    }

    pub fn submit_review_session_drafts(
        &self,
        session: &ReviewSession,
        decision: ReviewDecision,
        body: Option<String>,
        token: Option<String>,
    ) -> Result<PullRequestReview> {
        let review = session.build_github_review_request(decision, body)?;
        let token = token.unwrap_or_default();
        GitHubApi::with_token(token).create_pull_request_review(
            &session.target.owner,
            &session.target.repo,
            session.target.number,
            &review,
        )
    }

    pub fn submit_pull_request_review(
        &self,
        owner: &str,
        repo: &str,
        number: i32,
        review_id: i64,
        token: Option<String>,
        submit: &SubmitPullRequestReview,
    ) -> Result<PullRequestReview> {
        let token = token.unwrap_or_default();
        GitHubApi::with_token(token)
            .submit_pull_request_review(owner, repo, number, review_id, submit)
    }

    pub fn add_pull_request_review_thread_reply(
        &self,
        thread_node_id: &str,
        review_node_id: Option<&str>,
        token: Option<String>,
        body: &str,
    ) -> Result<GitHubPullRequestReviewThreadComment> {
        let token = token.unwrap_or_default();
        GitHubApi::with_token(token).add_pull_request_review_thread_reply(
            thread_node_id,
            review_node_id,
            body,
        )
    }

    pub fn update_pull_request_review_comment_graphql(
        &self,
        comment_node_id: &str,
        token: Option<String>,
        body: &str,
    ) -> Result<GitHubPullRequestReviewThreadComment> {
        let token = token.unwrap_or_default();
        GitHubApi::with_token(token)
            .update_pull_request_review_comment_graphql(comment_node_id, body)
    }

    pub fn delete_pull_request_review_comment_graphql(
        &self,
        comment_node_id: &str,
        token: Option<String>,
    ) -> Result<Option<GitHubPullRequestReviewThreadComment>> {
        let token = token.unwrap_or_default();
        GitHubApi::with_token(token).delete_pull_request_review_comment_graphql(comment_node_id)
    }

    pub fn set_pull_request_review_thread_resolution(
        &self,
        thread_node_id: &str,
        token: Option<String>,
        resolved: bool,
    ) -> Result<GitHubReviewThreadResolution> {
        let token = token.unwrap_or_default();
        GitHubApi::with_token(token)
            .set_pull_request_review_thread_resolution(thread_node_id, resolved)
    }

    pub fn load_review_session(
        &self,
        target: ReviewTarget,
        pull_request: PullRequestInfo,
    ) -> Result<ReviewSession> {
        Ok(self
            .review_store
            .load_session(&target, &pull_request.head_sha)?
            .unwrap_or_else(|| ReviewSession::new(target, pull_request)))
    }

    pub fn save_review_session(&self, session: &ReviewSession) -> Result<ReviewSessionKey> {
        let key = session.key();
        self.review_store.save_session(session)?;
        Ok(key)
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
        let compressed =
            ai::diff_compress::build_commit_diff_payload(&diff_text, ai::MAX_DIFF_BYTES);
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
    let mut repo = discovery::open_repository(repo_path)?;
    repo.commit_diff(has_staged)
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
    use crate::core::compare::{LayoutMode, RendererKind};
    use crate::core::vcs::model::{VcsCompareRequest, VcsCompareSpec};
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
                    request: VcsCompareRequest {
                        spec: VcsCompareSpec::Range {
                            from: left,
                            to: right,
                        },
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
