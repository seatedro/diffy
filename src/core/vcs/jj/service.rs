use std::ffi::OsString;
use std::path::{Path, PathBuf};

use carbon::TextStore;

use crate::core::compare::backends::compare_output_from_raw_patch;
use crate::core::compare::{CompareMode, CompareOutput, CompareSpec, ProgressSink};
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::backend::{VcsBackend, VcsRepository, VcsWatchPaths};
use crate::core::vcs::jj::cli::JjCli;
use crate::core::vcs::jj::parse::{
    parse_bookmark_list, parse_change_log, parse_conflict_list, parse_diff_summary,
};
use crate::core::vcs::model::{
    RefKind, RepoCapabilities, RepoLocation, RevisionId, VcsCompareRequest, VcsCompareSpec,
    VcsKind, VcsRef, VcsSnapshot,
};
use crate::events::RepositorySyncReason;

#[derive(Debug, Clone, Copy, Default)]
pub struct JjBackend;

impl VcsBackend for JjBackend {
    fn kind(&self) -> VcsKind {
        VcsKind::Jj
    }

    fn detect(&self, path: &Path) -> Result<Option<RepoLocation>> {
        let Some(root) = find_jj_root(path) else {
            return Ok(None);
        };
        Ok(Some(RepoLocation {
            kind: VcsKind::Jj,
            workspace_root: root.clone(),
            store_root: Some(root.join(".jj")),
        }))
    }

    fn open(&self, location: RepoLocation) -> Result<Box<dyn VcsRepository>> {
        Ok(Box::new(JjRepository::open(location)))
    }

    fn watch_paths(&self, location: &RepoLocation) -> Result<VcsWatchPaths> {
        Ok(VcsWatchPaths {
            metadata_dir: location
                .store_root
                .clone()
                .unwrap_or_else(|| location.workspace_root.join(".jj")),
            workdir: Some(location.workspace_root.clone()),
            watched_paths: vec![location.workspace_root.clone()],
        })
    }
}

pub struct JjRepository {
    cli: JjCli,
    location: RepoLocation,
    last_operation_id: Option<String>,
    last_snapshot: Option<VcsSnapshot>,
    diff_cache: Vec<DiffCacheEntry>,
    file_text_cache: Vec<FileTextCacheEntry>,
}

#[derive(Clone)]
struct DiffCacheEntry {
    operation_id: Option<String>,
    request: VcsCompareRequest,
    path: Option<String>,
    output: CompareOutput,
}

#[derive(Clone)]
struct FileTextCacheEntry {
    operation_id: Option<String>,
    revision: RevisionId,
    path: String,
    text: TextStore,
}

impl JjRepository {
    pub fn open(location: RepoLocation) -> Self {
        Self {
            cli: JjCli::new(location.workspace_root.clone()),
            location,
            last_operation_id: None,
            last_snapshot: None,
            diff_cache: Vec::new(),
            file_text_cache: Vec::new(),
        }
    }

    fn diff_args_for_spec(&self, spec: &VcsCompareSpec) -> Result<Vec<OsString>> {
        let mut args = vec![OsString::from("diff")];
        match spec {
            VcsCompareSpec::WorkingCopy => {
                args.push(OsString::from("-r"));
                args.push(OsString::from("@"));
            }
            VcsCompareSpec::Change { revision } => {
                args.push(OsString::from("-r"));
                args.push(OsString::from(revision));
            }
            VcsCompareSpec::Range { from, to } => {
                args.push(OsString::from("--from"));
                args.push(OsString::from(from));
                args.push(OsString::from("--to"));
                args.push(OsString::from(to));
            }
            VcsCompareSpec::MergeBaseRange { .. } => {
                return Err(DiffyError::General(
                    "jj merge-base compare is not supported yet".to_owned(),
                ));
            }
        }
        args.push(OsString::from("--git"));
        Ok(args)
    }

    fn current_operation_id(&self) -> Result<String> {
        let output = self.cli.run_ignored_wc(&[
            OsString::from("operation"),
            OsString::from("log"),
            OsString::from("--no-graph"),
            OsString::from("-n"),
            OsString::from("1"),
            OsString::from("-T"),
            OsString::from("id ++ \"\\n\""),
        ])?;
        Ok(output.trim().to_owned())
    }

    fn set_operation_id(&mut self, operation_id: String) {
        if self.last_operation_id.as_deref() != Some(operation_id.as_str()) {
            self.diff_cache.clear();
            self.file_text_cache.clear();
            self.last_snapshot = None;
        }
        self.last_operation_id = Some(operation_id);
    }

    fn ensure_read_epoch(&mut self) -> Result<Option<String>> {
        if self.last_operation_id.is_none() {
            self.cli.run(&[OsString::from("status")])?;
            let operation_id = self.current_operation_id()?;
            self.set_operation_id(operation_id);
        }
        Ok(self.last_operation_id.clone())
    }

