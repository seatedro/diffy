use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use diffy_ui::diff_view::decorator::DiffDecorator;
use quark_ui::FocusId;
use quark_ui::text_input::{Editor, EditorMode, TextEditCommand, TextEditOutcome, TextField};

use super::presentation::{MutationPresentation, PresentationStyle};
use crate::ai::{DEFAULT_STEERING_PROMPT, Provider};
use crate::app::text_targets::{
    COMMIT_MESSAGE, SETTINGS_ANTHROPIC_KEY, SETTINGS_OPENAI_KEY, SETTINGS_STEERING_PROMPT, TextCx,
    TextHost,
};
use crate::contracts::commands::{
    CommandDescriptor, CommandSource, ConfirmationRequest, ToastKind, ToastUpdate,
};
use crate::contracts::mutations::{
    ConfirmedMutation, FileOperationRequest, GenerateCommitMessageRequest, MutationSelection,
    MutationSource, MutationSourceKey, MutationsAction, MutationsEffect, MutationsEvent,
    PatchOperationRequest, StagedDigest,
};
use crate::contracts::navigation::RepositoryContext;
use crate::contracts::review::Secret;
use crate::contracts::shared::{FileKey, FileRevision, JobId, SharedContext, Side, SourceTarget};
use crate::contracts::shell::{Action, Controller, Intent, LogicalFocus, Reduction};
use crate::core::keymap::ShortcutCommand;
use crate::core::vcs::model::JjOperation;
use crate::core::vcs::model::{
    ChangeBucket, FileChange, FileOperation, PublishAction, PublishPlan, RefKind, RepoCapabilities,
    VcsOperation, VcsRef,
};
use crate::platform::secrets::AiKeyKind;

const JOB_BASE: u64 = 0xE << 48;
const MAX_SOURCES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineKey {
    pub file: FileKey,
    pub revision: FileRevision,
    pub side: Side,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Job {
    Patch,
    Files,
    Commit,
    Operation(VcsOperation),
    Fetch(String),
    Push(String),
    Publish,
    PublishPlan,
    Pull(String),
}

#[derive(Debug, Clone)]
enum SourceSlot {
    Loading,
    Ready(Arc<MutationSource>),
}

#[derive(Debug, Clone)]
struct PendingPatch {
    job: JobId,
    key: MutationSourceKey,
    selection: MutationSelection,
    operation: FileOperation,
}

#[derive(Debug, Default)]
pub struct AiState {
    pub generation: u64,
    pub active: bool,
    pub error: Option<String>,
    pub provider: Option<Provider>,
    pub steering: Option<String>,
    staged: Option<StagedDigest>,
    observed: Option<StagedDigest>,
    has_staged: bool,
    checks: u32,
    finished: bool,
    buffered: String,
    pub openai: Option<Secret>,
    pub anthropic: Option<Secret>,
    pub openai_editing: bool,
    pub anthropic_editing: bool,
}

#[derive(Debug, Default)]
pub struct PublishMenu {
    pub loading: bool,
    pub plan: Option<PublishPlan>,
}

pub struct MutationsController {
    presentation: Rc<MutationPresentation>,
    selection: BTreeSet<LineKey>,
    anchor: Option<LineKey>,
    sources: HashMap<MutationSourceKey, SourceSlot>,
    pending: Vec<PendingPatch>,
    jobs: HashMap<JobId, Job>,
    next_job: u64,
    commit: Editor,
    composer_open: bool,
    ai: AiState,
    publish: Option<PublishMenu>,
    openai_field: TextField,
    anthropic_field: TextField,
    steering: Editor,
    steering_synced: String,
}

impl std::fmt::Debug for MutationsController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MutationsController")
            .field("selection", &self.selection.len())
            .field("jobs", &self.jobs)
            .field("composer_open", &self.composer_open)
            .finish_non_exhaustive()
    }
}

impl Default for MutationsController {
    fn default() -> Self {
        Self {
            presentation: Rc::default(),
            selection: BTreeSet::new(),
            anchor: None,
            sources: HashMap::new(),
            pending: Vec::new(),
            jobs: HashMap::new(),
            next_job: JOB_BASE,
            commit: Editor::new(EditorMode::ProseInput),
            composer_open: false,
            ai: AiState::default(),
            publish: None,
            openai_field: TextField::new(""),
            anthropic_field: TextField::new(""),
            steering: Editor::new(EditorMode::ProseInput),
            steering_synced: String::new(),
        }
    }
}

fn toast(kind: ToastKind, id: Option<JobId>, message: impl Into<String>) -> Intent {
    Intent::Toast(ToastUpdate::Show {
        id,
        kind,
        message: message.into(),
        description: None,
    })
}

fn failure(id: Option<JobId>, message: impl Into<String>, detail: impl Into<String>) -> Intent {
    Intent::Toast(ToastUpdate::Show {
        id,
        kind: ToastKind::Error,
        message: message.into(),
        description: Some(detail.into()),
    })
}

fn error(message: impl Into<String>) -> Reduction {
    Reduction::changed().intent(Intent::Toast(ToastUpdate::error(message)))
}

fn progress(id: JobId, message: impl Into<String>) -> Intent {
    Intent::Toast(ToastUpdate::Progress {
        id,
        message: message.into(),
        completed: 0,
        total: 0,
    })
}

fn capabilities(cx: &SharedContext) -> Option<RepoCapabilities> {
    cx.repository
        .as_ref()
        .map(|repo| repo.snapshot.capabilities)
}

fn repo_path(cx: &SharedContext) -> Option<PathBuf> {
    cx.repository
        .as_ref()
        .map(|repo| repo.snapshot.path.clone())
}

fn committable(bucket: ChangeBucket, staging_area: bool) -> bool {
    if staging_area {
        bucket == ChangeBucket::Staged
    } else {
        matches!(bucket, ChangeBucket::WorkingCopy | ChangeBucket::Conflicted)
    }
}

fn active_ref(refs: &[VcsRef]) -> Option<&VcsRef> {
    refs.iter().find(|reference| {
        reference.active && matches!(reference.kind, RefKind::Branch | RefKind::Bookmark)
    })
}

fn upstream_pair(upstream: &str) -> Option<(String, String)> {
    upstream
        .split_once('/')
        .map(|(remote, branch)| (remote.to_owned(), branch.to_owned()))
}

fn remote_names(refs: &[VcsRef]) -> BTreeSet<String> {
    let mut remotes = BTreeSet::new();
    for reference in refs {
        if let Some((remote, _)) = reference
            .upstream
            .as_deref()
            .and_then(|u| u.split_once('/'))
        {
            remotes.insert(remote.to_owned());
        }
        if matches!(
            reference.kind,
            RefKind::RemoteBranch | RefKind::RemoteBookmark
        ) && let Some((remote, _)) = reference.name.split_once('/')
        {
            remotes.insert(remote.to_owned());
        }
    }
    remotes
}

fn line_ranges(lines: impl IntoIterator<Item = LineKey>) -> Vec<SourceTarget> {
    let mut targets: Vec<SourceTarget> = Vec::new();
    for key in lines {
        if let Some(last) = targets.last_mut()
            && last.file == key.file
            && last.revision == key.revision
            && last.side == key.side
            && last.lines.end == key.line
        {
            last.lines.end += 1;
            continue;
        }
        targets.push(SourceTarget {
            file: key.file,
            revision: key.revision,
            side: key.side,
            lines: key.line..key.line + 1,
        });
    }
    targets
}

