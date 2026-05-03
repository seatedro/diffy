use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use carbon::{FileDiff, FileStatus};

pub const COMPARE_SUMMARY_FILE_LIMIT: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareFileSummary {
    pub paths: CompareFilePaths,
    pub old_oid: Option<Arc<str>>,
    pub new_oid: Option<Arc<str>>,
    pub status: FileStatus,
    pub is_binary: bool,
    pub is_partial: bool,
    pub additions: u32,
    pub deletions: u32,
    pub stats_deferred: bool,
}

#[derive(Debug)]
pub struct ComparePathStore {
    dirs: Box<str>,
    dir_offsets: Box<[u32]>,
    basenames: Box<str>,
    basename_offsets: Box<[u32]>,
    entries: Box<[ComparePathEntry]>,
}

impl ComparePathStore {
    fn dir(&self, id: u32) -> &str {
        let Some(entry) = self.entries.get(id as usize).copied() else {
            return "";
        };
        let Some(start) = self.dir_offsets.get(entry.dir_id as usize).copied() else {
            return "";
        };
        let Some(end) = self.dir_offsets.get(entry.dir_id as usize + 1).copied() else {
            return "";
        };
        self.dirs.get(start as usize..end as usize).unwrap_or("")
    }

    fn basename(&self, id: u32) -> &str {
        let Some(entry) = self.entries.get(id as usize).copied() else {
            return "";
        };
        let Some(start) = self
            .basename_offsets
            .get(entry.basename_id as usize)
            .copied()
        else {
            return "";
        };
        let Some(end) = self
            .basename_offsets
            .get(entry.basename_id as usize + 1)
            .copied()
        else {
            return "";
        };
        self.basenames
            .get(start as usize..end as usize)
            .unwrap_or("")
    }

    fn path(&self, id: u32) -> Cow<'_, str> {
        let dir = self.dir(id);
        let basename = self.basename(id);
        if dir.is_empty() {
            Cow::Borrowed(basename)
        } else {
            Cow::Owned(format!("{dir}/{basename}"))
        }
    }

    fn path_chars(&self, id: u32) -> usize {
        let dir = self.dir(id);
        let basename = self.basename(id);
        if dir.is_empty() {
            basename.chars().count()
        } else {
            dir.chars().count() + 1 + basename.chars().count()
        }
    }

    fn write_path(&self, id: u32, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dir = self.dir(id);
        if !dir.is_empty() {
            f.write_str(dir)?;
            f.write_str("/")?;
        }
        f.write_str(self.basename(id))
    }

    fn push_path_to(&self, id: u32, out: &mut String) {
        let dir = self.dir(id);
        if !dir.is_empty() {
            out.push_str(dir);
            out.push('/');
        }
        out.push_str(self.basename(id));
    }
}

#[derive(Debug, Clone, Copy)]
struct ComparePathEntry {
    dir_id: u32,
    basename_id: u32,
}

#[derive(Debug, Clone)]
enum ComparePathInner {
    Owned(Arc<str>),
    Compact {
        store: Arc<ComparePathStore>,
        id: u32,
    },
}

impl ComparePathInner {
    fn path(&self) -> Cow<'_, str> {
        match self {
            Self::Owned(path) => Cow::Borrowed(path),
            Self::Compact { store, id } => store.path(*id),
        }
    }

    fn path_chars(&self) -> usize {
        match self {
            Self::Owned(path) => path.chars().count(),
            Self::Compact { store, id } => store.path_chars(*id),
        }
    }

    fn write_path(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Owned(path) => f.write_str(path),
            Self::Compact { store, id } => store.write_path(*id, f),
        }
    }

    fn push_path_to(&self, out: &mut String) {
        match self {
            Self::Owned(path) => out.push_str(path),
            Self::Compact { store, id } => store.push_path_to(*id, out),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ComparePath {
    inner: ComparePathInner,
}

impl ComparePath {
    pub fn new(path: &str) -> Self {
        Self {
            inner: ComparePathInner::Owned(Arc::from(path)),
        }
    }

    pub fn path(&self) -> Cow<'_, str> {
        self.inner.path()
    }

    pub fn path_chars(&self) -> usize {
        self.inner.path_chars()
    }

    pub fn push_path_to(&self, out: &mut String) {
        self.inner.push_path_to(out);
    }

    fn compact(store: Arc<ComparePathStore>, id: u32) -> Self {
        Self {
            inner: ComparePathInner::Compact { store, id },
        }
    }
}

impl Default for ComparePath {
    fn default() -> Self {
        Self::new("")
    }
}

impl From<&str> for ComparePath {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ComparePath {
    fn from(value: String) -> Self {
        Self {
            inner: ComparePathInner::Owned(Arc::from(value)),
        }
    }
}

impl From<Arc<str>> for ComparePath {
    fn from(value: Arc<str>) -> Self {
        Self {
            inner: ComparePathInner::Owned(value),
        }
    }
}

impl fmt::Display for ComparePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.write_path(f)
    }
}

