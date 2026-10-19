use std::sync::Arc;

use diffy_diff::{
    DiffDocument, DiffStats, DocumentBuilder, FileMeta, FileStatus, ObjectId, SourceCoverage,
    TextStore, diff_from_changes, myers::Change,
};
use gix::bstr::{BStr, ByteSlice};
use gix::diff::blob::{Algorithm, InternedInput, diff_with_slider_heuristics};

use crate::core::compare::backends::RENAME_DETECTION_LIMIT;
use crate::core::compare::output::{CompareEntry, CompareOutput};
use crate::core::compare::progress::{ComparePhase, ProgressSink};
use crate::core::compare::spec::CompareSpec;
use crate::core::compare::stats::COMPARE_SUMMARY_FILE_LIMIT;
use crate::core::error::{DiffyError, Result};
use crate::core::compare::spec::RendererKind;
use crate::core::vcs::git::status::status_items_from_entry_with_old_path;
use crate::core::vcs::git::{GitService, INDEX_REF, StatusItem, StatusScope, WORKDIR_REF};
use crate::core::vcs::model::ChangeBucket;

const LOADING_FILE_EMIT_STRIDE: usize = 16;
const LINE_STATS_MAX_WORKERS: usize = 8;
const DEFERRED_STATS_MIN_FILES_PER_WORKER: usize = 16;
pub const CONTEXT_LINES: u32 = 3;

pub fn compare(
    spec: &CompareSpec,
    git: &GitService,
    reporter: Option<&dyn ProgressSink>,
) -> Result<CompareOutput> {
    let (left, right) = resolve_compare_refs(spec, git)?;
    if let Some(r) = reporter {
        r.phase(ComparePhase::EnumeratingChanges);
    }
    if right == WORKDIR_REF {
        return CompareOutput::from_patch(&git.diff_two_refs(&left, &right)?);
    }
    let mut repo = open_gix_repo(git)?;
    let mut changes = collect_gix_changes(&mut repo, &left, &right, None, false)?;
    if changes.len() <= COMPARE_SUMMARY_FILE_LIMIT && gix_changes_may_include_rewrites(&changes) {
        changes = collect_gix_changes(&mut repo, &left, &right, None, true)?;
    }
    output_from_gix_changes(&repo, &changes, reporter)
}