fn keys(target: &SourceTarget) -> impl Iterator<Item = LineKey> + '_ {
    target.lines.clone().map(|line| LineKey {
        file: target.file,
        revision: target.revision,
        side: target.side,
        line,
    })
}

impl MutationsController {
    pub fn presentation(&self) -> Rc<MutationPresentation> {
        self.presentation.clone()
    }

    pub fn sync_style(&mut self, style: PresentationStyle) {
        if self.presentation.style() == style {
            return;
        }
        let current = &self.presentation;
        self.presentation = Rc::new(current.restyled(style));
    }

    pub fn selected_lines(&self) -> &BTreeSet<LineKey> {
        &self.selection
    }

    pub fn commit_editor(&self) -> &Editor {
        &self.commit
    }

    pub fn commit_editor_mut(&mut self) -> &mut Editor {
        &mut self.commit
    }

    pub fn composer_open(&self) -> bool {
        self.composer_open
    }

    pub fn ai(&self) -> &AiState {
        &self.ai
    }

    pub fn publish_menu(&self) -> Option<&PublishMenu> {
        self.publish.as_ref()
    }

    pub fn busy(&self) -> bool {
        self.jobs
            .values()
            .any(|job| matches!(job, Job::Patch | Job::Files))
    }

    pub fn commit_text(&self) -> &str {
        self.commit.text()
    }

    fn job(&mut self, job: Job) -> JobId {
        self.next_job += 1;
        let id = JobId(self.next_job);
        self.jobs.insert(id, job);
        id
    }

    fn refresh_presentation(&mut self, cx: &SharedContext) {
        let capabilities = capabilities(cx);
        let next = MutationPresentation::new(
            self.presentation.revision().wrapping_add(1),
            self.presentation.style(),
            capabilities.is_some_and(|c| c.staging_area),
            capabilities.is_some_and(|c| c.partial_hunk_mutation),
            self.selection.clone(),
        );
        if !self.presentation.same_content(&next) {
            self.presentation = Rc::new(next);
        }
    }