impl PartialEq for ComparePath {
    fn eq(&self, other: &Self) -> bool {
        self.path() == other.path()
    }
}

impl Eq for ComparePath {}

impl PartialEq<str> for ComparePath {
    fn eq(&self, other: &str) -> bool {
        self.path() == other
    }
}

impl PartialEq<&str> for ComparePath {
    fn eq(&self, other: &&str) -> bool {
        self.path() == *other
    }
}

impl PartialEq<String> for ComparePath {
    fn eq(&self, other: &String) -> bool {
        self.path() == other.as_str()
    }
}

impl Hash for ComparePath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self.path() {
            Cow::Borrowed(path) => path.hash(state),
            Cow::Owned(path) => path.hash(state),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CompareFilePaths {
    #[default]
    Unknown,
    Same(ComparePath),
    Added(ComparePath),
    Deleted(ComparePath),
    Renamed(Box<CompareRenamedPaths>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareRenamedPaths {
    pub old: ComparePath,
    pub new: ComparePath,
}

impl CompareRenamedPaths {
    fn new(old: ComparePath, new: ComparePath) -> Self {
        Self { old, new }
    }

    fn boxed(old: ComparePath, new: ComparePath) -> Box<Self> {
        Box::new(Self::new(old, new))
    }
}

impl CompareFilePaths {
    pub fn from_paths(old_path: Option<&str>, new_path: Option<&str>) -> Self {
        match (old_path, new_path) {
            (Some(old), Some(new)) if old == new => Self::Same(ComparePath::from(new)),
            (Some(old), Some(new)) => Self::Renamed(CompareRenamedPaths::boxed(
                ComparePath::from(old),
                ComparePath::from(new),
            )),
            (None, Some(new)) => Self::Added(ComparePath::from(new)),
            (Some(old), None) => Self::Deleted(ComparePath::from(old)),
            (None, None) => Self::Unknown,
        }
    }

    pub fn old_path(&self) -> Option<Cow<'_, str>> {
        match self {
            Self::Unknown | Self::Added(_) => None,
            Self::Same(path) | Self::Deleted(path) => Some(path.path()),
            Self::Renamed(paths) => Some(paths.old.path()),
        }
    }

    pub fn new_path(&self) -> Option<Cow<'_, str>> {
        match self {
            Self::Unknown | Self::Deleted(_) => None,
            Self::Same(path) | Self::Added(path) => Some(path.path()),
            Self::Renamed(paths) => Some(paths.new.path()),
        }
    }

    pub fn display_path(&self) -> Cow<'_, str> {
        match self {
            Self::Unknown => Cow::Borrowed(""),
            Self::Same(path) | Self::Added(path) | Self::Deleted(path) => path.path(),
            Self::Renamed(paths) => paths.new.path(),
        }
    }

    pub fn push_display_path_to(&self, out: &mut String) {
        match self {
            Self::Unknown => {}
            Self::Same(path) | Self::Added(path) | Self::Deleted(path) => path.push_path_to(out),
            Self::Renamed(paths) => paths.new.push_path_to(out),
        }
    }

    pub fn display_path_ref(&self) -> ComparePath {
        match self {
            Self::Unknown => ComparePath::default(),
            Self::Same(path) | Self::Added(path) | Self::Deleted(path) => path.clone(),
            Self::Renamed(paths) => paths.new.clone(),
        }
    }

    pub fn display_path_chars(&self) -> usize {
        match self {
            Self::Unknown => 0,
            Self::Same(path) | Self::Added(path) | Self::Deleted(path) => path.path_chars(),
            Self::Renamed(paths) => paths.new.path_chars(),
        }
    }
}

impl CompareFileSummary {
    pub fn from_paths_status(
        old_path: Option<&str>,
        new_path: Option<&str>,
        status: FileStatus,
        stats_deferred: bool,
    ) -> Self {
        Self {
            paths: CompareFilePaths::from_paths(old_path, new_path),
            old_oid: None,
            new_oid: None,
            status,
            is_binary: false,
            is_partial: stats_deferred,
            additions: 0,
            deletions: 0,
            stats_deferred,
        }
    }

