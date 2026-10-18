use std::path::{Path, PathBuf};

use quark_components::PickerItem;
use quark_ui::text_input::TextField;

use crate::contracts::navigation::{CompareField, NavigationAction};
use crate::contracts::shell::Action;
use crate::core::compare::CompareMode;
use crate::core::search::fuzzy::fuzzy_score;
use crate::fonts::{FontFamilyEntry, FontRole};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Repository,
    Ref(CompareField),
    Theme,
    Font(FontRole),
    CompareMenu,
    PullRequest,
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
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Repository => "Search recent or type a path",
            Self::Ref(_) => "Search refs or type a revision",
            Self::Theme => "Search themes",
            Self::Font(_) => "Search fonts",
            Self::CompareMenu => "Search compare options",
            Self::PullRequest => "Paste a GitHub pull request URL",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PickerChoice {
    Repository(PathBuf),
    Ref(String),
    Theme(String),
    Font(String),
    Action(Action),
    PullRequest(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PickerRow {
    pub label: String,
    pub detail: Option<String>,
    pub highlights: Vec<(usize, usize)>,
    pub choice: PickerChoice,
    pub current: bool,
}

impl PickerItem for PickerRow {
    fn label(&self) -> &str {
        &self.label
    }

    fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    fn highlight_ranges(&self) -> &[(usize, usize)] {
        &self.highlights
    }

    fn rhs(&self) -> Option<&str> {
        self.current.then_some("Current")
    }
}

#[derive(Debug, Clone)]
pub struct PickerCandidate {
    pub label: String,
    pub detail: Option<String>,
    pub choice: PickerChoice,
    pub current: bool,
}

impl PickerCandidate {
    pub fn new(label: impl Into<String>, choice: PickerChoice) -> Self {
        Self {
            label: label.into(),
            detail: None,
            choice,
            current: false,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
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
}

pub const PICKER_VISIBLE_ROWS: usize = 9;

impl PickerState {
    pub fn new(kind: PickerKind, candidates: Vec<PickerCandidate>) -> Self {
        let mut state = Self {
            kind,
            query: TextField::new(""),
            candidates,
            rows: Vec::new(),
            selected: 0,
            scroll_px: 0.0,
        };
        state.refilter();
        state
    }

    pub fn set_candidates(&mut self, candidates: Vec<PickerCandidate>) {
        self.candidates = candidates;
        self.refilter();
    }

    pub fn refilter(&mut self) {
        let query = self.query.text().trim();
        self.rows.clear();
        if query.is_empty() {
            self.rows
                .extend(self.candidates.iter().map(|c| row(c, Vec::new())));
            self.selected = self.rows.iter().position(|r| r.current).unwrap_or(0);
        } else {
            let mut scored: Vec<(i32, usize)> = self
                .candidates
                .iter()
                .enumerate()
                .filter_map(|(i, c)| fuzzy_score(query, &c.label).map(|s| (s, i)))
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            self.rows.extend(scored.into_iter().map(|(_, i)| {
                row(
                    &self.candidates[i],
                    prefix_highlight(query, &self.candidates[i].label),
                )
            }));
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
            detail: Some(match self.kind {
                PickerKind::Repository => "Open this path".to_owned(),
                PickerKind::PullRequest => "Load this pull request".to_owned(),
                _ => "Use this revision".to_owned(),
            }),
            highlights: Vec::new(),
            choice,
            current: false,
        })
    }

    pub fn move_selection(&mut self, delta: i32, row_stride: f32) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as i32 - 1;
        self.selected = (self.selected as i32 + delta).clamp(0, last) as usize;
        self.reveal(row_stride);
    }

    pub fn reveal(&mut self, row_stride: f32) {
        let top = self.selected as f32 * row_stride;
        let viewport = PICKER_VISIBLE_ROWS as f32 * row_stride;
        if top < self.scroll_px {
            self.scroll_px = top;
        } else if top + row_stride > self.scroll_px + viewport {
            self.scroll_px = top + row_stride - viewport;
        }
    }

    pub fn scroll(&mut self, lines: i32, row_stride: f32) {
        let max = (self.rows.len().saturating_sub(PICKER_VISIBLE_ROWS)) as f32 * row_stride;
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

fn row(candidate: &PickerCandidate, highlights: Vec<(usize, usize)>) -> PickerRow {
    PickerRow {
        label: candidate.label.clone(),
        detail: candidate.detail.clone(),
        highlights,
        choice: candidate.choice.clone(),
        current: candidate.current,
    }
}

fn prefix_highlight(query: &str, label: &str) -> Vec<(usize, usize)> {
    let lower = label.to_lowercase();
    match lower.find(&query.to_lowercase()) {
        Some(start) if lower.len() == label.len() => vec![(start, start + query.len())],
        _ => Vec::new(),
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

pub fn repository_candidates(recent: &[PathBuf], current: Option<&Path>) -> Vec<PickerCandidate> {
    recent
        .iter()
        .map(|path| {
            let label = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            PickerCandidate::new(label, PickerChoice::Repository(path.clone()))
                .detail(path.display().to_string())
                .current(current == Some(path.as_path()))
        })
        .collect()
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