    fn file_change(
        &self,
        cx: &SharedContext,
        file: FileKey,
        operation: FileOperation,
    ) -> Result<FileChange, &'static str> {
        let repo = cx
            .repository
            .as_ref()
            .ok_or("Open a repository before changing files.")?;
        let manifest = cx
            .manifest
            .as_ref()
            .ok_or("This file is no longer in the comparison.")?;
        let entry = manifest
            .files
            .iter()
            .find(|entry| entry.key == file)
            .ok_or("This file is no longer in the comparison.")?;
        let path = entry
            .paths
            .new_path()
            .or_else(|| entry.paths.old_path())
            .ok_or("This file has no path.")?;
        let staging = repo.snapshot.capabilities.staging_area;
        let order: &[ChangeBucket] = match (staging, operation) {
            (false, _) => &[ChangeBucket::WorkingCopy, ChangeBucket::Conflicted],
            (true, FileOperation::Stage) => &[ChangeBucket::Unstaged, ChangeBucket::Untracked],
            (true, FileOperation::Unstage) => &[ChangeBucket::Staged],
            (true, FileOperation::Discard) => &[
                ChangeBucket::Unstaged,
                ChangeBucket::Untracked,
                ChangeBucket::Staged,
            ],
        };
        order
            .iter()
            .find_map(|bucket| {
                repo.snapshot
                    .file_changes
                    .iter()
                    .find(|change| change.path == path && change.bucket == *bucket)
            })
            .cloned()
            .ok_or(match operation {
                FileOperation::Stage => "This file has no unstaged changes.",
                FileOperation::Unstage => "This file has no staged changes.",
                FileOperation::Discard => "This file has no changes to discard.",
            })
    }

    fn request_patch(
        &mut self,
        selection: MutationSelection,
        operation: FileOperation,
        confirmed: bool,
        cx: &SharedContext,
    ) -> Reduction {
        let Some(capabilities) = capabilities(cx) else {
            return error("Open a repository before changing lines.");
        };
        if !capabilities.partial_hunk_mutation {
            return error("This repository backend does not support line operations.");
        }
        let Some((file, revision)) = selection.file() else {
            return error("Select lines from one file at a time.");
        };
        if operation == FileOperation::Discard && !confirmed {
            return Reduction::default().intent(Intent::RequestConfirmation(ConfirmationRequest {
                title: "Discard changes?".to_owned(),
                message: "The selected changes will be removed from your working tree.".to_owned(),
                confirm_label: "Discard".to_owned(),
                destructive: true,
                confirm: Box::new(Action::from(MutationsAction::ApplyConfirmed(
                    ConfirmedMutation::Selection(selection),
                ))),
                return_focus: Some(LogicalFocus::Viewport),
            }));
        }
        if self.busy() {
            return Reduction::default();
        }
        let current = cx.manifest.as_ref().and_then(|manifest| {
            manifest
                .files
                .iter()
                .find(|entry| entry.key == file)
                .map(|entry| entry.revision)
        });
        if current != Some(revision) {
            return error("This file changed since it was shown. Try again.");
        }
        let change = match self.file_change(cx, file, operation) {
            Ok(change) => change,
            Err(message) => return error(message),
        };
        let (Some(repo_path), Some(repository)) = (
            repo_path(cx),
            cx.repository.as_ref().map(|repo| repo.revision),
        ) else {
            return error("Open a repository before changing lines.");
        };
        let key = MutationSourceKey {
            repo_path,
            file,
            revision,
            repository,
            change,
        };
        let job = self.job(Job::Patch);
        let pending = PendingPatch {
            job,
            key: key.clone(),
            selection,
            operation,
        };
        match self.sources.get(&key) {
            Some(SourceSlot::Ready(source)) => {
                let source = source.clone();
                Reduction::changed().effect(MutationsEffect::ApplyPatch(PatchOperationRequest {
                    job,
                    source,
                    selection: pending.selection,
                    operation,
                }))
            }
            Some(SourceSlot::Loading) => {
                self.pending.push(pending);
                Reduction::changed()
            }
            None => {
                if self.sources.len() >= MAX_SOURCES {
                    self.sources.clear();
                }
                self.sources.insert(key.clone(), SourceSlot::Loading);
                self.pending.push(pending);
                Reduction::changed().effect(MutationsEffect::LoadMutationSource(key))
            }
        }
    }

    fn source_ready(&mut self, source: Arc<MutationSource>) -> Reduction {
        let key = source.key().clone();
        if !matches!(self.sources.get(&key), Some(SourceSlot::Loading)) {
            return Reduction::default();
        }
        self.sources
            .insert(key.clone(), SourceSlot::Ready(source.clone()));
        let (ready, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|pending| pending.key == key);
        self.pending = waiting;
        let mut reduction = Reduction::changed();
        for pending in ready {
            reduction = reduction.effect(MutationsEffect::ApplyPatch(PatchOperationRequest {
                job: pending.job,
                source: source.clone(),
                selection: pending.selection,
                operation: pending.operation,
            }));
        }
        reduction
    }

    fn source_failed(&mut self, key: MutationSourceKey, message: String) -> Reduction {
        if !matches!(self.sources.get(&key), Some(SourceSlot::Loading)) {
            return Reduction::default();
        }
        self.sources.remove(&key);
        let (failed, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|pending| pending.key == key);
        self.pending = waiting;
        for pending in &failed {
            self.jobs.remove(&pending.job);
        }
        Reduction::changed().intent(failure(None, "Could not read the file's changes", message))
    }

    fn file_operation(
        &mut self,
        files: Vec<FileKey>,
        operation: FileOperation,
        confirmed: bool,
        cx: &SharedContext,
    ) -> Reduction {
        let Some(capabilities) = capabilities(cx) else {
            return error("Open a repository before changing files.");
        };
        if operation != FileOperation::Discard && !capabilities.staging_area {
            return error("This repository backend does not support staging.");
        }
        if files.is_empty() || self.busy() {
            return Reduction::default();
        }
        if operation == FileOperation::Discard && !confirmed {
            let message = if files.len() == 1 {
                "All changes to this file will be lost.".to_owned()
            } else {
                format!("All changes to {} files will be lost.", files.len())
            };
            return Reduction::default().intent(Intent::RequestConfirmation(ConfirmationRequest {
                title: "Discard changes?".to_owned(),
                message,
                confirm_label: "Discard".to_owned(),
                destructive: true,
                confirm: Box::new(Action::from(MutationsAction::ApplyConfirmed(
                    ConfirmedMutation::Files(files),
                ))),
                return_focus: Some(LogicalFocus::Viewport),
            }));
        }
        let mut changes = Vec::with_capacity(files.len());
        for file in files {
            match self.file_change(cx, file, operation) {
                Ok(change) => changes.push(change),
                Err(message) => return error(message),
            }
        }
        self.bucket_operation(changes, operation, cx)
    }

    fn bucket_operation(
        &mut self,
        file_changes: Vec<FileChange>,
        operation: FileOperation,
        cx: &SharedContext,
    ) -> Reduction {
        let Some(repo_path) = repo_path(cx) else {
            return error("Open a repository before changing files.");
        };
        if file_changes.is_empty() || self.busy() {
            return Reduction::default();
        }
        let job = self.job(Job::Files);
        Reduction::changed().effect(MutationsEffect::ApplyFileOperation(FileOperationRequest {
            job,
            repo_path,
            file_changes,
            operation,
        }))
    }

    fn all_in(
        &mut self,
        buckets: &[ChangeBucket],
        operation: FileOperation,
        cx: &SharedContext,
    ) -> Reduction {
        if !capabilities(cx).is_some_and(|c| c.staging_area) {
            return error("This repository backend does not support staging.");
        }
        let changes = cx
            .repository
            .as_ref()
            .map(|repo| {
                repo.snapshot
                    .file_changes
                    .iter()
                    .filter(|change| buckets.contains(&change.bucket))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        self.bucket_operation(changes, operation, cx)
    }

    fn selection_operation(
        &mut self,
        operation: FileOperation,
        confirmed: bool,
        cx: &SharedContext,
    ) -> Reduction {
        if self.selection.is_empty() {
            return error("Select changed lines first.");
        }
        let targets = line_ranges(self.selection.iter().copied());
        let reduction =
            self.request_patch(MutationSelection::Lines(targets), operation, confirmed, cx);
        if !reduction.effects.is_empty() || !self.pending.is_empty() {
            self.selection.clear();
            self.anchor = None;
            self.refresh_presentation(cx);
        }
        reduction
    }

    fn toggle_lines(&mut self, target: &SourceTarget) {
        let lines: Vec<_> = keys(target).collect();
        let all = lines.iter().all(|key| self.selection.contains(key));
        for key in &lines {
            if all {
                self.selection.remove(key);
            } else {
                self.selection.insert(*key);
            }
        }
        self.selection
            .retain(|key| key.file == target.file && key.revision == target.revision);
        self.anchor = lines.last().copied();
    }

    fn extend_lines(&mut self, target: &SourceTarget) {
        let Some(anchor) = self.anchor.filter(|a| {
            a.file == target.file && a.revision == target.revision && a.side == target.side
        }) else {
            self.toggle_lines(target);
            return;
        };
        let start = anchor.line.min(target.lines.start);
        let end = (anchor.line + 1).max(target.lines.end);
        let range = SourceTarget {
            lines: start..end,
            ..target.clone()
        };
        self.selection
            .retain(|key| key.file == target.file && key.revision == target.revision);
        self.selection.extend(keys(&range));
    }

    fn select_lines(&mut self, target: &SourceTarget) {
        self.selection.clear();
        self.selection.extend(keys(target));
        self.anchor = keys(target).last();
    }

    fn submit_commit(&mut self, cx: &SharedContext) -> Reduction {
        let Some(repo) = cx.repository.as_ref() else {
            return error("Open a repository before committing.");
        };
        let capabilities = repo.snapshot.capabilities;
        if !capabilities.create_commit {
            return error("This repository backend does not support commits.");
        }
        let message = self.commit.text().trim().to_owned();
        if message.is_empty() {
            return error("Write a commit message first.");
        }
        if !repo
            .snapshot
            .file_changes
            .iter()
            .any(|change| committable(change.bucket, capabilities.staging_area))
        {
            return error(if capabilities.staging_area {
                "Stage changes before committing."
            } else {
                "There are no changes to commit."
            });
        }
        if self.jobs.values().any(|job| *job == Job::Commit) {
            return Reduction::default();
        }
        let job = self.job(Job::Commit);
        Reduction::changed()
            .effect(MutationsEffect::CreateCommit {
                job,
                repo_path: repo.snapshot.path.clone(),
                message,
            })
            .intent(progress(job, "Committing\u{2026}"))
    }

    fn generate_commit_message(&mut self, cx: &SharedContext) -> Reduction {
        if self.ai.active {
            return Reduction::default();
        }
        let Some(repo) = cx.repository.as_ref() else {
            return error("Open a repository before generating a commit message.");
        };
        let key = |provider: Provider| {
            match provider {
                Provider::OpenAi => self.ai.openai.clone(),
                Provider::Anthropic => self.ai.anthropic.clone(),
            }
            .filter(|key| !key.is_empty())
            .map(|key| (provider, key))
        };
        let chosen = match self.ai.provider {
            Some(provider) => key(provider),
            None => key(Provider::Anthropic).or_else(|| key(Provider::OpenAi)),
        };
        let Some((provider, api_key)) = chosen else {
            return error(match self.ai.provider {
                Some(provider) => format!(
                    "Add a {} key under Settings \u{2192} Clankers first.",
                    provider.label()
                ),
                None => "Add an AI key under Settings \u{2192} Clankers first.".to_owned(),
            });
        };
        let staging = repo.snapshot.capabilities.staging_area;
        let has_staged = staging
            && repo
                .snapshot
                .file_changes
                .iter()
                .any(|change| change.bucket == ChangeBucket::Staged);
        let steering = self
            .ai
            .steering
            .as_ref()
            .unwrap_or(&cx.settings.settings.ai_steering_prompt);
        let steering_prompt = if steering.trim().is_empty() {
            DEFAULT_STEERING_PROMPT.to_owned()
        } else {
            steering.clone()
        };
        let subject_override = self
            .commit
            .text()
            .lines()
            .next()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_owned);
        if subject_override.is_some() && !self.commit.text().ends_with('\n') {
            self.commit.append("\n");
        }
        self.ai.generation = self.ai.generation.wrapping_add(1);
        self.ai.active = true;
        self.ai.error = None;
        self.ai.staged = None;
        self.ai.observed = None;
        self.ai.has_staged = has_staged;
        self.ai.checks = 0;
        self.ai.finished = false;
        self.ai.buffered.clear();
        self.composer_open = true;
        Reduction::changed().effect(MutationsEffect::GenerateCommitMessage(Box::new(
            GenerateCommitMessageRequest {
                generation: self.ai.generation,
                repository: repo.revision,
                repo_path: repo.snapshot.path.clone(),
                has_staged,
                provider,
                api_key,
                steering_prompt,
                subject_override,
            },
        )))
    }

    fn ai_current(&self, generation: u64) -> bool {
        self.ai.active && generation == self.ai.generation
    }

    fn stop_stale_generation(&mut self) -> Reduction {
        self.ai.active = false;
        self.ai.buffered.clear();
        self.ai.checks = 0;
        Reduction::changed().intent(toast(
            ToastKind::Info,
            None,
            "Staged changes moved, so commit message generation stopped.",
        ))
    }

    fn settle_generation(&mut self) {
        if self.ai.checks > 0 || !self.ai.active {
            return;
        }
        if !self.ai.buffered.is_empty() {
            let buffered = std::mem::take(&mut self.ai.buffered);
            self.commit.append(&buffered);
        }
        if self.ai.finished {
            self.ai.active = false;
        }
    }

    fn staged_seen(
        &mut self,
        generation: u64,
        staged: Option<StagedDigest>,
        started: bool,
    ) -> Reduction {
        if !self.ai_current(generation) {
            return Reduction::default();
        }
        if !started {
            self.ai.checks = self.ai.checks.saturating_sub(1);
        }
        let Some(staged) = staged else {
            return self.stop_stale_generation();
        };
        let expected = if started {
            let observed = self.ai.observed.take();
            self.ai.staged = Some(staged);
            observed
        } else if self.ai.staged.is_some() {
            self.ai.staged
        } else {
            self.ai.observed = Some(staged);
            None
        };
        if expected.is_some_and(|expected| expected != staged) {
            return self.stop_stale_generation();
        }
        self.settle_generation();
        Reduction::changed()
    }

    fn save_key_field(&mut self, kind: AiKeyKind) -> Reduction {
        let field = match kind {
            AiKeyKind::OpenAi => &mut self.openai_field,
            AiKeyKind::Anthropic => &mut self.anthropic_field,
        };
        let value = field.text().trim().to_owned();
        if value.is_empty() {
            return error("Paste a key first.");
        }
        field.set_text("");
        match kind {
            AiKeyKind::OpenAi => self.ai.openai_editing = false,
            AiKeyKind::Anthropic => self.ai.anthropic_editing = false,
        }
        self.set_ai_key(kind, Secret::new(value))
    }

    fn sync_steering(&mut self, prompt: &str) {
        let shown = if prompt.trim().is_empty() {
            DEFAULT_STEERING_PROMPT
        } else {
            prompt
        };
        if self.steering.text() == self.steering_synced && self.steering_synced != shown {
            self.steering.set_text(shown);
            self.steering_synced = shown.to_owned();
        }
    }

    pub fn key_field(&self, kind: AiKeyKind) -> &TextField {
        match kind {
            AiKeyKind::OpenAi => &self.openai_field,
            AiKeyKind::Anthropic => &self.anthropic_field,
        }
    }

    pub fn steering_editor(&self) -> &Editor {
        &self.steering
    }

    pub fn steering_editor_mut(&mut self) -> &mut Editor {
        &mut self.steering
    }

    fn set_ai_key(&mut self, kind: AiKeyKind, value: Secret) -> Reduction {
        let effect = if value.is_empty() {
            MutationsEffect::ClearAiKey { kind }
        } else {
            MutationsEffect::SaveAiKey {
                kind,
                value: value.clone(),
            }
        };
        let stored = (!value.is_empty()).then_some(value);
        match kind {
            AiKeyKind::OpenAi => self.ai.openai = stored,
            AiKeyKind::Anthropic => self.ai.anthropic = stored,
        }
        Reduction::changed().effect(effect)
    }

    fn run_operation(
        &mut self,
        operation: VcsOperation,
        confirmed: bool,
        cx: &SharedContext,
    ) -> Reduction {
        let Some(repo_path) = repo_path(cx) else {
            return error("Open a repository before running repository operations.");
        };
        if !cx
            .repository
            .as_ref()
            .is_some_and(|repo| repo.snapshot.capabilities.operation_log)
        {
            return error("This operation is not available for the current repository backend.");
        }
        if !confirmed && let Some(message) = operation.confirmation_message() {
            return Reduction::default().intent(Intent::RequestConfirmation(ConfirmationRequest {
                title: format!("{}?", operation.label()),
                message,
                confirm_label: operation.label(),
                destructive: true,
                confirm: Box::new(Action::from(MutationsAction::RunConfirmedOperation(
                    operation,
                ))),
                return_focus: None,
            }));
        }
        let label = format!("{}\u{2026}", operation.progress_label());
        let job = self.job(Job::Operation(operation.clone()));
        Reduction::changed()
            .effect(MutationsEffect::RunOperation {
                job,
                repo_path,
                operation,
            })
            .intent(progress(job, label))
    }

    fn remotes_allowed(
        cx: &SharedContext,
        what: &str,
    ) -> Result<(PathBuf, Rc<RepositoryContext>), Reduction> {
        let Some(repo) = cx.repository.clone() else {
            return Err(error(format!("Open a repository before {what}.")));
        };
        if !repo.snapshot.capabilities.remotes {
            return Err(error("This repository backend does not support remotes."));
        }
        Ok((repo.snapshot.path.clone(), repo))
    }

    fn fetch(&mut self, remotes: Vec<String>, repo_path: PathBuf) -> Reduction {
        if remotes.is_empty() {
            return error("No remotes are configured for this repository.");
        }
        let mut reduction = Reduction::changed();
        for remote in remotes {
            let job = self.job(Job::Fetch(remote.clone()));
            reduction = reduction
                .intent(progress(job, format!("Fetching {remote}\u{2026}")))
                .effect(MutationsEffect::FetchRemote {
                    job,
                    repo_path: repo_path.clone(),
                    remote,
                });
        }
        reduction
    }

    fn push(&mut self, force_with_lease: bool, cx: &SharedContext) -> Reduction {
        let (repo_path, repo) = match Self::remotes_allowed(cx, "pushing") {
            Ok(found) => found,
            Err(reduction) => return reduction,
        };
        let refs = &repo.snapshot.refs;
        let Some(branch) = active_ref(refs) else {
            return error("No active branch or bookmark to push.");
        };
        let name = branch.name.clone();
        let (remote, refspec) = match branch.upstream.as_deref().and_then(upstream_pair) {
            Some((remote, upstream)) => {
                (remote, format!("refs/heads/{name}:refs/heads/{upstream}"))
            }
            None => {
                let remotes = remote_names(refs);
                let Some(remote) = remotes
                    .get("origin")
                    .or_else(|| remotes.iter().next())
                    .cloned()
                else {
                    return error("No remotes are configured for this repository.");
                };
                (remote, format!("refs/heads/{name}:refs/heads/{name}"))
            }
        };
        let label = if force_with_lease {
            format!("Force-pushing {name} to {remote}\u{2026}")
        } else {
            format!("Pushing {name} to {remote}\u{2026}")
        };
        let job = self.job(Job::Push(remote.clone()));
        Reduction::changed()
            .intent(progress(job, label))
            .effect(MutationsEffect::Push {
                job,
                repo_path,
                remote,
                refspec,
                force_with_lease,
            })
    }

    fn publish(&mut self, action: Option<PublishAction>, cx: &SharedContext) -> Reduction {
        let (repo_path, _) = match Self::remotes_allowed(cx, "publishing") {
            Ok(found) => found,
            Err(reduction) => return reduction,
        };
        if let Some(action) = &action
            && let Some(reason) = &action.disabled_reason
        {
            return error(reason.clone());
        }
        let label = action.as_ref().map_or_else(
            || "Publishing\u{2026}".to_owned(),
            |a| format!("{}\u{2026}", a.label),
        );
        self.publish = None;
        let job = self.job(Job::Publish);
        Reduction::changed()
            .intent(progress(job, label))
            .effect(MutationsEffect::Publish {
                job,
                repo_path,
                action: action.map(Box::new),
            })
    }

    fn open_publish_menu(&mut self, cx: &SharedContext) -> Reduction {
        let (repo_path, repo) = match Self::remotes_allowed(cx, "publishing") {
            Ok(found) => found,
            Err(reduction) => return reduction,
        };
        self.publish = Some(PublishMenu {
            loading: true,
            plan: repo.snapshot.publish_plan.clone(),
        });
        let job = self.job(Job::PublishPlan);
        Reduction::changed().effect(MutationsEffect::LoadPublishPlan {
            job: Some(job),
            repo_path,
        })
    }

    fn pull(&mut self, cx: &SharedContext) -> Reduction {
        let Some(repo) = cx.repository.as_ref() else {
            return error("Open a repository before pulling.");
        };
        if !repo.snapshot.capabilities.pull_fast_forward {
            return error("This repository backend does not support fast-forward pull.");
        }
        let Some(branch) = active_ref(&repo.snapshot.refs) else {
            return error("No active branch or bookmark to pull into.");
        };
        let Some((remote, upstream)) = branch.upstream.as_deref().and_then(upstream_pair) else {
            return error(format!(
                "No upstream configured for {}. Push once to set one.",
                branch.name
            ));
        };
        let job = self.job(Job::Pull(remote.clone()));
        Reduction::changed()
            .intent(progress(
                job,
                format!("Pulling {} from {remote}\u{2026}", branch.name),
            ))
            .effect(MutationsEffect::PullFastForward {
                job,
                repo_path: repo.snapshot.path.clone(),
                remote,
                branch: upstream,
            })
    }

    fn finish(&mut self, job: JobId) -> Option<Job> {
        self.jobs.remove(&job)
    }

    fn reset(&mut self) {
        self.selection.clear();
        self.anchor = None;
        self.sources.clear();
        self.pending.clear();
        self.jobs.clear();
        self.publish = None;
        self.ai.active = false;
        self.ai.error = None;
    }
}