    pub fn from_file(file: &FileDiff) -> Self {
        Self {
            paths: CompareFilePaths::from_paths(file.old_path.as_deref(), file.new_path.as_deref()),
            old_oid: file.old_oid.as_ref().map(|oid| Arc::from(oid.0.as_str())),
            new_oid: file.new_oid.as_ref().map(|oid| Arc::from(oid.0.as_str())),
            status: file.status,
            is_binary: file.is_binary,
            is_partial: file.is_partial,
            additions: file.additions,
            deletions: file.deletions,
            stats_deferred: file.stats_deferred,
        }
    }

    pub fn to_file_diff(&self) -> FileDiff {
        FileDiff {
            old_path: self.paths.old_path().map(Cow::into_owned),
            new_path: self.paths.new_path().map(Cow::into_owned),
            old_oid: self
                .old_oid
                .as_ref()
                .map(|oid| carbon::ObjectId(oid.to_string())),
            new_oid: self
                .new_oid
                .as_ref()
                .map(|oid| carbon::ObjectId(oid.to_string())),
            status: self.status,
            is_binary: self.is_binary,
            is_partial: self.is_partial,
            additions: self.additions,
            deletions: self.deletions,
            stats_deferred: self.stats_deferred,
            ..FileDiff::default()
        }
    }

    pub fn stats_target(&self) -> CompareFileStatsTarget {
        CompareFileStatsTarget {
            paths: self.paths.clone(),
            status: self.status,
            is_binary: self.is_binary,
            additions: self.additions,
            deletions: self.deletions,
        }
    }

    pub fn fallback_stats(&self) -> (i32, i32) {
        (
            u32_to_i32_saturating(self.additions),
            u32_to_i32_saturating(self.deletions),
        )
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareFileStatsTarget {
    pub paths: CompareFilePaths,
    pub status: FileStatus,
    pub is_binary: bool,
    pub additions: u32,
    pub deletions: u32,
}

impl CompareFileStatsTarget {
    pub fn from_file(file: &FileDiff) -> Self {
        Self {
            paths: CompareFilePaths::from_paths(file.old_path.as_deref(), file.new_path.as_deref()),
            status: file.status,
            is_binary: file.is_binary,
            additions: file.additions,
            deletions: file.deletions,
        }
    }

    pub fn fallback_stats(&self) -> (i32, i32) {
        (
            u32_to_i32_saturating(self.additions),
            u32_to_i32_saturating(self.deletions),
        )
    }

    pub fn path(&self) -> Cow<'_, str> {
        self.paths.display_path()
    }
}

pub fn u32_to_i32_saturating(value: u32) -> i32 {
    value.min(i32::MAX as u32) as i32
}

pub fn compact_summary_paths(summaries: &mut [CompareFileSummary]) {
    if summaries.is_empty() {
        return;
    }

    let path_count = compact_path_count(summaries);
    let mut dirs = String::new();
    let mut dir_offsets = Vec::with_capacity(path_count.saturating_add(1));
    dir_offsets.push(0);
    dir_offsets.push(0);
    let mut dir_ids = HashMap::new();
    dir_ids.insert(String::new(), 0);
    let mut basenames = String::new();
    let mut basename_offsets = Vec::with_capacity(path_count.saturating_add(1));
    basename_offsets.push(0);
    let mut entries = Vec::with_capacity(path_count);

    for summary in summaries.iter() {
        push_paths_to_store(
            &summary.paths,
            &mut dirs,
            &mut dir_offsets,
            &mut dir_ids,
            &mut basenames,
            &mut basename_offsets,
            &mut entries,
        );
    }
    debug_assert_eq!(entries.len(), path_count);
    debug_assert_eq!(basename_offsets.len(), path_count.saturating_add(1));

    let store = Arc::new(ComparePathStore {
        dirs: dirs.into_boxed_str(),
        dir_offsets: dir_offsets.into_boxed_slice(),
        basenames: basenames.into_boxed_str(),
        basename_offsets: basename_offsets.into_boxed_slice(),
        entries: entries.into_boxed_slice(),
    });
    let mut next_id = 0;
    for summary in summaries.iter_mut() {
        summary.paths = compact_paths_with_ids(&summary.paths, Arc::clone(&store), &mut next_id);
    }

    debug_assert_eq!(next_id, path_count);
}

fn compact_path_count(summaries: &[CompareFileSummary]) -> usize {
    let mut count = 0;
    for summary in summaries {
        match &summary.paths {
            CompareFilePaths::Unknown => {}
            CompareFilePaths::Same(_)
            | CompareFilePaths::Added(_)
            | CompareFilePaths::Deleted(_) => {
                count += 1;
            }
            CompareFilePaths::Renamed(_) => {
                count += 2;
            }
        }
    }
    count
}

fn split_dir_basename(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some((dir, basename)) => (dir, basename),
        None => ("", path),
    }
}

