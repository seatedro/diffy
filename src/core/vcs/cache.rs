//! Shared epoch-keyed read caches for VCS backend services.
//!
//! Backends that re-run expensive reads (whole-compare diffs, per-path
//! diffs, diff stats, file text at a revision) can memoize them here, keyed on an opaque
//! read epoch. The epoch identifies the repository state the read was
//! produced under — for jj this is the operation id — so cache hits require
//! an exact epoch match and a `None` epoch only matches entries inserted
//! with a `None` epoch. Callers are responsible for calling [`clear`] after
//! writes or whenever the epoch changes; the caches never invalidate
//! entries on their own. Eviction is FIFO with small fixed caps so memory
//! stays bounded without tracking recency.
//!
//! [`clear`]: VcsReadCache::clear

use carbon::TextStore;

use crate::core::compare::CompareOutput;
use crate::core::vcs::model::{RevisionId, VcsCompareRequest};

const MAX_DIFF_CACHE_ENTRIES: usize = 8;
const MAX_FILE_TEXT_CACHE_ENTRIES: usize = 16;
const MAX_STATS_CACHE_ENTRIES: usize = 16;

#[derive(Clone)]
struct DiffCacheEntry {
    epoch: Option<String>,
    request: VcsCompareRequest,
    path: Option<String>,
    output: CompareOutput,
}

#[derive(Clone)]
struct StatsCacheEntry {
    epoch: Option<String>,
    request: VcsCompareRequest,
    stats: (i32, i32),
}

#[derive(Clone)]
struct FileTextCacheEntry {
    epoch: Option<String>,
    revision: RevisionId,
    path: String,
    text: TextStore,
}

/// Bounded diff, diff-stat, and file-text caches for a VCS repository
/// service.
#[derive(Default)]
pub struct VcsReadCache {
    diffs: Vec<DiffCacheEntry>,
    file_texts: Vec<FileTextCacheEntry>,
    stats: Vec<StatsCacheEntry>,
}

