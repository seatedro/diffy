use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};

use quark_components::FuzzyMatcher;

use super::picker::{PickerChoice, PickerRow, RowKind};
use crate::core::ref_catalog::relative_age;
use crate::core::repo_discovery::RepoScan;
use crate::core::search::fuzzy::match_ranges;

const PARENT_COMPONENTS: usize = 2;
pub const RECENT_LABEL: &str = "Recent";
pub const DISCOVERED_LABEL: &str = "On this Mac";

#[derive(Debug, Clone, PartialEq)]
pub struct RepoEntry {
    pub path: PathBuf,
    pub name: String,
    pub group: String,
    pub parent: String,
    pub alias: String,
    pub branch: Option<String>,
    pub age: String,
    pub current: bool,
}

impl RepoEntry {
    pub fn new(
        path: PathBuf,
        home: Option<&Path>,
        branch: Option<String>,
        seen: Option<i64>,
        now: i64,
        current: Option<&Path>,
    ) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let parent = path.parent().unwrap_or(&path);
        Self {
            name,
            group: tilde(parent, home),
            parent: short_parent(&path, home),
            alias: tilde(&path, home),
            branch,
            age: seen.map(|seen| relative_age(now, seen)).unwrap_or_default(),
            current: current == Some(path.as_path()),
            path,
        }
    }

    fn row(&self, highlights: Vec<(usize, usize)>, indent: bool) -> PickerRow {
        PickerRow {
            label: self.name.clone(),
            detail: (!indent).then(|| self.parent.clone()),
            meta: Some(self.age.clone()).filter(|age| !age.is_empty()),
            highlights,
            choice: PickerChoice::Repository(self.path.clone()),
            current: self.current,
            kind: RowKind::Item,
            branch: self.branch.clone(),
            indent,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RepoView {
    pub recent: Vec<RepoEntry>,
    pub discovered: Vec<RepoEntry>,
    pub expanded: Option<String>,
    pub scanning: bool,
}

impl RepoView {
    pub fn set_discovered(
        &mut self,
        scan: &RepoScan,
        home: Option<&Path>,
        now: i64,
        current: Option<&Path>,
    ) {
        self.discovered = scan
            .repos
            .iter()
            .map(|repo| {
                RepoEntry::new(
                    repo.path.clone(),
                    home,
                    repo.branch.clone(),
                    repo.active,
                    now,
                    current,
                )
            })
            .collect();
        for recent in &mut self.recent {
            if let Some(found) = scan.find(&recent.path) {
                recent.branch = found.branch.clone();
            }
        }
        let groups = self.groups();
        if self
            .expanded
            .as_ref()
            .is_none_or(|open| !groups.iter().any(|(label, _)| label == open))
        {
            self.expanded = self.default_group(&groups);
        }
    }

    pub fn total(&self) -> usize {
        self.unique().len()
    }

    pub fn toggle(&mut self, group: &str) {
        self.expanded = match &self.expanded {
            Some(open) if open == group => None,
            _ => Some(group.to_owned()),
        };
    }

    pub fn set_open(&mut self, group: &str, open: bool) {
        if open {
            self.expanded = Some(group.to_owned());
        } else if self.expanded.as_deref() == Some(group) {
            self.expanded = None;
        }
    }

    pub fn rows(&self, query: &str, matcher: &mut FuzzyMatcher) -> Vec<PickerRow> {
        if query.is_empty() {
            self.browse_rows()
        } else {
            self.search_rows(query, matcher)
        }
    }

    fn browse_rows(&self) -> Vec<PickerRow> {
        let mut rows = Vec::new();
        if !self.recent.is_empty() {
            rows.push(heading(RECENT_LABEL, None, false));
            rows.extend(self.recent.iter().map(|entry| entry.row(Vec::new(), false)));
        }
        if self.discovered.is_empty() && !self.scanning {
            return rows;
        }
        rows.push(heading(
            DISCOVERED_LABEL,
            Some(self.discovered.len()),
            self.scanning,
        ));
        for (label, entries) in self.groups() {
            let expanded = self.expanded.as_deref() == Some(label.as_str());
            rows.push(PickerRow {
                label: label.clone(),
                detail: None,
                meta: None,
                highlights: Vec::new(),
                choice: PickerChoice::Group(label),
                current: false,
                kind: RowKind::Group {
                    count: entries.len(),
                    expanded,
                },
                branch: None,
                indent: false,
            });
            if expanded {
                rows.extend(entries.iter().map(|entry| entry.row(Vec::new(), true)));
            }
        }
        rows
    }

    fn search_rows(&self, query: &str, matcher: &mut FuzzyMatcher) -> Vec<PickerRow> {
        matcher.set_query(query);
        let mut positions = Vec::new();
        let mut scored: Vec<(i32, usize, PickerRow)> = Vec::new();
        for (order, (entry, recent)) in self.unique().into_iter().enumerate() {
            positions.clear();
            let row = if let Some(score) = matcher.score(&entry.name, &mut positions) {
                Some((
                    score,
                    entry.row(match_ranges(&entry.name, &positions), false),
                ))
            } else {
                positions.clear();
                matcher
                    .score(&entry.alias, &mut positions)
                    .map(|score| (score, entry.row(Vec::new(), false)))
            };
            if let Some((score, row)) = row {
                let rank = if recent {
                    order
                } else {
                    order + self.recent.len()
                };
                scored.push((score, rank, row));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, _, row)| row).collect()
    }

    fn unique(&self) -> Vec<(&RepoEntry, bool)> {
        let mut seen: HashSet<&Path> = HashSet::new();
        let recent = self.recent.iter().map(|entry| (entry, true));
        let discovered = self.discovered.iter().map(|entry| (entry, false));
        recent
            .chain(discovered)
            .filter(|(entry, _)| seen.insert(entry.path.as_path()))
            .collect()
    }

    fn groups(&self) -> Vec<(String, Vec<&RepoEntry>)> {
        let mut groups: BTreeMap<&str, Vec<&RepoEntry>> = BTreeMap::new();
        for entry in &self.discovered {
            groups.entry(entry.group.as_str()).or_default().push(entry);
        }
        let mut groups: Vec<(String, Vec<&RepoEntry>)> = groups
            .into_iter()
            .map(|(label, entries)| (label.to_owned(), entries))
            .collect();
        groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
        groups
    }

    fn default_group(&self, groups: &[(String, Vec<&RepoEntry>)]) -> Option<String> {
        groups
            .iter()
            .find(|(_, entries)| entries.iter().any(|entry| entry.current))
            .or_else(|| groups.first())
            .map(|(label, _)| label.clone())
    }
}

fn heading(label: &str, count: Option<usize>, busy: bool) -> PickerRow {
    PickerRow {
        label: label.to_owned(),
        detail: None,
        meta: None,
        highlights: Vec::new(),
        choice: PickerChoice::Heading,
        current: false,
        kind: RowKind::Section { count, busy },
        branch: None,
        indent: false,
    }
}

pub fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

fn short_parent(path: &Path, home: Option<&Path>) -> String {
    let Some(parent) = path.parent() else {
        return String::new();
    };
    let (rooted, rest) = match home.and_then(|home| parent.strip_prefix(home).ok()) {
        Some(rest) => ("~", rest),
        None => ("", parent),
    };
    let parts: Vec<String> = rest
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if parts.len() > PARENT_COMPONENTS {
        format!(
            "\u{2026}/{}",
            parts[parts.len() - PARENT_COMPONENTS..].join("/")
        )
    } else {
        format!("{rooted}/{}", parts.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::repo_discovery::DiscoveredRepo;

    fn scan(paths: &[&str]) -> RepoScan {
        RepoScan {
            repos: paths
                .iter()
                .map(|path| DiscoveredRepo {
                    path: PathBuf::from(path),
                    branch: Some("main".to_owned()),
                    active: None,
                })
                .collect(),
            ..RepoScan::default()
        }
    }

    fn labels(rows: &[PickerRow]) -> Vec<String> {
        rows.iter().map(|row| row.label.clone()).collect()
    }

    #[test]
    fn browse_groups_by_parent_and_opens_the_current_group() {
        let home = Path::new("/h");
        let current = Path::new("/h/b/two");
        let mut view = RepoView::default();
        view.set_discovered(
            &scan(&["/h/a/x", "/h/a/y", "/h/a/z", "/h/b/one", "/h/b/two"]),
            Some(home),
            0,
            Some(current),
        );
        let rows = view.rows("", &mut FuzzyMatcher::default());
        assert_eq!(
            labels(&rows),
            ["On this Mac", "~/a", "~/b", "one", "two"].map(str::to_owned)
        );
        assert!(rows[4].current && rows[4].indent);
        view.toggle("~/a");
        let rows = view.rows("", &mut FuzzyMatcher::default());
        assert_eq!(
            labels(&rows),
            ["On this Mac", "~/a", "x", "y", "z", "~/b"].map(str::to_owned)
        );
    }

    #[test]
    fn search_matches_names_and_paths_across_recent_and_discovered() {
        let home = Path::new("/h");
        let mut view = RepoView {
            recent: vec![RepoEntry::new(
                PathBuf::from("/h/a/quark"),
                Some(home),
                None,
                None,
                0,
                None,
            )],
            ..RepoView::default()
        };
        view.set_discovered(&scan(&["/h/a/quark", "/h/a/diffy"]), Some(home), 0, None);
        assert_eq!(view.total(), 2);
        let mut matcher = FuzzyMatcher::default();
        assert_eq!(labels(&view.rows("qrk", &mut matcher)), ["quark"]);
        assert_eq!(labels(&view.rows("~/a/dif", &mut matcher)), ["diffy"]);
    }
}
