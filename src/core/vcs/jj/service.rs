use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use carbon::TextStore;

use crate::core::compare::backends::compare_output_from_raw_patch;
#[cfg(feature = "difftastic")]
use crate::core::compare::backends::{DifftasticChangedPath, compare_changed_paths};
use crate::core::compare::{
    COMPARE_SUMMARY_FILE_LIMIT, CompareFileStatsTarget, CompareFileSummary, CompareOutput,
    ProgressSink, RendererKind,
};
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::backend::{VcsBackend, VcsRepository, VcsWatchPaths};
use crate::core::vcs::jj::cli::JjCli;
use crate::core::vcs::jj::parse::{
    parse_bookmark_list, parse_change_log, parse_conflict_list, parse_diff_summary,
    parse_operation_log,
};
use crate::core::vcs::model::{
    ChangeIdToken, FileChange, FileChangeStatus, FileOperation, JjOperation, PublishAction,
    PublishActionKind, PublishOutcome, PublishPlan, RefKind, RepoCapabilities, RepoLocation,
    RevisionId, VCS_PROFILE_JJ, VcsCompareRequest, VcsCompareSpec, VcsKind, VcsOperation, VcsRef,
    VcsSnapshot,
};
use crate::events::RepositorySyncReason;

const JJ_FILE_STATS_PATH_CHUNK: usize = 512;

#[derive(Debug, Clone, Copy, Default)]
pub struct JjBackend;

impl VcsBackend for JjBackend {
    fn kind(&self) -> VcsKind {
        VcsKind::JJ
    }