impl Controller for MutationsController {
    type Action = MutationsAction;
    type Event = MutationsEvent;

    fn reduce_action(&mut self, action: Self::Action, cx: &SharedContext) -> Reduction {
        use MutationsAction as A;
        let selected = cx.selected_file.map(|file| file.key);
        match action {
            A::StageSelectedFile => self.file_operation(
                selected.into_iter().collect(),
                FileOperation::Stage,
                false,
                cx,
            ),
            A::UnstageSelectedFile => self.file_operation(
                selected.into_iter().collect(),
                FileOperation::Unstage,
                false,
                cx,
            ),
            A::DiscardSelectedFile => self.file_operation(
                selected.into_iter().collect(),
                FileOperation::Discard,
                false,
                cx,
            ),
            A::StageFile(file) => self.file_operation(vec![file], FileOperation::Stage, false, cx),
            A::UnstageFile(file) => {
                self.file_operation(vec![file], FileOperation::Unstage, false, cx)
            }
            A::DiscardFile(file) => {
                self.file_operation(vec![file], FileOperation::Discard, false, cx)
            }
            A::StageAllFiles => self.all_in(
                &[ChangeBucket::Unstaged, ChangeBucket::Untracked],
                FileOperation::Stage,
                cx,
            ),
            A::UnstageAllFiles => self.all_in(&[ChangeBucket::Staged], FileOperation::Unstage, cx),
            A::StageHunk | A::UnstageHunk | A::DiscardHunk => {
                let operation = match action {
                    A::StageHunk => FileOperation::Stage,
                    A::UnstageHunk => FileOperation::Unstage,
                    _ => FileOperation::Discard,
                };
                match cx.selection.clone() {
                    Some(target) => self.request_patch(
                        MutationSelection::HunksAt(vec![target]),
                        operation,
                        false,
                        cx,
                    ),
                    None => error("Move the cursor into a hunk first."),
                }
            }
            A::StageHunkAt(target) => self.request_patch(
                MutationSelection::Hunks(vec![target]),
                FileOperation::Stage,
                false,
                cx,
            ),
            A::UnstageHunkAt(target) => self.request_patch(
                MutationSelection::Hunks(vec![target]),
                FileOperation::Unstage,
                false,
                cx,
            ),
            A::DiscardHunkAt(target) => self.request_patch(
                MutationSelection::Hunks(vec![target]),
                FileOperation::Discard,
                false,
                cx,
            ),
            A::SelectLines(target) => {
                self.select_lines(&target);
                self.refresh_presentation(cx);
                Reduction::changed()
            }
            A::ToggleLineSelection(target) => {
                self.toggle_lines(&target);
                self.refresh_presentation(cx);
                Reduction::changed()
            }
            A::ExtendLineSelection(target) => {
                self.extend_lines(&target);
                self.refresh_presentation(cx);
                Reduction::changed()
            }
            A::ToggleCurrentLineSelection | A::ToggleCurrentLineSelectionRange => {
                let Some(target) = cx.selection.clone() else {
                    return error("Move the cursor to a changed line before selecting lines.");
                };
                if action == A::ToggleCurrentLineSelection {
                    self.toggle_lines(&target);
                } else {
                    self.extend_lines(&target);
                }
                self.refresh_presentation(cx);
                Reduction::changed()
            }
            A::ClearLineSelection => {
                self.selection.clear();
                self.anchor = None;
                self.refresh_presentation(cx);
                Reduction::changed()
            }
            A::StageSelectedLines => self.selection_operation(FileOperation::Stage, false, cx),
            A::UnstageSelectedLines => self.selection_operation(FileOperation::Unstage, false, cx),
            A::DiscardSelectedLines => self.selection_operation(FileOperation::Discard, false, cx),
            A::ApplyConfirmed(ConfirmedMutation::Selection(selection)) => {
                self.request_patch(selection, FileOperation::Discard, true, cx)
            }
            A::ApplyConfirmed(ConfirmedMutation::Files(files)) => {
                self.file_operation(files, FileOperation::Discard, true, cx)
            }
            A::SubmitCommit => self.submit_commit(cx),
            A::RunOperation(operation) => self.run_operation(operation, false, cx),
            A::RunConfirmedOperation(operation) => self.run_operation(operation, true, cx),
            A::FetchRemote(remote) => match Self::remotes_allowed(cx, "fetching") {
                Ok((repo_path, _)) => self.fetch(vec![remote], repo_path),
                Err(reduction) => reduction,
            },
            A::FetchAllRemotes => match Self::remotes_allowed(cx, "fetching") {
                Ok((repo_path, repo)) => {
                    let remotes = remote_names(&repo.snapshot.refs).into_iter().collect();
                    self.fetch(remotes, repo_path)
                }
                Err(reduction) => reduction,
            },
            A::Push { force_with_lease } => self.push(force_with_lease, cx),
            A::PublishDefault => self.publish(None, cx),
            A::OpenPublishMenu => self.open_publish_menu(cx),
            A::Publish(action) => self.publish(Some(*action), cx),
            A::ClosePublishMenu => {
                self.publish = None;
                Reduction::changed()
            }
            A::PullCurrentBranch => self.pull(cx),
            A::SetAiKey { kind, value } => self.set_ai_key(kind, value),
            A::ClearAiKey { kind } => {
                match kind {
                    AiKeyKind::OpenAi => {
                        self.ai.openai = None;
                        self.ai.openai_editing = false;
                    }
                    AiKeyKind::Anthropic => {
                        self.ai.anthropic = None;
                        self.ai.anthropic_editing = false;
                    }
                }
                Reduction::changed().effect(MutationsEffect::ClearAiKey { kind })
            }
            A::SetAiKeyEditing { kind, editing } => {
                let focus = match kind {
                    AiKeyKind::OpenAi => {
                        self.ai.openai_editing = editing;
                        self.openai_field.set_text("");
                        LogicalFocus::SettingsOpenAiKey
                    }
                    AiKeyKind::Anthropic => {
                        self.ai.anthropic_editing = editing;
                        self.anthropic_field.set_text("");
                        LogicalFocus::SettingsAnthropicKey
                    }
                };
                let reduction = Reduction::changed();
                if editing {
                    reduction.effect(crate::contracts::shell::ShellEffect::Focus(Some(focus)))
                } else {
                    reduction
                }
            }
            A::GenerateCommitMessage => self.generate_commit_message(cx),
            A::OpenCommitComposer => {
                self.composer_open = true;
                Reduction::changed().effect(crate::contracts::shell::ShellEffect::Focus(Some(
                    LogicalFocus::CommitMessage,
                )))
            }
            A::ScrollCommitMessage(lines) => {
                let step = self.commit.scroll_line_height_px();
                self.commit.scroll(lines as f32 * step);
                Reduction::changed()
            }
            A::SetAiProvider(provider) => {
                self.ai.provider = provider;
                Reduction::changed()
            }
            A::SaveAiKeyField(kind) => self.save_key_field(kind),
            A::SaveSteeringPrompt => {
                let prompt = self.steering.text().trim().to_owned();
                self.steering_synced = self.steering.text().to_owned();
                self.ai.steering = Some(prompt);
                Reduction::changed().intent(toast(
                    ToastKind::Success,
                    None,
                    "Steering prompt saved.",
                ))
            }
            A::ResetSteeringPrompt => {
                self.steering.set_text(DEFAULT_STEERING_PROMPT);
                self.steering_synced = DEFAULT_STEERING_PROMPT.to_owned();
                self.ai.steering = Some(String::new());
                Reduction::changed()
            }
            A::ScrollSteeringPrompt(lines) => {
                let step = self.steering.scroll_line_height_px();
                self.steering.scroll(lines as f32 * step);
                Reduction::changed()
            }
            A::CloseCommitComposer => {
                self.composer_open = false;
                Reduction::changed().effect(crate::contracts::shell::ShellEffect::Focus(Some(
                    LogicalFocus::Viewport,
                )))
            }
        }
    }

