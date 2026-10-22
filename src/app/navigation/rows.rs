use crate::core::search::fuzzy::match_ranges;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use quark_components::FuzzyMatcher;

use crate::contracts::navigation::SidebarMode;
use crate::contracts::viewport::pipeline::ManifestFile;
use crate::core::vcs::model::ChangeBucket;

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Folder {
        path: Rc<str>,
        depth: u16,
        expanded: bool,
    },
    File {
        file: u32,
        depth: u16,
    },
    Section {
        bucket: ChangeBucket,
        files: u32,
    },
    Item {
        file: u32,
        item: u32,
    },
}

const SECTION_ORDER: [ChangeBucket; 5] = [
    ChangeBucket::Conflicted,
    ChangeBucket::Staged,
    ChangeBucket::Unstaged,
    ChangeBucket::Untracked,
    ChangeBucket::WorkingCopy,
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct RowKey {
    files: usize,
    len: usize,
    mode: SidebarMode,
    filter: String,
    expanded: u64,
    selected: Option<u32>,
}

#[derive(Debug, Default)]
pub struct RowCache {
    key: Option<RowKey>,
    order: Option<Rc<[u32]>>,
    rows: Vec<Row>,
    file_rows: Vec<usize>,
    sorted: Option<(usize, Vec<u32>)>,
    matcher: FuzzyMatcher,
    positions: Vec<u32>,
    highlights: HashMap<u32, Vec<(usize, usize)>>,
}

impl RowCache {
    pub fn invalidate(&mut self) {
        self.key = None;
    }

    pub fn set_order(&mut self, order: Option<Rc<[u32]>>) {
        let same = match (&self.order, &order) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.order = order;
            self.key = None;
        }
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn file_rows(&self) -> &[usize] {
        &self.file_rows
    }

    pub fn ensure(
        &mut self,
        files: &Arc<[ManifestFile]>,
        mode: SidebarMode,
        filter: &str,
        expanded: &HashSet<String>,
        expanded_revision: u64,
        selected: Option<u32>,
    ) {
        let filter = filter.trim();
        let key = RowKey {
            files: Arc::as_ptr(files) as *const u8 as usize,
            len: files.len(),
            mode,
            filter: filter.to_owned(),
            expanded: expanded_revision,
            selected,
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.rows.clear();
        self.highlights.clear();
        if !filter.is_empty() {
            self.build_filtered(files, filter);
        } else if mode == SidebarMode::Tree {
            self.build_tree(files, key.files, expanded);
        } else {
            let order: Vec<u32> = match self.order.as_deref() {
                Some(order) if order.len() == files.len() => order.to_vec(),
                _ => (0..files.len() as u32).collect(),
            };
            if files.iter().any(staged_bucket) {
                self.build_sections(files, &order);
            } else {
                self.rows
                    .extend(order.into_iter().map(|file| Row::File { file, depth: 0 }));
            }
        }
        if let Some(file) = selected
            && let Some(at) = self
                .rows
                .iter()
                .position(|row| matches!(row, Row::File { file: f, .. } if *f == file))
        {
            let count = files
                .get(file as usize)
                .map_or(0, |f| f.outline.len() as u32);
            self.rows.splice(
                at + 1..at + 1,
                (0..count).map(|item| Row::Item { file, item }),
            );
        }
        self.file_rows.clear();
        self.file_rows.extend(
            self.rows
                .iter()
                .enumerate()
                .filter(|(_, row)| matches!(row, Row::File { .. }))
                .map(|(i, _)| i),
        );
        self.key = Some(key);
    }

    pub fn highlights(&self, file: u32) -> &[(usize, usize)] {
        self.highlights.get(&file).map_or(&[], Vec::as_slice)
    }

    pub fn row_of_file(&self, file: u32) -> Option<usize> {
        self.file_rows
            .iter()
            .copied()
            .find(|&row| matches!(self.rows[row], Row::File { file: f, .. } if f == file))
    }

    fn build_filtered(&mut self, files: &[ManifestFile], filter: &str) {
        self.matcher.set_query(filter);
        let mut scored: Vec<(i32, u32)> = Vec::new();
        let mut path = String::new();
        for (index, file) in files.iter().enumerate() {
            path.clear();
            file.paths.push_display_path_to(&mut path);
            self.positions.clear();
            if let Some(score) = self.matcher.score(&path, &mut self.positions) {
                scored.push((score, index as u32));
                self.highlights
                    .insert(index as u32, match_ranges(&path, &self.positions));
            }
        }
        scored.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.rows.extend(
            scored
                .into_iter()
                .map(|(_, file)| Row::File { file, depth: 0 }),
        );
    }

    fn build_sections(&mut self, files: &[ManifestFile], order: &[u32]) {
        let mut groups: [Vec<u32>; SECTION_ORDER.len()] = Default::default();
        for &index in order {
            let bucket = files[index as usize]
                .buckets
                .iter()
                .copied()
                .reduce(dominant)
                .unwrap_or(ChangeBucket::WorkingCopy);
            let slot = SECTION_ORDER.iter().position(|b| *b == bucket).unwrap_or(0);
            groups[slot].push(index);
        }
        for (bucket, group) in SECTION_ORDER.into_iter().zip(groups) {
            if group.is_empty() {
                continue;
            }
            self.rows.push(Row::Section {
                bucket,
                files: group.len() as u32,
            });
            self.rows
                .extend(group.into_iter().map(|file| Row::File { file, depth: 0 }));
        }
    }

    fn build_tree(&mut self, files: &[ManifestFile], identity: usize, expanded: &HashSet<String>) {
        if self
            .sorted
            .as_ref()
            .is_none_or(|(id, order)| *id != identity || order.len() != files.len())
        {
            let mut order: Vec<u32> = (0..files.len() as u32).collect();
            order.sort_by_cached_key(|&i| files[i as usize].paths.display_path().into_owned());
            self.sorted = Some((identity, order));
        }
        let Some((_, order)) = &self.sorted else {
            return;
        };
        let mut open: Vec<String> = Vec::new();
        let mut collapsed: Option<String> = None;
        let mut path = String::new();
        for &index in order {
            path.clear();
            files[index as usize].paths.push_display_path_to(&mut path);
            if let Some(prefix) = &collapsed {
                if is_within(&path, prefix) {
                    continue;
                }
                collapsed = None;
            }
            let common = open
                .iter()
                .take_while(|folder| is_within(&path, folder))
                .count();
            open.truncate(common);
            let separators: Vec<usize> = path.match_indices('/').map(|(i, _)| i).collect();
            let mut hidden = false;
            for (depth, &end) in separators.iter().enumerate().skip(common) {
                let full = &path[..end];
                let is_open = expanded.contains(full);
                self.rows.push(Row::Folder {
                    path: Rc::from(full),
                    depth: depth as u16,
                    expanded: is_open,
                });
                if !is_open {
                    collapsed = Some(full.to_owned());
                    hidden = true;
                    break;
                }
                open.push(full.to_owned());
            }
            if !hidden {
                self.rows.push(Row::File {
                    file: index,
                    depth: separators.len() as u16,
                });
            }
        }
    }
}

fn staged_bucket(file: &ManifestFile) -> bool {
    file.buckets.iter().any(|b| *b != ChangeBucket::WorkingCopy)
}

fn dominant(a: ChangeBucket, b: ChangeBucket) -> ChangeBucket {
    let rank = |bucket| match bucket {
        ChangeBucket::Conflicted => 0,
        ChangeBucket::Unstaged => 1,
        ChangeBucket::Untracked => 2,
        ChangeBucket::Staged => 3,
        ChangeBucket::WorkingCopy => 4,
    };
    if rank(b) < rank(a) { b } else { a }
}

pub fn section_label(bucket: ChangeBucket) -> &'static str {
    match bucket {
        ChangeBucket::Conflicted => "Conflicts",
        ChangeBucket::Staged => "Staged",
        ChangeBucket::Unstaged => "Unstaged",
        ChangeBucket::Untracked => "Untracked",
        ChangeBucket::WorkingCopy => "Changes",
    }
}

fn is_within(path: &str, folder: &str) -> bool {
    path.len() > folder.len() && path.starts_with(folder) && path.as_bytes()[folder.len()] == b'/'
}

pub fn folder_prefixes(path: &str, out: &mut HashSet<String>) {
    for (index, _) in path.match_indices('/') {
        out.insert(path[..index].to_owned());
    }
}

pub fn file_name(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some((dir, name)) => (dir, name),
        None => ("", path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::shared::{FileKey, FileRevision};
    use crate::core::compare::stats::CompareFilePaths;
    use diffy_diff::FileStatus;

    fn manifest(paths: &[&str]) -> Arc<[ManifestFile]> {
        paths
            .iter()
            .enumerate()
            .map(|(i, path)| {
                ManifestFile::new(
                    FileKey(i as u64 + 1),
                    FileRevision(1),
                    CompareFilePaths::from_paths(Some(path), Some(path)),
                    FileStatus::Modified,
                )
            })
            .collect()
    }

    fn labels(cache: &RowCache, files: &[ManifestFile]) -> Vec<String> {
        cache
            .rows()
            .iter()
            .map(|row| match row {
                Row::Folder {
                    path,
                    depth,
                    expanded,
                } => {
                    format!(
                        "{}{}{}",
                        "  ".repeat(*depth as usize),
                        path,
                        if *expanded { "/" } else { "/+" }
                    )
                }
                Row::File { file, depth } => format!(
                    "{}{}",
                    "  ".repeat(*depth as usize),
                    files[*file as usize].paths.display_path()
                ),
                Row::Item { item, .. } => format!("item {item}"),
                Row::Section { bucket, files } => {
                    format!("# {} {files}", section_label(*bucket))
                }
            })
            .collect()
    }

    #[test]
    fn tree_rows_follow_expanded_folders() {
        let files = manifest(&[
            "src/b.rs",
            "README",
            "src/a/x.rs",
            "src/a/y.rs",
            "docs/z.md",
        ]);
        let mut cache = RowCache::default();
        let mut expanded = HashSet::new();
        cache.ensure(&files, SidebarMode::Tree, "", &expanded, 0, None);
        assert_eq!(labels(&cache, &files), ["README", "docs/+", "src/+"]);

        expanded.insert("src".to_owned());
        cache.ensure(&files, SidebarMode::Tree, "", &expanded, 1, None);
        assert_eq!(
            labels(&cache, &files),
            ["README", "docs/+", "src/", "  src/a/+", "  src/b.rs"]
        );

        expanded.insert("src/a".to_owned());
        cache.ensure(&files, SidebarMode::Tree, "", &expanded, 2, None);
        assert_eq!(
            labels(&cache, &files),
            [
                "README",
                "docs/+",
                "src/",
                "  src/a/",
                "    src/a/x.rs",
                "    src/a/y.rs",
                "  src/b.rs"
            ]
        );
        assert_eq!(cache.file_rows().len(), 4);
    }

    #[test]
    fn filter_ranks_matches_in_a_flat_list() {
        let files = manifest(&["src/main.rs", "src/lib.rs", "docs/main.md"]);
        let mut cache = RowCache::default();
        cache.ensure(&files, SidebarMode::Tree, "lib", &HashSet::new(), 0, None);
        assert_eq!(labels(&cache, &files), ["src/lib.rs"]);
        cache.ensure(&files, SidebarMode::Tree, "zzz", &HashSet::new(), 0, None);
        assert!(cache.rows().is_empty());
    }

    #[test]
    fn working_tree_files_group_by_bucket() {
        let plain = manifest(&["a.rs", "b.rs", "c.rs", "d.rs"]);
        let buckets = [
            vec![ChangeBucket::Staged],
            vec![ChangeBucket::Staged, ChangeBucket::Unstaged],
            vec![ChangeBucket::Untracked],
            vec![ChangeBucket::Staged],
        ];
        let files: Arc<[ManifestFile]> = plain
            .iter()
            .zip(buckets)
            .map(|(file, buckets)| ManifestFile {
                buckets,
                ..file.clone()
            })
            .collect();
        let mut cache = RowCache::default();
        cache.ensure(&files, SidebarMode::List, "", &HashSet::new(), 0, None);
        assert_eq!(
            labels(&cache, &files),
            [
                "# Staged 2",
                "a.rs",
                "d.rs",
                "# Unstaged 1",
                "b.rs",
                "# Untracked 1",
                "c.rs"
            ]
        );
        assert_eq!(cache.file_rows(), [1, 2, 4, 6]);
        cache.ensure(&plain, SidebarMode::List, "", &HashSet::new(), 0, None);
        assert_eq!(labels(&cache, &plain), ["a.rs", "b.rs", "c.rs", "d.rs"]);
    }
}
