use crate::core::search::fuzzy::match_ranges;
use std::collections::HashMap;

use quark_components::FuzzyMatcher;

use crate::core::vcs::git::RefMetadata;
use crate::core::vcs::model::{RefKind, VcsChange, VcsRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefSection {
    Recent,
    Tags,
    Branches,
    Commits,
}

impl RefSection {
    pub const ALL: [Self; 4] = [Self::Recent, Self::Tags, Self::Branches, Self::Commits];

    pub fn label(self) -> &'static str {
        match self {
            Self::Recent => "Recent",
            Self::Tags => "Tags",
            Self::Branches => "Branches",
            Self::Commits => "Commits",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefEntryKind {
    WorkingTree,
    Branch,
    RemoteBranch,
    Tag,
    Commit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefEntry {
    pub name: String,
    pub kind: RefEntryKind,
    pub section: RefSection,
    pub oid: String,
    pub subject: String,
    pub author: String,
    pub timestamp: i64,
}

impl RefEntry {
    pub fn short_oid(&self) -> &str {
        &self.oid[..self.oid.len().min(7)]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefCatalog {
    pub entries: Vec<RefEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefRow {
    Section {
        section: RefSection,
        count: u32,
    },
    Entry {
        index: u32,
        ranges: Vec<(usize, usize)>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefResults {
    pub rows: Vec<RefRow>,
}

pub const WORKING_TREE_LABEL: &str = "Working tree";
const RECENT_LIMIT: usize = 5;

pub fn build_catalog(
    refs: &[VcsRef],
    changes: &[VcsChange],
    metadata: &[RefMetadata],
    recent: &[String],
    working_tree: bool,
) -> RefCatalog {
    let meta: HashMap<&str, &RefMetadata> = metadata.iter().map(|m| (m.name.as_str(), m)).collect();
    let entry = |r: &VcsRef, kind: RefEntryKind, section: RefSection| {
        let m = meta.get(r.name.as_str());
        RefEntry {
            name: r.name.clone(),
            kind,
            section,
            oid: m.map_or_else(|| r.target.id.clone(), |m| m.oid.clone()),
            subject: m.map(|m| m.subject.clone()).unwrap_or_default(),
            author: m.map(|m| m.author.clone()).unwrap_or_default(),
            timestamp: m.map_or(0, |m| m.timestamp),
        }
    };
    let mut tags = Vec::new();
    let mut local = Vec::new();
    let mut remote = Vec::new();
    for r in refs {
        match r.kind {
            RefKind::Tag => tags.push(entry(r, RefEntryKind::Tag, RefSection::Tags)),
            RefKind::Branch | RefKind::Bookmark => local.push((
                r.active,
                entry(r, RefEntryKind::Branch, RefSection::Branches),
            )),
            RefKind::RemoteBranch | RefKind::RemoteBookmark => {
                remote.push(entry(r, RefEntryKind::RemoteBranch, RefSection::Branches))
            }
            RefKind::Head | RefKind::WorkingCopy | RefKind::PullRequest => {}
        }
    }
    let newest =
        |a: &RefEntry, b: &RefEntry| b.timestamp.cmp(&a.timestamp).then(a.name.cmp(&b.name));
    tags.sort_by(newest);
    local.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| newest(&a.1, &b.1)));
    remote.sort_by(newest);

    let mut entries = Vec::with_capacity(tags.len() + local.len() + remote.len() + 8);
    if working_tree {
        entries.push(RefEntry {
            name: WORKING_TREE_LABEL.to_owned(),
            kind: RefEntryKind::WorkingTree,
            section: RefSection::Recent,
            oid: String::new(),
            subject: "Uncommitted changes".to_owned(),
            author: String::new(),
            timestamp: 0,
        });
    }
    let named = tags
        .iter()
        .chain(local.iter().map(|(_, e)| e))
        .chain(remote.iter());
    let by_name: HashMap<&str, &RefEntry> = named.map(|e| (e.name.as_str(), e)).collect();
    entries.extend(
        recent
            .iter()
            .filter_map(|name| by_name.get(name.as_str()))
            .take(RECENT_LIMIT)
            .map(|e| RefEntry {
                section: RefSection::Recent,
                ..(*e).clone()
            }),
    );
    entries.extend(tags);
    entries.extend(local.into_iter().map(|(_, e)| e));
    entries.extend(remote);
    entries.extend(changes.iter().map(|change| RefEntry {
        name: change.short_revision.clone(),
        kind: RefEntryKind::Commit,
        section: RefSection::Commits,
        oid: change.revision.id.clone(),
        subject: change.summary.clone(),
        author: change.author_name.clone(),
        timestamp: change.timestamp,
    }));
    RefCatalog { entries }
}

pub fn filter(catalog: &RefCatalog, query: &str) -> RefResults {
    let query = query.trim();
    let mut matcher = FuzzyMatcher::new(query);
    let mut positions = Vec::new();
    let mut groups: [Vec<(i32, u32, Vec<(usize, usize)>)>; 4] = Default::default();
    for (index, entry) in catalog.entries.iter().enumerate() {
        positions.clear();
        let Some(score) = matcher.score(&entry.name, &mut positions) else {
            continue;
        };
        let slot = RefSection::ALL
            .iter()
            .position(|s| *s == entry.section)
            .unwrap_or(0);
        groups[slot].push((score, index as u32, match_ranges(&entry.name, &positions)));
    }
    let mut rows = Vec::new();
    for (section, mut group) in RefSection::ALL.into_iter().zip(groups) {
        if group.is_empty() {
            continue;
        }
        if !query.is_empty() {
            group.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        rows.push(RefRow::Section {
            section,
            count: group.len() as u32,
        });
        rows.extend(
            group
                .into_iter()
                .map(|(_, index, ranges)| RefRow::Entry { index, ranges }),
        );
    }
    RefResults { rows }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub fn relative_age(now: i64, timestamp: i64) -> String {
    if timestamp <= 0 {
        return String::new();
    }
    let seconds = (now - timestamp).max(0);
    let (value, unit) = match seconds {
        s if s < 60 => return "now".to_owned(),
        s if s < 3_600 => (s / 60, "min"),
        s if s < 86_400 => (s / 3_600, "h"),
        s if s < 604_800 => (s / 86_400, "d"),
        s if s < 2_629_800 => (s / 604_800, "w"),
        s if s < 31_557_600 => (s / 2_629_800, "mo"),
        s => (s / 31_557_600, "y"),
    };
    format!("{value} {unit}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::vcs::model::RevisionId;

    fn vcs_ref(name: &str, kind: RefKind) -> VcsRef {
        VcsRef {
            name: name.to_owned(),
            kind,
            target: RevisionId::git(format!("{name}0000000")),
            active: false,
            upstream: None,
            ahead_behind: None,
        }
    }

    fn meta(name: &str, timestamp: i64) -> RefMetadata {
        RefMetadata {
            name: name.to_owned(),
            oid: "abcdef1234".to_owned(),
            subject: format!("subject {name}"),
            author: "author".to_owned(),
            timestamp,
        }
    }

    #[test]
    fn catalog_groups_recent_tags_and_branches_newest_first() {
        let refs = [
            vcs_ref("v1", RefKind::Tag),
            vcs_ref("v2", RefKind::Tag),
            vcs_ref("main", RefKind::Branch),
            vcs_ref("origin/main", RefKind::RemoteBranch),
        ];
        let metadata = [meta("v1", 10), meta("v2", 20), meta("main", 5)];
        let catalog = build_catalog(&refs, &[], &metadata, &["main".to_owned()], true);
        let names: Vec<(&str, RefSection)> = catalog
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.section))
            .collect();
        assert_eq!(
            names,
            [
                (WORKING_TREE_LABEL, RefSection::Recent),
                ("main", RefSection::Recent),
                ("v2", RefSection::Tags),
                ("v1", RefSection::Tags),
                ("main", RefSection::Branches),
                ("origin/main", RefSection::Branches),
            ]
        );
        assert_eq!(catalog.entries[2].subject, "subject v2");
    }

    #[test]
    fn filter_counts_matches_per_section_and_highlights() {
        let refs: Vec<VcsRef> = (0..10_000)
            .map(|i| vcs_ref(&format!("tag-{i}"), RefKind::Tag))
            .chain([vcs_ref("release/v0.1", RefKind::Branch)])
            .collect();
        let catalog = build_catalog(&refs, &[], &[], &[], false);
        let results = filter(&catalog, "v0.1");
        assert_eq!(
            results.rows,
            [
                RefRow::Section {
                    section: RefSection::Branches,
                    count: 1
                },
                RefRow::Entry {
                    index: 10_000,
                    ranges: vec![(8, 12)]
                },
            ]
        );
        let all = filter(&catalog, "");
        assert_eq!(all.rows.len(), 10_001 + 2);
    }

    #[test]
    fn ages_are_short() {
        assert_eq!(relative_age(1_000_000, 1_000_000 - 7_200), "2 h");
        assert_eq!(relative_age(10_000_000, 10_000_000 - 3 * 604_800), "3 w");
        assert_eq!(relative_age(5, 0), "");
    }
}
