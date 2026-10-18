use std::borrow::Cow;
use std::sync::Arc;

use diffy_diff::{
    ChangeSummary, ContentChange, DiffDocument, DiffStats, FileMeta, FileStatus, ObjectId,
};

use crate::core::compare::stats::{CompareFilePaths, compact_paths_by};
use crate::core::error::{DiffyError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareEntry {
    pub paths: CompareFilePaths,
    pub old_id: Option<ObjectId>,
    pub new_id: Option<ObjectId>,
    pub old_mode: Option<Arc<str>>,
    pub new_mode: Option<Arc<str>>,
    pub summary: ChangeSummary,
    pub stats: DiffStats,
}

impl CompareEntry {
    pub fn deferred(old_path: Option<&str>, new_path: Option<&str>, status: FileStatus) -> Self {
        Self {
            paths: CompareFilePaths::from_paths(old_path, new_path),
            old_id: None,
            new_id: None,
            old_mode: None,
            new_mode: None,
            summary: ChangeSummary {
                status,
                ..ChangeSummary::default()
            },
            stats: DiffStats::Pending,
        }
    }

    pub fn with_ids(mut self, old_id: Option<ObjectId>, new_id: Option<ObjectId>) -> Self {
        self.old_id = old_id;
        self.new_id = new_id;
        if let (Some(old), Some(new)) = (&self.old_id, &self.new_id)
            && self.summary.content == ContentChange::Unknown
        {
            self.summary.content = if old.matches(new) {
                ContentChange::Unchanged
            } else {
                ContentChange::Changed
            };
        }
        self
    }

    pub fn with_modes(mut self, old_mode: Option<Arc<str>>, new_mode: Option<Arc<str>>) -> Self {
        self.summary.mode_changed =
            matches!((&old_mode, &new_mode), (Some(old), Some(new)) if old != new);
        self.old_mode = old_mode;
        self.new_mode = new_mode;
        self
    }

    pub fn with_binary(mut self, binary: bool) -> Self {
        self.summary.binary = binary;
        if binary {
            self.stats = DiffStats::Known {
                additions: 0,
                deletions: 0,
            };
        }
        self
    }

    pub fn from_document(doc: &DiffDocument, file: u32) -> Self {
        let meta = &doc.files().meta[file as usize];
        Self {
            paths: CompareFilePaths::from_paths(meta.old_path.as_deref(), meta.new_path.as_deref()),
            old_id: meta.old_id.clone(),
            new_id: meta.new_id.clone(),
            old_mode: meta.old_mode.clone(),
            new_mode: meta.new_mode.clone(),
            summary: doc.summary(file),
            stats: doc.stats(file),
        }
    }

    pub fn file_meta(&self) -> FileMeta {
        FileMeta {
            old_path: self.paths.old_path().map(|path| Arc::from(path.as_ref())),
            new_path: self.paths.new_path().map(|path| Arc::from(path.as_ref())),
            status: self.summary.status,
            binary: self.summary.binary,
            old_mode: self.old_mode.clone(),
            new_mode: self.new_mode.clone(),
            old_id: self.old_id.clone(),
            new_id: self.new_id.clone(),
            content: self.summary.content,
        }
    }

    pub fn status(&self) -> FileStatus {
        self.summary.status
    }

    pub fn is_binary(&self) -> bool {
        self.summary.binary
    }

    pub fn path(&self) -> Cow<'_, str> {
        self.paths.display_path()
    }

    pub fn push_path_to(&self, out: &mut String) {
        self.paths.push_display_path_to(out);
    }

    pub fn path_chars(&self) -> usize {
        self.paths.display_path_chars()
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompareOutput {
    pub entries: Vec<CompareEntry>,
    pub documents: Vec<Option<Arc<DiffDocument>>>,
    pub used_fallback: bool,
    pub fallback_message: String,
}

impl CompareOutput {
    pub fn deferred(entries: Vec<CompareEntry>) -> Self {
        let mut output = Self {
            entries,
            ..Self::default()
        };
        output.compact_paths();
        output
    }

    pub fn from_document(doc: &DiffDocument) -> Self {
        let mut output = Self::default();
        for file in 0..doc.file_count() {
            output.push_document(Arc::new(doc.extract_file(file)));
        }
        output
    }

    pub fn from_patch(raw: &str) -> Result<Self> {
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        let doc =
            diffy_diff::parse_unified(raw).map_err(|error| DiffyError::Parse(error.to_string()))?;
        Ok(Self::from_document(&doc))
    }

    pub fn push_document(&mut self, doc: Arc<DiffDocument>) {
        debug_assert_eq!(doc.file_count(), 1);
        self.documents.resize(self.entries.len(), None);
        self.entries.push(CompareEntry::from_document(&doc, 0));
        self.documents.push(Some(doc));
    }

    pub fn push_entry(&mut self, entry: CompareEntry) {
        self.entries.push(entry);
    }

    pub fn file_count(&self) -> usize {
        self.entries.len()
    }

    pub fn entry(&self, index: usize) -> Option<&CompareEntry> {
        self.entries.get(index)
    }

    pub fn document(&self, index: usize) -> Option<&Arc<DiffDocument>> {
        self.documents.get(index)?.as_ref()
    }

    pub fn take_document(&mut self, index: usize) -> Option<Arc<DiffDocument>> {
        self.documents.get_mut(index)?.take()
    }

    pub fn set_stats(&mut self, index: usize, stats: DiffStats) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.stats = stats;
        }
    }

    pub fn for_each_pending_stats(
        &self,
        limit: usize,
        mut visit: impl FnMut(usize, &CompareEntry),
    ) {
        for (index, entry) in self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.stats.is_pending())
            .take(limit)
        {
            visit(index, entry);
        }
    }

    pub fn totals(&self) -> DiffStats {
        let mut additions = 0u32;
        let mut deletions = 0u32;
        for entry in &self.entries {
            let Some((add, del)) = entry.stats.known() else {
                return DiffStats::Pending;
            };
            additions = additions.saturating_add(add);
            deletions = deletions.saturating_add(del);
        }
        DiffStats::Known {
            additions,
            deletions,
        }
    }

    pub fn max_path_chars(&self) -> usize {
        self.entries
            .iter()
            .map(CompareEntry::path_chars)
            .max()
            .unwrap_or(0)
    }

    pub fn compact_paths(&mut self) {
        compact_paths_by(&mut self.entries, |entry| &mut entry.paths);
        self.entries.shrink_to_fit();
    }
}