    fn reduce_event(&mut self, event: Self::Event, _cx: &SharedContext) -> Reduction {
        use MutationsEvent as E;
        match event {
            E::MutationSourceReady { source } => self.source_ready(source),
            E::MutationSourceFailed { key, message } => self.source_failed(key, message),
            E::FileOperationApplied { job, .. } => {
                self.finish(job);
                Reduction::changed()
            }
            E::FileOperationFailed { job, message, .. } => {
                if self.finish(job).is_none() {
                    return Reduction::default();
                }
                Reduction::changed().intent(Intent::Toast(ToastUpdate::error(message)))
            }
            E::CommitCreated { job, .. } => {
                if self.finish(job).is_none() {
                    return Reduction::default();
                }
                self.commit.set_text("");
                self.composer_open = false;
                self.ai.active = false;
                Reduction::changed().intent(toast(ToastKind::Success, Some(job), "Commit created."))
            }
            E::CommitFailed { job, message, .. } => {
                if self.finish(job).is_none() {
                    return Reduction::default();
                }
                Reduction::changed().intent(failure(Some(job), "Commit failed", message))
            }
            E::OperationComplete { job, message, .. } => {
                self.finish(job);
                Reduction::changed().intent(toast(ToastKind::Success, Some(job), message))
            }
            E::OperationFailed {
                job,
                operation,
                message,
            } => {
                self.finish(job);
                Reduction::changed().intent(failure(
                    Some(job),
                    format!("{} failed", operation.label()),
                    message,
                ))
            }
            E::FetchProgress {
                job,
                received_objects,
                total_objects,
                received_bytes,
            } => {
                let Some(Job::Fetch(remote)) = self.jobs.get(&job) else {
                    return Reduction::default();
                };
                let kib = received_bytes / 1024;
                let message = if kib > 0 {
                    format!(
                        "Fetching {remote}: {received_objects}/{total_objects} objects ({kib} KiB)\u{2026}"
                    )
                } else {
                    format!("Fetching {remote}: {received_objects}/{total_objects} objects\u{2026}")
                };
                Reduction::default().intent(Intent::Toast(ToastUpdate::Progress {
                    id: job,
                    message,
                    completed: received_objects as u64,
                    total: total_objects as u64,
                }))
            }
            E::FetchComplete { job, remote, .. } => {
                self.finish(job);
                Reduction::changed().intent(toast(
                    ToastKind::Success,
                    Some(job),
                    format!("Fetched {remote}"),
                ))
            }
            E::FetchFailed {
                job,
                remote,
                message,
            } => {
                self.finish(job);
                Reduction::changed().intent(failure(
                    Some(job),
                    format!("Fetch from {remote} failed"),
                    message,
                ))
            }
            E::PushProgress {
                job,
                current,
                total,
                bytes,
            } => {
                let Some(Job::Push(remote)) = self.jobs.get(&job) else {
                    return Reduction::default();
                };
                let kib = bytes / 1024;
                let message = if kib > 0 {
                    format!("Pushing to {remote}: {current}/{total} objects ({kib} KiB)\u{2026}")
                } else {
                    format!("Pushing to {remote}: {current}/{total} objects\u{2026}")
                };
                Reduction::default().intent(Intent::Toast(ToastUpdate::Progress {
                    id: job,
                    message,
                    completed: current as u64,
                    total: total as u64,
                }))
            }
            E::PushComplete {
                job,
                remote,
                branch,
                ..
            } => {
                self.finish(job);
                Reduction::changed().intent(toast(
                    ToastKind::Success,
                    Some(job),
                    format!("Pushed {branch} to {remote}"),
                ))
            }
            E::PushFailed {
                job,
                remote,
                message,
            } => {
                self.finish(job);
                Reduction::changed().intent(failure(
                    Some(job),
                    format!("Push to {remote} failed"),
                    message,
                ))
            }
            E::PublishComplete { job, label, .. } => {
                self.finish(job);
                Reduction::changed().intent(toast(ToastKind::Success, Some(job), label))
            }
            E::PublishFailed { job, message } => {
                self.finish(job);
                Reduction::changed().intent(failure(Some(job), "Publish failed", message))
            }
            E::PublishPlanReady { job, plan, .. } => {
                if let Some(job) = job {
                    self.finish(job);
                }
                match self.publish.as_mut() {
                    Some(menu) => {
                        menu.loading = false;
                        menu.plan = Some(*plan);
                        Reduction::changed()
                    }
                    None => Reduction::default(),
                }
            }
            E::PublishPlanFailed { job, message } => {
                if let Some(job) = job {
                    self.finish(job);
                }
                if let Some(menu) = self.publish.as_mut() {
                    menu.loading = false;
                }
                Reduction::changed().intent(failure(None, "Publish options failed", message))
            }
            E::PullComplete {
                job,
                remote,
                branch,
                already_up_to_date,
                behind,
                ..
            } => {
                self.finish(job);
                let message = if already_up_to_date {
                    format!("{branch} is already up to date with {remote}")
                } else if behind == 0 {
                    format!("Pulled {branch} from {remote}")
                } else {
                    format!("Fast-forwarded {branch} by {behind} commit(s) from {remote}")
                };
                Reduction::changed().intent(toast(ToastKind::Success, Some(job), message))
            }
            E::PullFailed {
                job,
                branch,
                message,
                ..
            } => {
                self.finish(job);
                Reduction::changed().intent(failure(
                    Some(job),
                    format!("Pull into {branch} failed"),
                    message,
                ))
            }
            E::WorkerStopped => {
                self.jobs.clear();
                self.pending.clear();
                self.sources.clear();
                error("Version control worker stopped. Restart Diffy.")
            }
            E::AiKeysLoaded { openai, anthropic } => {
                self.ai.openai = openai.filter(|key| !key.is_empty());
                self.ai.anthropic = anthropic.filter(|key| !key.is_empty());
                Reduction::changed()
            }
            E::AiKeysLoadFailed { message } => {
                tracing::warn!("failed to load AI keys from keyring: {message}");
                Reduction::default()
            }
            E::AiKeySaveFailed { message } => {
                error(format!("Couldn't save AI key to keyring: {message}"))
            }
            E::CommitMessageStarted { generation, staged } => {
                self.staged_seen(generation, Some(staged), true)
            }
            E::StagedDiffChecked { generation, staged } => {
                self.staged_seen(generation, staged, false)
            }
            E::CommitMessageChunk {
                generation, chunk, ..
            } => {
                if !self.ai_current(generation) {
                    tracing::debug!(
                        generation,
                        current = self.ai.generation,
                        "ai: dropping stale chunk"
                    );
                    return Reduction::default();
                }
                if self.ai.checks > 0 {
                    self.ai.buffered.push_str(&chunk);
                    return Reduction::default();
                }
                self.commit.append(&chunk);
                Reduction::changed()
            }
            E::CommitMessageFinished { generation, .. } => {
                if !self.ai_current(generation) {
                    return Reduction::default();
                }
                self.ai.finished = true;
                self.settle_generation();
                Reduction::changed()
            }
            E::CommitMessageFailed {
                generation,
                message,
                ..
            } => {
                if !self.ai_current(generation) {
                    return Reduction::default();
                }
                self.ai.active = false;
                self.ai.error = Some(message.clone());
                Reduction::changed().intent(failure(
                    None,
                    "Commit message generation failed",
                    message,
                ))
            }
        }
    }