pub fn worktree_items(git: &GitService) -> Result<Vec<StatusItem>> {
    let mut items: Vec<StatusItem> = git
        .status_entries()?
        .into_iter()
        .flat_map(|(path, old_path, status)| {
            status_items_from_entry_with_old_path(path, old_path, status)
        })
        .collect();
    items.sort_by(|a, b| {
        scope_rank(a.scope)
            .cmp(&scope_rank(b.scope))
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(items)
}

fn scope_rank(scope: StatusScope) -> u8 {
    match scope {
        StatusScope::Staged => 0,
        StatusScope::Unstaged => 1,
        StatusScope::Untracked => 2,
    }
}

pub fn bucket_of(scope: StatusScope) -> ChangeBucket {
    match scope {
        StatusScope::Staged => ChangeBucket::Staged,
        StatusScope::Unstaged => ChangeBucket::Unstaged,
        StatusScope::Untracked => ChangeBucket::Untracked,
    }
}

pub fn scope_of(bucket: ChangeBucket) -> StatusScope {
    match bucket {
        ChangeBucket::Staged => StatusScope::Staged,
        ChangeBucket::Untracked => StatusScope::Untracked,
        ChangeBucket::Unstaged | ChangeBucket::WorkingCopy | ChangeBucket::Conflicted => {
            StatusScope::Unstaged
        }
    }
}

pub fn bucket_refs(bucket: ChangeBucket) -> (Option<&'static str>, &'static str) {
    match scope_of(bucket) {
        StatusScope::Staged => (Some("HEAD"), INDEX_REF),
        StatusScope::Unstaged => (Some(INDEX_REF), WORKDIR_REF),
        StatusScope::Untracked => (None, WORKDIR_REF),
    }
}

fn item_status(item: &StatusItem) -> FileStatus {
    match item.status.as_str() {
        "A" | "U" => FileStatus::Added,
        "D" => FileStatus::Deleted,
        "R" => FileStatus::Renamed,
        "C" => FileStatus::Copied,
        _ => FileStatus::Modified,
    }
}

fn item_paths(item: &StatusItem) -> (Option<&str>, Option<&str>) {
    match item_status(item) {
        FileStatus::Added => (None, Some(item.path.as_str())),
        FileStatus::Deleted => (Some(item.path.as_str()), None),
        _ => (
            Some(item.old_path.as_deref().unwrap_or(&item.path)),
            Some(item.path.as_str()),
        ),
    }
}

pub fn worktree_entry(item: &StatusItem) -> CompareEntry {
    let (old, new) = item_paths(item);
    CompareEntry::deferred(old, new, item_status(item)).with_bucket(bucket_of(item.scope))
}

pub fn compare_worktree(
    git: &GitService,
    renderer: RendererKind,
    reporter: Option<&dyn ProgressSink>,
) -> Result<CompareOutput> {
    if let Some(r) = reporter {
        r.phase(ComparePhase::EnumeratingChanges);
    }
    let items = worktree_items(git)?;
    if items.len() > COMPARE_SUMMARY_FILE_LIMIT {
        return Ok(CompareOutput::deferred(
            items.iter().map(worktree_entry).collect(),
        ));
    }
    let staged = bucket_documents(git, StatusScope::Staged, &items)?;
    let unstaged = bucket_documents(git, StatusScope::Unstaged, &items)?;
    let total = items.len() as u32;
    let mut output = CompareOutput::default();
    for (index, item) in items.iter().enumerate() {
        if let Some(r) = reporter
            && (index.is_multiple_of(LOADING_FILE_EMIT_STRIDE) || index + 1 == items.len())
        {
            r.phase(ComparePhase::LoadingFiles {
                files_seen: index as u32 + 1,
                files_total: total,
            });
        }
        let from_patch = match item.scope {
            StatusScope::Staged => staged.get(&item.path),
            StatusScope::Unstaged => unstaged.get(&item.path),
            StatusScope::Untracked => None,
        };
        let doc = match from_patch {
            Some(doc) if renderer == RendererKind::Builtin => Some(doc.clone()),
            patch => worktree_document(git, item, renderer, patch.map(|doc| doc.files().meta[0].clone()))?,
        };
        let Some(doc) = doc else {
            output.push_entry(worktree_entry(item));
            continue;
        };
        let mut entry = CompareEntry::from_document(&doc, 0).with_bucket(bucket_of(item.scope));
        entry.paths = worktree_entry(item).paths;
        output.entries.push(entry);
        output.documents.resize(output.entries.len() - 1, None);
        output.documents.push(Some(Arc::new(doc)));
    }
    output.documents.resize(output.entries.len(), None);
    Ok(output)
}

pub fn compare_worktree_entry(
    git: &GitService,
    entry: &CompareEntry,
    renderer: RendererKind,
) -> Result<Option<DiffDocument>> {
    let Some(bucket) = entry.bucket else {
        return Ok(None);
    };
    let item = StatusItem {
        path: entry.path().into_owned(),
        old_path: entry
            .paths
            .old_path()
            .filter(|old| *old != entry.path())
            .map(|old| old.into_owned()),
        scope: scope_of(bucket),
        status: match entry.status() {
            FileStatus::Added if bucket == ChangeBucket::Untracked => "U",
            FileStatus::Added => "A",
            FileStatus::Deleted => "D",
            FileStatus::Renamed => "R",
            FileStatus::Copied => "C",
            FileStatus::Modified => "M",
        }
        .to_owned(),
    };
    let meta = if item.scope == StatusScope::Untracked {
        None
    } else {
        let patch = CompareOutput::from_patch_bytes(&git.status_item_exact_patch(&item)?)?;
        match patch.document(0) {
            Some(doc) if renderer == RendererKind::Builtin => return Ok(Some((**doc).clone())),
            Some(doc) => Some(doc.files().meta[0].clone()),
            None => None,
        }
    };
    worktree_document(git, &item, renderer, meta)
}

fn bucket_documents(
    git: &GitService,
    scope: StatusScope,
    items: &[StatusItem],
) -> Result<std::collections::HashMap<String, DiffDocument>> {
    let mut docs = std::collections::HashMap::new();
    if !items.iter().any(|item| item.scope == scope) {
        return Ok(docs);
    }
    let raw = git.diff_for_commit(scope == StatusScope::Staged)?;
    let output = CompareOutput::from_patch(&raw)?;
    for (index, entry) in output.entries.iter().enumerate() {
        if let Some(doc) = output.document(index) {
            docs.insert(entry.path().into_owned(), (**doc).clone());
        }
    }
    Ok(docs)
}

fn worktree_document(
    git: &GitService,
    item: &StatusItem,
    renderer: RendererKind,
    meta: Option<FileMeta>,
) -> Result<Option<DiffDocument>> {
    let (old_ref, new_ref) = bucket_refs(bucket_of(item.scope));
    let (old_path, new_path) = item_paths(item);
    let read = |reference: Option<&str>, path: Option<&str>| match (reference, path) {
        (Some(reference), Some(path)) => git.read_file_bytes_at(reference, path).map(Some),
        _ => Ok(None),
    };
    let old = read(old_ref, old_path)?;
    let new = read(Some(new_ref), new_path)?;
    let meta = meta.unwrap_or_else(|| FileMeta {
        old_path: old.as_ref().and(old_path).map(Arc::from),
        new_path: new.as_ref().and(new_path).map(Arc::from),
        status: item_status(item),
        ..FileMeta::default()
    });
    let (old, new) = (old.unwrap_or_default(), new.unwrap_or_default());
    if renderer == RendererKind::Semantic {
        let changed = crate::core::compare::semantic::ChangedPath {
            status: meta.status,
            old_path: meta.old_path.as_deref().map(str::to_owned),
            new_path: meta.new_path.as_deref().map(str::to_owned),
            old_content: old,
            new_content: new,
        };
        let mut doc = crate::core::compare::semantic::document_from_changed(&changed)?;
        doc.set_identity(0, (meta.old_id.clone(), meta.new_id.clone()), (meta.old_mode.clone(), meta.new_mode.clone()));
        return Ok(Some(doc));
    }
    let _ = renderer;
    document_from_bytes(meta, &old, &new).map(Some)
}

pub fn compare_path(
    spec: &CompareSpec,
    path: &str,
    git: &GitService,
    renames: bool,
) -> Result<CompareOutput> {
    let (left, right) = resolve_compare_refs(spec, git)?;
    let raw = if renames {
        git.diff_two_refs_path(&left, &right, path)?
    } else {
        git.diff_two_refs_path_no_renames(&left, &right, path)?
    };
    CompareOutput::from_patch(&raw)
}

pub fn compare_entry(entry: &CompareEntry, git: &GitService) -> Result<Option<DiffDocument>> {
    let meta = entry.file_meta();
    if entry.is_binary() {
        return binary_document(meta).map(Some);
    }
    let loadable = match entry.status() {
        FileStatus::Added => entry.new_id.is_some(),
        FileStatus::Deleted => entry.old_id.is_some(),
        _ => entry.old_id.is_some() && entry.new_id.is_some(),
    };
    if !loadable {
        return Ok(None);
    }
    let repo = open_gix_repo(git)?;
    let old = load_object(&repo, entry.old_id.as_ref())?;
    let new = load_object(&repo, entry.new_id.as_ref())?;
    document_from_bytes(meta, old.as_bytes(), new.as_bytes()).map(Some)
}

pub fn compare_text(left: &str, right: &str, display_path: &str) -> Result<CompareOutput> {
    let mut output = CompareOutput::default();
    if left == right {
        return Ok(output);
    }
    let status = if left.is_empty() {
        FileStatus::Added
    } else if right.is_empty() {
        FileStatus::Deleted
    } else {
        FileStatus::Modified
    };
    let meta = FileMeta {
        old_path: (status != FileStatus::Added).then(|| Arc::from(display_path)),
        new_path: (status != FileStatus::Deleted).then(|| Arc::from(display_path)),
        status,
        ..FileMeta::default()
    };
    let doc = document_from_bytes(meta, left.as_bytes(), right.as_bytes())?;
    output.push_document(Arc::new(doc));
    Ok(output)
}

pub fn entry_stats(
    spec: &CompareSpec,
    git: &GitService,
    entries: &[CompareEntry],
) -> Vec<Option<DiffStats>> {
    if entries.is_empty() {
        return Vec::new();
    }
    let Ok((left, right)) = resolve_compare_refs(spec, git) else {
        return vec![None; entries.len()];
    };
    if right == WORKDIR_REF {
        return vec![None; entries.len()];
    }
    let repo_path = git.repo_path();
    let worker_count = deferred_stats_worker_count(entries.len());
    if worker_count == 1 {
        return entry_stats_chunk(repo_path, &left, &right, entries);
    }
    let chunk_size = entries.len().div_ceil(worker_count);
    let mut results = Vec::with_capacity(entries.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = entries
            .chunks(chunk_size)
            .map(|chunk| {
                let (left, right) = (left.as_str(), right.as_str());
                scope.spawn(move || entry_stats_chunk(repo_path, left, right, chunk))
            })
            .collect();
        for (handle, chunk) in handles.into_iter().zip(entries.chunks(chunk_size)) {
            match handle.join() {
                Ok(stats) => results.extend(stats),
                Err(_) => results.extend(std::iter::repeat_n(None, chunk.len())),
            }
        }
    });
    results
}

pub(crate) fn document_from_bytes(
    mut meta: FileMeta,
    old: &[u8],
    new: &[u8],
) -> Result<DiffDocument> {
    if is_binary_bytes(old) || is_binary_bytes(new) {
        return binary_document(meta);
    }
    let changes = line_changes(old, new);
    let (old, new) = (TextStore::from_bytes(old), TextStore::from_bytes(new));
    meta.binary = false;
    diff_from_changes(meta, old, new, &changes, CONTEXT_LINES)
        .map_err(|error| DiffyError::Parse(error.to_string()))
}

pub(crate) fn binary_document(mut meta: FileMeta) -> Result<DiffDocument> {
    meta.binary = true;
    let mut builder = DocumentBuilder::new();
    builder
        .file(
            meta,
            SourceCoverage::PatchOnly,
            TextStore::default(),
            TextStore::default(),
        )
        .finish()
        .map_err(|error| DiffyError::Parse(error.to_string()))?;
    Ok(builder.build())
}

pub(crate) fn line_changes(old: &[u8], new: &[u8]) -> Vec<Change> {
    let input = InternedInput::new(old, new);
    diff_with_slider_heuristics(Algorithm::Histogram, &input)
        .hunks()
        .map(|hunk| Change {
            old: hunk.before,
            new: hunk.after,
        })
        .collect()
}

pub(crate) fn line_stats(old: &[u8], new: &[u8]) -> (u32, u32) {
    let input = InternedInput::new(old, new);
    let diff = diff_with_slider_heuristics(Algorithm::Histogram, &input);
    (diff.count_additions(), diff.count_removals())
}

fn output_from_gix_changes(
    repo: &gix::Repository,
    changes: &[GixChange],
    reporter: Option<&dyn ProgressSink>,
) -> Result<CompareOutput> {
    if changes.len() > COMPARE_SUMMARY_FILE_LIMIT {
        return Ok(CompareOutput::deferred(
            changes.iter().map(entry_from_gix_change).collect(),
        ));
    }
    let files_total = changes.len() as u32;
    if let Some(r) = reporter {
        r.phase(ComparePhase::LoadingFiles {
            files_seen: 0,
            files_total,
        });
    }
    let mut output = CompareOutput::default();
    for (index, change) in changes.iter().enumerate() {
        if let Some(r) = reporter
            && (index.is_multiple_of(LOADING_FILE_EMIT_STRIDE) || index + 1 == changes.len())
        {
            r.phase(ComparePhase::LoadingFiles {
                files_seen: (index + 1) as u32,
                files_total,
            });
        }
        let Some((old, new)) = gix_change_contents(repo, change)? else {
            continue;
        };
        let meta = entry_from_gix_change(change).file_meta();
        let doc = document_from_bytes(meta, old.as_bytes(), new.as_bytes())?;
        output.push_document(Arc::new(doc));
    }
    Ok(output)
}

type EntrySide = Option<(gix::objs::tree::EntryMode, gix::ObjectId)>;

pub(crate) fn entry_from_gix_change(change: &GixChange) -> CompareEntry {
    let (old_path, new_path, old, new, status): (_, _, EntrySide, EntrySide, _) = match change {
        GixChange::Addition {
            location,
            entry_mode,
            id,
            ..
        } => (
            None,
            Some(location.as_bstr()),
            None,
            Some((*entry_mode, *id)),
            FileStatus::Added,
        ),
        GixChange::Deletion {
            location,
            entry_mode,
            id,
            ..
        } => (
            Some(location.as_bstr()),
            None,
            Some((*entry_mode, *id)),
            None,
            FileStatus::Deleted,
        ),
        GixChange::Modification {
            location,
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
        } => (
            Some(location.as_bstr()),
            Some(location.as_bstr()),
            Some((*previous_entry_mode, *previous_id)),
            Some((*entry_mode, *id)),
            FileStatus::Modified,
        ),
        GixChange::Rewrite {
            source_location,
            source_entry_mode,
            source_id,
            entry_mode,
            id,
            location,
            copy,
            ..
        } => (
            Some(source_location.as_bstr()),
            Some(location.as_bstr()),
            Some((*source_entry_mode, *source_id)),
            Some((*entry_mode, *id)),
            if *copy {
                FileStatus::Copied
            } else {
                FileStatus::Renamed
            },
        ),
    };
    let id = |side: EntrySide| side.and_then(|(_, id)| ObjectId::parse(&id.to_string()));
    let mode = |side: EntrySide| side.map(|(mode, _)| Arc::<str>::from(gix_mode(&mode)));
    CompareEntry::deferred(old_path.map(bstr_to_str), new_path.map(bstr_to_str), status)
        .with_ids(id(old), id(new))
        .with_modes(mode(old), mode(new))
}

fn load_object<'repo>(
    repo: &'repo gix::Repository,
    id: Option<&ObjectId>,
) -> Result<GixBlobContent<'repo>> {
    match id {
        Some(id) => load_gix_blob_id(repo, gix_object_id(id.as_str())?),
        None => Ok(GixBlobContent::Empty),
    }
}