    fn detect(&self, path: &Path) -> Result<Option<RepoLocation>> {
        let Some(root) = find_jj_root(path) else {
            return Ok(None);
        };
        Ok(Some(RepoLocation {
            kind: VcsKind::JJ,
            profile: VCS_PROFILE_JJ,
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
            worktree_metadata_paths: Vec::new(),
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

#[derive(Debug, Clone)]
struct JjPublishTarget {
    revision: String,
    commit_id: String,
    short_commit_id: String,
    short_change_id: String,
    short_change_id_prefix_len: usize,
    summary: String,
    fell_back_from_empty_working_copy: bool,
}

#[derive(Debug, Clone)]
struct MovableBookmark {
    name: String,
    target: String,
    allow_backwards: bool,
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
            VcsCompareSpec::MergeBaseRange { base, head } => {
                args.push(OsString::from("--from"));
                args.push(OsString::from(jj_fork_point_revset(base, head)));
                args.push(OsString::from("--to"));
                args.push(OsString::from(head));
            }
        }
        args.push(OsString::from("--git"));
        Ok(args)
    }

    fn diff_summary_args_for_spec(&self, spec: &VcsCompareSpec) -> Result<Vec<OsString>> {
        let mut args = self.diff_args_for_spec(spec)?;
        args.retain(|arg| arg != "--git");
        args.push(OsString::from("--summary"));
        Ok(args)
    }

    fn diff_stat_args_for_spec(&self, spec: &VcsCompareSpec) -> Result<Vec<OsString>> {
        let mut args = self.diff_args_for_spec(spec)?;
        args.retain(|arg| arg != "--git");
        args.push(OsString::from("--stat"));
        Ok(args)
    }

    #[cfg(feature = "difftastic")]
    fn compare_difftastic(
        &self,
        request: &VcsCompareRequest,
        reporter: Option<&dyn ProgressSink>,
        only_path: Option<&str>,
    ) -> Result<CompareOutput> {
        let mut summary_args = self.diff_summary_args_for_spec(&request.spec)?;
        if let Some(path) = only_path {
            summary_args.push(jj_root_pathspec(path));
        }
        let summary = self.cli.run_ignored_wc(&summary_args)?;
        let changes = parse_diff_summary(&summary);
        if only_path.is_none() && changes.len() > COMPARE_SUMMARY_FILE_LIMIT {
            let mut output = CompareOutput {
                file_summaries: changes
                    .into_iter()
                    .map(|change| {
                        CompareFileSummary::from_paths_status(
                            change.old_path.as_deref(),
                            Some(change.path.as_str()),
                            carbon_status_from_file_change_status(change.status),
                            true,
                        )
                    })
                    .collect(),
                ..CompareOutput::default()
            };
            output.compact_file_summaries();
            return Ok(output);
        }

        let (old_rev, new_rev) = difftastic_revisions_for_spec(&request.spec);
        let mut changed_paths = Vec::with_capacity(changes.len());
        for change in changes {
            changed_paths.push(self.difftastic_changed_path(&change, &old_rev, &new_rev)?);
        }
        compare_changed_paths(changed_paths, reporter)
    }

    #[cfg(feature = "difftastic")]
    fn difftastic_changed_path(
        &self,
        change: &FileChange,
        old_rev: &str,
        new_rev: &str,
    ) -> Result<DifftasticChangedPath> {
        let old_path = match change.status {
            FileChangeStatus::Added | FileChangeStatus::Untracked => None,
            _ => Some(change.old_path.as_deref().unwrap_or(change.path.as_str())),
        };
        let new_path = match change.status {
            FileChangeStatus::Deleted => None,
            _ => Some(change.path.as_str()),
        };
        let old_content = old_path
            .map(|path| self.jj_file_show_bytes(old_rev, path))
            .transpose()?
            .unwrap_or_default();
        let new_content = new_path
            .map(|path| self.jj_file_show_bytes(new_rev, path))
            .transpose()?
            .unwrap_or_default();
        let is_binary = matches!(
            change.status,
            FileChangeStatus::Binary | FileChangeStatus::Conflicted | FileChangeStatus::TypeChanged
        ) || looks_binary(&old_content)
            || looks_binary(&new_content);

        Ok(DifftasticChangedPath {
            status: difftastic_status_label(change.status).to_owned(),
            old_path: old_path.map(str::to_owned),
            new_path: new_path.map(str::to_owned),
            old_content,
            new_content,
            is_binary,
        })
    }

    #[cfg(feature = "difftastic")]
    fn jj_file_show_bytes(&self, revision: &str, path: &str) -> Result<Vec<u8>> {
        self.cli.run_bytes_ignored_wc(&[
            OsString::from("file"),
            OsString::from("show"),
            OsString::from("-r"),
            OsString::from(revision),
            jj_root_pathspec(path),
        ])
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

    fn clear_after_write(&mut self) {
        self.last_operation_id = None;
        self.last_snapshot = None;
        self.diff_cache.clear();
        self.file_text_cache.clear();
    }

    fn remote_names(&self) -> Result<Vec<String>> {
        let output = self.cli.run_ignored_wc(&[
            OsString::from("git"),
            OsString::from("remote"),
            OsString::from("list"),
        ])?;
        Ok(parse_remote_names(&output).into_iter().collect())
    }

    fn preferred_remote(&self) -> Result<String> {
        let remotes = self.remote_names()?;
        remotes
            .iter()
            .find(|remote| remote.as_str() == "origin")
            .cloned()
            .or_else(|| remotes.first().cloned())
            .ok_or_else(|| {
                DiffyError::General("No remotes are configured for this repository.".to_owned())
            })
    }

    fn default_publish_target(&self) -> Result<JjPublishTarget> {
        // jj refuses to push undescribed changes (`jj git push --change @`
        // returns "Won't push commit ... because it has no description"), so
        // even when `@` has a working-copy diff we still prefer `@-` whenever
        // `@` has no description. If there is no described parent to publish,
        // the user must describe the current change first.
        let head_target = self.publish_target("@")?;
        let target = if head_target.summary.trim().is_empty() {
            let mut parent = self.publish_target("@-").map_err(|_| {
                DiffyError::General(
                    "Describe the current jj change before publishing it.".to_owned(),
                )
            })?;
            parent.fell_back_from_empty_working_copy = true;
            parent
        } else {
            head_target
        };
        if target.summary.trim().is_empty() {
            Err(DiffyError::General(
                "Describe the jj change before publishing it.".to_owned(),
            ))
        } else {
            Ok(target)
        }
    }

    fn publish_target(&self, revision: &str) -> Result<JjPublishTarget> {
        let output = self.cli.run_ignored_wc(&[
            OsString::from("log"),
            OsString::from("--no-graph"),
            OsString::from("-r"),
            OsString::from(revision),
            OsString::from("-T"),
            OsString::from(
                "commit_id ++ \"\\t\" ++ commit_id.shortest() ++ \"\\t\" ++ change_id.shortest(8).prefix() ++ \"\\t\" ++ change_id.shortest(8).rest() ++ \"\\t\" ++ description.first_line() ++ \"\\n\"",
            ),
        ])?;
        let mut fields = output.trim_end().splitn(5, '\t');
        let commit_id = fields.next().unwrap_or_default().to_owned();
        let short_commit_id = fields.next().unwrap_or_default().to_owned();
        let change_id_prefix = fields.next().unwrap_or_default();
        let change_id_rest = fields.next().unwrap_or_default();
        let summary = fields.next().unwrap_or_default().to_owned();
        if commit_id.is_empty() {
            return Err(DiffyError::General(format!(
                "Could not resolve jj revision {revision} for publishing."
            )));
        }
        let short_change_id = format!("{change_id_prefix}{change_id_rest}");
        let short_change_id_prefix_len = change_id_prefix.len();
        Ok(JjPublishTarget {
            revision: revision.to_owned(),
            commit_id,
            short_commit_id,
            short_change_id,
            short_change_id_prefix_len,
            summary,
            fell_back_from_empty_working_copy: false,
        })
    }

    fn remote_bookmarks_at(&self, commit_id: &str, remote: &str) -> Result<Vec<String>> {
        let output = self.cli.run_ignored_wc(&[
            OsString::from("bookmark"),
            OsString::from("list"),
            OsString::from("--all-remotes"),
            OsString::from("-T"),
            OsString::from(
                "name ++ \"\\t\" ++ if(self.remote(), self.remote(), \"\") ++ \"\\t\" ++ normal_target.commit_id() ++ \"\\n\"",
            ),
        ])?;
        Ok(parse_remote_bookmark_targets(&output, remote, commit_id))
    }

    fn local_bookmarks_at(&self, commit_id: &str) -> Result<Vec<String>> {
        let output = self.cli.run_ignored_wc(&[
            OsString::from("bookmark"),
            OsString::from("list"),
            OsString::from("-T"),
            OsString::from("name ++ \"\\t\" ++ normal_target.commit_id() ++ \"\\n\""),
        ])?;
        Ok(parse_bookmark_list(&output)
            .into_iter()
            .filter(|reference| reference.target.id == commit_id)
            .map(|reference| reference.name)
            .collect())
    }

    fn movable_bookmarks(&self, revision: &str) -> Result<Vec<MovableBookmark>> {
        let revset_after = format!("{revision}::");
        let revset = format!("::{revision} | {revset_after}");
        let output = self.cli.run_ignored_wc(&[
            OsString::from("bookmark"),
            OsString::from("list"),
            OsString::from("-r"),
            OsString::from(revset),
            OsString::from("-T"),
            OsString::from(format!(
                "name ++ \"\\t\" ++ normal_target.commit_id() ++ \"\\t\" ++ normal_target.contained_in(\"{revset_after}\") ++ \"\\n\""
            )),
        ])?;
        let mut bookmarks = output
            .lines()
            .filter_map(parse_movable_bookmark_line)
            .collect::<Vec<_>>();
        bookmarks.sort_by(|left, right| {
            bookmark_priority(&left.name)
                .cmp(&bookmark_priority(&right.name))
                .then(left.name.cmp(&right.name))
        });
        Ok(bookmarks)
    }

    fn generated_bookmark_name(target: &JjPublishTarget) -> String {
        let suffix = if target.short_change_id.is_empty() {
            target.short_commit_id.as_str()
        } else {
            target.short_change_id.as_str()
        };
        format!("push-{suffix}")
    }

    fn change_id_token(target: &JjPublishTarget) -> Option<ChangeIdToken> {
        if target.short_change_id.is_empty() {
            None
        } else {
            Some(ChangeIdToken {
                text: target.short_change_id.clone(),
                prefix_len: target
                    .short_change_id_prefix_len
                    .min(target.short_change_id.len())
                    .max(1),
            })
        }
    }

    fn push_change_label(target: &JjPublishTarget) -> String {
        let id = if target.short_change_id.is_empty() {
            target.short_commit_id.as_str()
        } else {
            target.short_change_id.as_str()
        };
        if target.summary.is_empty() {
            format!("Publish change {id}")
        } else {
            format!("Publish change {id}: {}", target.summary)
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
        let remotes = self
            .cli
            .run_ignored_wc(&[
                OsString::from("git"),
                OsString::from("remote"),
                OsString::from("list"),
            ])
            .unwrap_or_default();
        let remote_bookmarks = self
            .cli
            .run_ignored_wc(&[
                OsString::from("bookmark"),
                OsString::from("list"),
                OsString::from("--all-remotes"),
                OsString::from("-T"),
                OsString::from(
                    "name ++ \"\\t\" ++ if(self.remote(), self.remote(), \"\") ++ \"\\t\" ++ normal_target.commit_id() ++ \"\\n\"",
                ),
            ])
            .unwrap_or_default();
        let operation_log = self.cli.run_ignored_wc(&[
            OsString::from("operation"),
            OsString::from("log"),
            OsString::from("--no-graph"),
            OsString::from("-n"),
            OsString::from("24"),
            OsString::from("-T"),
            OsString::from(
                "id ++ \"\\t\" ++ id.short() ++ \"\\t\" ++ user ++ \"\\t\" ++ time.start() ++ \"\\t\" ++ description.first_line() ++ \"\\n\"",
            ),
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
                backend: VcsKind::JJ,
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
        let current_target = refs[0].target.clone();
        let remote_names = parse_remote_names(&remotes);
        let remote_refs = parse_remote_bookmark_list(&remote_bookmarks, &remote_names);
        refs.extend(
            parse_bookmark_list(&bookmarks)
                .into_iter()
                .map(|mut reference| {
                    reference.active = reference.target == current_target;
                    reference.upstream = matching_remote_bookmark(&reference.name, &remote_refs)
                        .map(|remote| format!("{remote}/{}", reference.name));
                    reference
                }),
        );
        refs.extend(remote_refs);

        let snapshot = VcsSnapshot {
            location: self.location.clone(),
            reason,
            change_kind: None,
            capabilities: jj_capabilities(),
            refs,
            changes,
            operation_log: parse_operation_log(&operation_log),
            file_changes,
        };
        self.last_snapshot = Some(snapshot.clone());
        Ok(snapshot)
    }

    fn resolve_compare_request(&mut self, request: &VcsCompareRequest) -> Result<(String, String)> {
        match &request.spec {
            VcsCompareSpec::WorkingCopy => Ok(("@-".to_owned(), "@".to_owned())),
            VcsCompareSpec::Change { revision } => Ok((String::new(), revision.clone())),
            VcsCompareSpec::Range { from, to } => Ok((from.clone(), to.clone())),
            VcsCompareSpec::MergeBaseRange { base, head } => Ok((base.clone(), head.clone())),
        }
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
        #[cfg(feature = "difftastic")]
        if request.renderer == RendererKind::Difftastic {
            let output = self.compare_difftastic(request, _reporter, None)?;
            self.insert_diff_cache(operation_id, request.clone(), None, output.clone());
            return Ok(output);
        }

        let summary_args = self.diff_summary_args_for_spec(&request.spec)?;
        let summary = self.cli.run_ignored_wc(&summary_args)?;
        let summaries = compare_summaries_from_jj_diff_summary(&summary, true);
        if summaries.len() > COMPARE_SUMMARY_FILE_LIMIT {
            let mut output = CompareOutput {
                file_summaries: summaries,
                ..CompareOutput::default()
            };
            output.compact_file_summaries();
            self.insert_diff_cache(operation_id, request.clone(), None, output.clone());
            return Ok(output);
        }
        let args = self.diff_args_for_spec(&request.spec)?;
        let raw_diff = self.cli.run_ignored_wc(&args)?;
        let output = compare_output_from_raw_patch(&raw_diff)?;
        self.insert_diff_cache(operation_id, request.clone(), None, output.clone());
        Ok(output)
    }

    fn compare_stats(&mut self, request: &VcsCompareRequest) -> Result<(i32, i32)> {
        self.ensure_read_epoch()?;
        let args = self.diff_stat_args_for_spec(&request.spec)?;
        let stat = self.cli.run_ignored_wc(&args)?;
        if let Some(total) = parse_jj_diff_stat_total(&stat) {
            return Ok(total);
        }

        let raw_diff = self
            .cli
            .run_ignored_wc(&self.diff_args_for_spec(&request.spec)?)?;
        let output = compare_output_from_raw_patch(&raw_diff)?;
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

    fn compare_file_stats(
        &mut self,
        request: &VcsCompareRequest,
        files: &[CompareFileStatsTarget],
    ) -> Result<Vec<(i32, i32)>> {
        self.ensure_read_epoch()?;
        let mut stats_by_path = HashMap::with_capacity(files.len());
        for chunk in files.chunks(JJ_FILE_STATS_PATH_CHUNK) {
            let mut args = self.diff_args_for_spec(&request.spec)?;
            args.extend(chunk.iter().filter_map(|file| {
                let path = file.path();
                (!path.is_empty()).then(|| OsString::from(format!("file:{path}")))
            }));
            let raw_diff = self.cli.run_ignored_wc(&args)?;
            let output = compare_output_from_raw_patch(&raw_diff)?;
            for file in output.carbon.files {
                let stats = (
                    u32_to_i32_saturating(file.additions),
                    u32_to_i32_saturating(file.deletions),
                );
                if let Some(path) = file.new_path.as_ref().or(file.old_path.as_ref()) {
                    stats_by_path.insert(path.clone(), stats);
                }
            }
        }
        Ok(files
            .iter()
            .map(|file| {
                let path = file.path();
                stats_by_path
                    .get(path.as_ref())
                    .copied()
                    .unwrap_or_else(|| file.fallback_stats())
            })
            .collect())
    }

    fn compare_path(
        &mut self,
        request: &VcsCompareRequest,
        path: &str,
        _deferred_file: Option<&CompareFileSummary>,
    ) -> Result<CompareOutput> {
        let operation_id = self.ensure_read_epoch()?;
        if let Some(output) = self.cached_diff(operation_id.as_deref(), request, Some(path)) {
            return Ok(output);
        }
        #[cfg(feature = "difftastic")]
        if request.renderer == RendererKind::Difftastic {
            let output = self.compare_difftastic(request, None, Some(path))?;
            self.insert_diff_cache(
                operation_id,
                request.clone(),
                Some(path.to_owned()),
                output.clone(),
            );
            return Ok(output);
        }

        let mut args = self.diff_args_for_spec(&request.spec)?;
        args.push(jj_root_pathspec(path));
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

    fn file_change_diff(
        &mut self,
        change: &FileChange,
        _renderer: RendererKind,
    ) -> Result<CompareOutput> {
        self.compare_working_file(&change.path)
    }

    fn commit_diff(&mut self, _has_staged: bool) -> Result<String> {
        self.ensure_read_epoch()?;
        self.cli.run_ignored_wc(&[
            OsString::from("diff"),
            OsString::from("-r"),
            OsString::from("@"),
            OsString::from("--git"),
        ])
    }

    fn apply_file_operation(
        &mut self,
        change: &FileChange,
        operation: FileOperation,
    ) -> Result<()> {
        if operation != FileOperation::Discard {
            return Err(DiffyError::General(
                "jj does not support stage or unstage operations".to_owned(),
            ));
        }
        let mut args = vec![OsString::from("restore")];
        if let Some(old_path) = change.old_path.as_deref() {
            args.push(OsString::from(old_path));
        }
        args.push(OsString::from(change.path.as_str()));
        self.cli.run(&args)?;
        self.clear_after_write();
        Ok(())
    }

    fn create_commit(&mut self, message: &str) -> Result<()> {
        self.cli.run(&[
            OsString::from("commit"),
            OsString::from("-m"),
            OsString::from(message),
        ])?;
        self.clear_after_write();
        Ok(())
    }

    fn run_operation(&mut self, operation: &VcsOperation) -> Result<String> {
        match operation {
            VcsOperation::Jj(JjOperation::NewChange) => {
                self.cli.run(&[OsString::from("new")])?;
                self.clear_after_write();
                Ok(operation.success_message())
            }
            VcsOperation::Jj(JjOperation::NewSiblingChange) => {
                self.cli
                    .run(&[OsString::from("new"), OsString::from("@-")])?;
                self.clear_after_write();
                Ok(operation.success_message())
            }
            VcsOperation::Jj(JjOperation::DuplicateChange) => {
                self.cli
                    .run(&[OsString::from("duplicate"), OsString::from("@")])?;
                self.clear_after_write();
                Ok(operation.success_message())
            }
            VcsOperation::Jj(JjOperation::AbandonChange) => {
                self.cli
                    .run(&[OsString::from("abandon"), OsString::from("@")])?;
                self.clear_after_write();
                Ok(operation.success_message())
            }
            VcsOperation::Jj(JjOperation::SquashIntoParent) => {
                self.cli.run(&[
                    OsString::from("squash"),
                    OsString::from("--revision"),
                    OsString::from("@"),
                    OsString::from("--use-destination-message"),
                ])?;
                self.clear_after_write();
                Ok(operation.success_message())
            }
            VcsOperation::Jj(JjOperation::AbsorbIntoStack) => {
                self.cli.run(&[
                    OsString::from("absorb"),
                    OsString::from("--from"),
                    OsString::from("@"),
                ])?;
                self.clear_after_write();
                Ok(operation.success_message())
            }
            VcsOperation::Jj(JjOperation::UndoLastOperation) => {
                self.cli.run(&[
                    OsString::from("op"),
                    OsString::from("undo"),
                    OsString::from("@"),
                ])?;
                self.clear_after_write();
                Ok(operation.success_message())
            }
            VcsOperation::JjRebaseCurrentChangeOnto { destination } => {
                self.cli.run(&[
                    OsString::from("rebase"),
                    OsString::from("--branch"),
                    OsString::from("@"),
                    OsString::from("--destination"),
                    OsString::from(destination.as_str()),
                ])?;
                self.clear_after_write();
                Ok(operation.success_message())
            }
            VcsOperation::JjEditRevision { revision, .. } => {
                self.cli
                    .run(&[OsString::from("edit"), OsString::from(revision.as_str())])?;
                self.clear_after_write();
                Ok(operation.success_message())
            }
            VcsOperation::JjRestoreOperation { operation_id, .. } => {
                self.cli.run(&[
                    OsString::from("operation"),
                    OsString::from("restore"),
                    OsString::from(operation_id.as_str()),
                ])?;
                self.clear_after_write();
                Ok(operation.success_message())
            }
        }
    }

    fn fetch_remote(&mut self, remote: &str) -> Result<()> {
        self.cli.run(&[
            OsString::from("git"),
            OsString::from("fetch"),
            OsString::from("--remote"),
            OsString::from(remote),
        ])?;
        self.clear_after_write();
        Ok(())
    }

    fn push(&mut self, remote: &str, refspec: &str, _force_with_lease: bool) -> Result<()> {
        let bookmark = bookmark_from_refspec(refspec)
            .ok_or_else(|| DiffyError::General("jj push requires a bookmark refspec".to_owned()))?;
        self.cli.run(&[
            OsString::from("git"),
            OsString::from("push"),
            OsString::from("--remote"),
            OsString::from(remote),
            OsString::from("--bookmark"),
            OsString::from(bookmark),
            OsString::from("--allow-new"),
        ])?;
        self.clear_after_write();
        Ok(())
    }

    fn publish_plan(&mut self) -> Result<PublishPlan> {
        self.ensure_read_epoch()?;
        let remote = self.preferred_remote()?;
        let target = self.default_publish_target()?;
        let bookmarks = self.local_bookmarks_at(&target.commit_id)?;
        let remote_bookmarks = self
            .remote_bookmarks_at(&target.commit_id, &remote)
            .unwrap_or_default();
        let target_disabled_reason = (!remote_bookmarks.is_empty()).then(|| {
            if target.fell_back_from_empty_working_copy {
                format!(
                    "Commit or describe the working copy before publishing. {} is already on {remote}.",
                    target.revision
                )
            } else {
                format!("{} is already on {remote}.", target.revision)
            }
        });
        let mut movable_bookmarks = self
            .movable_bookmarks(&target.revision)
            .unwrap_or_default()
            .into_iter()
            .filter(|bookmark| bookmark.target != target.commit_id)
            .take(6)
            .collect::<Vec<_>>();
        let change_id_token = Self::change_id_token(&target);
        let primary = if let Some(bookmark) = bookmarks.first() {
            PublishAction {
                label: format!("Push bookmark {bookmark}"),
                description: format!(
                    "Push jj bookmark {bookmark} at {} to {remote}",
                    target.short_commit_id
                ),
                kind: PublishActionKind::PushBookmark {
                    remote: remote.clone(),
                    bookmark: bookmark.clone(),
                },
                disabled_reason: target_disabled_reason.clone(),
                change_id_token: None,
            }
        } else {
            PublishAction {
                label: Self::push_change_label(&target),
                description: format!(
                    "Publish jj revision {} to {remote}; jj will create or update a generated bookmark",
                    target.revision
                ),
                kind: PublishActionKind::PushChange {
                    remote: remote.clone(),
                    revision: target.revision.clone(),
                },
                disabled_reason: target_disabled_reason.clone(),
                change_id_token: change_id_token.clone(),
            }
        };
        let mut alternatives = Vec::new();
        if !matches!(primary.kind, PublishActionKind::PushChange { .. }) {
            alternatives.push(PublishAction {
                label: Self::push_change_label(&target),
                description: format!("Publish jj revision {} directly", target.revision),
                kind: PublishActionKind::PushChange {
                    remote: remote.clone(),
                    revision: target.revision.clone(),
                },
                disabled_reason: target_disabled_reason.clone(),
                change_id_token: change_id_token.clone(),
            });
        }
        let generated_bookmark = Self::generated_bookmark_name(&target);
        if !bookmarks
            .iter()
            .any(|bookmark| bookmark == &generated_bookmark)
        {
            alternatives.push(PublishAction {
                label: format!("Create bookmark {generated_bookmark} and push"),
                description: format!(
                    "Create jj bookmark {generated_bookmark} at {} and push it to {remote}",
                    target.short_commit_id
                ),
                kind: PublishActionKind::CreateBookmarkAndPush {
                    remote: remote.clone(),
                    bookmark: generated_bookmark,
                    revision: target.revision.clone(),
                },
                disabled_reason: target_disabled_reason.clone(),
                change_id_token: change_id_token.clone(),
            });
        }
        for bookmark in movable_bookmarks.drain(..) {
            alternatives.push(PublishAction {
                label: format!("Move bookmark {} here and push", bookmark.name),
                description: format!(
                    "Move jj bookmark {} to {} and push it to {remote}",
                    bookmark.name, target.short_commit_id
                ),
                kind: PublishActionKind::MoveBookmarkAndPush {
                    remote: remote.clone(),
                    bookmark: bookmark.name,
                    revision: target.revision.clone(),
                    allow_backwards: bookmark.allow_backwards,
                },
                disabled_reason: target_disabled_reason.clone(),
                change_id_token: None,
            });
        }
        alternatives.push(PublishAction {
            label: "Push tracked bookmarks".to_owned(),
            description: format!("Push tracked jj bookmarks in the default revset to {remote}"),
            kind: PublishActionKind::PushTracked { remote },
            disabled_reason: None,
            change_id_token: None,
        });
        Ok(PublishPlan {
            primary,
            alternatives,
        })
    }

    fn publish(&mut self, action: &PublishAction) -> Result<PublishOutcome> {
        if let Some(reason) = &action.disabled_reason {
            return Err(DiffyError::General(reason.clone()));
        }
        match &action.kind {
            PublishActionKind::PushChange { remote, revision } => {
                self.cli.run(&[
                    OsString::from("git"),
                    OsString::from("push"),
                    OsString::from("--remote"),
                    OsString::from(remote),
                    OsString::from("--change"),
                    OsString::from(revision),
                ])?;
            }
            PublishActionKind::PushBookmark { remote, bookmark } => {
                self.cli.run(&[
                    OsString::from("git"),
                    OsString::from("push"),
                    OsString::from("--remote"),
                    OsString::from(remote),
                    OsString::from("--bookmark"),
                    OsString::from(bookmark),
                    OsString::from("--allow-new"),
                ])?;
            }
            PublishActionKind::PushTracked { remote } => {
                self.cli.run(&[
                    OsString::from("git"),
                    OsString::from("push"),
                    OsString::from("--remote"),
                    OsString::from(remote),
                    OsString::from("--tracked"),
                ])?;
            }
            PublishActionKind::MoveBookmarkAndPush {
                remote,
                bookmark,
                revision,
                allow_backwards,
            } => {
                let mut move_args = vec![
                    OsString::from("bookmark"),
                    OsString::from("move"),
                    OsString::from(bookmark),
                    OsString::from("--to"),
                    OsString::from(revision),
                ];
                if *allow_backwards {
                    move_args.push(OsString::from("--allow-backwards"));
                }
                self.cli.run(&move_args)?;
                self.cli.run(&[
                    OsString::from("git"),
                    OsString::from("push"),
                    OsString::from("--remote"),
                    OsString::from(remote),
                    OsString::from("--bookmark"),
                    OsString::from(bookmark),
                    OsString::from("--allow-new"),
                ])?;
            }
            PublishActionKind::CreateBookmarkAndPush {
                remote,
                bookmark,
                revision,
            } => {
                self.cli.run(&[
                    OsString::from("bookmark"),
                    OsString::from("create"),
                    OsString::from(bookmark),
                    OsString::from("-r"),
                    OsString::from(revision),
                ])?;
                self.cli.run(&[
                    OsString::from("git"),
                    OsString::from("push"),
                    OsString::from("--remote"),
                    OsString::from(remote),
                    OsString::from("--bookmark"),
                    OsString::from(bookmark),
                    OsString::from("--allow-new"),
                ])?;
            }
            PublishActionKind::PushRef { .. } => {
                return Err(DiffyError::General(
                    "jj cannot run a Git refspec publish action".to_owned(),
                ));
            }
        }
        self.clear_after_write();
        Ok(PublishOutcome {
            label: completed_publish_label(&action.label),
        })
    }

    fn compare_working_file(&mut self, path: &str) -> Result<CompareOutput> {
        let operation_id = self.ensure_read_epoch()?;
        let request = VcsCompareRequest {
            spec: VcsCompareSpec::WorkingCopy,
            layout: crate::core::compare::LayoutMode::Unified,
            renderer: RendererKind::Builtin,
        };
        if let Some(output) = self.cached_diff(operation_id.as_deref(), &request, Some(path)) {
            return Ok(output);
        }
        let raw_diff = self.cli.run_ignored_wc(&[
            OsString::from("diff"),
            OsString::from("-r"),
            OsString::from("@"),
            OsString::from("--git"),
            jj_root_pathspec(path),
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
            jj_root_pathspec(path),
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

fn compare_summaries_from_jj_diff_summary(
    output: &str,
    stats_deferred: bool,
) -> Vec<CompareFileSummary> {
    parse_diff_summary(output)
        .into_iter()
        .map(|change| {
            CompareFileSummary::from_paths_status(
                change.old_path.as_deref(),
                Some(change.path.as_str()),
                carbon_status_from_file_change_status(change.status),
                stats_deferred,
            )
        })
        .collect()
}

fn carbon_status_from_file_change_status(status: FileChangeStatus) -> carbon::FileStatus {
    match status {
        FileChangeStatus::Added | FileChangeStatus::Untracked => carbon::FileStatus::Added,
        FileChangeStatus::Deleted => carbon::FileStatus::Deleted,
        FileChangeStatus::Renamed => carbon::FileStatus::Renamed,
        FileChangeStatus::TypeChanged => carbon::FileStatus::ModeChanged,
        FileChangeStatus::Binary => carbon::FileStatus::Binary,
        FileChangeStatus::Copied | FileChangeStatus::Modified | FileChangeStatus::Conflicted => {
            carbon::FileStatus::Modified
        }
    }
}

fn parse_jj_diff_stat_total(output: &str) -> Option<(i32, i32)> {
    let summary = output.lines().rev().find(|line| {
        line.contains(" changed") && (line.contains(" insertion") || line.contains(" deletion"))
    })?;
    Some((
        parse_stat_count_before(summary, " insertion").unwrap_or(0),
        parse_stat_count_before(summary, " deletion").unwrap_or(0),
    ))
}

fn parse_stat_count_before(line: &str, label: &str) -> Option<i32> {
    let label_start = line.find(label)?;
    let prefix = line[..label_start].trim_end();
    let digits_start = prefix
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_ascii_digit())
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0);
    prefix[digits_start..].parse().ok()
}

pub fn jj_capabilities() -> RepoCapabilities {
    RepoCapabilities {
        staging_area: false,
        branches: false,
        bookmarks: true,
        tags: false,
        remotes: true,
        pull_fast_forward: false,
        create_commit: true,
        partial_file_restore: true,
        partial_hunk_mutation: false,
        operation_log: true,
        github_pull_requests: false,
    }
}

fn parse_remote_names(output: &str) -> std::collections::BTreeSet<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|remote| !remote.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_remote_bookmark_list(
    output: &str,
    remote_names: &std::collections::BTreeSet<String>,
) -> Vec<VcsRef> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let name = fields.next()?.trim();
            let remote = fields.next()?.trim();
            let target = fields.next()?.trim();
            if name.is_empty()
                || remote.is_empty()
                || target.is_empty()
                || !remote_names.contains(remote)
            {
                return None;
            }
            Some((name, remote, target))
        })
        .map(|(name, remote, target)| VcsRef {
            name: format!("{name}@{remote}"),
            kind: RefKind::RemoteBookmark,
            target: RevisionId {
                backend: VcsKind::JJ,
                id: target.to_owned(),
            },
            active: false,
            upstream: Some(format!("{remote}/{name}")),
            ahead_behind: None,
        })
        .collect()
}

fn matching_remote_bookmark<'a>(local_name: &str, remote_refs: &'a [VcsRef]) -> Option<&'a str> {
    remote_refs.iter().find_map(|reference| {
        let upstream = reference.upstream.as_deref()?;
        let (remote, name) = upstream.split_once('/')?;
        (name == local_name).then_some(remote)
    })
}

fn parse_remote_bookmark_targets(output: &str, remote: &str, commit_id: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let name = fields.next()?.trim();
            let line_remote = fields.next()?.trim();
            let target = fields.next()?.trim();
            (line_remote == remote && target == commit_id && !name.is_empty())
                .then(|| name.to_owned())
        })
        .collect()
}

fn jj_root_pathspec(path: &str) -> OsString {
    OsString::from(format!("root:{path}"))
}

fn jj_fork_point_revset(base: &str, head: &str) -> String {
    format!("fork_point(({base})|({head}))")
}

#[cfg(feature = "difftastic")]
fn difftastic_revisions_for_spec(spec: &VcsCompareSpec) -> (String, String) {
    match spec {
        VcsCompareSpec::WorkingCopy => ("@-".to_owned(), "@".to_owned()),
        VcsCompareSpec::Change { revision } => (format!("({revision})-"), revision.clone()),
        VcsCompareSpec::Range { from, to } => (from.clone(), to.clone()),
        VcsCompareSpec::MergeBaseRange { base, head } => {
            (jj_fork_point_revset(base, head), head.clone())
        }
    }
}

#[cfg(feature = "difftastic")]
fn difftastic_status_label(status: FileChangeStatus) -> &'static str {
    match status {
        FileChangeStatus::Added | FileChangeStatus::Untracked => "A",
        FileChangeStatus::Deleted => "D",
        FileChangeStatus::Renamed => "R",
        _ => "M",
    }
}

#[cfg(feature = "difftastic")]
fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(1024).any(|byte| *byte == 0)
}

fn parse_movable_bookmark_line(line: &str) -> Option<MovableBookmark> {
    let mut fields = line.splitn(3, '\t');
    let name = fields.next()?.trim();
    let target = fields.next()?.trim();
    let allow_backwards = fields.next()?.trim() == "true";
    if name.is_empty() || target.is_empty() {
        return None;
    }
    Some(MovableBookmark {
        name: name.to_owned(),
        target: target.to_owned(),
        allow_backwards,
    })
}

fn bookmark_priority(name: &str) -> usize {
    match name {
        "main" | "master" => 0,
        name if !name.contains('/') => 1,
        _ => 2,
    }
}

fn bookmark_from_refspec(refspec: &str) -> Option<String> {
    let source = refspec
        .split_once(':')
        .map_or(refspec, |(source, _)| source);
    source
        .strip_prefix("refs/heads/")
        .or_else(|| source.strip_prefix("refs/bookmarks/"))
        .or_else(|| (!source.is_empty()).then_some(source))
        .map(str::to_owned)
}

fn completed_publish_label(label: &str) -> String {
    label
        .strip_prefix("Publish ")
        .map(|suffix| format!("Published {suffix}"))
        .or_else(|| {
            label
                .strip_prefix("Push ")
                .map(|suffix| format!("Pushed {suffix}"))
        })
        .unwrap_or_else(|| label.to_owned())
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

    use super::{
        JjBackend, compare_summaries_from_jj_diff_summary, jj_fork_point_revset,
        parse_jj_diff_stat_total,
    };
    use crate::core::compare::{CompareFileStatsTarget, LayoutMode, RendererKind};
    use crate::core::vcs::backend::VcsBackend;
    use crate::core::vcs::model::{
        ChangeBucket, FileChangeStatus, FileOperation, PublishActionKind, VcsCompareRequest,
        VcsCompareSpec, VcsKind,
    };
    use crate::events::RepositorySyncReason;

    #[test]
    fn jj_merge_base_revset_uses_fork_point() {
        assert_eq!(
            jj_fork_point_revset("main", "feature"),
            "fork_point((main)|(feature))"
        );
    }

    #[cfg(feature = "difftastic")]
    #[test]
    fn jj_difftastic_revisions_follow_compare_spec() {
        assert_eq!(
            super::difftastic_revisions_for_spec(&VcsCompareSpec::MergeBaseRange {
                base: "main".to_owned(),
                head: "feature".to_owned(),
            }),
            (
                "fork_point((main)|(feature))".to_owned(),
                "feature".to_owned()
            )
        );
    }

    #[test]
    fn jj_diff_summary_builds_compare_summaries() {
        let summaries = compare_summaries_from_jj_diff_summary(
            "A README.md\nM src/lib.rs\nR src/{old => new}.rs\n",
            true,
        );

        assert_eq!(summaries.len(), 3);
        assert_eq!(summaries[0].path(), "README.md");
        assert_eq!(summaries[0].status, carbon::FileStatus::Added);
        assert!(summaries[0].stats_deferred);
        assert_eq!(summaries[1].path(), "src/lib.rs");
        assert_eq!(summaries[1].status, carbon::FileStatus::Modified);
        assert_eq!(summaries[2].paths.old_path().as_deref(), Some("src/old.rs"));
        assert_eq!(summaries[2].paths.new_path().as_deref(), Some("src/new.rs"));
        assert_eq!(summaries[2].status, carbon::FileStatus::Renamed);
    }

    #[test]
    fn jj_diff_stat_summary_parses_total_stats() {
        assert_eq!(
            parse_jj_diff_stat_total(
                "src/lib.rs | 3 ++-\nREADME.md | 1 -\n2 files changed, 2 insertions(+), 2 deletions(-)\n",
            ),
            Some((2, 2))
        );
        assert_eq!(
            parse_jj_diff_stat_total("README.md | 1 +\n1 file changed, 1 insertion(+)\n"),
            Some((1, 0))
        );
    }

    #[test]
    fn jj_backend_snapshots_and_diffs_working_copy() {
        let Some(repo_dir) = init_jj_repo() else {
            return;
        };
        fs::write(repo_dir.path().join("README.md"), "hello\n").unwrap();

        let backend = JjBackend;
        let location = backend.detect(repo_dir.path()).unwrap().unwrap();
        assert_eq!(location.kind, VcsKind::JJ);

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

        let request = VcsCompareRequest {
            spec: VcsCompareSpec::Range {
                from: "@-".to_owned(),
                to: "@".to_owned(),
            },
            layout: LayoutMode::Unified,
            renderer: RendererKind::Builtin,
        };
        let (additions, deletions) = repo.compare_stats(&request).unwrap();
        assert_eq!((additions, deletions), (1, 0));
        let file_stats = repo
            .compare_file_stats(
                &request,
                &[CompareFileStatsTarget::from_file(&output.carbon.files[0])],
            )
            .unwrap();
        assert_eq!(file_stats, vec![(1, 0)]);

        let readme_change = snapshot
            .file_changes
            .iter()
            .find(|file| file.path == "README.md")
            .unwrap()
            .clone();
        repo.apply_file_operation(&readme_change, FileOperation::Discard)
            .unwrap();
        let snapshot = repo
            .snapshot(RepositorySyncReason::Rescan, None)
            .expect("jj snapshot after restore");
        assert!(
            !snapshot
                .file_changes
                .iter()
                .any(|file| file.path == "README.md")
        );

        fs::write(repo_dir.path().join("COMMIT.md"), "committed\n").unwrap();
        repo.create_commit("add commit from diffy").unwrap();
        let description = Command::new("jj")
            .arg("--no-pager")
            .arg("--color=never")
            .arg("--quiet")
            .arg("log")
            .arg("--no-graph")
            .arg("-r")
            .arg("@-")
            .arg("-T")
            .arg("description.first_line()")
            .current_dir(repo_dir.path())
            .output()
            .unwrap();
        assert!(description.status.success());
        assert_eq!(
            String::from_utf8(description.stdout).unwrap(),
            "add commit from diffy"
        );

        let request = VcsCompareRequest {
            spec: VcsCompareSpec::Change {
                revision: "@".to_owned(),
            },
            layout: LayoutMode::Unified,
            renderer: RendererKind::Builtin,
        };
        assert_eq!(
            request.spec,
            VcsCompareSpec::Change {
                revision: "@".to_owned()
            }
        );
    }

    #[test]
    fn jj_publish_plan_defaults_to_change_then_bookmark() {
        let Some(repo_dir) = init_jj_repo() else {
            return;
        };
        let remote_dir = TempDir::new().unwrap();
        let status = Command::new("jj")
            .arg("--quiet")
            .arg("git")
            .arg("remote")
            .arg("add")
            .arg("origin")
            .arg(remote_dir.path())
            .current_dir(repo_dir.path())
            .status()
            .unwrap();
        assert!(status.success());

        let backend = JjBackend;
        let location = backend.detect(repo_dir.path()).unwrap().unwrap();
        let mut repo = backend.open(location).unwrap();
        fs::write(repo_dir.path().join("PUBLISH.md"), "publish me\n").unwrap();
        repo.create_commit("publish me").unwrap();

        let plan = repo.publish_plan().unwrap();
        match &plan.primary.kind {
            PublishActionKind::PushChange { remote, revision } => {
                assert_eq!(remote, "origin");
                assert_eq!(revision, "@-");
            }
            other => panic!("expected jj change publish, got {other:?}"),
        }
        assert!(plan.alternatives.iter().any(|action| {
            matches!(
                action.kind,
                PublishActionKind::CreateBookmarkAndPush { ref remote, .. } if remote == "origin"
            )
        }));

        let status = Command::new("jj")
            .arg("--quiet")
            .arg("bookmark")
            .arg("create")
            .arg("main")
            .arg("-r")
            .arg("@-")
            .current_dir(repo_dir.path())
            .status()
            .unwrap();
        assert!(status.success());

        let plan = repo.publish_plan().unwrap();
        match &plan.primary.kind {
            PublishActionKind::PushBookmark { remote, bookmark } => {
                assert_eq!(remote, "origin");
                assert_eq!(bookmark, "main");
            }
            other => panic!("expected jj bookmark publish, got {other:?}"),
        }
        assert!(plan.alternatives.iter().any(|action| {
            matches!(
                action.kind,
                PublishActionKind::PushChange { ref remote, ref revision }
                    if remote == "origin" && revision == "@-"
            )
        }));
    }

    #[test]
    fn jj_publish_plan_disables_parent_target_when_working_copy_is_uncommitted_and_parent_pushed() {
        let Some(repo_dir) = init_jj_repo() else {
            return;
        };
        let remote_dir = TempDir::new().unwrap();
        let status = Command::new("git")
            .arg("init")
            .arg("--bare")
            .arg(remote_dir.path())
            .status()
            .unwrap();
        assert!(status.success());
        let status = Command::new("jj")
            .arg("--quiet")
            .arg("git")
            .arg("remote")
            .arg("add")
            .arg("origin")
            .arg(remote_dir.path())
            .current_dir(repo_dir.path())
            .status()
            .unwrap();
        assert!(status.success());

        let backend = JjBackend;
        let location = backend.detect(repo_dir.path()).unwrap().unwrap();
        let mut repo = backend.open(location).unwrap();
        fs::write(repo_dir.path().join("PUBLISHED.md"), "already published\n").unwrap();
        repo.create_commit("already published").unwrap();

        let status = Command::new("jj")
            .arg("--quiet")
            .arg("git")
            .arg("push")
            .arg("--remote")
            .arg("origin")
            .arg("--change")
            .arg("@-")
            .arg("--allow-new")
            .current_dir(repo_dir.path())
            .status()
            .unwrap();
        assert!(status.success());

        fs::write(
            repo_dir.path().join("UNCOMMITTED.md"),
            "not described yet\n",
        )
        .unwrap();
        let plan = repo.publish_plan().unwrap();
        assert!(
            plan.primary.disabled_reason.is_some(),
            "primary should not publish @- when @- is already on origin"
        );
        assert!(plan.alternatives.iter().any(|action| {
            matches!(action.kind, PublishActionKind::CreateBookmarkAndPush { .. })
                && action.disabled_reason.is_some()
        }));
        assert!(plan.alternatives.iter().any(|action| {
            matches!(action.kind, PublishActionKind::PushTracked { .. })
                && action.disabled_reason.is_none()
        }));
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
        for (key, value) in [("user.name", "Diffy"), ("user.email", "diffy@example.com")] {
            let status = Command::new("jj")
                .arg("--quiet")
                .arg("config")
                .arg("set")
                .arg("--repo")
                .arg(key)
                .arg(value)
                .current_dir(repo_dir.path())
                .status()
                .unwrap();
            assert!(status.success());
        }
        Some(repo_dir)
    }
}