    fn apply_intent(&mut self, intent: &Intent, cx: &SharedContext) -> Reduction {
        match intent {
            Intent::Startup(startup) => {
                self.sync_steering(&startup.settings.ai_steering_prompt);
                Reduction::default().effect(MutationsEffect::LoadAiKeys)
            }
            Intent::SettingsChanged(snapshot) => {
                if self.ai.steering.is_none() {
                    self.sync_steering(&snapshot.settings.ai_steering_prompt);
                }
                Reduction::changed()
            }
            Intent::ActivateWorkspace { .. } => {
                self.reset();
                self.composer_open = false;
                self.commit.set_text("");
                self.refresh_presentation(cx);
                Reduction::changed()
            }
            Intent::RepositoryChanged(repo) => {
                self.sources
                    .retain(|key, _| key.repository == repo.revision);
                let stale: Vec<_> = self
                    .pending
                    .iter()
                    .filter(|pending| pending.key.repository != repo.revision)
                    .map(|pending| pending.job)
                    .collect();
                self.pending
                    .retain(|pending| pending.key.repository == repo.revision);
                let mut reduction = Reduction::changed();
                if !stale.is_empty() {
                    for job in &stale {
                        self.jobs.remove(job);
                    }
                    reduction = reduction.intent(Intent::Toast(ToastUpdate::error(
                        "The repository changed before the lines were applied. Try again.",
                    )));
                }
                if self.ai.active {
                    self.ai.checks += 1;
                    reduction = reduction.effect(MutationsEffect::CheckStagedDiff {
                        generation: self.ai.generation,
                        repo_path: repo.snapshot.path.clone(),
                        has_staged: self.ai.has_staged,
                    });
                }
                self.refresh_presentation(cx);
                reduction
            }
            Intent::PublishManifest(manifest) => {
                let live: HashMap<FileKey, FileRevision> = manifest
                    .files
                    .iter()
                    .map(|file| (file.key, file.revision))
                    .collect();
                let before = self.selection.len();
                self.selection
                    .retain(|key| live.get(&key.file) == Some(&key.revision));
                if self
                    .anchor
                    .is_some_and(|key| live.get(&key.file) != Some(&key.revision))
                {
                    self.anchor = None;
                }
                self.refresh_presentation(cx);
                Reduction {
                    changed: before != self.selection.len(),
                    ..Reduction::default()
                }
            }
            _ => Reduction::default(),
        }
    }
}