fn entry_stats_chunk(
    repo_path: &str,
    left: &str,
    right: &str,
    entries: &[CompareEntry],
) -> Vec<Option<DiffStats>> {
    let Ok(repo) = open_gix_repo_path(repo_path) else {
        return vec![None; entries.len()];
    };
    let (Ok(left), Ok(right)) = (
        gix_tree_for_oid(&repo, left),
        gix_tree_for_oid(&repo, right),
    ) else {
        return vec![None; entries.len()];
    };
    entries
        .iter()
        .map(|entry| {
            entry_stats_with_trees(entry, &repo, &left, &right)
                .ok()
                .flatten()
        })
        .collect()
}

fn entry_stats_with_trees(
    entry: &CompareEntry,
    repo: &gix::Repository,
    left: &gix::Tree<'_>,
    right: &gix::Tree<'_>,
) -> Result<Option<DiffStats>> {
    let known = |additions, deletions| DiffStats::Known {
        additions,
        deletions,
    };
    if entry.is_binary() {
        return Ok(Some(known(0, 0)));
    }
    let old = if entry.status() == FileStatus::Added {
        Some(GixBlobContent::Empty)
    } else {
        load_gix_tree_blob_content(repo, left, entry.paths.old_path().as_deref())?
    };
    let new = if entry.status() == FileStatus::Deleted {
        Some(GixBlobContent::Empty)
    } else {
        load_gix_tree_blob_content(repo, right, entry.paths.new_path().as_deref())?
    };
    let (Some(old), Some(new)) = (old, new) else {
        return Ok(None);
    };
    if is_binary_bytes(old.as_bytes()) || is_binary_bytes(new.as_bytes()) {
        return Ok(Some(known(0, 0)));
    }
    let (additions, deletions) = line_stats(old.as_bytes(), new.as_bytes());
    Ok(Some(known(additions, deletions)))
}

