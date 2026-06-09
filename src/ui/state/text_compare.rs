use crate::actions::TextCompareAction;
use crate::effects::Effect;

use super::*;

pub(super) fn reduce_action(state: &mut AppState, action: TextCompareAction) -> Vec<Effect> {
    state.apply_text_compare_action(action)
}

impl AppState {
    pub(super) fn apply_text_compare_action(&mut self, action: TextCompareAction) -> Vec<Effect> {
        match action {
            TextCompareAction::SetView(view) => {
                self.text_compare.view = view;
                if view == TextCompareView::Diff {
                    self.set_focus(Some(FocusTarget::Editor));
                }
                Vec::new()
            }
            TextCompareAction::SwapSides => {
                std::mem::swap(
                    &mut self.text_compare.left_editor,
                    &mut self.text_compare.right_editor,
                );
                self.mark_text_compare_dirty();
                Vec::new()
            }
            TextCompareAction::ClearSide(side) => {
                match side {
                    TextCompareSide::Left => {
                        self.text_compare.left_editor.request_clear();
                        self.set_focus(Some(FocusTarget::TextCompareLeft));
                    }
                    TextCompareSide::Right => {
                        self.text_compare.right_editor.request_clear();
                        self.set_focus(Some(FocusTarget::TextCompareRight));
                    }
                }
                self.mark_text_compare_dirty();
                Vec::new()
            }
            TextCompareAction::CompareNow => self.kickoff_text_compare(),
            TextCompareAction::SetLanguage(language) => {
                if self.text_compare.language != language {
                    self.text_compare.language = language;
                    self.mark_text_compare_dirty();
                }
                Vec::new()
            }
        }
    }

    pub(super) fn new_text_compare(&mut self) -> Vec<Effect> {
        self.workspace_clear_compare();
        self.text_compare = TextCompareState::default();
        self.workspace
            .source
            .set(&self.store, WorkspaceSource::TextCompare);
        self.workspace.status.set(&self.store, AsyncStatus::Ready);
        self.workspace.mode.set(&self.store, WorkspaceMode::Ready);
        self.workspace.compare_progress.set(&self.store, None);
        self.github.pull_request.active.set(&self.store, None);
        self.github
            .pull_request
            .review_composer
            .set(&self.store, ReviewCommentComposerState::default());
        self.review_comment_editor.request_clear();
        self.editor_clear_document();
        self.clear_overlays();
        self.set_focus(Some(FocusTarget::TextCompareLeft));
        vec![self.invalidate_syntax_epoch_effect()]
    }

    pub(super) fn mark_text_compare_dirty(&mut self) {
        self.sync_text_compare_syntax_paths();
        self.text_compare.generation = self.text_compare.generation.saturating_add(1);
        self.text_compare.status = AsyncStatus::Idle;
        if self.workspace.source.get(&self.store) == WorkspaceSource::TextCompare {
            self.workspace.status.set(&self.store, AsyncStatus::Ready);
        }
    }

    pub(super) fn sync_text_compare_syntax_paths(&mut self) {
        let detected = detect_text_compare_language(
            self.text_compare.left_editor.text_str(),
            self.text_compare.right_editor.text_str(),
        );
        self.text_compare.detected_language = detected;
        let effective = match self.text_compare.language {
            TextCompareLanguage::Auto => detected.unwrap_or(TextCompareLanguage::PlainText),
            language => language,
        };
        let path = effective.scratch_path().to_owned();
        self.text_compare.path_hint = path.clone();
        self.text_compare.left_editor.set_syntax_path(path.clone());
        self.text_compare.right_editor.set_syntax_path(path);
    }

    fn kickoff_text_compare(&mut self) -> Vec<Effect> {
        if self.workspace.source.get(&self.store) != WorkspaceSource::TextCompare {
            self.workspace
                .source
                .set(&self.store, WorkspaceSource::TextCompare);
        }
        // Text and repo compares share one workspace-wide generation space:
        // `CompareScheduler`'s epoch is a monotonic high-water mark, so seed
        // the bump from whichever counter is ahead. Deriving it from
        // `text_compare.generation` alone would rewind
        // `workspace.compare_generation` below the scheduler epoch and every
        // later repo file/stats job would be silently dropped as stale.
        let generation = self
            .text_compare
            .generation
            .max(self.workspace.compare_generation.get(&self.store))
            .saturating_add(1);
        self.text_compare.generation = generation;
        self.text_compare.status = AsyncStatus::Loading;
        self.workspace
            .compare_generation
            .set(&self.store, generation);
        self.workspace.status.set(&self.store, AsyncStatus::Loading);
        self.workspace.mode.set(&self.store, WorkspaceMode::Ready);
        self.workspace.active_file_loading.set(&self.store, None);
        self.workspace.compare_progress.set(&self.store, None);
        self.clear_overlays();
        self.sync_text_compare_syntax_paths();

        let request = TextCompareRequest {
            left_text: self.text_compare.left_editor.text(),
            right_text: self.text_compare.right_editor.text(),
            display_path: self.text_compare.path_hint.clone(),
            renderer: self.compare.renderer.get(&self.store),
            layout: self.compare.layout.get(&self.store),
        };

        vec![
            self.invalidate_syntax_epoch_effect(),
            CompareEffect::RunText(Task {
                generation,
                request,
            })
            .into(),
        ]
    }