fn push_store_text(text: &mut String, offsets: &mut Vec<u32>, value: &str) -> u32 {
    let id = u32::try_from(offsets.len().saturating_sub(1)).unwrap_or(u32::MAX);
    text.push_str(value);
    offsets.push(u32::try_from(text.len()).unwrap_or(u32::MAX));
    id
}

fn push_paths_to_store(
    paths: &CompareFilePaths,
    dirs: &mut String,
    dir_offsets: &mut Vec<u32>,
    dir_ids: &mut HashMap<String, u32>,
    basenames: &mut String,
    basename_offsets: &mut Vec<u32>,
    entries: &mut Vec<ComparePathEntry>,
) {
    match paths {
        CompareFilePaths::Unknown => {}
        CompareFilePaths::Same(path)
        | CompareFilePaths::Added(path)
        | CompareFilePaths::Deleted(path) => {
            push_compact_path(
                dirs,
                dir_offsets,
                dir_ids,
                basenames,
                basename_offsets,
                entries,
                &path.path(),
            );
        }
        CompareFilePaths::Renamed(paths) => {
            push_compact_path(
                dirs,
                dir_offsets,
                dir_ids,
                basenames,
                basename_offsets,
                entries,
                &paths.old.path(),
            );
            push_compact_path(
                dirs,
                dir_offsets,
                dir_ids,
                basenames,
                basename_offsets,
                entries,
                &paths.new.path(),
            );
        }
    }
}

fn push_compact_path(
    dirs: &mut String,
    dir_offsets: &mut Vec<u32>,
    dir_ids: &mut HashMap<String, u32>,
    basenames: &mut String,
    basename_offsets: &mut Vec<u32>,
    entries: &mut Vec<ComparePathEntry>,
    path: &str,
) {
    let (dir, basename) = split_dir_basename(path);
    let dir_id = match dir_ids.get(dir) {
        Some(id) => *id,
        None => {
            let id = push_store_text(dirs, dir_offsets, dir);
            dir_ids.insert(dir.to_owned(), id);
            id
        }
    };
    let basename_id = push_store_text(basenames, basename_offsets, basename);
    entries.push(ComparePathEntry {
        dir_id,
        basename_id,
    });
}

fn compact_paths_with_ids(
    paths: &CompareFilePaths,
    store: Arc<ComparePathStore>,
    next_id: &mut usize,
) -> CompareFilePaths {
    match paths {
        CompareFilePaths::Unknown => CompareFilePaths::Unknown,
        CompareFilePaths::Same(_) => CompareFilePaths::Same(next_compact_path(store, next_id)),
        CompareFilePaths::Added(_) => CompareFilePaths::Added(next_compact_path(store, next_id)),
        CompareFilePaths::Deleted(_) => {
            CompareFilePaths::Deleted(next_compact_path(store, next_id))
        }
        CompareFilePaths::Renamed(_) => CompareFilePaths::Renamed(CompareRenamedPaths::boxed(
            next_compact_path(Arc::clone(&store), next_id),
            next_compact_path(store, next_id),
        )),
    }
}

fn next_compact_path(store: Arc<ComparePathStore>, next_id: &mut usize) -> ComparePath {
    let id = u32::try_from(*next_id).unwrap_or(u32::MAX);
    *next_id += 1;
    ComparePath::compact(store, id)
}

#[cfg(test)]
mod tests {
    use super::{CompareFileSummary, compact_summary_paths};
    use carbon::FileStatus;

    #[test]
    fn compact_summary_paths_preserves_root_and_nested_paths() {
        let mut summaries = vec![
            CompareFileSummary::from_paths_status(
                Some(".clang-format"),
                Some(".clang-format"),
                FileStatus::Modified,
                true,
            ),
            CompareFileSummary::from_paths_status(
                None,
                Some("Documentation/.renames.txt"),
                FileStatus::Added,
                true,
            ),
            CompareFileSummary::from_paths_status(
                Some("drivers/old.c"),
                Some("drivers/new.c"),
                FileStatus::Renamed,
                true,
            ),
        ];

        compact_summary_paths(&mut summaries);

        assert_eq!(summaries[0].path(), ".clang-format");
        assert_eq!(summaries[0].path_chars(), ".clang-format".chars().count());
        assert_eq!(summaries[1].path(), "Documentation/.renames.txt");
        assert_eq!(
            summaries[1].path_chars(),
            "Documentation/.renames.txt".chars().count()
        );
        assert_eq!(
            summaries[2].paths.old_path().as_deref(),
            Some("drivers/old.c")
        );
        assert_eq!(
            summaries[2].paths.new_path().as_deref(),
            Some("drivers/new.c")
        );
        assert_eq!(summaries[2].path_chars(), "drivers/new.c".chars().count());
    }
}
