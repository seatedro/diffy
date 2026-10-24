use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use diffy_diff::LineGroup;
use diffy_ui::diff_view::decorator::DiffDecorator;
use quark_ui::FocusId;
use quark_ui::text_input::{Editor, EditorMode, TextEditCommand, TextEditOutcome, TextField};

use super::presentation::{MutationPresentation, PresentationStyle};
use crate::ai::{CliAuth, CliCommand, CliScan, CliTool, DEFAULT_STEERING_PROMPT, Provider};
use crate::app::text_targets::{
    COMMIT_MESSAGE, SETTINGS_ANTHROPIC_KEY, SETTINGS_OPENAI_KEY, SETTINGS_STEERING_PROMPT, TextCx,
    TextHost,
};
use crate::contracts::commands::{
    CommandDescriptor, CommandSource, ConfirmationRequest, ToastKind, ToastUpdate,
};
use crate::contracts::mutations::{
    AiBackend, ConfirmedMutation, FileOperationRequest, GenerateCommitMessageRequest,
    MutationSelection, MutationSource, MutationSourceKey, MutationsAction, MutationsEffect,
    MutationsEvent, PatchOperationRequest, RankFilesJob, StagedDigest,
};
use crate::contracts::navigation::{RankFilesRequest, RepositoryContext};
use crate::contracts::review::Secret;
use crate::contracts::shared::{FileKey, FileRevision, JobId, SharedContext, Side, SourceTarget};
use crate::contracts::shell::{Action, Controller, Intent, LogicalFocus, Reduction};
use crate::core::compare::{LayoutMode, RendererKind};
use crate::core::keymap::ShortcutCommand;
use crate::core::vcs::model::JjOperation;
use crate::core::vcs::model::{
    ChangeBucket, FileChange, FileOperation, PublishAction, PublishPlan, RefKind, RepoCapabilities,
    VcsOperation, VcsRef,
};
use crate::core::vcs::source::SourceIdentity;
use crate::core::vcs::worktree::WorktreeSnapshot;
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
    UndoCommit(String),
    RestoreDiscard,
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
struct DisplayBinding {
    change: FileChange,
    identity: Option<SourceIdentity>,
}

#[derive(Debug, Clone)]
struct PendingPatch {
    job: JobId,
    key: MutationSourceKey,
    expected: Option<SourceIdentity>,
    selection: MutationSelection,
    operation: FileOperation,
}

#[derive(Debug, Default)]
pub struct AiState {
    pub generation: u64,
    pub active: bool,
    pub error: Option<String>,
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
    pub cli: Option<Arc<CliScan>>,
    pub scanning: bool,
    scan_generation: u64,
    waiting: Vec<AiTask>,
}

#[derive(Debug)]
enum AiTask {
    CommitMessage,
    RankFiles(RankFilesRequest),
}

enum Backend {
    Ready(AiBackend),
    Waiting(Reduction),
    Unavailable(String),
}

enum CliChoice {
    Ready(CliCommand),
    SignedOut(CliTool),
    Missing,
}

fn pick_cli(scan: &CliScan, provider: Option<Provider>) -> CliChoice {
    let tools: Vec<CliTool> = match provider {
        Some(provider) => vec![CliTool::for_provider(provider)],
        None => CliTool::ALL.to_vec(),
    };
    let found: Vec<_> = tools.iter().filter_map(|tool| scan.find(*tool)).collect();
    let usable = found
        .iter()
        .find(|info| info.auth == CliAuth::SignedIn)
        .or_else(|| found.iter().find(|info| info.auth == CliAuth::Unknown));
    if let Some(command) = usable.and_then(|info| scan.command(info.tool)) {
        return CliChoice::Ready(command);
    }
    match found.first() {
        Some(info) => CliChoice::SignedOut(info.tool),
        None => CliChoice::Missing,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendState {
    pub sha: String,
    pub short: String,
    draft: String,
}

#[derive(Debug, Clone)]
enum UndoEntry {
    Commit {
        repo_path: PathBuf,
        sha: String,
        message: String,
    },
    Discard(Arc<WorktreeSnapshot>),
}

const MAX_UNDO: usize = 8;
const UNDO_MS: u64 = 8_000;

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
    commit_menu: bool,
    amend: Option<AmendState>,
    amend_loading: bool,
    undo: Vec<(JobId, UndoEntry)>,
    ai: AiState,
    publish: Option<PublishMenu>,
    openai_field: TextField,
    anthropic_field: TextField,
    steering: Editor,
    steering_setting: Option<String>,
}

impl std::fmt::Debug for MutationsController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MutationsController")
            .field("selection", &self.selection.len())
            .field("jobs", &self.jobs)
            .field("amend", &self.amend)
            .finish_non_exhaustive()
    }
}