impl VcsReadCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cached_diff(
        &self,
        epoch: Option<&str>,
        request: &VcsCompareRequest,
        path: Option<&str>,
    ) -> Option<CompareOutput> {
        self.diffs
            .iter()
            .find(|entry| {
                entry.epoch.as_deref() == epoch
                    && entry.request == *request
                    && entry.path.as_deref() == path
            })
            .map(|entry| entry.output.clone())
    }

    pub fn insert_diff(
        &mut self,
        epoch: Option<String>,
        request: VcsCompareRequest,
        path: Option<String>,
        output: CompareOutput,
    ) {
        if self.diffs.len() >= MAX_DIFF_CACHE_ENTRIES {
            self.diffs.remove(0);
        }
        self.diffs.push(DiffCacheEntry {
            epoch,
            request,
            path,
            output,
        });
    }

    pub fn cached_stats(
        &self,
        epoch: Option<&str>,
        request: &VcsCompareRequest,
    ) -> Option<(i32, i32)> {
        self.stats
            .iter()
            .find(|entry| entry.epoch.as_deref() == epoch && entry.request == *request)
            .map(|entry| entry.stats)
    }

    pub fn insert_stats(
        &mut self,
        epoch: Option<String>,
        request: VcsCompareRequest,
        stats: (i32, i32),
    ) {
        if self.stats.len() >= MAX_STATS_CACHE_ENTRIES {
            self.stats.remove(0);
        }
        self.stats.push(StatsCacheEntry {
            epoch,
            request,
            stats,
        });
    }

    pub fn cached_file_text(
        &self,
        epoch: Option<&str>,
        revision: &RevisionId,
        path: &str,
    ) -> Option<TextStore> {
        self.file_texts
            .iter()
            .find(|entry| {
                entry.epoch.as_deref() == epoch && entry.revision == *revision && entry.path == path
            })
            .map(|entry| entry.text.clone())
    }

    pub fn insert_file_text(
        &mut self,
        epoch: Option<String>,
        revision: RevisionId,
        path: String,
        text: TextStore,
    ) {
        if self.file_texts.len() >= MAX_FILE_TEXT_CACHE_ENTRIES {
            self.file_texts.remove(0);
        }
        self.file_texts.push(FileTextCacheEntry {
            epoch,
            revision,
            path,
            text,
        });
    }

    pub fn clear(&mut self) {
        self.diffs.clear();
        self.file_texts.clear();
        self.stats.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::compare::{LayoutMode, RendererKind};
    use crate::core::vcs::model::{VcsCompareSpec, VcsKind};

    fn request(revision: &str) -> VcsCompareRequest {
        VcsCompareRequest {
            spec: VcsCompareSpec::Change {
                revision: revision.to_owned(),
            },
            layout: LayoutMode::Unified,
            renderer: RendererKind::Builtin,
        }
    }

    fn revision(id: &str) -> RevisionId {
        RevisionId {
            backend: VcsKind::JJ,
            id: id.to_owned(),
        }
    }

    #[test]
    fn diff_hits_require_matching_epoch_request_and_path() {
        let mut cache = VcsReadCache::new();
        cache.insert_diff(
            Some("op-1".to_owned()),
            request("abc"),
            Some("src/lib.rs".to_owned()),
            CompareOutput::default(),
        );

        assert!(
            cache
                .cached_diff(Some("op-1"), &request("abc"), Some("src/lib.rs"))
                .is_some()
        );
        assert!(
            cache
                .cached_diff(Some("op-2"), &request("abc"), Some("src/lib.rs"))
                .is_none()
        );
        assert!(
            cache
                .cached_diff(None, &request("abc"), Some("src/lib.rs"))
                .is_none()
        );
        assert!(
            cache
                .cached_diff(Some("op-1"), &request("def"), Some("src/lib.rs"))
                .is_none()
        );
        assert!(
            cache
                .cached_diff(Some("op-1"), &request("abc"), None)
                .is_none()
        );
    }

    #[test]
    fn file_text_hits_require_matching_epoch_revision_and_path() {
        let mut cache = VcsReadCache::new();
        cache.insert_file_text(
            Some("op-1".to_owned()),
            revision("abc"),
            "src/lib.rs".to_owned(),
            TextStore::from_text("hello\n".to_owned()),
        );

        assert!(
            cache
                .cached_file_text(Some("op-1"), &revision("abc"), "src/lib.rs")
                .is_some()
        );
        assert!(
            cache
                .cached_file_text(Some("op-2"), &revision("abc"), "src/lib.rs")
                .is_none()
        );
        assert!(
            cache
                .cached_file_text(Some("op-1"), &revision("def"), "src/lib.rs")
                .is_none()
        );
    }

    #[test]
    fn diff_cache_evicts_oldest_entry_at_capacity() {
        let mut cache = VcsReadCache::new();
        for index in 0..=MAX_DIFF_CACHE_ENTRIES {
            cache.insert_diff(
                Some("op-1".to_owned()),
                request(&format!("rev-{index}")),
                None,
                CompareOutput::default(),
            );
        }

        assert!(
            cache
                .cached_diff(Some("op-1"), &request("rev-0"), None)
                .is_none()
        );
        assert!(
            cache
                .cached_diff(Some("op-1"), &request("rev-1"), None)
                .is_some()
        );
        assert_eq!(cache.diffs.len(), MAX_DIFF_CACHE_ENTRIES);
    }

    #[test]
    fn stats_hits_require_matching_epoch_and_request() {
        let mut cache = VcsReadCache::new();
        cache.insert_stats(Some("op-1".to_owned()), request("abc"), (3, 1));

        assert_eq!(
            cache.cached_stats(Some("op-1"), &request("abc")),
            Some((3, 1))
        );
        assert!(cache.cached_stats(Some("op-2"), &request("abc")).is_none());
        assert!(cache.cached_stats(None, &request("abc")).is_none());
        assert!(cache.cached_stats(Some("op-1"), &request("def")).is_none());
    }

    #[test]
    fn clear_drops_all_caches() {
        let mut cache = VcsReadCache::new();
        cache.insert_diff(None, request("abc"), None, CompareOutput::default());
        cache.insert_file_text(
            None,
            revision("abc"),
            "src/lib.rs".to_owned(),
            TextStore::from_text(String::new()),
        );
        cache.insert_stats(None, request("abc"), (1, 2));
        cache.clear();
        assert!(cache.cached_diff(None, &request("abc"), None).is_none());
        assert!(
            cache
                .cached_file_text(None, &revision("abc"), "src/lib.rs")
                .is_none()
        );
        assert!(cache.cached_stats(None, &request("abc")).is_none());
    }
}
