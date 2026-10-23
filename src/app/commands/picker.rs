use crate::core::search::fuzzy::match_ranges;
use std::path::PathBuf;
use std::sync::Arc;

use neo_frizbee::{Config, Matcher};

use quark_components::FuzzyMatcher;
use quark_ui::element::ScrollbarVisibility;
use quark_ui::text_input::TextField;

use super::repo_picker::RepoView;
use crate::app::viewport::outline;
use crate::contracts::navigation::{CompareField, NavigationAction};
use crate::contracts::shell::Action;
use crate::contracts::viewport::files::FileAt;
use crate::contracts::viewport::symbols::{DefinitionTarget, SymbolsAction};
use crate::core::compare::CompareMode;
use crate::core::symbols::SymbolIndex;
use crate::core::vcs::model::RevisionId;
use crate::fonts::{FontFamilyEntry, FontRole};
use diffy_syntax::Symbol;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Repository,
    Ref(CompareField),
    Theme,
    Font(FontRole),
    CompareMenu,
    PullRequest,
    File,
    Definition,
}

impl PickerKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Repository => "Open repository",
            Self::Ref(CompareField::Left) => "Base ref",
            Self::Ref(CompareField::Right) => "Head ref",
            Self::Theme => "Theme",
            Self::Font(FontRole::Ui) => "UI font",
            Self::Font(FontRole::Mono) => "Code font",
            Self::CompareMenu => "Compare",
            Self::PullRequest => "Open pull request",
            Self::File => "Go to file",
            Self::Definition => "Definitions",
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Repository => "Search repositories or type a path",
            Self::Ref(_) => "Search refs or type a revision",
            Self::Theme => "Search themes",
            Self::Font(_) => "Search fonts",
            Self::CompareMenu => "Search compare options",
            Self::PullRequest => "Paste a GitHub pull request URL",
            Self::File => "Search files, @ symbols here, # all symbols",
            Self::Definition => "Filter definitions",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PickerChoice {
    Heading,
    Group(String),
    Repository(PathBuf),
    Ref(String),
    Theme(String),
    Font(String),
    Action(Action),
    PullRequest(String),
    File(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RowKind {
    #[default]
    Item,
    Section {
        count: Option<usize>,
        busy: bool,
    },
    Group {
        count: usize,
        expanded: bool,
    },
    Worktree {
        main: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PickerRow {
    pub label: String,
    pub detail: Option<String>,
    pub meta: Option<String>,
    pub highlights: Vec<(usize, usize)>,
    pub choice: PickerChoice,
    pub current: bool,
    pub kind: RowKind,
    pub branch: Option<String>,
    pub indent: bool,
}

impl PickerRow {
    pub fn selectable(&self) -> bool {
        !matches!(self.kind, RowKind::Section { .. })
    }
}

#[derive(Debug, Clone)]
pub struct PickerCandidate {
    pub label: String,
    pub detail: Option<String>,
    pub meta: Option<String>,
    pub alias: Option<String>,
    pub choice: PickerChoice,
    pub current: bool,
}

impl PickerCandidate {
    pub fn new(label: impl Into<String>, choice: PickerChoice) -> Self {
        Self {
            label: label.into(),
            detail: None,
            meta: None,
            alias: None,
            choice,
            current: false,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.alias = Some(alias.into());
        self
    }

    pub fn meta(mut self, meta: impl Into<String>) -> Self {
        self.meta = Some(meta.into()).filter(|meta: &String| !meta.is_empty());
        self
    }

    pub fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }
}

#[derive(Debug)]
pub struct PickerState {
    pub kind: PickerKind,
    pub query: TextField,
    pub candidates: Vec<PickerCandidate>,
    pub rows: Vec<PickerRow>,
    pub selected: usize,
    pub scroll_px: f32,
    pub scrollbar: ScrollbarVisibility,
    pub pending_reveal: bool,
    pub repos: Option<RepoView>,
    pub files: Option<Arc<[String]>>,
    pub symbols: Option<Arc<SymbolIndex>>,
    pub document: Option<FileAt>,
    pub head: Option<RevisionId>,
    pub anchored: bool,
    matcher: FuzzyMatcher,
}

pub const PICKER_VISIBLE_ROWS: usize = 9;
pub const REPO_PICKER_VISIBLE_ROWS: usize = 14;
pub const TYPED_PATH: &str = "Open this path";

impl PickerState {
    pub fn new(kind: PickerKind, candidates: Vec<PickerCandidate>) -> Self {
        let mut state = Self {
            kind,
            query: TextField::new(""),
            candidates,
            rows: Vec::new(),
            selected: 0,
            scroll_px: 0.0,
            scrollbar: ScrollbarVisibility::new(),
            pending_reveal: true,
            repos: None,
            files: None,
            symbols: None,
            document: None,
            head: None,
            anchored: false,
            matcher: FuzzyMatcher::default(),
        };
        state.refilter();
        state
    }

    pub fn repositories(view: RepoView, anchored: bool) -> Self {
        let mut state = Self::new(PickerKind::Repository, Vec::new());
        state.repos = Some(view);
        state.anchored = anchored;
        state.refilter();
        state
    }

    pub fn visible_rows(&self) -> usize {
        if self.repos.is_some() {
            REPO_PICKER_VISIBLE_ROWS
        } else {
            PICKER_VISIBLE_ROWS
        }
    }

    pub fn refresh(&mut self) {
        let keep = self.selected_choice().cloned();
        let scroll = self.scroll_px;
        self.refilter();
        self.scroll_px = scroll;
        if let Some(index) = keep.and_then(|keep| self.rows.iter().position(|r| r.choice == keep)) {
            self.selected = index;
        }
    }

    pub fn set_files(&mut self, files: Arc<[String]>) {
        let loaded = self.files.replace(files).is_some();
        if loaded {
            self.refresh();
        } else {
            self.refilter();
        }
    }

    pub fn symbol_mode(&self) -> Option<SymbolMode> {
        if self.kind != PickerKind::File {
            return None;
        }
        match self.query.text().trim_start().chars().next() {
            Some('@') => Some(SymbolMode::File),
            Some('#') => Some(SymbolMode::Workspace),
            _ => None,
        }
    }

    pub fn empty_message(&self) -> &'static str {
        match self.symbol_mode() {
            Some(_) if self.symbols.is_none() => "Indexing symbols",
            Some(SymbolMode::File) if self.document.is_none() => "Open a file first",
            Some(SymbolMode::Workspace) if self.query.text().trim().len() <= 1 => {
                "Type a symbol name"
            }
            Some(_) => "No symbols",
            None if self.kind == PickerKind::File && self.files.is_none() => "Loading files",
            None => "No matches",
        }
    }

    fn document_symbol_rows(&self, query: &str) -> Vec<PickerRow> {
        let (Some(index), Some(document)) = (&self.symbols, &self.document) else {
            return Vec::new();
        };
        if self.head.as_ref() != Some(&document.revision) {
            return Vec::new();
        }
        let Some(file) = index.file(&document.path) else {
            return Vec::new();
        };
        let symbols = &file.symbols;
        let ranked: Vec<(usize, Vec<(usize, usize)>)> = if query.is_empty() {
            (0..symbols.len()).map(|i| (i, Vec::new())).collect()
        } else {
            let names: Vec<&str> = symbols.iter().map(|symbol| &*symbol.name).collect();
            let config = symbol_config(query);
            let mut matcher = Matcher::new(query, &config);
            let mut found = matcher.match_list(&names);
            found.sort_by(|a, b| {
                b.score.cmp(&a.score).then(
                    symbols[a.index as usize]
                        .line
                        .cmp(&symbols[b.index as usize].line),
                )
            });
            found
                .into_iter()
                .map(|m| {
                    let i = m.index as usize;
                    (i, name_highlights(&mut matcher, names[i]))
                })
                .collect()
        };
        ranked
            .into_iter()
            .take(SYMBOL_ROWS)
            .map(|(i, highlights)| {
                let symbol = &symbols[i];
                symbol_row(
                    symbol,
                    format!("line {}", symbol.line + 1),
                    &document.path,
                    &document.revision,
                    highlights,
                )
            })
            .collect()
    }

    fn workspace_symbol_rows(&self, query: &str) -> Vec<PickerRow> {
        let (Some(index), Some(head)) = (&self.symbols, &self.head) else {
            return Vec::new();
        };
        if query.is_empty() {
            return Vec::new();
        }
        let config = symbol_config(query);
        let mut matcher = Matcher::new(query, &config);
        let names = index.names();
        let mut found = matcher.match_list(names);
        found.sort_by(|a, b| {
            b.score.cmp(&a.score).then(
                names[a.index as usize]
                    .len()
                    .cmp(&names[b.index as usize].len()),
            )
        });
        let mut rows = Vec::new();
        for m in found {
            let name = &names[m.index as usize];
            let highlights = name_highlights(&mut matcher, name);
            for (file, symbol) in index.definitions_of(m.index as usize) {
                rows.push(symbol_row(
                    symbol,
                    format!("{}:{}", file.path, symbol.line + 1),
                    &file.path,
                    head,
                    highlights.clone(),
                ));
                if rows.len() == SYMBOL_ROWS {
                    return rows;
                }
            }
        }
        rows
    }

    pub fn set_candidates(&mut self, candidates: Vec<PickerCandidate>) {
        self.candidates = candidates;
        self.refilter();
    }

    pub fn refilter(&mut self) {
        let query = self.query.text().trim();
        self.rows.clear();
        if let Some(view) = &self.repos {
            self.rows = view.rows(query, &mut self.matcher);
            if !query.is_empty()
                && let Some(typed) = self.typed_choice(query)
            {
                self.rows.push(typed);
            }
            self.selected = self
                .rows
                .iter()
                .position(|r| r.current)
                .or_else(|| self.rows.iter().position(PickerRow::selectable))
                .unwrap_or(0);
            self.scroll_px = 0.0;
            self.pending_reveal = true;
            return;
        }
        if self.kind == PickerKind::File {
            let rows = match self.symbol_mode() {
                Some(SymbolMode::File) => self.document_symbol_rows(query[1..].trim()),
                Some(SymbolMode::Workspace) => self.workspace_symbol_rows(query[1..].trim()),
                None => file_rows(self.files.as_deref().unwrap_or_default(), query),
            };
            self.rows = rows;
            self.selected = 0;
            self.scroll_px = 0.0;
            return;
        }
        if query.is_empty() {
            self.rows
                .extend(self.candidates.iter().map(|c| row(c, Vec::new())));
            self.selected = self.rows.iter().position(|r| r.current).unwrap_or(0);
        } else {
            self.matcher.set_query(query);
            let mut positions = Vec::new();
            let mut scored: Vec<(i32, usize, Vec<(usize, usize)>)> = Vec::new();
            for (i, candidate) in self.candidates.iter().enumerate() {
                positions.clear();
                if let Some(score) = self.matcher.score(&candidate.label, &mut positions) {
                    scored.push((score, i, match_ranges(&candidate.label, &positions)));
                } else if let Some(score) = candidate
                    .alias
                    .as_deref()
                    .and_then(|alias| self.matcher.score(alias, &mut positions))
                {
                    scored.push((score, i, Vec::new()));
                }
            }
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            self.rows.extend(
                scored
                    .into_iter()
                    .map(|(_, i, ranges)| row(&self.candidates[i], ranges)),
            );
            if let Some(typed) = self.typed_choice(query) {
                self.rows.push(typed);
            }
            self.selected = 0;
        }
        self.scroll_px = 0.0;
    }

    fn typed_choice(&self, query: &str) -> Option<PickerRow> {
        let choice = match self.kind {
            PickerKind::Repository => PickerChoice::Repository(expand_path(query)),
            PickerKind::Ref(_) => PickerChoice::Ref(query.to_owned()),
            PickerKind::PullRequest => PickerChoice::PullRequest(query.to_owned()),
            _ => return None,
        };
        if self.rows.iter().any(|r| r.label == query) {
            return None;
        }
        Some(PickerRow {
            label: query.to_owned(),
            meta: None,
            detail: Some(match self.kind {
                PickerKind::Repository => TYPED_PATH.to_owned(),
                PickerKind::PullRequest => "Load this pull request".to_owned(),
                _ => "Use this revision".to_owned(),
            }),
            highlights: Vec::new(),
            choice,
            current: false,
            kind: RowKind::Item,
            branch: None,
            indent: false,
        })
    }

    pub fn reveal_pending(&mut self, row_stride: f32) {
        if self.pending_reveal && row_stride > 0.0 {
            self.pending_reveal = false;
            self.reveal(row_stride);
        }
    }

    pub fn move_selection(&mut self, delta: i32, row_stride: f32) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as i32 - 1;
        let step = delta.signum();
        let mut target = self.selected as i32;
        let mut remaining = delta.abs();
        while remaining > 0 {
            let mut next = target + step;
            while (0..=last).contains(&next) && !self.rows[next as usize].selectable() {
                next += step;
            }
            if !(0..=last).contains(&next) {
                break;
            }
            target = next;
            remaining -= 1;
        }
        self.selected = target as usize;
        self.reveal(row_stride);
    }

    pub fn reveal(&mut self, row_stride: f32) {
        let top = self.selected as f32 * row_stride;
        let viewport = self.visible_rows() as f32 * row_stride;
        if top < self.scroll_px {
            self.scroll_px = top;
        } else if top + row_stride > self.scroll_px + viewport {
            self.scroll_px = top + row_stride - viewport;
        }
    }

    pub fn scroll(&mut self, lines: i32, row_stride: f32) {
        let max = (self.rows.len().saturating_sub(self.visible_rows())) as f32 * row_stride;
        self.scroll_px = (self.scroll_px + lines as f32 * row_stride).clamp(0.0, max.max(0.0));
    }

    pub fn selected_choice(&self) -> Option<&PickerChoice> {
        self.rows.get(self.selected).map(|row| &row.choice)
    }

    pub fn complete_directory(&mut self) -> bool {
        if self.kind != PickerKind::Repository {
            return false;
        }
        let Some(PickerChoice::Repository(path)) = self.selected_choice().cloned() else {
            return false;
        };
        if !path.is_dir() {
            return false;
        }
        let mut text = path.display().to_string();
        if !text.ends_with(std::path::MAIN_SEPARATOR) {
            text.push(std::path::MAIN_SEPARATOR);
        }
        self.query.set_text(text);
        self.refilter();
        true
    }
}

const FILE_ROWS: usize = 500;
const SYMBOL_ROWS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolMode {
    File,
    Workspace,
}

fn symbol_config(query: &str) -> Config {
    Config {
        max_typos: Some(u16::from(query.len() >= FILE_TYPO_MIN_LEN)),
        ..Config::default()
    }
}

fn name_highlights(matcher: &mut Matcher, name: &str) -> Vec<(usize, usize)> {
    matcher
        .match_one_indices(name, 0)
        .map(|found| {
            let mut positions = found.indices;
            positions.sort_unstable();
            match_ranges(name, &positions)
        })
        .unwrap_or_default()
}

fn symbol_row(
    symbol: &Symbol,
    detail: String,
    path: &str,
    revision: &RevisionId,
    highlights: Vec<(usize, usize)>,
) -> PickerRow {
    let target = DefinitionTarget {
        path: path.to_owned(),
        revision: revision.clone(),
        line: symbol.line,
        column: symbol.column,
        len: symbol.name.len() as u32,
        kind: Some(symbol.kind),
    };
    PickerRow {
        label: symbol.name.to_string(),
        detail: Some(detail),
        meta: Some(outline::kind_label(symbol.kind).to_owned()),
        highlights,
        choice: PickerChoice::Action(SymbolsAction::Jump(target).into()),
        current: false,
        kind: RowKind::Item,
        branch: None,
        indent: false,
    }
}
const FILE_NAME_BONUS: u32 = 16;
const FILE_TYPO_MIN_LEN: usize = 8;

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn file_row(path: &str, highlights: Vec<(usize, usize)>) -> PickerRow {
    let name = file_name(path);
    let dir = &path[..path.len() - name.len()];
    PickerRow {
        label: name.to_owned(),
        detail: Some(dir.trim_end_matches('/').to_owned()).filter(|dir| !dir.is_empty()),
        meta: None,
        highlights,
        choice: PickerChoice::File(path.to_owned()),
        current: false,
        kind: RowKind::Item,
        branch: None,
        indent: false,
    }
}

fn file_rows(paths: &[String], query: &str) -> Vec<PickerRow> {
    if query.is_empty() {
        return paths
            .iter()
            .take(FILE_ROWS)
            .map(|path| file_row(path, Vec::new()))
            .collect();
    }
    let config = Config {
        max_typos: Some(u16::from(query.len() >= FILE_TYPO_MIN_LEN)),
        ..Config::default()
    };
    let mut matcher = Matcher::new(query, &config);
    let mut scores = vec![0u32; paths.len()];
    for found in matcher.match_list(paths) {
        scores[found.index as usize] = u32::from(found.score) + 1;
    }
    let names: Vec<&str> = paths.iter().map(|path| file_name(path)).collect();
    for found in matcher.match_list(&names) {
        let score = &mut scores[found.index as usize];
        *score = (*score).max(u32::from(found.score) + 1 + FILE_NAME_BONUS);
    }
    let mut ranked: Vec<(u32, usize)> = scores
        .iter()
        .enumerate()
        .filter(|(_, score)| **score > 0)
        .map(|(index, score)| (*score, index))
        .collect();
    ranked.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(paths[a.1].len().cmp(&paths[b.1].len())));
    ranked
        .into_iter()
        .take(FILE_ROWS)
        .map(|(_, index)| {
            let path = &paths[index];
            let name = file_name(path);
            let highlights = matcher
                .match_one_indices(name, 0)
                .map(|found| {
                    let mut positions = found.indices;
                    positions.sort_unstable();
                    match_ranges(name, &positions)
                })
                .unwrap_or_default();
            file_row(path, highlights)
        })
        .collect()
}

fn row(candidate: &PickerCandidate, highlights: Vec<(usize, usize)>) -> PickerRow {
    PickerRow {
        label: candidate.label.clone(),
        detail: candidate.detail.clone(),
        meta: candidate.meta.clone(),
        highlights,
        choice: candidate.choice.clone(),
        current: candidate.current,
        kind: RowKind::Item,
        branch: None,
        indent: false,
    }
}

pub fn expand_path(query: &str) -> PathBuf {
    match query.strip_prefix('~') {
        Some(rest) => match dirs::home_dir() {
            Some(home) => home.join(rest.trim_start_matches(['/', '\\'])),
            None => PathBuf::from(query),
        },
        None => PathBuf::from(query),
    }
}

pub fn font_candidates(entries: &[FontFamilyEntry], current: &str) -> Vec<PickerCandidate> {
    entries
        .iter()
        .map(|entry| {
            PickerCandidate::new(
                entry.label.clone(),
                PickerChoice::Font(entry.family.clone()),
            )
            .detail(entry.source.label())
            .current(entry.family == current)
        })
        .collect()
}

pub fn compare_menu_candidates(mode: CompareMode) -> Vec<PickerCandidate> {
    let action = |a: NavigationAction| PickerChoice::Action(a.into());
    let modes = [
        (
            CompareMode::ThreeDot,
            "Three-dot compare",
            "Changes since the merge base",
        ),
        (
            CompareMode::TwoDot,
            "Two-dot compare",
            "Direct difference between refs",
        ),
        (
            CompareMode::SingleCommit,
            "Single commit",
            "Changes introduced by one commit",
        ),
    ];
    let mut candidates: Vec<PickerCandidate> = modes
        .into_iter()
        .map(|(m, label, detail)| {
            PickerCandidate::new(label, action(NavigationAction::SetCompareMode(m)))
                .detail(detail)
                .current(m == mode)
        })
        .collect();
    candidates.extend([
        PickerCandidate::new("Working tree", action(NavigationAction::ShowWorkingTree))
            .detail("Uncommitted changes"),
        PickerCandidate::new("Swap refs", action(NavigationAction::SwapRefs)),
        PickerCandidate::new(
            "Choose base ref",
            PickerChoice::Action(
                crate::contracts::commands::CommandsAction::OpenRefPicker(CompareField::Left)
                    .into(),
            ),
        ),
        PickerCandidate::new(
            "Choose head ref",
            PickerChoice::Action(
                crate::contracts::commands::CommandsAction::OpenRefPicker(CompareField::Right)
                    .into(),
            ),
        ),
        PickerCandidate::new("Start compare", action(NavigationAction::StartCompare)),
    ]);
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates() -> Vec<PickerCandidate> {
        ["alpha", "beta", "gamma"]
            .into_iter()
            .map(|name| PickerCandidate::new(name, PickerChoice::Ref(name.to_owned())))
            .collect()
    }

    #[test]
    fn filtering_ranks_matches_and_offers_the_typed_revision() {
        let mut picker = PickerState::new(PickerKind::Ref(CompareField::Left), candidates());
        assert_eq!(picker.rows.len(), 3);
        picker.query.set_text("bet");
        picker.refilter();
        assert_eq!(picker.rows[0].label, "beta");
        assert_eq!(picker.rows.last().map(|r| r.label.as_str()), Some("bet"));
        picker.query.set_text("zz9");
        picker.refilter();
        assert_eq!(picker.rows.len(), 1);
        assert_eq!(
            picker.selected_choice(),
            Some(&PickerChoice::Ref("zz9".to_owned()))
        );
    }

    #[test]
    fn selection_clamps_and_scrolls_into_view() {
        let rows = (0..20)
            .map(|i| PickerCandidate::new(format!("ref{i}"), PickerChoice::Ref(i.to_string())))
            .collect();
        let mut picker = PickerState::new(PickerKind::Theme, rows);
        picker.move_selection(-3, 10.0);
        assert_eq!(picker.selected, 0);
        picker.move_selection(12, 10.0);
        assert_eq!(picker.selected, 12);
        assert_eq!(picker.scroll_px, 40.0);
        picker.move_selection(100, 10.0);
        assert_eq!(picker.selected, 19);
    }
}