pub(crate) type GixChange = gix::object::tree::diff::ChangeDetached;

pub(crate) fn resolve_compare_refs(
    spec: &CompareSpec,
    git: &GitService,
) -> Result<(String, String)> {
    match spec.mode {
        crate::core::compare::spec::CompareMode::TwoDot => {
            if spec.right_ref == WORKDIR_REF {
                Ok((git.resolve_ref(&spec.left_ref)?, WORKDIR_REF.to_owned()))
            } else {
                Ok((
                    git.resolve_ref(&spec.left_ref)?,
                    git.resolve_ref(&spec.right_ref)?,
                ))
            }
        }
        crate::core::compare::spec::CompareMode::ThreeDot
        | crate::core::compare::spec::CompareMode::SingleCommit => {
            Ok(git.resolve_comparison(&spec.left_ref, &spec.right_ref, spec.mode)?)
        }
    }
}

pub(crate) fn open_gix_repo(git: &GitService) -> Result<gix::Repository> {
    open_gix_repo_path(git.repo_path())
}

pub(crate) fn open_gix_repo_path(repo_path: &str) -> Result<gix::Repository> {
    let mut repo = gix::open(repo_path).map_err(gix_error)?;
    repo.object_cache_size_if_unset(64 * 1024 * 1024);
    Ok(repo)
}