    fn cached_diff(
        &self,
        operation_id: Option<&str>,
        request: &VcsCompareRequest,
        path: Option<&str>,
    ) -> Option<CompareOutput> {
        self.diff_cache
            .iter()
            .find(|entry| {
                entry.operation_id.as_deref() == operation_id
                    && entry.request == *request
                    && entry.path.as_deref() == path
            })
            .map(|entry| entry.output.clone())
    }

    fn insert_diff_cache(
        &mut self,
        operation_id: Option<String>,
        request: VcsCompareRequest,
        path: Option<String>,
        output: CompareOutput,
    ) {
        const MAX_DIFF_CACHE_ENTRIES: usize = 8;
        if self.diff_cache.len() >= MAX_DIFF_CACHE_ENTRIES {
            self.diff_cache.remove(0);
        }
        self.diff_cache.push(DiffCacheEntry {
            operation_id,
            request,
            path,
            output,
        });
    }

    fn cached_file_text(
        &self,
        operation_id: Option<&str>,
        revision: &RevisionId,
        path: &str,
    ) -> Option<TextStore> {
        self.file_text_cache
            .iter()
            .find(|entry| {
                entry.operation_id.as_deref() == operation_id
                    && entry.revision == *revision
                    && entry.path == path
            })
            .map(|entry| entry.text.clone())
    }

    fn insert_file_text_cache(
        &mut self,
        operation_id: Option<String>,
        revision: RevisionId,
        path: String,
        text: TextStore,
    ) {
        const MAX_FILE_TEXT_CACHE_ENTRIES: usize = 16;
        if self.file_text_cache.len() >= MAX_FILE_TEXT_CACHE_ENTRIES {
            self.file_text_cache.remove(0);
        }
        self.file_text_cache.push(FileTextCacheEntry {
            operation_id,
            revision,
            path,
            text,
        });
    }

    fn conflict_list(&self) -> Result<String> {
        match self.cli.run_ignored_wc(&[
            OsString::from("resolve"),
            OsString::from("--list"),
            OsString::from("-r"),
            OsString::from("@"),
        ]) {
            Ok(output) => Ok(output),
            Err(error) if error.to_string().contains("No conflicts found") => Ok(String::new()),
            Err(error) => Err(error),
        }
    }
}

impl VcsRepository for JjRepository {
    fn location(&self) -> &RepoLocation {
        &self.location
    }

    fn capabilities(&self) -> RepoCapabilities {
        jj_capabilities()
    }

    fn resolve_ref(&mut self, reference: &str) -> Result<(String, String)> {
        let output = self.cli.run_ignored_wc(&[
            OsString::from("log"),
            OsString::from("--no-graph"),
            OsString::from("-r"),
            OsString::from(reference),
            OsString::from("-T"),
            OsString::from(
                "commit_id.shortest() ++ \"\\t\" ++ description.first_line() ++ \"\\n\"",
            ),
        ])?;
        let mut fields = output.trim_end().splitn(2, '\t');
        let short_id = fields.next().unwrap_or_default().to_owned();
        let summary = fields.next().unwrap_or_default().to_owned();
        Ok((short_id, summary))
    }