fn commit_editor() -> Editor {
    Editor::new(EditorMode::ProseInput)
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
            commit: commit_editor(),
            commit_menu: false,
            amend: None,
            amend_loading: false,
            undo: Vec::new(),
            ai: AiState::default(),
            publish: None,
            openai_field: TextField::new(""),
            anthropic_field: TextField::new(""),
            steering: Editor::new(EditorMode::ProseInput),
            steering_setting: None,
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

fn missing_backend(preferred: Option<Provider>, use_cli: bool) -> String {
    match (preferred, use_cli) {
        (Some(provider), true) => format!(
            "Install {} or add a {} key under Settings \u{2192} AI.",
            CliTool::for_provider(provider).label(),
            provider.label()
        ),
        (Some(provider), false) => format!(
            "Add a {} key under Settings \u{2192} AI first.",
            provider.label()
        ),
        (None, true) => {
            "Install Claude Code or Codex, or add an AI key under Settings \u{2192} AI.".to_owned()
        }
        (None, false) => "Add an AI key under Settings \u{2192} AI first.".to_owned(),
    }
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

fn mutable_file(cx: &SharedContext, file: FileKey, revision: FileRevision) -> bool {
    cx.manifest
        .as_ref()
        .filter(|manifest| manifest.working_tree)
        .and_then(|manifest| manifest.file(file))
        .is_some_and(|entry| entry.revision == revision && !entry.buckets.is_empty())
}

pub fn working_tree(cx: &SharedContext) -> bool {
    cx.manifest
        .as_ref()
        .is_some_and(|manifest| manifest.working_tree)
}

fn undo_toast(id: JobId, message: impl Into<String>, undo: MutationsAction) -> Intent {
    Intent::Toast(ToastUpdate::Undo {
        id,
        kind: ToastKind::Success,
        message: message.into(),
        undo: Box::new(undo.into()),
        expires_ms: UNDO_MS,
        expired: Some(Box::new(MutationsAction::ForgetUndo(id).into())),
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

pub(super) fn commit_message(text: &str) -> String {
    let text = text.trim();
    match text.split_once('\n') {
        Some((subject, body))
            if !body
                .lines()
                .next()
                .is_some_and(|line| line.trim().is_empty()) =>
        {
            format!("{}\n\n{body}", subject.trim_end())
        }
        _ => text.to_owned(),
    }
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

fn grouped(keys: impl Iterator<Item = LineKey>, groups: &[LineGroup]) -> Vec<LineKey> {
    let mut out = BTreeSet::new();
    for key in keys {
        out.insert(key);
        let within = |g: &&LineGroup| match key.side {
            Side::New => g.new.contains(&key.line),
            Side::Old => g.old.contains(&key.line),
        };
        for group in groups.iter().filter(within) {
            out.extend(group.new.clone().map(|line| LineKey {
                side: Side::New,
                line,
                ..key
            }));
            out.extend(group.old.clone().map(|line| LineKey {
                side: Side::Old,
                line,
                ..key
            }));
        }
    }
    out.into_iter().collect()
}

fn line_groups(cx: &SharedContext, target: &SourceTarget) -> Arc<[LineGroup]> {
    let settings = &cx.settings.settings;
    let semantic = settings.viewport.layout == LayoutMode::Unified
        && settings
            .last_compare
            .as_ref()
            .is_some_and(|compare| compare.renderer == RendererKind::Semantic);
    cx.manifest
        .as_ref()
        .filter(|_| semantic)
        .and_then(|manifest| manifest.file(target.file))
        .filter(|file| file.revision == target.revision)
        .map_or_else(|| Arc::from([]), |file| file.line_groups.clone())
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

    pub fn commit_menu_open(&self) -> bool {
        self.commit_menu
    }

    pub fn amending(&self) -> Option<&AmendState> {
        self.amend.as_ref()
    }

    pub fn can_undo(&self, job: JobId) -> bool {
        self.undo.iter().any(|(id, _)| *id == job)
    }

    fn remember_undo(&mut self, job: JobId, entry: UndoEntry) {
        if self.undo.len() >= MAX_UNDO {
            self.undo.remove(0);
        }
        self.undo.push((job, entry));
    }

    fn take_undo(&mut self, job: JobId) -> Option<UndoEntry> {
        let index = self.undo.iter().position(|(id, _)| *id == job)?;
        Some(self.undo.remove(index).1)
    }

    fn latest_undo(&self, commit: bool) -> Option<JobId> {
        self.undo
            .iter()
            .rev()
            .find(|(_, entry)| matches!(entry, UndoEntry::Commit { .. }) == commit)
            .map(|(id, _)| *id)
    }

    pub fn syncing(&self) -> bool {
        self.jobs.values().any(|job| {
            matches!(
                job,
                Job::Fetch(_) | Job::Push(_) | Job::Pull(_) | Job::Publish
            )
        })
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
        let next = MutationPresentation::new(
            self.presentation.revision().wrapping_add(1),
            self.presentation.style(),
            capabilities(cx).is_some_and(|c| c.partial_hunk_mutation),
            cx.manifest.clone(),
            self.selection.clone(),
        );
        if !self.presentation.same_content(&next) {
            self.presentation = Rc::new(next);
        }
    }

    fn display_binding(
        &self,
        cx: &SharedContext,
        file: FileKey,
        operation: FileOperation,
    ) -> Result<DisplayBinding, &'static str> {
        let repo = cx
            .repository
            .as_ref()
            .ok_or("Open a repository before changing files.")?;
        let entry = cx
            .manifest
            .as_ref()
            .and_then(|manifest| manifest.files.iter().find(|entry| entry.key == file))
            .ok_or("This file is no longer in the comparison.")?;
        if entry.buckets.is_empty() {
            return Err("Only working-tree changes can be staged or discarded.");
        }
        let wanted: &[ChangeBucket] = match (repo.snapshot.capabilities.staging_area, operation) {
            (false, _) => &[ChangeBucket::WorkingCopy, ChangeBucket::Conflicted],
            (true, FileOperation::Stage) => &[ChangeBucket::Unstaged, ChangeBucket::Untracked],
            (true, FileOperation::Unstage) => &[ChangeBucket::Staged],
            (true, FileOperation::Discard) => &[ChangeBucket::Unstaged, ChangeBucket::Untracked],
        };
        let mut candidates = wanted
            .iter()
            .copied()
            .filter(|bucket| entry.buckets.contains(bucket));
        let bucket = candidates.next().ok_or(match operation {
            FileOperation::Stage => "This file has no unstaged changes.",
            FileOperation::Unstage => "This file has no staged changes.",
            FileOperation::Discard => "This file has no unstaged changes to discard.",
        })?;
        if candidates.next().is_some() {
            return Err("This file's changes are ambiguous here. Act on the whole file instead.");
        }
        let path = entry
            .paths
            .new_path()
            .or_else(|| entry.paths.old_path())
            .ok_or("This file has no path.")?;
        let change = repo
            .snapshot
            .file_changes
            .iter()
            .find(|change| change.path == path && change.bucket == bucket)
            .cloned()
            .ok_or("This file changed since it was shown. Try again.")?;
        let identity = (entry.old_id.is_some() || entry.new_id.is_some()).then(|| SourceIdentity {
            old: entry.old_id.clone(),
            new: entry.new_id.clone(),
        });
        Ok(DisplayBinding { change, identity })
    }

    fn request_patch(
        &mut self,
        selection: MutationSelection,
        operation: FileOperation,
        cx: &SharedContext,
    ) -> Reduction {
        let Some(capabilities) = capabilities(cx) else {
            return error("Open a repository before changing lines.");
        };
        if !working_tree(cx) {
            return error("Only working-tree changes can be staged or discarded.");
        }
        if !capabilities.partial_hunk_mutation {
            return error("This repository backend does not support line operations.");
        }
        let Some((file, revision)) = selection.file() else {
            return error("Select lines from one file at a time.");
        };
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
        let binding = match self.display_binding(cx, file, operation) {
            Ok(binding) => binding,
            Err(message) => return error(message),
        };
        let change = binding.change;
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
            expected: binding.identity,
            selection,
            operation,
        };
        match self.sources.get(&key) {
            Some(SourceSlot::Ready(source)) => {
                let source = source.clone();
                let mut reduction = Reduction::changed();
                self.dispatch_patch(pending, source, &mut reduction);
                reduction
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
            self.dispatch_patch(pending, source.clone(), &mut reduction);
        }
        reduction
    }

    fn dispatch_patch(
        &mut self,
        pending: PendingPatch,
        source: Arc<MutationSource>,
        reduction: &mut Reduction,
    ) {
        if pending
            .expected
            .as_ref()
            .is_some_and(|expected| !source.identity().matches(expected))
        {
            self.jobs.remove(&pending.job);
            self.sources.remove(source.key());
            reduction.intents.push(Intent::Toast(ToastUpdate::error(
                "The file on disk no longer matches the diff shown. Refresh and try again.",
            )));
            return;
        }
        reduction.effects.push(
            MutationsEffect::ApplyPatch(PatchOperationRequest {
                job: pending.job,
                source,
                selection: pending.selection,
                operation: pending.operation,
            })
            .into(),
        );
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
        if !working_tree(cx) {
            return error("Only working-tree changes can be staged or discarded.");
        }
        if operation != FileOperation::Discard && !capabilities.staging_area {
            return error("This repository backend does not support staging.");
        }
        if files.is_empty() || self.busy() {
            return Reduction::default();
        }
        let mut changes = Vec::with_capacity(files.len());
        for file in &files {
            match self.display_binding(cx, *file, operation) {
                Ok(binding) => changes.push(binding.change),
                Err(message) => return error(message),
            }
        }
        let deletes = changes
            .iter()
            .filter(|change| change.bucket == ChangeBucket::Untracked)
            .count();
        if operation == FileOperation::Discard && deletes > 0 && !confirmed {
            let message = if deletes == 1 {
                "The untracked file will be deleted from disk.".to_owned()
            } else {
                format!("{deletes} untracked files will be deleted from disk.")
            };
            return Reduction::default().intent(Intent::RequestConfirmation(ConfirmationRequest {
                title: "Delete untracked files?".to_owned(),
                message,
                confirm_label: "Delete".to_owned(),
                destructive: true,
                confirm: Box::new(Action::from(MutationsAction::ApplyConfirmed(
                    ConfirmedMutation::Files(files),
                ))),
                return_focus: Some(LogicalFocus::Viewport),
            }));
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
        if !working_tree(cx) {
            return error("Only working-tree changes can be staged or discarded.");
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

    fn selection_operation(&mut self, operation: FileOperation, cx: &SharedContext) -> Reduction {
        if self.selection.is_empty() {
            return error("Select changed lines first.");
        }
        let targets = line_ranges(self.selection.iter().copied());
        let reduction = self.request_patch(MutationSelection::Lines(targets), operation, cx);
        if !reduction.effects.is_empty() || !self.pending.is_empty() {
            self.selection.clear();
            self.anchor = None;
            self.refresh_presentation(cx);
        }
        reduction
    }

    fn toggle_lines(&mut self, target: &SourceTarget, groups: &[LineGroup]) {
        let lines = grouped(keys(target), groups);
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
        self.anchor = keys(target).last();
    }

    fn extend_lines(&mut self, target: &SourceTarget, groups: &[LineGroup]) {
        let Some(anchor) = self.anchor.filter(|a| {
            a.file == target.file && a.revision == target.revision && a.side == target.side
        }) else {
            self.toggle_lines(target, groups);
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
        self.selection.extend(grouped(keys(&range), groups));
    }

    fn select_lines(&mut self, target: &SourceTarget, groups: &[LineGroup]) {
        self.selection.clear();
        self.selection.extend(grouped(keys(target), groups));
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
        let message = commit_message(self.commit.text());
        if message.is_empty() {
            return error("Write a commit message first.");
        }
        let amend = self.amend.is_some();
        if !amend
            && !repo
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
                amend,
            })
            .intent(progress(
                job,
                if amend {
                    "Amending\u{2026}"
                } else {
                    "Committing\u{2026}"
                },
            ))
    }

    fn generate_commit_message(&mut self, cx: &SharedContext) -> Reduction {
        if self.ai.active {
            return Reduction::default();
        }
        let Some(repo) = cx.repository.as_ref() else {
            return error("Open a repository before generating a commit message.");
        };
        let backend = match self.ai_backend(|| AiTask::CommitMessage, cx) {
            Backend::Ready(backend) => backend,
            Backend::Waiting(reduction) => return reduction,
            Backend::Unavailable(message) => return error(message),
        };
        let staging = repo.snapshot.capabilities.staging_area;
        let has_staged = staging
            && repo
                .snapshot
                .file_changes
                .iter()
                .any(|change| change.bucket == ChangeBucket::Staged);
        let steering = &cx.settings.settings.ai_steering_prompt;
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
        self.commit_menu = false;
        Reduction::changed().effect(MutationsEffect::GenerateCommitMessage(Box::new(
            GenerateCommitMessageRequest {
                generation: self.ai.generation,
                repository: repo.revision,
                repo_path: repo.snapshot.path.clone(),
                has_staged,
                backend,
                steering_prompt,
                subject_override,
            },
        )))
    }

    fn ai_backend(&mut self, task: impl FnOnce() -> AiTask, cx: &SharedContext) -> Backend {
        let settings = &cx.settings.settings;
        let preferred = settings.ai_provider;
        if settings.ai_use_cli {
            match self.ai.cli.as_deref() {
                Some(scan) if !self.ai.scanning => match pick_cli(scan, preferred) {
                    CliChoice::Ready(command) => return Backend::Ready(AiBackend::Cli(command)),
                    CliChoice::SignedOut(tool) => {
                        return Backend::Unavailable(format!(
                            "{} is signed out. {}",
                            tool.label(),
                            tool.sign_in_hint()
                        ));
                    }
                    CliChoice::Missing => {}
                },
                _ => {
                    let task = task();
                    let kind = std::mem::discriminant(&task);
                    self.ai
                        .waiting
                        .retain(|queued| std::mem::discriminant(queued) != kind);
                    self.ai.waiting.push(task);
                    return Backend::Waiting(self.scan_clis(false));
                }
            }
        }
        let key = |provider: Provider| {
            match provider {
                Provider::OpenAi => self.ai.openai.clone(),
                Provider::Anthropic => self.ai.anthropic.clone(),
            }
            .filter(|key| !key.is_empty())
            .map(|api_key| AiBackend::Api { provider, api_key })
        };
        let chosen = match preferred {
            Some(provider) => key(provider),
            None => key(Provider::Anthropic).or_else(|| key(Provider::OpenAi)),
        };
        match chosen {
            Some(backend) => Backend::Ready(backend),
            None => Backend::Unavailable(missing_backend(preferred, settings.ai_use_cli)),
        }
    }

    fn run_ai_task(&mut self, task: AiTask, cx: &SharedContext) -> Reduction {
        match task {
            AiTask::CommitMessage => self.generate_commit_message(cx),
            AiTask::RankFiles(request) => self.rank_files(&request, cx),
        }
    }

    fn rank_files(&mut self, request: &RankFilesRequest, cx: &SharedContext) -> Reduction {
        match self.ai_backend(|| AiTask::RankFiles(request.clone()), cx) {
            Backend::Ready(backend) => {
                Reduction::default().effect(MutationsEffect::RankFiles(Box::new(RankFilesJob {
                    request: request.clone(),
                    backend,
                })))
            }
            Backend::Waiting(reduction) => reduction,
            Backend::Unavailable(reason) => {
                Reduction::default().intent(Intent::RankFilesUnavailable {
                    job: request.job,
                    reason,
                })
            }
        }
    }

    fn ai_current(&self, generation: u64) -> bool {
        self.ai.active && generation == self.ai.generation
    }

    fn stop_stale_generation(&mut self) -> Reduction {
        self.ai.generation = self.ai.generation.wrapping_add(1);
        self.ai.active = false;
        self.ai.buffered.clear();
        self.ai.checks = 0;
        Reduction::changed()
            .effect(MutationsEffect::CancelCommitMessage {
                generation: self.ai.generation,
            })
            .intent(toast(
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
        if self.steering_setting.as_deref() == Some(prompt) {
            return;
        }
        self.steering_setting = Some(prompt.to_owned());
        let shown = if prompt.trim().is_empty() {
            DEFAULT_STEERING_PROMPT
        } else {
            prompt
        };
        if self.steering.text() != shown {
            self.steering.set_text(shown);
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

    fn cancel_generation(&mut self) -> Reduction {
        self.ai
            .waiting
            .retain(|task| !matches!(task, AiTask::CommitMessage));
        if !self.ai.active {
            return Reduction::default();
        }
        self.ai.generation = self.ai.generation.wrapping_add(1);
        self.ai.active = false;
        self.ai.buffered.clear();
        self.ai.checks = 0;
        Reduction::changed().effect(MutationsEffect::CancelCommitMessage {
            generation: self.ai.generation,
        })
    }

    fn scan_clis(&mut self, force: bool) -> Reduction {
        if self.ai.scanning {
            return Reduction::default();
        }
        if !force && self.ai.cli.is_some() {
            return Reduction::default();
        }
        self.ai.scanning = true;
        self.ai.scan_generation = self.ai.scan_generation.wrapping_add(1);
        Reduction::changed().effect(MutationsEffect::ScanAiClis {
            generation: self.ai.scan_generation,
        })
    }

    fn start_amend(&mut self, cx: &SharedContext) -> Reduction {
        self.commit_menu = false;
        let Some(repo) = cx.repository.as_ref() else {
            return error("Open a repository before amending.");
        };
        if !repo.snapshot.capabilities.staging_area || !working_tree(cx) {
            return error("Amend is only available for Git working trees.");
        }
        if self.amend.is_some() || self.amend_loading {
            return Reduction::changed();
        }
        self.amend_loading = true;
        Reduction::changed().effect(MutationsEffect::LoadLastCommit {
            repo_path: repo.snapshot.path.clone(),
        })
    }

    fn cancel_amend(&mut self) -> Reduction {
        self.amend_loading = false;
        let Some(amend) = self.amend.take() else {
            return Reduction::default();
        };
        self.commit.set_text(&amend.draft);
        Reduction::changed()
    }

    fn undo_commit(&mut self, id: JobId) -> Reduction {
        match self.take_undo(id) {
            Some(UndoEntry::Commit {
                repo_path,
                sha,
                message,
            }) => {
                let job = self.job(Job::UndoCommit(message));
                Reduction::changed()
                    .intent(Intent::Toast(ToastUpdate::Dismiss(id)))
                    .effect(MutationsEffect::UndoCommit {
                        job,
                        repo_path,
                        sha,
                    })
            }
            Some(entry) => {
                self.undo.push((id, entry));
                Reduction::default()
            }
            None => error("That commit can no longer be undone."),
        }
    }

    fn undo_discard(&mut self, id: JobId) -> Reduction {
        match self.take_undo(id) {
            Some(UndoEntry::Discard(snapshot)) => {
                let job = self.job(Job::RestoreDiscard);
                Reduction::changed()
                    .intent(Intent::Toast(ToastUpdate::Dismiss(id)))
                    .effect(MutationsEffect::RestoreDiscard { job, snapshot })
            }
            Some(entry) => {
                self.undo.push((id, entry));
                Reduction::default()
            }
            None => error("Those changes can no longer be restored."),
        }
    }

    fn reset(&mut self) {
        self.undo.clear();
        self.amend = None;
        self.amend_loading = false;
        self.commit_menu = false;
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
            // The router resolves these to the hunk the diff marks, and
            // these, the files the list has; one arriving here found none.
            A::StageHunk | A::UnstageHunk | A::DiscardHunk => {
                error("Scroll a hunk into view first.")
            }
            A::ApplyListedFiles(_) => error("Select a file first."),
            A::StageHunkAt(target) => self.request_patch(
                MutationSelection::Hunks(vec![target]),
                FileOperation::Stage,
                cx,
            ),
            A::UnstageHunkAt(target) => self.request_patch(
                MutationSelection::Hunks(vec![target]),
                FileOperation::Unstage,
                cx,
            ),
            A::DiscardHunkAt(target) => self.request_patch(
                MutationSelection::Hunks(vec![target]),
                FileOperation::Discard,
                cx,
            ),
            A::SelectLines(target)
            | A::ToggleLineSelection(target)
            | A::ExtendLineSelection(target)
                if !mutable_file(cx, target.file, target.revision) =>
            {
                Reduction::default()
            }
            A::SelectLines(target) => {
                self.select_lines(&target, &line_groups(cx, &target));
                self.refresh_presentation(cx);
                Reduction::changed()
            }
            A::ToggleLineSelection(target) => {
                self.toggle_lines(&target, &line_groups(cx, &target));
                self.refresh_presentation(cx);
                Reduction::changed()
            }
            A::ExtendLineSelection(target) => {
                self.extend_lines(&target, &line_groups(cx, &target));
                self.refresh_presentation(cx);
                Reduction::changed()
            }
            A::ToggleCurrentLineSelection | A::ToggleCurrentLineSelectionRange => {
                let Some(target) = cx.selection.clone() else {
                    return error("Move the cursor to a changed line before selecting lines.");
                };
                if !mutable_file(cx, target.file, target.revision) {
                    return Reduction::default();
                }
                let groups = line_groups(cx, &target);
                if action == A::ToggleCurrentLineSelection {
                    self.toggle_lines(&target, &groups);
                } else {
                    self.extend_lines(&target, &groups);
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
            A::StageSelectedLines => self.selection_operation(FileOperation::Stage, cx),
            A::UnstageSelectedLines => self.selection_operation(FileOperation::Unstage, cx),
            A::DiscardSelectedLines => self.selection_operation(FileOperation::Discard, cx),
            A::ApplyConfirmed(ConfirmedMutation::Selection(selection)) => {
                self.request_patch(selection, FileOperation::Discard, cx)
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
            A::PublishDefault
                if cx
                    .repository
                    .as_ref()
                    .and_then(|repo| repo.snapshot.publish_plan.as_ref())
                    .is_some_and(|plan| !plan.has_remote()) =>
            {
                self.open_publish_menu(cx)
            }
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
            A::OpenCommitComposer => Reduction::changed().effect(
                crate::contracts::shell::ShellEffect::Focus(Some(LogicalFocus::CommitMessage)),
            ),
            A::StartAmend => self.start_amend(cx),
            A::CancelAmend => self.cancel_amend(),
            A::ToggleCommitMenu => {
                self.commit_menu = !self.commit_menu;
                Reduction::changed()
            }
            A::CloseCommitMenu => {
                self.commit_menu = false;
                Reduction::changed()
            }
            A::CancelGeneration => self.cancel_generation(),
            A::ScanAiClis { force } => self.scan_clis(force),
            A::UndoCommit(id) => self.undo_commit(id),
            A::UndoDiscard(id) => self.undo_discard(id),
            A::ForgetUndo(id) => {
                self.take_undo(id);
                Reduction::default()
            }
            A::ApplyFiles { files, operation } => self.file_operation(files, operation, false, cx),
            A::ApplyBucket { bucket, operation } => self.all_in(&[bucket], operation, cx),
            A::ScrollCommitMessage(lines) => {
                let step = self.commit.scroll_line_height_px();
                self.commit.scroll(lines as f32 * step);
                Reduction::changed()
            }
            A::SaveAiKeyField(kind) => self.save_key_field(kind),
            A::ScrollSteeringPrompt(lines) => {
                let step = self.steering.scroll_line_height_px();
                self.steering.scroll(lines as f32 * step);
                Reduction::changed()
            }
        }
    }

    fn reduce_event(&mut self, event: Self::Event, cx: &SharedContext) -> Reduction {
        use MutationsEvent as E;
        match event {
            E::MutationSourceReady { source } => self.source_ready(source),
            E::MutationSourceFailed { key, message } => self.source_failed(key, message),
            E::FileOperationApplied { job, undo, .. } => {
                if self.finish(job).is_none() {
                    return Reduction::default();
                }
                match undo {
                    Some(snapshot) => {
                        let count = snapshot.paths().count();
                        let message = match snapshot.paths().next() {
                            Some(path) if count == 1 => {
                                format!("Discarded changes in {}", path.display())
                            }
                            _ => format!("Discarded changes in {count} files"),
                        };
                        self.remember_undo(job, UndoEntry::Discard(snapshot));
                        Reduction::changed().intent(undo_toast(
                            job,
                            message,
                            MutationsAction::UndoDiscard(job),
                        ))
                    }
                    None => Reduction::changed(),
                }
            }
            E::LastCommitLoaded { commit, .. } => {
                if !self.amend_loading {
                    return Reduction::default();
                }
                self.amend_loading = false;
                let Some(commit) = commit else {
                    return error("There is no commit to amend yet.");
                };
                let draft = self.commit.text().to_owned();
                self.commit.set_text(&commit.message);
                self.amend = Some(AmendState {
                    sha: commit.sha,
                    short: commit.short,
                    draft,
                });
                Reduction::changed().effect(crate::contracts::shell::ShellEffect::Focus(Some(
                    LogicalFocus::CommitMessage,
                )))
            }
            E::LastCommitFailed { message } => {
                self.amend_loading = false;
                Reduction::changed().intent(failure(
                    None,
                    "Could not load the last commit",
                    message,
                ))
            }
            E::CommitUndone { job, .. } => {
                let Some(Job::UndoCommit(message)) = self.finish(job) else {
                    return Reduction::default();
                };
                if self.commit.text().trim().is_empty() {
                    self.commit.set_text(&message);
                }
                Reduction::changed().intent(toast(
                    ToastKind::Info,
                    Some(job),
                    "Commit undone; its changes are staged again.",
                ))
            }
            E::CommitUndoFailed { job, message } => {
                self.finish(job);
                Reduction::changed().intent(failure(
                    Some(job),
                    "Could not undo the commit",
                    message,
                ))
            }
            E::DiscardRestored { job, .. } => {
                self.finish(job);
                Reduction::changed().intent(toast(
                    ToastKind::Info,
                    Some(job),
                    "Restored the discarded changes.",
                ))
            }
            E::DiscardRestoreFailed { job, message } => {
                self.finish(job);
                Reduction::changed().intent(failure(
                    Some(job),
                    "Could not restore the changes",
                    message,
                ))
            }
            E::FileOperationFailed { job, message, .. } => {
                if self.finish(job).is_none() {
                    return Reduction::default();
                }
                Reduction::changed().intent(Intent::Toast(ToastUpdate::error(message)))
            }
            E::CommitCreated { job, path, commit } => {
                if self.finish(job).is_none() {
                    return Reduction::default();
                }
                let amended = self.amend.take().is_some();
                self.commit.set_text("");
                self.ai.active = false;
                let message = if amended {
                    format!("Amended {}", commit.short)
                } else {
                    format!("Committed {}", commit.short)
                };
                self.remember_undo(
                    job,
                    UndoEntry::Commit {
                        repo_path: path,
                        sha: commit.sha,
                        message: commit.message,
                    },
                );
                if amended {
                    self.take_undo(job);
                    return Reduction::changed().intent(toast(
                        ToastKind::Success,
                        Some(job),
                        message,
                    ));
                }
                Reduction::changed().intent(undo_toast(
                    job,
                    message,
                    MutationsAction::UndoCommit(job),
                ))
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
                    format!(
                        "Fast-forwarded {branch} by {} from {remote}",
                        super::view::commits(behind)
                    )
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
            E::AiClisScanned { generation, scan } => {
                if generation != self.ai.scan_generation {
                    return Reduction::default();
                }
                self.ai.scanning = false;
                self.ai.cli = Some(scan);
                let mut reduction = Reduction::changed();
                for task in std::mem::take(&mut self.ai.waiting) {
                    reduction.merge(self.run_ai_task(task, cx));
                }
                reduction
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
                let mut reduction = Reduction::default().effect(MutationsEffect::LoadAiKeys);
                if startup.settings.ai_use_cli {
                    reduction.merge(self.scan_clis(false));
                }
                reduction
            }
            Intent::SettingsChanged(snapshot) => {
                self.sync_steering(&snapshot.settings.ai_steering_prompt);
                let mut reduction = Reduction::changed();
                if snapshot.settings.ai_use_cli {
                    reduction.merge(self.scan_clis(false));
                }
                reduction
            }
            Intent::RankFiles(request) => self.rank_files(request, cx),
            Intent::ActivateWorkspace { .. } => {
                let mut reduction = self.cancel_generation();
                self.reset();
                self.commit.set_text("");
                self.refresh_presentation(cx);
                reduction.changed = true;
                reduction
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
        let start = out.len();
        self.repository_commands(cx, out);
        let working = working_tree(cx);
        let buckets: Vec<ChangeBucket> = cx
            .selected_file
            .and_then(|file| {
                cx.manifest
                    .as_ref()?
                    .files
                    .iter()
                    .find(|entry| entry.key == file.key)
                    .map(|entry| entry.buckets.clone())
            })
            .unwrap_or_default();
        let has = |wanted: &[ChangeBucket]| buckets.iter().any(|bucket| wanted.contains(bucket));
        let keep = |id: &str| match id {
            "mutations.stage_file" => has(&[ChangeBucket::Unstaged, ChangeBucket::Untracked]),
            "mutations.unstage_file" => has(&[ChangeBucket::Staged]),
            "mutations.discard_file" => has(&[
                ChangeBucket::Unstaged,
                ChangeBucket::Untracked,
                ChangeBucket::WorkingCopy,
            ]),
            "mutations.fetch" | "mutations.pull" => true,
            id if id.starts_with("mutations.jj.") => true,
            _ => working,
        };
        let mut index = 0;
        out.retain(|command| {
            index += 1;
            index <= start || keep(command.id)
        });
        if !working {
            return;
        }
        if let Some(job) = self.latest_undo(true) {
            out.push(CommandDescriptor::new(
                "mutations.undo_commit",
                "Undo Last Commit",
                MutationsAction::UndoCommit(job),
            ));
        }
        if let Some(job) = self.latest_undo(false) {
            out.push(CommandDescriptor::new(
                "mutations.undo_discard",
                "Undo Discard",
                MutationsAction::UndoDiscard(job),
            ));
        }
    }
}

impl MutationsController {
    fn repository_commands(&self, cx: &SharedContext, out: &mut Vec<CommandDescriptor>) {
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
                    MutationsAction::ApplyListedFiles(FileOperation::Stage),
                )
                .shortcut(ShortcutCommand::StageFile)
                .enabled(file),
            );
            out.push(
                CommandDescriptor::new(
                    "mutations.unstage_file",
                    "Unstage file",
                    MutationsAction::ApplyListedFiles(FileOperation::Unstage),
                )
                .shortcut(ShortcutCommand::UnstageFile)
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
                MutationsAction::ApplyListedFiles(FileOperation::Discard),
            )
            .shortcut(ShortcutCommand::DiscardFile)
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
                .shortcut(ShortcutCommand::GenerateCommitMessage)
                .enabled(!self.ai.active),
            );
            if capabilities.staging_area {
                out.push(CommandDescriptor::new(
                    "mutations.amend",
                    "Amend Last Commit",
                    MutationsAction::StartAmend,
                ));
            }
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