pub(crate) fn gix_error(error: impl std::fmt::Display) -> DiffyError {
    DiffyError::General(format!("Gitoxide error: {error}"))
}

pub(crate) fn collect_gix_changes(
    repo: &mut gix::Repository,
    left: &str,
    right: &str,
    path_filter: Option<&str>,
    track_rewrites: bool,
) -> Result<Vec<GixChange>> {
    let left_tree = gix_tree_for_oid(repo, left)?;
    let right_tree = gix_tree_for_oid(repo, right)?;
    let mut options = gix::diff::Options::default();
    options.track_path();
    if track_rewrites {
        options.track_rewrites(Some(gix::diff::Rewrites {
            limit: RENAME_DETECTION_LIMIT,
            ..Default::default()
        }));
    } else {
        options.track_rewrites(None);
    }
    let mut changes = repo
        .diff_tree_to_tree(Some(&left_tree), Some(&right_tree), Some(options))
        .map_err(gix_error)?;
    changes.retain(gix_change_is_file);
    if let Some(path) = path_filter {
        changes.retain(|change| gix_change_matches_path(change, path));
    }
    Ok(changes)
}

pub(crate) fn gix_tree_for_oid<'repo>(
    repo: &'repo gix::Repository,
    oid: &str,
) -> Result<gix::Tree<'repo>> {
    let oid = gix_object_id(oid)?;
    let object = repo.find_object(oid).map_err(gix_error)?;
    object.peel_to_tree().map_err(gix_error)
}