    fn snapshot(
        &mut self,
        reason: RepositorySyncReason,
        _reporter: Option<&dyn ProgressSink>,
    ) -> Result<VcsSnapshot> {
        // jj 0.32 snapshots at the start of normal read commands. Run one
        // non-ignored status first, then keep the rest of this refresh on the
        // stable working-copy commit with `--ignore-working-copy`.
        self.cli.run(&[OsString::from("status")])?;
        let operation_id = self.current_operation_id()?;
        if self.last_operation_id.as_deref() == Some(operation_id.as_str())
            && let Some(snapshot) = self.last_snapshot.as_ref()
        {
            let mut snapshot = snapshot.clone();
            snapshot.reason = reason;
            snapshot.change_kind = None;
            return Ok(snapshot);
        }
        self.set_operation_id(operation_id.clone());

        let summary = self.cli.run_ignored_wc(&[
            OsString::from("diff"),
            OsString::from("-r"),
            OsString::from("@"),
            OsString::from("--summary"),
        ])?;
        let change_log = self.cli.run_ignored_wc(&[
            OsString::from("log"),
            OsString::from("--no-graph"),
            OsString::from("--revisions"),
            OsString::from("ancestors(@, 200) ~ root()"),
            OsString::from("-T"),
            OsString::from(
                "change_id ++ \"\\t\" ++ change_id.shortest(8).prefix() ++ \"\\t\" ++ change_id.shortest(8).rest() ++ \"\\t\" ++ commit_id ++ \"\\t\" ++ description.first_line() ++ \"\\t\" ++ author.name() ++ \"\\t\" ++ committer.timestamp() ++ \"\\n\"",
            ),
        ])?;
        let bookmarks = self.cli.run_ignored_wc(&[
            OsString::from("bookmark"),
            OsString::from("list"),
            OsString::from("-T"),
            OsString::from("name ++ \"\\t\" ++ normal_target.commit_id() ++ \"\\n\""),
        ])?;
        let conflicts = self.conflict_list()?;
        let mut file_changes = parse_diff_summary(&summary);
        file_changes.extend(parse_conflict_list(&conflicts));
        let conflicted = file_changes
            .iter()
            .any(|file| file.status == crate::core::vcs::model::FileChangeStatus::Conflicted);
        let mut changes = parse_change_log(&change_log);
        for change in &mut changes {
            change.flags.conflicted = conflicted;
        }
        let current_revision = changes
            .first()
            .map(|change| change.revision.clone())
            .unwrap_or_else(|| RevisionId {
                backend: VcsKind::Jj,
                id: "@".to_owned(),
            });

        let mut refs = vec![VcsRef {
            name: "@".to_owned(),
            kind: RefKind::WorkingCopy,
            target: current_revision,
            active: true,
            upstream: None,
            ahead_behind: None,
        }];
        refs.extend(parse_bookmark_list(&bookmarks));

        let snapshot = VcsSnapshot {
            location: self.location.clone(),
            reason,
            change_kind: None,
            capabilities: jj_capabilities(),
            refs,
            changes,
            file_changes,
        };
        self.last_snapshot = Some(snapshot.clone());
        Ok(snapshot)
    }

    fn resolve_compare_spec(
        &mut self,
        spec: &CompareSpec,
    ) -> Result<(String, String, VcsCompareRequest)> {
        let vcs_spec = match spec.mode {
            CompareMode::SingleCommit => {
                let revision = if spec.right_ref.is_empty() {
                    spec.left_ref.clone()
                } else {
                    spec.right_ref.clone()
                };
                VcsCompareSpec::Change { revision }
            }
            CompareMode::TwoDot => VcsCompareSpec::Range {
                from: spec.left_ref.clone(),
                to: spec.right_ref.clone(),
            },
            CompareMode::ThreeDot => VcsCompareSpec::MergeBaseRange {
                base: spec.left_ref.clone(),
                head: spec.right_ref.clone(),
            },
        };
        Ok((
            spec.left_ref.clone(),
            spec.right_ref.clone(),
            VcsCompareRequest {
                spec: vcs_spec,
                layout: spec.layout,
                renderer: spec.renderer,
            },
        ))
    }

    fn compare(
        &mut self,
        request: &VcsCompareRequest,
        _reporter: Option<&dyn ProgressSink>,
    ) -> Result<CompareOutput> {
        let operation_id = self.ensure_read_epoch()?;
        if let Some(output) = self.cached_diff(operation_id.as_deref(), request, None) {
            return Ok(output);
        }
        let args = self.diff_args_for_spec(&request.spec)?;
        let raw_diff = self.cli.run_ignored_wc(&args)?;
        let output = compare_output_from_raw_patch(&raw_diff)?;
        self.insert_diff_cache(operation_id, request.clone(), None, output.clone());
        Ok(output)
    }

    fn compare_stats(&mut self, request: &VcsCompareRequest) -> Result<(i32, i32)> {
        let output = self.compare(request, None)?;
        let additions = output
            .carbon
            .files
            .iter()
            .map(|file| u32_to_i32_saturating(file.additions))
            .sum();
        let deletions = output
            .carbon
            .files
            .iter()
            .map(|file| u32_to_i32_saturating(file.deletions))
            .sum();
        Ok((additions, deletions))
    }

    fn compare_path(
        &mut self,
        request: &VcsCompareRequest,
        path: &str,
        _deferred_file: Option<&carbon::FileDiff>,
    ) -> Result<CompareOutput> {
        let operation_id = self.ensure_read_epoch()?;
        if let Some(output) = self.cached_diff(operation_id.as_deref(), request, Some(path)) {
            return Ok(output);
        }
        let mut args = self.diff_args_for_spec(&request.spec)?;
        args.push(OsString::from(format!("file:{path}")));
        let raw_diff = self.cli.run_ignored_wc(&args)?;
        let output = compare_output_from_raw_patch(&raw_diff)?;
        self.insert_diff_cache(
            operation_id,
            request.clone(),
            Some(path.to_owned()),
            output.clone(),
        );
        Ok(output)
    }