    pub(super) fn handle_text_compare_finished(
        &mut self,
        payload: TextCompareFinished,
    ) -> Vec<Effect> {
        // Drop results superseded by a newer text compare (text generation
        // moved on) or by any newer workspace compare (repo compare, cancel,
        // or repo open bumped `compare_generation` past us). Rewinding the
        // workspace generation here would strand it below the scheduler's
        // monotonic epoch.
        if payload.generation != self.text_compare.generation
            || payload.generation != self.workspace.compare_generation.get(&self.store)
        {
            return Vec::new();
        }

        self.text_compare.status = AsyncStatus::Ready;
        self.text_compare.last_compared_generation = Some(payload.generation);
        self.text_compare.path_hint = payload.display_path.clone();
        self.text_compare.view = TextCompareView::Diff;
        self.workspace
            .source
            .set(&self.store, WorkspaceSource::TextCompare);
        self.workspace.status.set(&self.store, AsyncStatus::Ready);
        self.workspace.mode.set(&self.store, WorkspaceMode::Ready);
        self.compare.layout.set(&self.store, payload.layout);
        self.compare.renderer.set(&self.store, payload.renderer);
        self.compare
            .resolved_left
            .set(&self.store, Some("Original".to_owned()));
        self.compare
            .resolved_right
            .set(&self.store, Some("Changed".to_owned()));
        self.workspace
            .raw_diff_len
            .set(&self.store, payload.output.raw_diff_len);
        self.workspace
            .used_fallback
            .set(&self.store, payload.output.used_fallback);
        self.workspace
            .fallback_message
            .set(&self.store, payload.output.fallback_message.clone());
        let stats_snapshot = compare_output_stats_snapshot(&payload.output);
        self.workspace
            .compare_total_stats
            .set(&self.store, Some(stats_snapshot.hydrated_total));
        self.workspace.compare_hydrated_stats.set(&self.store, None);
        self.workspace
            .compare_deferred_stats_remaining
            .set(&self.store, Some(0));
        self.workspace
            .compare_deferred_stats_cursor
            .set(&self.store, 0);
        self.workspace
            .compare_total_stats_loading
            .set(&self.store, false);
        self.set_compare_stats_hydration(CompareStatsHydrationState::Idle);
        self.workspace
            .compare_history_pending
            .set(&self.store, None);
        self.workspace.range_commits.set(&self.store, Vec::new());
        self.workspace.pre_drill_compare.set(&self.store, None);
        self.workspace.files.set(&self.store, Vec::new());
        self.workspace
            .compare_output
            .set(&self.store, Some(payload.output));
        self.clear_file_cache();
        self.reset_file_scroll_layout();
        self.workspace.global_scroll_top_px.set(&self.store, 0);
        self.workspace.expansions.update(&self.store, |m| m.clear());
        self.editor_clear_document();

        let mut effects = Vec::new();
        if self.workspace_file_count() > 0 {
            effects.extend(self.select_file(0, true));
            self.set_focus(Some(FocusTarget::Editor));
        } else {
            self.workspace.selected_file_index.set(&self.store, None);
            self.workspace.selected_file_path.set(&self.store, None);
            self.workspace.selected_change_bucket.set(&self.store, None);
            self.workspace.active_file.set(&self.store, None);
            self.workspace.active_file_loading.set(&self.store, None);
        }

        let (used_fallback, fallback_message) = (
            self.workspace.used_fallback.get(&self.store),
            self.workspace.fallback_message.get(&self.store),
        );
        if used_fallback && !fallback_message.is_empty() {
            self.push_info(&fallback_message);
        }
        effects
    }

    pub(super) fn handle_text_compare_failed(
        &mut self,
        generation: u64,
        message: String,
    ) -> Vec<Effect> {
        if generation == self.text_compare.generation {
            self.text_compare.status = AsyncStatus::Failed;
            self.workspace.status.set(&self.store, AsyncStatus::Failed);
            self.text_compare.view = TextCompareView::Edit;
            self.push_error(&message);
        }
        Vec::new()
    }

    pub fn text_compare_is_stale(&self) -> bool {
        self.text_compare.last_compared_generation != Some(self.text_compare.generation)
    }
}

fn detect_text_compare_language(left: &str, right: &str) -> Option<TextCompareLanguage> {
    detect_text_compare_language_one(right).or_else(|| detect_text_compare_language_one(left))
}