pub(crate) fn gix_object_id(oid: &str) -> Result<gix::ObjectId> {
    gix::ObjectId::from_hex(oid.as_bytes()).map_err(gix_error)
}

pub(crate) fn gix_change_matches_path(change: &GixChange, path: &str) -> bool {
    let path = path.as_bytes();
    change.location().as_bytes() == path || change.source_location().as_bytes() == path
}

pub(crate) fn gix_change_is_file(change: &GixChange) -> bool {
    match change {
        GixChange::Addition { entry_mode, .. } | GixChange::Deletion { entry_mode, .. } => {
            entry_mode.is_no_tree()
        }
        GixChange::Modification {
            previous_entry_mode,
            entry_mode,
            ..
        } => previous_entry_mode.is_no_tree() && entry_mode.is_no_tree(),
        GixChange::Rewrite {
            source_entry_mode,
            entry_mode,
            ..
        } => source_entry_mode.is_no_tree() && entry_mode.is_no_tree(),
    }
}

pub(crate) fn gix_changes_may_include_rewrites(changes: &[GixChange]) -> bool {
    let mut has_addition = false;
    let mut has_deletion = false;
    for change in changes {
        match change {
            GixChange::Addition { .. } => has_addition = true,
            GixChange::Deletion { .. } => has_deletion = true,
            GixChange::Modification { .. } | GixChange::Rewrite { .. } => {}
        }
        if has_addition && has_deletion {
            return true;
        }
    }
    false
}