#[cfg(test)]
mod tests {
    use diffy_diff::{ContentChange, DiffStats, FileStatus, ObjectId};

    use super::{CompareEntry, CompareOutput};

    const PATCH: &str = "diff --git a/src/a.rs b/src/b.rs\n\
                         similarity index 90%\n\
                         rename from src/a.rs\n\
                         rename to src/b.rs\n\
                         index 1111111..2222222 100644\n\
                         --- a/src/a.rs\n\
                         +++ b/src/b.rs\n\
                         @@ -1 +1 @@\n\
                         -old\n\
                         +new\n\
                         diff --git a/run.sh b/run.sh\n\
                         old mode 100644\n\
                         new mode 100755\n";

    #[test]
    fn patches_split_into_one_document_per_entry() {
        let output = CompareOutput::from_patch(PATCH).unwrap();
        assert_eq!(output.file_count(), 2);
        let renamed = output.entry(0).unwrap();
        assert_eq!(renamed.path(), "src/b.rs");
        assert_eq!(renamed.paths.old_path().as_deref(), Some("src/a.rs"));
        assert_eq!(renamed.summary.label(), "renamed, modified");
        assert_eq!(renamed.stats.known(), Some((1, 1)));
        assert_eq!(
            renamed.old_id.as_ref().map(ObjectId::as_str),
            Some("1111111")
        );
        assert_eq!(output.document(0).unwrap().file_count(), 1);
        let mode = output.entry(1).unwrap();
        assert!(mode.summary.is_mode_only());
        assert_eq!(
            output.totals(),
            DiffStats::Known {
                additions: 1,
                deletions: 1
            }
        );
        assert!(CompareOutput::from_patch("").unwrap().entries.is_empty());
    }

    #[test]
    fn deferred_entries_stay_pending_and_compact() {
        let mut output = CompareOutput::deferred(vec![
            CompareEntry::deferred(None, Some("docs/new.md"), FileStatus::Added),
            CompareEntry::deferred(Some("src/x.rs"), Some("src/y.rs"), FileStatus::Renamed)
                .with_ids(ObjectId::parse("abc1234"), ObjectId::parse("abc1234")),
        ]);
        assert_eq!(output.entry(0).unwrap().path(), "docs/new.md");
        assert_eq!(output.totals(), DiffStats::Pending);
        let renamed = output.entry(1).unwrap();
        assert_eq!(renamed.summary.content, ContentChange::Unchanged);
        assert!(renamed.summary.is_rename_only());
        let mut pending = Vec::new();
        output.for_each_pending_stats(10, |index, _| pending.push(index));
        assert_eq!(pending, [0, 1]);
        output.set_stats(
            0,
            DiffStats::Known {
                additions: 3,
                deletions: 0,
            },
        );
        output.set_stats(
            1,
            DiffStats::Known {
                additions: 0,
                deletions: 0,
            },
        );
        assert_eq!(output.totals().known(), Some((3, 0)));
        assert!(output.document(0).is_none());
    }
}