fn detect_text_compare_language_one(source: &str) -> Option<TextCompareLanguage> {
    let trimmed = source.trim_start();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.starts_with("#!/") {
        let first_line = trimmed.lines().next().unwrap_or_default();
        if first_line.contains("python") {
            return Some(TextCompareLanguage::Python);
        }
        if first_line.contains("bash") || first_line.contains("sh") || first_line.contains("zsh") {
            return Some(TextCompareLanguage::Shell);
        }
    }

    let lower = trimmed.to_ascii_lowercase();
    if looks_like_json(trimmed) {
        return Some(TextCompareLanguage::Json);
    }
    if lower.contains("fn main(")
        || lower.contains("pub fn ")
        || lower.contains("impl ")
        || lower.contains("let mut ")
        || lower.contains("use std::")
    {
        return Some(TextCompareLanguage::Rust);
    }
    let has_ts_type_annotation =
        (lower.contains("const ") || lower.contains("let ") || lower.contains("function "))
            && (lower.contains(": ") || lower.contains("?:") || lower.contains(" as "));
    if lower.contains("import ") && lower.contains(" from ")
        || lower.contains("export ")
        || lower.contains("interface ")
        || lower.contains("type ") && (lower.contains(" = ") || lower.contains("<"))
        || has_ts_type_annotation
    {
        return Some(TextCompareLanguage::TypeScript);
    }
    if lower.contains("function ") || lower.contains("const ") || lower.contains("let ") {
        return Some(TextCompareLanguage::JavaScript);
    }
    if lower.contains("def ") || lower.contains("class ") && lower.contains(":") {
        return Some(TextCompareLanguage::Python);
    }
    if lower.starts_with("package ") || lower.contains("\nfunc ") {
        return Some(TextCompareLanguage::Go);
    }
    if lower.contains("[package]") || lower.contains("[dependencies]") {
        return Some(TextCompareLanguage::Toml);
    }
    if lower.contains("mkderivation") || lower.contains("with import") {
        return Some(TextCompareLanguage::Nix);
    }
    None
}

fn looks_like_json(source: &str) -> bool {
    let trimmed = source.trim();
    (trimmed.starts_with('{') && trimmed.ends_with('}')
        || trimmed.starts_with('[') && trimmed.ends_with(']'))
        && trimmed.contains(':')
        && trimmed.contains('"')
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextCompareView {
    #[default]
    Edit,
    Diff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextCompareSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextCompareLanguage {
    #[default]
    Auto,
    PlainText,
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Json,
    Toml,
    Shell,
    Nix,
    C,
    Cpp,
    Zig,
}

impl TextCompareLanguage {
    pub const OPTIONS: &'static [Self] = &[
        Self::Auto,
        Self::PlainText,
        Self::Rust,
        Self::TypeScript,
        Self::JavaScript,
        Self::Python,
        Self::Go,
        Self::Json,
        Self::Toml,
        Self::Shell,
        Self::Nix,
        Self::C,
        Self::Cpp,
        Self::Zig,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::PlainText => "Plain text",
            Self::Rust => "Rust",
            Self::TypeScript => "TypeScript",
            Self::JavaScript => "JavaScript",
            Self::Python => "Python",
            Self::Go => "Go",
            Self::Json => "JSON",
            Self::Toml => "TOML",
            Self::Shell => "Shell",
            Self::Nix => "Nix",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Zig => "Zig",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::PlainText => "Text",
            Self::Rust => "Rust",
            Self::TypeScript => "TS",
            Self::JavaScript => "JS",
            Self::Python => "Py",
            Self::Go => "Go",
            Self::Json => "JSON",
            Self::Toml => "TOML",
            Self::Shell => "Sh",
            Self::Nix => "Nix",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Zig => "Zig",
        }
    }

    pub fn scratch_path(self) -> &'static str {
        match self {
            Self::Auto | Self::PlainText => "text.txt",
            Self::Rust => "scratch.rs",
            Self::TypeScript => "scratch.ts",
            Self::JavaScript => "scratch.js",
            Self::Python => "scratch.py",
            Self::Go => "scratch.go",
            Self::Json => "scratch.json",
            Self::Toml => "scratch.toml",
            Self::Shell => "scratch.sh",
            Self::Nix => "scratch.nix",
            Self::C => "scratch.c",
            Self::Cpp => "scratch.cpp",
            Self::Zig => "scratch.zig",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TextCompareState {
    pub left_editor: Editor,
    pub right_editor: Editor,
    pub language: TextCompareLanguage,
    pub detected_language: Option<TextCompareLanguage>,
    pub path_hint: String,
    pub view: TextCompareView,
    pub generation: u64,
    pub last_compared_generation: Option<u64>,
    pub status: AsyncStatus,
}

impl Default for TextCompareState {
    fn default() -> Self {
        let mut left_editor = Editor::new(EditorMode::CodeInput);
        let mut right_editor = Editor::new(EditorMode::CodeInput);
        left_editor.set_syntax_path("text.txt");
        right_editor.set_syntax_path("text.txt");
        Self {
            left_editor,
            right_editor,
            language: TextCompareLanguage::Auto,
            detected_language: None,
            path_hint: "text.txt".to_owned(),
            view: TextCompareView::default(),
            generation: 0,
            last_compared_generation: None,
            status: AsyncStatus::Idle,
        }
    }
}