pub(crate) fn gix_change_contents<'repo>(
    repo: &'repo gix::Repository,
    change: &GixChange,
) -> Result<Option<(GixBlobContent<'repo>, GixBlobContent<'repo>)>> {
    match change {
        GixChange::Addition { entry_mode, id, .. } => {
            if !entry_mode.is_no_tree() {
                return Ok(None);
            }
            Ok(Some((GixBlobContent::Empty, load_gix_blob_id(repo, *id)?)))
        }
        GixChange::Deletion { entry_mode, id, .. } => {
            if !entry_mode.is_no_tree() {
                return Ok(None);
            }
            Ok(Some((load_gix_blob_id(repo, *id)?, GixBlobContent::Empty)))
        }
        GixChange::Modification {
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
            ..
        } => {
            if !previous_entry_mode.is_no_tree() || !entry_mode.is_no_tree() {
                return Ok(None);
            }
            Ok(Some((
                load_gix_blob_id(repo, *previous_id)?,
                load_gix_blob_id(repo, *id)?,
            )))
        }
        GixChange::Rewrite {
            source_entry_mode,
            source_id,
            entry_mode,
            id,
            ..
        } => {
            if !source_entry_mode.is_no_tree() || !entry_mode.is_no_tree() {
                return Ok(None);
            }
            Ok(Some((
                load_gix_blob_id(repo, *source_id)?,
                load_gix_blob_id(repo, *id)?,
            )))
        }
    }
}

pub(crate) fn load_gix_tree_blob_content<'repo>(
    repo: &'repo gix::Repository,
    tree: &gix::Tree<'_>,
    path: Option<&str>,
) -> Result<Option<GixBlobContent<'repo>>> {
    let Some(path) = path else {
        return Ok(Some(GixBlobContent::Empty));
    };
    let Some(entry) = tree.lookup_entry_by_path(path).map_err(gix_error)? else {
        return Ok(None);
    };
    if !entry.mode().is_blob_or_symlink() {
        return Ok(None);
    }
    Ok(Some(load_gix_blob_id(repo, entry.object_id())?))
}

pub(crate) fn load_gix_blob_id(
    repo: &gix::Repository,
    oid: gix::ObjectId,
) -> Result<GixBlobContent<'_>> {
    if oid.is_null() {
        return Ok(GixBlobContent::Empty);
    }
    Ok(GixBlobContent::Blob(
        repo.find_blob(oid).map_err(gix_error)?,
    ))
}

pub(crate) fn is_binary_bytes(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|byte| *byte == 0)
}

pub(crate) fn deferred_stats_worker_count(file_count: usize) -> usize {
    let available = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let useful = file_count.div_ceil(DEFERRED_STATS_MIN_FILES_PER_WORKER);
    available.min(LINE_STATS_MAX_WORKERS).min(useful).max(1)
}

pub(crate) fn bstr_to_str(path: &BStr) -> &str {
    path.to_str().unwrap_or("")
}

pub(crate) fn gix_mode(mode: &gix::objs::tree::EntryMode) -> String {
    format!("{:06o}", mode.value())
}

pub(crate) enum GixBlobContent<'repo> {
    Empty,
    Blob(gix::Blob<'repo>),
}

impl GixBlobContent<'_> {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Empty => &[],
            Self::Blob(blob) => &blob.data,
        }
    }
}