    fn compare_working_file(&mut self, path: &str) -> Result<CompareOutput> {
        let operation_id = self.ensure_read_epoch()?;
        let request = VcsCompareRequest {
            spec: VcsCompareSpec::WorkingCopy,
            layout: crate::core::compare::LayoutMode::Unified,
            renderer: crate::core::compare::RendererKind::Builtin,
        };
        if let Some(output) = self.cached_diff(operation_id.as_deref(), &request, Some(path)) {
            return Ok(output);
        }
        let raw_diff = self.cli.run_ignored_wc(&[
            OsString::from("diff"),
            OsString::from("-r"),
            OsString::from("@"),
            OsString::from("--git"),
            OsString::from(format!("file:{path}")),
        ])?;
        let output = compare_output_from_raw_patch(&raw_diff)?;
        self.insert_diff_cache(operation_id, request, Some(path.to_owned()), output.clone());
        Ok(output)
    }

    fn read_file_text(&mut self, revision: &RevisionId, path: &str) -> Result<TextStore> {
        let operation_id = self.ensure_read_epoch()?;
        if let Some(text) = self.cached_file_text(operation_id.as_deref(), revision, path) {
            return Ok(text);
        }
        let output = self.cli.run_ignored_wc(&[
            OsString::from("file"),
            OsString::from("show"),
            OsString::from("-r"),
            OsString::from(revision.id.as_str()),
            OsString::from(format!("file:{path}")),
        ])?;
        let text = TextStore::from_text(output);
        self.insert_file_text_cache(
            operation_id,
            revision.clone(),
            path.to_owned(),
            text.clone(),
        );
        Ok(text)
    }
}

fn u32_to_i32_saturating(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

pub fn jj_capabilities() -> RepoCapabilities {
    RepoCapabilities {
        staging_area: false,
        branches: false,
        bookmarks: true,
        tags: false,
        remotes: false,
        pull_fast_forward: false,
        partial_file_restore: true,
        partial_hunk_mutation: false,
        operation_log: true,
        github_pull_requests: false,
    }
}

fn find_jj_root(path: &Path) -> Option<PathBuf> {
    let mut current = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    loop {
        if current.join(".jj").is_dir() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;

    use tempfile::TempDir;

    use super::JjBackend;
    use crate::core::compare::{CompareMode, CompareSpec, LayoutMode, RendererKind};
    use crate::core::vcs::backend::VcsBackend;
    use crate::core::vcs::model::{ChangeBucket, FileChangeStatus, VcsCompareSpec, VcsKind};
    use crate::events::RepositorySyncReason;

    #[test]
    fn jj_backend_snapshots_and_diffs_working_copy() {
        let Some(repo_dir) = init_jj_repo() else {
            return;
        };
        fs::write(repo_dir.path().join("README.md"), "hello\n").unwrap();

        let backend = JjBackend;
        let location = backend.detect(repo_dir.path()).unwrap().unwrap();
        assert_eq!(location.kind, VcsKind::Jj);

        let mut repo = backend.open(location).unwrap();
        let snapshot = repo
            .snapshot(RepositorySyncReason::Open, None)
            .expect("jj snapshot");
        assert!(snapshot.capabilities.bookmarks);
        assert!(!snapshot.capabilities.staging_area);
        assert!(snapshot.file_changes.iter().any(|file| {
            file.path == "README.md"
                && file.status == FileChangeStatus::Added
                && file.bucket == ChangeBucket::WorkingCopy
        }));

        let output = repo.compare_working_file("README.md").unwrap();
        assert_eq!(output.carbon.files.len(), 1);
        assert_eq!(output.carbon.files[0].path(), "README.md");

        let spec = CompareSpec {
            left_ref: "@-".to_owned(),
            right_ref: "@".to_owned(),
            mode: CompareMode::TwoDot,
            layout: LayoutMode::Unified,
            renderer: RendererKind::Builtin,
        };
        let (_, _, request) = repo.resolve_compare_spec(&spec).unwrap();
        let (additions, deletions) = repo.compare_stats(&request).unwrap();
        assert_eq!((additions, deletions), (1, 0));

        let single_spec = CompareSpec {
            left_ref: "@".to_owned(),
            right_ref: String::new(),
            mode: CompareMode::SingleCommit,
            layout: LayoutMode::Unified,
            renderer: RendererKind::Builtin,
        };
        let (_, _, request) = repo.resolve_compare_spec(&single_spec).unwrap();
        assert_eq!(
            request.spec,
            VcsCompareSpec::Change {
                revision: "@".to_owned()
            }
        );
    }

    fn init_jj_repo() -> Option<TempDir> {
        if Command::new("jj").arg("--version").output().is_err() {
            return None;
        }
        let repo_dir = TempDir::new().unwrap();
        let status = Command::new("jj")
            .arg("--quiet")
            .arg("git")
            .arg("init")
            .arg(repo_dir.path())
            .status()
            .unwrap();
        assert!(status.success());
        Some(repo_dir)
    }
}