impl TextHost for MutationsController {
    fn owns_text(&self, target: FocusId) -> bool {
        [
            COMMIT_MESSAGE,
            SETTINGS_OPENAI_KEY,
            SETTINGS_ANTHROPIC_KEY,
            SETTINGS_STEERING_PROMPT,
        ]
        .contains(&target)
    }

    fn edit_text(
        &mut self,
        target: FocusId,
        command: TextEditCommand,
        cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        match target {
            COMMIT_MESSAGE => Some(self.commit.apply_at(command, cx.now_ms)),
            SETTINGS_STEERING_PROMPT => Some(self.steering.apply_at(command, cx.now_ms)),
            SETTINGS_OPENAI_KEY => Some(self.openai_field.apply_at(command, cx.now_ms)),
            SETTINGS_ANTHROPIC_KEY => Some(self.anthropic_field.apply_at(command, cx.now_ms)),
            _ => None,
        }
    }

    fn set_preedit(
        &mut self,
        target: FocusId,
        text: String,
        cursor: Option<(usize, usize)>,
    ) -> bool {
        match target {
            COMMIT_MESSAGE => self.commit.set_preedit(text, cursor),
            SETTINGS_STEERING_PROMPT => self.steering.set_preedit(text, cursor),
            SETTINGS_OPENAI_KEY => self.openai_field.set_preedit(text, cursor),
            SETTINGS_ANTHROPIC_KEY => self.anthropic_field.set_preedit(text, cursor),
            _ => return false,
        }
        true
    }

    fn set_text_value(
        &mut self,
        target: FocusId,
        value: String,
        _cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        match target {
            COMMIT_MESSAGE => self.commit.set_text(&value),
            SETTINGS_STEERING_PROMPT => self.steering.set_text(&value),
            SETTINGS_OPENAI_KEY => self.openai_field.set_text(value),
            SETTINGS_ANTHROPIC_KEY => self.anthropic_field.set_text(value),
            _ => return None,
        }
        Some(TextEditOutcome {
            text_changed: true,
            ..TextEditOutcome::default()
        })
    }
}

impl CommandSource for MutationsController {
    fn commands(&self, cx: &SharedContext, out: &mut Vec<CommandDescriptor>) {
        let Some(capabilities) = capabilities(cx) else {
            return;
        };
        let lines = !self.selection.is_empty();
        let partial = capabilities.partial_hunk_mutation;
        let file = cx.selected_file.is_some();
        let (stage, unstage, discard) = if lines {
            (
                MutationsAction::StageSelectedLines,
                MutationsAction::UnstageSelectedLines,
                MutationsAction::DiscardSelectedLines,
            )
        } else {
            (
                MutationsAction::StageHunk,
                MutationsAction::UnstageHunk,
                MutationsAction::DiscardHunk,
            )
        };
        let target = if lines { "selected lines" } else { "hunk" };
        if capabilities.staging_area {
            out.push(
                CommandDescriptor::new("mutations.stage", format!("Stage {target}"), stage)
                    .shortcut(ShortcutCommand::Stage)
                    .enabled(partial),
            );
            out.push(
                CommandDescriptor::new("mutations.unstage", format!("Unstage {target}"), unstage)
                    .shortcut(ShortcutCommand::Unstage)
                    .enabled(partial),
            );
            out.push(
                CommandDescriptor::new(
                    "mutations.stage_file",
                    "Stage file",
                    MutationsAction::StageSelectedFile,
                )
                .enabled(file),
            );
            out.push(
                CommandDescriptor::new(
                    "mutations.unstage_file",
                    "Unstage file",
                    MutationsAction::UnstageSelectedFile,
                )
                .enabled(file),
            );
            out.push(
                CommandDescriptor::new(
                    "mutations.stage_all",
                    "Stage all changes",
                    MutationsAction::StageAllFiles,
                )
                .shortcut(ShortcutCommand::StageAll),
            );
            out.push(
                CommandDescriptor::new(
                    "mutations.unstage_all",
                    "Unstage all changes",
                    MutationsAction::UnstageAllFiles,
                )
                .shortcut(ShortcutCommand::UnstageAll),
            );
        }
        out.push(
            CommandDescriptor::new("mutations.discard", format!("Discard {target}"), discard)
                .shortcut(ShortcutCommand::Discard)
                .enabled(partial),
        );
        out.push(
            CommandDescriptor::new(
                "mutations.discard_file",
                "Discard file",
                MutationsAction::DiscardSelectedFile,
            )
            .enabled(file),
        );
        out.push(
            CommandDescriptor::new(
                "mutations.toggle_line",
                "Toggle line selection",
                MutationsAction::ToggleCurrentLineSelection,
            )
            .shortcut(ShortcutCommand::ToggleLineSelection)
            .enabled(partial),
        );
        out.push(
            CommandDescriptor::new(
                "mutations.toggle_line_range",
                "Extend line selection",
                MutationsAction::ToggleCurrentLineSelectionRange,
            )
            .shortcut(ShortcutCommand::ToggleLineSelectionRange)
            .enabled(partial),
        );
        if lines {
            out.push(CommandDescriptor::new(
                "mutations.clear_lines",
                "Clear line selection",
                MutationsAction::ClearLineSelection,
            ));
        }
        if capabilities.create_commit {
            out.push(
                CommandDescriptor::new(
                    "mutations.commit_message",
                    "Write commit message",
                    MutationsAction::OpenCommitComposer,
                )
                .shortcut(ShortcutCommand::FocusCommitMessage),
            );
            out.push(
                CommandDescriptor::new(
                    "mutations.commit",
                    "Create commit",
                    MutationsAction::SubmitCommit,
                )
                .shortcut(ShortcutCommand::SubmitCommit)
                .enabled(!self.commit.text().trim().is_empty()),
            );
            out.push(
                CommandDescriptor::new(
                    "mutations.generate",
                    "Generate commit message",
                    MutationsAction::GenerateCommitMessage,
                )
                .enabled(!self.ai.active),
            );
        }
        if capabilities.remotes {
            out.push(
                CommandDescriptor::new(
                    "mutations.fetch",
                    "Fetch all remotes",
                    MutationsAction::FetchAllRemotes,
                )
                .shortcut(ShortcutCommand::FetchRemotes),
            );
            out.push(
                CommandDescriptor::new(
                    "mutations.publish_menu",
                    "Publish\u{2026}",
                    MutationsAction::OpenPublishMenu,
                )
                .shortcut(ShortcutCommand::OpenPublishMenu),
            );
            out.push(CommandDescriptor::new(
                "mutations.publish",
                "Publish",
                MutationsAction::PublishDefault,
            ));
            out.push(CommandDescriptor::new(
                "mutations.push_force",
                "Force push with lease",
                MutationsAction::Push {
                    force_with_lease: true,
                },
            ));
        }
        if capabilities.pull_fast_forward {
            out.push(
                CommandDescriptor::new(
                    "mutations.pull",
                    "Pull current branch",
                    MutationsAction::PullCurrentBranch,
                )
                .shortcut(ShortcutCommand::PullCurrentBranch),
            );
        }
        if capabilities.operation_log {
            for operation in JjOperation::ALL {
                out.push(
                    CommandDescriptor::new(
                        jj_command_id(operation),
                        operation.label(),
                        MutationsAction::RunOperation(VcsOperation::Jj(operation)),
                    )
                    .detail(operation.detail()),
                );
            }
        }
    }
}

fn jj_command_id(operation: JjOperation) -> &'static str {
    match operation {
        JjOperation::NewChange => "mutations.jj.new",
        JjOperation::NewSiblingChange => "mutations.jj.new_sibling",
        JjOperation::DuplicateChange => "mutations.jj.duplicate",
        JjOperation::AbandonChange => "mutations.jj.abandon",
        JjOperation::SquashIntoParent => "mutations.jj.squash",
        JjOperation::AbsorbIntoStack => "mutations.jj.absorb",
        JjOperation::UndoLastOperation => "mutations.jj.undo",
    }
}
