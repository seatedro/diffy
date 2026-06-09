use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use super::{
    ActiveFile, ActiveFileLoading, AppState, AsyncStatus, CarbonStyleOverlays, CardTextSelection,
    CompareField, FILE_HEIGHT_SPARSE_MIN_COUNT, FileHeightIndex, FileListEntry, FocusTarget,
    OverlayEntry, OverlaySurface, PickerItem, PickerLabelStyle, PreparedActiveFile, SidebarMode,
    SidebarTab, TextCompareLanguage, TextCompareView, ViewportAnchorBias, VirtualDiffItemKind,
    WorkspaceMode, WorkspaceSource, prepare_active_file, vcs_compare_request,
};
use crate::core::compare::{
    CompareFileSummary, CompareMode, CompareOutput, LayoutMode, RendererKind,
};
use crate::core::text::TokenBuffer;
use crate::core::vcs::model::{
    ChangeBucket, ChangeFlags, FileChange, FileChangeStatus, JjOperation, RefKind,
    RepoCapabilities, RepoLocation, RevisionId, VcsChange, VcsKind, VcsOperation,
    VcsOperationLogEntry, VcsRef,
};
use crate::editor::EditorMode;
use crate::editor::diff::render_doc::{RenderDoc, build_render_doc_from_carbon};
use crate::effects::{
    AiEffect, CompareEffect, CompareWorkPriority, Effect, GitHubEffect, RepositoryEffect,
    SettingsEffect, SyntaxEffect,
};
use crate::events::{
    AppEvent, CompareEvent, CompareFileFinished, CompareFileStat, CompareFileStatsReady,
    CompareStatsReady, GitHubEvent, RepositoryEvent, TextCompareFinished,
};
use crate::platform::persistence::Settings;
use crate::platform::startup::{Args, StartupOptions};

fn carbon_summary_for_path(index: usize, path: &str) -> carbon::FileDiff {
    carbon::FileDiff {
        id: carbon::FileId(index as u32),
        old_path: Some(path.to_owned()),
        new_path: Some(path.to_owned()),
        is_partial: true,
        ..carbon::FileDiff::default()
    }
}

fn carbon_context_file(index: usize, path: &str, text: &str) -> carbon::FileDiff {
    carbon::parse_unified_patch(&format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n {text}\n"
    ))
    .unwrap()
    .files
    .into_iter()
    .next()
    .map(|mut file| {
        file.id = carbon::FileId(index as u32);
        file
    })
    .unwrap()
}

#[test]
fn new_text_compare_enters_text_workspace_with_left_focus() {
    let mut state = AppState::default();

    state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);

    assert_eq!(
        state.workspace.source.get(&state.store),
        WorkspaceSource::TextCompare
    );
    assert_eq!(state.text_compare.view, TextCompareView::Edit);
    assert_eq!(state.text_compare.left_editor.mode(), EditorMode::CodeInput);
    assert_eq!(
        state.text_compare.right_editor.mode(),
        EditorMode::CodeInput
    );
    assert_eq!(state.text_compare.language, TextCompareLanguage::Auto);
    assert_eq!(state.text_compare.path_hint, "text.txt");
    assert_eq!(
        state.ui.focus.get(&state.store),
        Some(FocusTarget::TextCompareLeft)
    );
}

#[test]
fn text_compare_paste_routes_to_focused_side() {
    let mut state = AppState::default();
    state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);

    state.apply_action(crate::actions::TextEditAction::Paste("left".to_owned()));
    state.apply_action(crate::actions::AppAction::SetFocus(Some(
        FocusTarget::TextCompareRight,
    )));
    state.apply_action(crate::actions::TextEditAction::Paste("right".to_owned()));

    assert_eq!(state.text_compare.left_editor.text(), "left");
    assert_eq!(state.text_compare.right_editor.text(), "right");
    assert_eq!(state.text_compare.left_editor.line_count(), 1);
    assert_eq!(state.text_compare.right_editor.line_count(), 1);
}

#[test]
fn text_compare_auto_language_detects_pasted_rust() {
    let mut state = AppState::default();
    state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);

    state.apply_action(crate::actions::TextEditAction::Paste(
        "pub fn main() {\n    println!(\"hi\");\n}\n".to_owned(),
    ));

    assert_eq!(
        state.text_compare.detected_language,
        Some(TextCompareLanguage::Rust)
    );
    assert_eq!(state.text_compare.path_hint, "scratch.rs");
}

#[test]
fn text_compare_auto_language_detects_pasted_typescript() {
    let mut state = AppState::default();
    state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);

    state.apply_action(crate::actions::TextEditAction::Paste(
        "const answer: number = 42;\nexport { answer };\n".to_owned(),
    ));

    assert_eq!(
        state.text_compare.detected_language,
        Some(TextCompareLanguage::TypeScript)
    );
    assert_eq!(state.text_compare.path_hint, "scratch.ts");
}

#[test]
fn text_compare_language_override_sets_compare_path() {
    let mut state = AppState::default();
    state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);

    state.apply_action(crate::actions::TextCompareAction::SetLanguage(
        TextCompareLanguage::TypeScript,
    ));
    state.apply_action(crate::actions::TextEditAction::Paste(
        "pub fn main() {}\n".to_owned(),
    ));
    let effects = state.apply_action(crate::actions::TextCompareAction::CompareNow);
    let request_path = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Compare(CompareEffect::RunText(task)) => {
                Some(task.request.display_path.as_str())
            }
            _ => None,
        })
        .unwrap();

    assert_eq!(state.text_compare.language, TextCompareLanguage::TypeScript);
    assert_eq!(state.text_compare.path_hint, "scratch.ts");
    assert_eq!(request_path, "scratch.ts");
}

#[test]
fn text_compare_swap_sides_preserves_text_and_marks_stale() {
    let mut state = AppState::default();
    state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);
    state.text_compare.left_editor.set_text("old");
    state.text_compare.right_editor.set_text("new");
    let generation = state.text_compare.generation;

    state.apply_action(crate::actions::TextCompareAction::SwapSides);

    assert_eq!(state.text_compare.left_editor.text(), "new");
    assert_eq!(state.text_compare.right_editor.text(), "old");
    assert!(state.text_compare.generation > generation);
    assert!(state.text_compare_is_stale());
}

#[test]
fn stale_text_compare_finished_event_is_ignored() {
    let mut state = AppState::default();
    state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);
    let effects = state.apply_action(crate::actions::TextCompareAction::CompareNow);
    let generation = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Compare(CompareEffect::RunText(task)) => Some(task.generation),
            _ => None,
        })
        .unwrap();
    state.apply_action(crate::actions::TextEditAction::Paste("newer".to_owned()));

    state.apply_event(AppEvent::from(CompareEvent::TextCompareFinished(
        TextCompareFinished {
            generation,
            display_path: "text.txt".to_owned(),
            renderer: RendererKind::Builtin,
            layout: LayoutMode::Unified,
            output: CompareOutput::default(),
        },
    )));

    assert!(state.workspace.compare_output.get(&state.store).is_none());
    assert_eq!(state.text_compare.view, TextCompareView::Edit);
}

// Regression test: `CompareScheduler` keeps a monotonic epoch high-water
// mark, so a text compare that rewinds `workspace.compare_generation` below
// it makes every later repo file/stats job get dropped silently (perpetual
// "Loading diff..."). Text compares must bump the shared counter forward.
#[test]
fn text_compare_generation_never_rewinds_workspace_generation() {
    let mut state = AppState::default();
    // Simulate prior repo compares having advanced the shared counter (and
    // with it the scheduler epoch).
    state.workspace.compare_generation.set(&state.store, 5);
    state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);
    let effects = state.apply_action(crate::actions::TextCompareAction::CompareNow);
    let generation = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Compare(CompareEffect::RunText(task)) => Some(task.generation),
            _ => None,
        })
        .unwrap();

    assert!(generation > 5);
    assert_eq!(
        state.workspace.compare_generation.get(&state.store),
        generation
    );
    assert_eq!(state.text_compare.generation, generation);
}

#[test]
fn text_compare_finished_installs_diff_view() {
    let mut state = AppState::default();
    state.apply_action(crate::actions::WorkspaceAction::NewTextCompare);
    let generation = state.text_compare.generation.saturating_add(1);
    state.text_compare.generation = generation;
    state
        .workspace
        .compare_generation
        .set(&state.store, generation);
    let output = crate::core::compare::compare_text(
        "old\n",
        "new\n",
        "text.txt",
        RendererKind::Builtin,
        LayoutMode::Unified,
    )
    .unwrap();

    state.apply_event(AppEvent::from(CompareEvent::TextCompareFinished(
        TextCompareFinished {
            generation,
            display_path: "text.txt".to_owned(),
            renderer: RendererKind::Builtin,
            layout: LayoutMode::Unified,
            output,
        },
    )));

    assert_eq!(state.text_compare.view, TextCompareView::Diff);
    assert_eq!(
        state.workspace.source.get(&state.store),
        WorkspaceSource::TextCompare
    );
    assert!(state.workspace.active_file.get(&state.store).is_some());
}

fn status_state_with_two_hunks() -> AppState {
    let state = AppState::default();
    let repo_path = PathBuf::from("/repo");
    let path = "src/lib.rs".to_owned();
    let token_buffer = TokenBuffer::default();
    let carbon_file = carbon::parse_unified_patch(
        "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,2 @@
 fn one() {
-    old_first();
 }
@@ -8,3 +7,2 @@
 fn two() {
-    old_second();
 }
",
    )
    .unwrap()
    .files
    .into_iter()
    .next()
    .unwrap();
    let carbon_expansion = carbon::ExpansionState::default();
    let render_doc = build_render_doc_from_carbon(
        &carbon_file,
        0,
        &carbon_expansion,
        &CarbonStyleOverlays::default(),
        &token_buffer,
    );
    let (left_ref, right_ref) =
        crate::ui::vcs::profile(None).status_compare_refs(ChangeBucket::Unstaged);

    state.compare.repo_path.set(&state.store, Some(repo_path));
    state
        .repository
        .capabilities
        .set(&state.store, Some(RepoCapabilities::git()));
    state
        .workspace
        .source
        .set(&state.store, WorkspaceSource::Status);
    state.workspace.status.set(&state.store, AsyncStatus::Ready);
    state
        .workspace
        .status_operation_pending
        .set(&state.store, false);
    state.workspace.mode.set(&state.store, WorkspaceMode::Ready);
    state.workspace.files.set(
        &state.store,
        vec![FileListEntry {
            path: path.as_str().into(),
        }],
    );
    state.workspace.status_file_changes.set(
        &state.store,
        vec![FileChange {
            path: path.clone(),
            old_path: None,
            status: FileChangeStatus::Modified,
            bucket: ChangeBucket::Unstaged,
        }],
    );
    state
        .workspace
        .selected_file_index
        .set(&state.store, Some(0));
    state
        .workspace
        .selected_file_path
        .set(&state.store, Some(path.clone()));
    state
        .workspace
        .selected_change_bucket
        .set(&state.store, Some(ChangeBucket::Unstaged));
    state.workspace.active_file.set(
        &state.store,
        Some(ActiveFile {
            index: 0,
            path,
            carbon_file: Arc::new(carbon_file.clone()),
            carbon_expansion,
            carbon_overlays: CarbonStyleOverlays::default(),
            render_doc: Arc::new(render_doc),
            token_buffer,
            left_ref,
            right_ref,
            file_line_count: None,
            old_file_lines: None,
            file_lines: None,
            syntax_pending: Vec::new(),
            syntax_covered: Vec::new(),
            last_used_tick: 0,
        }),
    );

    state
}

fn loaded_state_with_files(paths: &[&str]) -> AppState {
    let state = AppState::default();
    let carbon_files: Vec<carbon::FileDiff> = paths
        .iter()
        .enumerate()
        .map(|(index, path)| carbon_context_file(index, path, "loaded"))
        .collect();
    let entries: Vec<FileListEntry> = carbon_files
        .iter()
        .map(|file| FileListEntry {
            path: file.path().into(),
        })
        .collect();

    state.workspace.compare_output.set(
        &state.store,
        Some(CompareOutput {
            carbon: carbon::DiffDocument {
                files: carbon_files,
            },
            ..CompareOutput::default()
        }),
    );
    state
        .workspace
        .source
        .set(&state.store, WorkspaceSource::Compare);
    state.workspace.files.set(&state.store, entries);
    state.workspace.mode.set(&state.store, WorkspaceMode::Ready);
    state.file_list.row_height.set(&state.store, 36.0);
    state.file_list.gap.set(&state.store, 4.0);
    state.file_list.viewport_height.set(&state.store, 80.0);
    state
}

#[test]
fn bootstrap_with_no_repo_starts_empty_workspace() {
    let startup = StartupOptions::from_parts(
        Args::parse_from(["diffy"]),
        None,
        "client".to_owned(),
        false,
    );

    let (state, effects) = AppState::bootstrap(startup, Settings::default());
    assert_eq!(state.workspace.mode.get(&state.store), WorkspaceMode::Empty);
    assert_eq!(
        state.ui.focus.get(&state.store),
        Some(FocusTarget::WorkspacePrimaryButton)
    );
    assert!(effects.iter().all(|e| matches!(
        e,
        Effect::GitHub(GitHubEffect::LoadGitHubToken)
            | Effect::Ai(AiEffect::LoadAiKeys)
            | Effect::Syntax(SyntaxEffect::InstallCommonSyntaxPacks)
    )));
}

#[test]
fn bootstrap_with_repo_starts_repo_sync() {
    let startup = StartupOptions::from_parts(
        Args {
            repo: Some("C:\\repo".into()),
            left: Some("main".to_owned()),
            right: None,
            compare_mode: Some(CompareMode::TwoDot),
            layout: Some(LayoutMode::Unified),
            renderer: Some(RendererKind::Builtin),
            file_index: None,
            file_path: None,
            open_pr: None,
        },
        None,
        "client".to_owned(),
        false,
    );

    let (state, effects) = AppState::bootstrap(startup, Settings::default());
    assert_eq!(state.workspace.mode.get(&state.store), WorkspaceMode::Empty);
    assert_eq!(state.active_overlay_name(), None);
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(
                e,
                Effect::Repository(RepositoryEffect::SyncRepository { .. })
                    | Effect::Repository(RepositoryEffect::WatchRepository { .. })
            ))
            .count(),
        2
    );
}

#[test]
fn overlay_close_restores_prior_focus() {
    let startup = StartupOptions::from_parts(
        Args::parse_from(["diffy"]),
        None,
        "client".to_owned(),
        false,
    );
    let (mut state, _) = AppState::bootstrap(startup, Settings::default());
    state.apply_action(crate::actions::AppAction::SetFocus(Some(
        FocusTarget::TitleBar,
    )));
    state.apply_action(crate::actions::OverlayAction::OpenCommandPalette);
    assert_eq!(state.overlays_top(), Some(OverlaySurface::CommandPalette));
    state.apply_action(crate::actions::OverlayAction::CloseOverlay);
    assert_eq!(
        state.ui.focus.get(&state.store),
        Some(FocusTarget::TitleBar)
    );
}

#[test]
fn pixel_scroll_actions_clamp_file_list_and_viewport() {
    let mut state = AppState::default();

    state.workspace.files.set(
        &state.store,
        vec![
            FileListEntry {
                path: "a.rs".into(),
            },
            FileListEntry {
                path: "b.rs".into(),
            },
            FileListEntry {
                path: "c.rs".into(),
            },
            FileListEntry {
                path: "d.rs".into(),
            },
            FileListEntry {
                path: "e.rs".into(),
            },
        ],
    );
    state.file_list.row_height.set(&state.store, 36.0);
    state.file_list.gap.set(&state.store, 4.0);
    state.file_list.viewport_height.set(&state.store, 80.0);

    state.apply_action(crate::actions::FileListAction::ScrollFileListPx(50));
    assert_eq!(state.file_list.scroll_offset_px.get(&state.store), 50.0);

    state.apply_action(crate::actions::FileListAction::ScrollFileListPx(500));
    assert_eq!(state.file_list.scroll_offset_px.get(&state.store), 116.0);

    state.apply_action(crate::actions::FileListAction::ScrollFileListPx(-500));
    assert_eq!(state.file_list.scroll_offset_px.get(&state.store), 0.0);

    state.editor.content_height_px.set(&state.store, 600);
    state.editor.viewport_height_px.set(&state.store, 200);

    state.apply_action(crate::actions::EditorAction::ScrollViewportPx(75));
    assert_eq!(state.editor.scroll_top_px.get(&state.store), 75);

    state.apply_action(crate::actions::EditorAction::ScrollViewportPx(500));
    assert_eq!(state.editor.scroll_top_px.get(&state.store), 400);

    state.apply_action(crate::actions::EditorAction::ScrollViewportPx(-500));
    assert_eq!(state.editor.scroll_top_px.get(&state.store), 0);
}

#[test]
fn file_height_index_keeps_uniform_large_lists_sparse() {
    let mut index = FileHeightIndex::default();
    index.rebuild(vec![192; FILE_HEIGHT_SPARSE_MIN_COUNT + 1]);

    assert_eq!(index.len(), FILE_HEIGHT_SPARSE_MIN_COUNT + 1);
    assert_eq!(
        index.total_u32(),
        ((FILE_HEIGHT_SPARSE_MIN_COUNT + 1) as u32) * 192
    );
    assert!(matches!(index, FileHeightIndex::Sparse { .. }));
    assert_eq!(index.locate(192 * 7 + 12), Some((7, 12)));
}

#[test]
fn sparse_file_height_index_updates_prefix_and_locate() {
    let mut index = FileHeightIndex::default();
    index.rebuild(vec![100; FILE_HEIGHT_SPARSE_MIN_COUNT + 2]);
    index.update(3, 250);
    index.update(7, 40);

    assert_eq!(index.prefix_u32(4), 550);
    assert_eq!(index.prefix_u32(8), 890);
    assert_eq!(index.locate(549), Some((3, 249)));
    assert_eq!(index.locate(550), Some((4, 0)));
    assert_eq!(index.locate(849), Some((6, 99)));
    assert_eq!(index.locate(850), Some((7, 0)));
    assert_eq!(index.locate(889), Some((7, 39)));
    assert_eq!(index.locate(890), Some((8, 0)));
}

#[test]
fn clicking_a_visible_file_does_not_force_sidebar_reveal() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
    state.file_list.scroll_offset_px.set(&state.store, 10.0);

    state.apply_action(crate::actions::FileListAction::SelectFile(0));

    assert_eq!(
        state.workspace.selected_file_index.get(&state.store),
        Some(0)
    );
    assert_eq!(state.file_list.scroll_offset_px.get(&state.store), 10.0);
}

#[test]
fn keyboard_file_navigation_still_reveals_selection() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs", "d.rs"]);
    state
        .workspace
        .selected_file_index
        .set(&state.store, Some(0));
    state
        .workspace
        .selected_file_path
        .set(&state.store, Some("a.rs".into()));
    state.file_list.scroll_offset_px.set(&state.store, 50.0);

    state.apply_action(crate::actions::FileListAction::SelectNextFile);

    assert_eq!(
        state.workspace.selected_file_index.get(&state.store),
        Some(1)
    );
    assert_eq!(state.file_list.scroll_offset_px.get(&state.store), 40.0);
}

#[test]
fn next_file_action_selects_adjacent_file_in_single_file_mode() {
    let mut state = loaded_state_with_files(&["src/ui/state/mod.rs", "src/ui/state/text_edit.rs"]);
    state.apply_action(crate::actions::FileListAction::SelectFile(0));

    state.apply_action(crate::actions::EditorAction::GoToNextFile);
    state.sync_editor_scroll_from_global();

    assert_eq!(
        state.workspace.selected_file_index.get(&state.store),
        Some(1)
    );
    assert_eq!(
        state
            .workspace
            .selected_file_path
            .get(&state.store)
            .as_deref(),
        Some("src/ui/state/text_edit.rs")
    );
    assert_eq!(
        state
            .workspace
            .active_file
            .get(&state.store)
            .as_ref()
            .map(|file| file.path.as_str()),
        Some("src/ui/state/text_edit.rs")
    );
    assert_eq!(state.workspace.global_scroll_top_px.get(&state.store), 0);
}

#[test]
fn next_file_action_selects_next_file_when_tail_is_short() {
    let mut state = loaded_state_with_files(&["src/ui/state/mod.rs", "src/ui/state/text_edit.rs"]);
    state.settings.continuous_scroll = true;
    state.editor.viewport_height_px.set(&state.store, 10_000);
    state.apply_action(crate::actions::FileListAction::SelectFile(0));

    state.apply_action(crate::actions::EditorAction::GoToNextFile);

    assert_eq!(
        state.workspace.selected_file_index.get(&state.store),
        Some(1)
    );
    assert_eq!(
        state
            .workspace
            .selected_file_path
            .get(&state.store)
            .as_deref(),
        Some("src/ui/state/text_edit.rs")
    );
    assert_eq!(
        state
            .workspace
            .active_file
            .get(&state.store)
            .as_ref()
            .map(|file| file.path.as_str()),
        Some("src/ui/state/text_edit.rs")
    );
}

#[test]
fn continuous_scroll_keeps_short_tail_at_natural_bottom() {
    let state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
    state.editor.viewport_height_px.set(&state.store, 10_000);

    assert_eq!(state.global_max_scroll_top_px(), 0);
}

#[test]
fn continuous_scroll_first_height_measurement_keeps_total_cache_in_sync_with_index() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
    state.settings.continuous_scroll = true;
    state.recompute_file_scroll_total_height_px();

    assert_eq!(state.workspace.measured_px_per_row_q16.get(&state.store), 0);

    assert!(state.update_file_content_height_px(0, 1_200));

    assert_eq!(
        state
            .workspace
            .file_scroll_total_height_px
            .get(&state.store),
        state.virtual_diff_document.total_u32()
    );
}

#[test]
fn continuous_scroll_keeps_bottom_anchor_when_visible_file_height_grows() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
    state.settings.continuous_scroll = true;
    state.editor.viewport_height_px.set(&state.store, 100);
    state
        .workspace
        .file_content_heights
        .set(&state.store, vec![Some(200), Some(200), Some(200)]);
    state.recompute_file_scroll_total_height_px();

    let old_max = state.global_max_scroll_top_px();
    assert_eq!(old_max, 500);
    state
        .workspace
        .global_scroll_top_px
        .set(&state.store, old_max);

    assert!(state.update_file_content_height_px(2, 350));

    assert_eq!(state.global_max_scroll_top_px(), 650);
    assert_eq!(
        state.workspace.global_scroll_top_px.get(&state.store),
        state.global_max_scroll_top_px()
    );
}

#[test]
fn continuous_scroll_follow_end_anchor_is_explicit_after_scrolling_to_bottom() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
    state.settings.continuous_scroll = true;
    state.editor.viewport_height_px.set(&state.store, 100);
    state
        .workspace
        .file_content_heights
        .set(&state.store, vec![Some(200), Some(200), Some(200)]);
    state.recompute_file_scroll_total_height_px();

    let old_max = state.global_max_scroll_top_px();
    state.scroll_viewport_to_global(old_max);

    let anchor = state.virtual_scroll.anchor.expect("bottom anchor");
    assert_eq!(anchor.bias, ViewportAnchorBias::FollowEnd);

    assert!(state.update_file_content_height_px(2, 350));

    assert_eq!(
        state.workspace.global_scroll_top_px.get(&state.store),
        state.global_max_scroll_top_px()
    );
}

#[test]
fn continuous_scroll_preserves_top_anchor_when_prior_file_height_changes() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
    state.settings.continuous_scroll = true;
    state.editor.viewport_height_px.set(&state.store, 100);
    state
        .workspace
        .file_content_heights
        .set(&state.store, vec![Some(200), Some(200), Some(200)]);
    state.recompute_file_scroll_total_height_px();

    state.scroll_viewport_to_global(250);
    let anchor = state.virtual_scroll.anchor.expect("top anchor");
    assert_eq!(anchor.item_id.index, 1);
    assert_eq!(anchor.intra_item_offset_px, 50);
    assert_eq!(anchor.bias, ViewportAnchorBias::PreserveTop);

    assert!(state.update_file_content_height_px(0, 300));

    assert_eq!(state.workspace.global_scroll_top_px.get(&state.store), 350);
    let anchor = state.virtual_scroll.anchor.expect("rebased anchor");
    assert_eq!(anchor.item_id.index, 1);
    assert_eq!(anchor.intra_item_offset_px, 50);
}

#[test]
fn continuous_scroll_preserves_bottom_anchor_when_prior_file_height_changes() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
    state.settings.continuous_scroll = true;
    state.editor.viewport_height_px.set(&state.store, 100);
    state
        .workspace
        .file_content_heights
        .set(&state.store, vec![Some(200), Some(200), Some(200)]);
    state.recompute_file_scroll_total_height_px();

    state.set_viewport_anchor_for_global(350, ViewportAnchorBias::PreserveBottom);
    let anchor = state.virtual_scroll.anchor.expect("bottom-edge anchor");
    assert_eq!(anchor.item_id.index, 2);
    assert_eq!(anchor.intra_item_offset_px, 50);

    assert!(state.update_file_content_height_px(0, 300));

    assert_eq!(state.workspace.global_scroll_top_px.get(&state.store), 450);
    let anchor = state.virtual_scroll.anchor.expect("rebased anchor");
    assert_eq!(anchor.bias, ViewportAnchorBias::PreserveBottom);
    assert_eq!(anchor.item_id.index, 2);
    assert_eq!(anchor.intra_item_offset_px, 50);
}

#[test]
fn continuous_scroll_keeps_bottom_anchor_after_pending_scrollbar_drag_height_update() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
    state.settings.continuous_scroll = true;
    state.editor.viewport_height_px.set(&state.store, 100);
    state
        .workspace
        .file_content_heights
        .set(&state.store, vec![Some(200), Some(200), Some(200)]);
    state.recompute_file_scroll_total_height_px();

    let old_max = state.global_max_scroll_top_px();
    assert_eq!(old_max, 500);
    state
        .workspace
        .global_scroll_top_px
        .set(&state.store, old_max);
    state.begin_viewport_scrollbar_drag(600, 100, old_max, old_max);

    assert!(!state.update_file_content_height_px(2, 350));
    state.end_viewport_scrollbar_drag();

    assert_eq!(state.global_max_scroll_top_px(), 650);
    assert_eq!(
        state.workspace.global_scroll_top_px.get(&state.store),
        state.global_max_scroll_top_px()
    );
}

#[test]
fn continuous_scroll_does_not_treat_zero_max_as_bottom_anchor() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
    state.settings.continuous_scroll = true;
    state.editor.viewport_height_px.set(&state.store, 1_000);
    state
        .workspace
        .file_content_heights
        .set(&state.store, vec![Some(200), Some(200), Some(200)]);
    state.recompute_file_scroll_total_height_px();

    assert_eq!(state.global_max_scroll_top_px(), 0);
    assert!(state.update_file_content_height_px(2, 700));

    assert_eq!(state.global_max_scroll_top_px(), 100);
    assert_eq!(state.workspace.global_scroll_top_px.get(&state.store), 0);
}

#[test]
fn virtual_diff_document_keeps_large_compare_ranges_sparse_and_anchorable() {
    let count = FILE_HEIGHT_SPARSE_MIN_COUNT + 32;
    let summaries = (0..count)
        .map(|index| {
            let path = format!("kernel/file_{index}.c");
            CompareFileSummary::from_paths_status(
                Some(&path),
                Some(&path),
                carbon::FileStatus::Modified,
                true,
            )
        })
        .collect::<Vec<_>>();
    let mut state = AppState::default();
    state.settings.continuous_scroll = true;
    state.editor.viewport_height_px.set(&state.store, 900);
    state
        .workspace
        .source
        .set(&state.store, WorkspaceSource::Compare);
    state.workspace.compare_output.set(
        &state.store,
        Some(CompareOutput {
            file_summaries: summaries,
            ..CompareOutput::default()
        }),
    );
    state.recompute_file_scroll_total_height_px();

    assert!(matches!(
        state.virtual_diff_document.height_index,
        FileHeightIndex::Sparse { .. }
    ));

    let target = state.global_max_scroll_top_px() / 2;
    state.scroll_viewport_to_global(target);
    let anchor = state.virtual_scroll.anchor.expect("compare anchor");

    assert_eq!(anchor.item_id.source, WorkspaceSource::Compare);
    assert_eq!(
        anchor.item_id.generation,
        state.workspace_render_generation()
    );
    assert!(anchor.item_id.index < count);
}

#[test]
fn virtual_diff_document_rejects_stale_measurement_item_ids() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs"]);
    state.settings.continuous_scroll = true;
    state.recompute_file_scroll_total_height_px();
    let item_id = state.virtual_diff_document.item_id(1).expect("item id");

    assert!(state.update_virtual_diff_item_height_px(item_id, 300));
    state.workspace.compare_generation.set(&state.store, 1);

    assert!(!state.update_virtual_diff_item_height_px(item_id, 500));
    assert_eq!(
        state
            .workspace
            .file_content_heights
            .with(&state.store, |heights| heights.get(1).copied().flatten()),
        Some(300)
    );
}

#[test]
fn continuous_compare_count_keeps_sidebar_files_when_output_is_partially_hydrated() {
    let mut state = AppState::default();
    state.settings.continuous_scroll = true;
    state.editor.viewport_height_px.set(&state.store, 10_000);
    state
        .workspace
        .source
        .set(&state.store, WorkspaceSource::Compare);
    state.workspace.compare_output.set(
        &state.store,
        Some(CompareOutput {
            carbon: carbon::DiffDocument {
                files: vec![carbon_context_file(0, "a.rs", "loaded")],
            },
            ..CompareOutput::default()
        }),
    );
    state.workspace.files.set(
        &state.store,
        vec![
            FileListEntry {
                path: "a.rs".into(),
            },
            FileListEntry {
                path: "b.rs".into(),
            },
            FileListEntry {
                path: "c.rs".into(),
            },
        ],
    );

    assert_eq!(state.workspace_file_count(), 3);

    let (doc, _effects) = state.build_continuous_viewport_document();
    let doc = doc.expect("viewport doc");

    assert_eq!(doc.slot_indices, vec![0, 1, 2]);
    assert_eq!(doc.slot_loading, vec![false, true, true]);
}

#[test]
fn continuous_viewport_document_exposes_virtual_stream_rows() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs"]);
    state.settings.continuous_scroll = true;
    state.editor.viewport_height_px.set(&state.store, 900);
    state
        .cache_compare_file_from_output(0, "a.rs")
        .expect("cached first file");
    state
        .cache_compare_file_from_output(1, "b.rs")
        .expect("cached second file");

    let (doc, _effects) = state.build_continuous_viewport_document();
    let doc = doc.expect("viewport doc");

    assert!(
        doc.stream_items
            .iter()
            .any(|item| item.id.kind == VirtualDiffItemKind::FileHeader)
    );
    assert!(
        doc.stream_items
            .iter()
            .any(|item| item.id.kind == VirtualDiffItemKind::Hunk)
    );
    assert!(
        doc.stream_items
            .iter()
            .any(|item| item.id.kind == VirtualDiffItemKind::DiffRow)
    );
    assert!(
        doc.stream_items
            .windows(2)
            .all(|items| items[0].sort_key <= items[1].sort_key)
    );
    assert!(
        doc.stream_items
            .iter()
            .all(|item| item.estimated_height_px > 0)
    );
}

#[test]
fn continuous_viewport_document_backfills_before_tail_file() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
    state.settings.continuous_scroll = true;
    state.editor.viewport_height_px.set(&state.store, 500);
    state
        .workspace
        .file_content_heights
        .set(&state.store, vec![Some(800), Some(800), Some(800)]);
    state.recompute_file_scroll_total_height_px();
    state
        .workspace
        .global_scroll_top_px
        .set(&state.store, 1_700);

    let (doc, _effects) = state.build_continuous_viewport_document();
    let doc = doc.expect("viewport doc");

    assert_eq!(doc.slot_indices, vec![1, 2]);
    assert_eq!(doc.start_index, 1);
    assert_eq!(doc.start_offset_px, 800);
    assert_eq!(doc.scroll_top_px, 900);
}

#[test]
fn continuous_viewport_document_follow_end_builds_from_tail() {
    let mut state =
        loaded_state_with_files(&["a.rs", "b.rs", "c.rs", "d.rs", "e.rs", "f.rs", "tail.rs"]);
    state.settings.continuous_scroll = true;
    state.editor.viewport_height_px.set(&state.store, 500);
    state.workspace.file_content_heights.set(
        &state.store,
        vec![
            Some(1_000),
            Some(1_000),
            Some(1_000),
            Some(1_000),
            Some(1_000),
            Some(1_000),
            Some(100),
        ],
    );
    state.recompute_file_scroll_total_height_px();

    state.scroll_viewport_to_global(state.global_max_scroll_top_px());

    let (doc, _effects) = state.build_continuous_viewport_document();
    let doc = doc.expect("viewport doc");

    assert_eq!(doc.slot_indices, vec![5, 6]);
    assert_eq!(doc.start_index, 5);
    assert_eq!(doc.start_offset_px, 5_000);
    assert_eq!(doc.scroll_top_px, 600);
}

#[test]
fn next_file_action_resolves_current_file_from_selected_path() {
    let mut state = loaded_state_with_files(&[
        "src/core/compare/backends/git_diff.rs",
        "src/core/compare/mod.rs",
        "src/core/compare/service.rs",
        "src/core/compare/stats.rs",
        "src/core/frecency.rs",
        "src/ui/state/mod.rs",
        "src/ui/state/text_edit.rs",
        "src/ui/toolbar.rs",
    ]);
    state.settings.continuous_scroll = true;
    state
        .workspace
        .selected_file_index
        .set(&state.store, Some(0));
    state
        .workspace
        .selected_file_path
        .set(&state.store, Some("src/ui/state/mod.rs".to_owned()));

    state.apply_action(crate::actions::EditorAction::GoToNextFile);

    assert_eq!(
        state.workspace.selected_file_index.get(&state.store),
        Some(6)
    );
    assert_eq!(
        state
            .workspace
            .selected_file_path
            .get(&state.store)
            .as_deref(),
        Some("src/ui/state/text_edit.rs")
    );
}

#[test]
fn selecting_a_file_requests_async_syntax_without_mutating_compare_output() {
    let mut state = AppState::default();
    let mut output = CompareOutput::default();
    output.carbon.files = vec![carbon_context_file(
        0,
        "src/lib.rs",
        "fn answer() -> i32 { 42 }",
    )];
    state
        .workspace
        .compare_output
        .set(&state.store, Some(output));
    state
        .workspace
        .source
        .set(&state.store, WorkspaceSource::Compare);
    state.workspace.files.set(
        &state.store,
        vec![FileListEntry {
            path: "src/lib.rs".into(),
        }],
    );
    state
        .compare
        .repo_path
        .set(&state.store, Some(PathBuf::from("/tmp/repo")));
    state.workspace.mode.set(&state.store, WorkspaceMode::Ready);

    let effects = state.apply_action(crate::actions::FileListAction::SelectFile(0));

    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::Syntax(SyntaxEffect::LoadFileSyntax(task))
                if task.request.path == "src/lib.rs"
                    && task.request.window.start == 0
                    && task.request.window.end > 0
        )
    }));
    state.workspace.compare_output.with(&state.store, |co| {
        let output = co.as_ref().expect("compare output");
        assert_eq!(output.carbon.files[0].path(), "src/lib.rs");
        assert_eq!(output.carbon.files[0].hunks.len(), 1);
    });
}

#[test]
fn prepare_active_file_builds_from_carbon_text() {
    let carbon_file = carbon::parse_unified_patch(
        "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
 fn answer() -> i32 { 42 }
",
    )
    .unwrap()
    .files
    .into_iter()
    .next()
    .unwrap();

    let prepared = prepare_active_file(0, &carbon_file);

    assert_eq!(prepared.carbon_file.path(), "src/lib.rs");
    assert!(prepared.render_doc.lines.iter().any(|render_line| {
        prepared.render_doc.line_text(render_line.left_text) == "fn answer() -> i32 { 42 }"
            || prepared.render_doc.line_text(render_line.right_text) == "fn answer() -> i32 { 42 }"
    }));
}

#[test]
fn small_compare_file_selection_stays_synchronous() {
    let mut state = AppState::default();
    let mut output = CompareOutput::default();
    let mut carbon_file = carbon_context_file(0, "src/lib.rs", "fn answer() -> i32 { 42 }");
    carbon_file.additions = 10;
    carbon_file.deletions = 5;
    output.carbon.files = vec![carbon_file];

    state
        .workspace
        .compare_output
        .set(&state.store, Some(output));
    state
        .workspace
        .source
        .set(&state.store, WorkspaceSource::Compare);
    state.workspace.files.set(
        &state.store,
        vec![FileListEntry {
            path: "src/lib.rs".into(),
        }],
    );
    state.workspace.mode.set(&state.store, WorkspaceMode::Ready);
    state
        .compare
        .repo_path
        .set(&state.store, Some(PathBuf::from("/repo")));

    let effects = state.apply_action(crate::actions::FileListAction::SelectFile(0));

    assert!(effects.iter().any(|effect| {
        matches!(effect, Effect::Syntax(SyntaxEffect::EnsureSyntaxPackForPath { path }) if path == "src/lib.rs")
    }));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Compare(CompareEffect::LoadFile(_))))
    );
    assert!(
        state
            .workspace
            .active_file_loading
            .get(&state.store)
            .is_none()
    );
    assert_eq!(
        state
            .workspace
            .active_file
            .get(&state.store)
            .as_ref()
            .map(|file| file.path.as_str()),
        Some("src/lib.rs")
    );
}

#[test]
fn selecting_large_compare_file_dispatches_async_load() {
    let mut state = loaded_state_with_files(&["src/big.rs"]);
    state
        .workspace
        .compare_output
        .update(&state.store, |output| {
            let file = &mut output.as_mut().expect("compare output").carbon.files[0];
            file.additions = 1_500;
        });
    state
        .compare
        .repo_path
        .set(&state.store, Some(PathBuf::from("/repo")));
    state.compare.left_ref.set(&state.store, "v5.5".to_owned());
    state.compare.right_ref.set(&state.store, "v5.6".to_owned());
    state
        .compare
        .renderer
        .set(&state.store, RendererKind::Builtin);
    state.compare.layout.set(&state.store, LayoutMode::Unified);
    state.compare.mode.set(&state.store, CompareMode::TwoDot);

    let effects = state.apply_action(crate::actions::FileListAction::SelectFile(0));

    assert!(matches!(
        effects.as_slice(),
        [
            Effect::Syntax(SyntaxEffect::EnsureSyntaxPackForPath { path }),
            Effect::Compare(CompareEffect::LoadFile(task))
        ]
            if path == "src/big.rs"
                && task.request.index == 0
                && task.request.path == "src/big.rs"
    ));
    assert_eq!(
        state.workspace.active_file_loading.get(&state.store),
        Some(ActiveFileLoading {
            index: 0,
            path: "src/big.rs".to_owned(),
            priority: CompareWorkPriority::InteractiveSelectedFile,
        })
    );
    assert!(state.workspace.active_file.get(&state.store).is_none());
}

#[test]
fn selecting_deferred_compare_file_dispatches_async_load() {
    let mut state = loaded_state_with_files(&["src/kernel.c"]);
    state
        .workspace
        .compare_output
        .update(&state.store, |output| {
            let file = &mut output.as_mut().expect("compare output").carbon.files[0];
            file.is_partial = true;
            file.hunks.clear();
            file.blocks.clear();
        });
    state
        .compare
        .repo_path
        .set(&state.store, Some(PathBuf::from("/repo")));

    let effects = state.apply_action(crate::actions::FileListAction::SelectFile(0));

    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::Compare(CompareEffect::LoadFile(task))
                if task.request.index == 0
                    && task.request.path == "src/kernel.c"
                    && task.request.deferred_file.as_ref().is_some_and(|file| file.is_partial)
        )
    }));
    assert_eq!(
        state.workspace.active_file_loading.get(&state.store),
        Some(ActiveFileLoading {
            index: 0,
            path: "src/kernel.c".to_owned(),
            priority: CompareWorkPriority::InteractiveSelectedFile,
        })
    );
    assert!(state.workspace.active_file.get(&state.store).is_none());
}

#[test]
fn scrollbar_drag_loads_visible_compare_files_without_selecting_them() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
    state.settings.continuous_scroll = true;
    state.editor.viewport_height_px.set(&state.store, 240);
    state
        .compare
        .repo_path
        .set(&state.store, Some(PathBuf::from("/repo")));
    state
        .workspace
        .compare_output
        .update(&state.store, |output| {
            for file in &mut output.as_mut().expect("compare output").carbon.files {
                file.is_partial = true;
                file.hunks.clear();
                file.blocks.clear();
            }
        });
    state.begin_viewport_scrollbar_drag(900, 240, 300, 660);

    let (_doc, effects) = state.build_continuous_viewport_document();

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Compare(CompareEffect::LoadFile(_))))
    );
    assert!(
        state
            .workspace
            .active_file_loading
            .get(&state.store)
            .is_none()
    );
}

#[test]
fn overscan_prefetch_does_not_enqueue_syntax_work() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs", "c.rs"]);
    state
        .compare
        .repo_path
        .set(&state.store, Some(PathBuf::from("/repo")));

    let effects = state.prefetch_compare_files_forward(0, 1_000);

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Syntax(SyntaxEffect::LoadFileSyntax(_)))),
        "overscan should warm file diffs without adding syntax windows"
    );
    state.workspace.file_cache.with(&state.store, |files| {
        assert!(files.values().all(|file| file.syntax_pending.is_empty()));
    });
}

#[test]
fn offscreen_viewport_slots_do_not_enqueue_syntax_work() {
    let mut state = loaded_state_with_files(&["a.rs"]);
    state
        .cache_compare_file_from_output(0, "a.rs")
        .expect("cached file");
    let key = state.compare_slot_key_at(0, "a.rs");

    let window = state.viewport_slot_syntax_window(&key, 1_000, 120, 0, 240);

    assert_eq!(window, None);
}

#[test]
fn syntax_budget_counts_inflight_requests_after_cache_eviction() {
    let mut state = loaded_state_with_files(&["a.rs"]);
    state
        .compare
        .repo_path
        .set(&state.store, Some(PathBuf::from("/repo")));
    state
        .cache_compare_file_from_output(0, "a.rs")
        .expect("cached file");
    let key = state.compare_slot_key_at(0, "a.rs");

    let effect = state.request_viewport_slot_syntax_window(
        &key,
        crate::core::syntax::annotator::SyntaxRowWindow { start: 0, end: 32 },
    );

    assert!(matches!(
        effect,
        Some(Effect::Syntax(SyntaxEffect::LoadFileSyntax(_)))
    ));
    state.workspace.file_cache.update(&state.store, |files| {
        files.clear();
    });
    assert_eq!(state.syntax_pending_window_count(), 0);
    assert_eq!(state.syntax_requests.inflight_len(), 1);
}

#[test]
fn syntax_epoch_invalidation_clears_attached_pending_windows() {
    let mut state = loaded_state_with_files(&["a.rs", "b.rs"]);
    state
        .cache_compare_file_from_output(0, "a.rs")
        .expect("cached file");
    state
        .cache_compare_file_from_output(1, "b.rs")
        .expect("cached file");
    let active = state
        .workspace
        .file_cache
        .with(&state.store, |files| files.get(&0).cloned())
        .expect("cached active");
    state.workspace.active_file.set(&state.store, Some(active));
    let pending = super::SyntaxPendingWindow {
        request_id: 1,
        window: crate::core::syntax::annotator::SyntaxRowWindow { start: 0, end: 32 },
    };
    state.workspace.active_file.update(&state.store, |active| {
        active
            .as_mut()
            .expect("active file")
            .syntax_pending
            .push(pending);
    });
    state.workspace.file_cache.update(&state.store, |files| {
        for file in files.values_mut() {
            file.syntax_pending.push(pending);
        }
    });
    state.syntax_requests.insert_inflight(0, 1);

    let effect = state.invalidate_syntax_epoch_effect();

    assert!(matches!(
        effect,
        Effect::Syntax(SyntaxEffect::SetFileSyntaxEpoch { .. })
    ));
    assert_eq!(state.syntax_pending_window_count(), 0);
    assert_eq!(state.syntax_requests.inflight_len(), 0);
}

#[test]
fn context_expansion_invalidates_existing_syntax_windows() {
    let mut state = status_state_with_two_hunks();
    let stale_window = crate::core::syntax::annotator::SyntaxRowWindow { start: 0, end: 8 };

    state.workspace.active_file.update(&state.store, |active| {
        let active = active.as_mut().expect("active file");
        active.syntax_pending.push(super::SyntaxPendingWindow {
            request_id: 7,
            window: stale_window,
        });
        active.syntax_covered.push(stale_window);
        let range = active
            .token_buffer
            .append(&[crate::core::text::DiffTokenSpan {
                offset: 0,
                length: 2,
                kind: Default::default(),
                intensity: Default::default(),
            }]);
        active
            .carbon_overlays
            .insert_syntax(0, carbon::DiffSide::Old, 0, range);
    });

    state.apply_context_expansion(
        crate::events::ContextDirection::All,
        0,
        0,
        Arc::new((0..12).map(|index| format!("old {index}")).collect()),
        Arc::new((0..12).map(|index| format!("new {index}")).collect()),
    );

    state.workspace.active_file.with(&state.store, |active| {
        let active = active.as_ref().expect("active file");
        assert!(active.syntax_pending.is_empty());
        assert!(active.syntax_covered.is_empty());
        assert_eq!(active.token_buffer.len(), 0);
    });
}

#[test]
fn context_expansion_retires_old_syntax_epoch_before_requeue() {
    let mut state = status_state_with_two_hunks();
    state.workspace.active_file.update(&state.store, |active| {
        let active = active.as_mut().expect("active file");
        active.old_file_lines = Some(Arc::new(
            (0..12).map(|index| format!("old {index}")).collect(),
        ));
        active.file_lines = Some(Arc::new(
            (0..12).map(|index| format!("new {index}")).collect(),
        ));
    });
    state.workspace.compare_generation.set(&state.store, 1);
    for request_id in 0..super::MAX_PENDING_SYNTAX_WINDOWS as u64 {
        state.syntax_requests.insert_inflight(0, request_id);
    }

    let effects = state.dispatch_context_expansion(0, crate::events::ContextDirection::All, 0);

    assert!(matches!(
        effects.first(),
        Some(Effect::Syntax(SyntaxEffect::SetFileSyntaxEpoch { .. }))
    ));
    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::Syntax(SyntaxEffect::LoadFileSyntax(task))
                if task.request.syntax_epoch == state.syntax_requests.epoch()
        )
    }));
    assert_eq!(state.syntax_requests.inflight_len(), 1);
}

#[test]
fn syntax_pack_install_retires_old_epoch_before_refresh() {
    let mut state = status_state_with_two_hunks();
    for request_id in 0..super::MAX_PENDING_SYNTAX_WINDOWS as u64 {
        state.syntax_requests.insert_inflight(0, request_id);
    }

    let effects = state.handle_syntax_packs_installed(&["rust".to_owned()]);

    assert!(matches!(
        effects.first(),
        Some(Effect::Syntax(SyntaxEffect::SetFileSyntaxEpoch { .. }))
    ));
    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::Syntax(SyntaxEffect::LoadFileSyntax(task))
                if task.request.syntax_epoch == state.syntax_requests.epoch()
        )
    }));
    assert_eq!(state.syntax_requests.inflight_len(), 1);
}

#[test]
fn compare_file_finished_ignores_stale_path() {
    let mut state = loaded_state_with_files(&["src/lib.rs"]);
    state.workspace.compare_generation.set(&state.store, 7);
    state
        .workspace
        .selected_file_index
        .set(&state.store, Some(0));
    state
        .workspace
        .selected_file_path
        .set(&state.store, Some("src/lib.rs".to_owned()));
    state.workspace.active_file_loading.set(
        &state.store,
        Some(ActiveFileLoading {
            index: 0,
            path: "src/lib.rs".to_owned(),
            priority: CompareWorkPriority::InteractiveSelectedFile,
        }),
    );

    state.apply_event(AppEvent::from(CompareEvent::CompareFileFinished(
        CompareFileFinished {
            generation: 7,
            index: 0,
            path: "src/other.rs".to_owned(),
            prepared: PreparedActiveFile {
                carbon_file: carbon::FileDiff::default(),
                carbon_expansion: carbon::ExpansionState::default(),
                carbon_overlays: CarbonStyleOverlays::default(),
                render_doc: Arc::new(RenderDoc::default()),
                token_buffer: TokenBuffer::default(),
            },
        },
    )));

    assert!(state.workspace.active_file.get(&state.store).is_none());
    assert_eq!(
        state.workspace.active_file_loading.get(&state.store),
        Some(ActiveFileLoading {
            index: 0,
            path: "src/lib.rs".to_owned(),
            priority: CompareWorkPriority::InteractiveSelectedFile,
        })
    );
}

#[test]
fn overlay_list_pixel_scroll_action_clamps_active_overlay() {
    let mut state = AppState::default();
    state.overlays.stack.update(&state.store, |stack| {
        stack.push(super::OverlayEntry {
            surface: OverlaySurface::RepoPicker,
            focus_return: None,
        });
    });
    let picker_entries: Vec<super::PickerEntry> = (0..12)
        .map(|index| super::PickerEntry {
            label: format!("repo-{index}"),
            detail: format!("C:\\repo-{index}"),
            value: format!("C:\\repo-{index}"),
            highlights: Vec::new(),
            label_style: PickerLabelStyle::Default,
            icon: None,
            section_header: false,
        })
        .collect();
    state
        .overlays
        .picker
        .entries
        .set(&state.store, picker_entries);
    state
        .overlays
        .picker
        .list
        .update(&state.store, |l| l.viewport_height_px = 120);

    state.apply_action(crate::actions::OverlayAction::ScrollActiveOverlayListPx(50));
    assert_eq!(
        state
            .overlays
            .picker
            .list
            .with(&state.store, |l| l.scroll_top_px),
        50
    );

    state.apply_action(crate::actions::OverlayAction::ScrollActiveOverlayListPx(
        1_000,
    ));
    assert_eq!(
        state
            .overlays
            .picker
            .list
            .with(&state.store, |l| l.scroll_top_px),
        312
    );

    state.apply_action(crate::actions::OverlayAction::ScrollActiveOverlayListPx(
        -1_000,
    ));
    assert_eq!(
        state
            .overlays
            .picker
            .list
            .with(&state.store, |l| l.scroll_top_px),
        0
    );
}

#[test]
fn closing_overlays_restores_previous_focus() {
    let mut state = AppState::default();
    state.apply_action(crate::actions::AppAction::SetFocus(Some(
        FocusTarget::FileList,
    )));

    state.apply_action(crate::actions::OverlayAction::OpenCommandPalette);
    assert_eq!(
        state.ui.focus.get(&state.store),
        Some(FocusTarget::CommandPaletteInput)
    );

    // Each nested overlay records its own restore target.
    state.apply_action(crate::actions::OverlayAction::OpenGitHubAuthModal);
    assert_eq!(
        state.ui.focus.get(&state.store),
        Some(FocusTarget::AuthPrimaryAction)
    );

    state.apply_action(crate::actions::OverlayAction::CloseOverlay);
    assert_eq!(state.overlays_top(), Some(OverlaySurface::CommandPalette));
    assert_eq!(
        state.ui.focus.get(&state.store),
        Some(FocusTarget::CommandPaletteInput)
    );

    state.apply_action(crate::actions::OverlayAction::CloseOverlay);
    assert_eq!(state.overlays_top(), None);
    assert_eq!(
        state.ui.focus.get(&state.store),
        Some(FocusTarget::FileList)
    );
}

#[test]
fn clearing_overlay_stack_restores_pre_overlay_focus() {
    let mut state = AppState::default();
    state.apply_action(crate::actions::AppAction::SetFocus(Some(
        FocusTarget::FileList,
    )));
    state.apply_action(crate::actions::OverlayAction::OpenCommandPalette);
    state.apply_action(crate::actions::OverlayAction::OpenGitHubAuthModal);

    state.clear_overlays();

    assert_eq!(state.overlays_top(), None);
    assert_eq!(
        state.ui.focus.get(&state.store),
        Some(FocusTarget::FileList)
    );
}

#[test]
fn stage_hunk_at_stages_the_given_index() {
    let mut state = status_state_with_two_hunks();

    let effects = state.apply_action(crate::actions::RepositoryAction::StageHunkAt(1));

    let [Effect::Repository(RepositoryEffect::ApplyPatchOperation(request))] = effects.as_slice()
    else {
        panic!("expected one patch effect, got {:?}", effects);
    };
    assert!(request.patch.contains("old_second();"));
    assert!(!request.patch.contains("old_first();"));
}

#[test]
fn stage_hunk_reads_the_hovered_hunk_index() {
    let mut state = status_state_with_two_hunks();
    state.editor.hovered_hunk_index.set(&state.store, Some(1));

    let effects = state.apply_action(crate::actions::RepositoryAction::StageHunk);

    let [Effect::Repository(RepositoryEffect::ApplyPatchOperation(request))] = effects.as_slice()
    else {
        panic!("expected one patch effect");
    };
    assert!(request.patch.contains("old_second();"));
}

#[test]
fn stage_hunk_without_partial_hunk_capability_is_ignored() {
    let mut state = status_state_with_two_hunks();
    let mut capabilities = RepoCapabilities::git();
    capabilities.staging_area = false;
    capabilities.partial_hunk_mutation = false;
    state
        .repository
        .capabilities
        .set(&state.store, Some(capabilities));

    let effects = state.apply_action(crate::actions::RepositoryAction::StageHunkAt(0));

    assert!(effects.is_empty());
}

#[test]
fn status_operation_failure_clears_the_pending_flag() {
    let mut state = status_state_with_two_hunks();
    let _ = state.apply_action(crate::actions::RepositoryAction::StageHunkAt(0));
    assert!(state.workspace.status_operation_pending.get(&state.store));

    let _ = state.apply_event(AppEvent::from(RepositoryEvent::FileOperationFailed {
        path: PathBuf::from("/repo"),
        message: "patch failed".to_owned(),
    }));

    assert!(!state.workspace.status_operation_pending.get(&state.store));
}

#[test]
fn ref_picker_rebuilds_matches_while_typing_and_keeps_raw_git_revisions_selectable() {
    let mut state = AppState::default();
    state.repository.refs.set(
        &state.store,
        vec![VcsRef {
            name: "main".to_owned(),
            kind: RefKind::Branch,
            target: RevisionId::git("0000000000000000000000000000000000000000"),
            active: true,
            upstream: None,
            ahead_behind: None,
        }],
    );

    state.open_ref_picker(CompareField::Left);
    state.apply_action(crate::actions::TextEditAction::InsertText("mai".to_owned()));

    let branch_highlights = state.overlays.picker.entries.with(&state.store, |entries| {
        entries
            .iter()
            .find(|entry| entry.value == "main")
            .expect("main branch entry")
            .highlights
            .clone()
    });
    assert_eq!(branch_highlights, vec![(0, 3)]);

    let mut state = AppState::default();
    state.open_ref_picker(CompareField::Left);
    state.apply_action(crate::actions::TextEditAction::InsertText(
        "HEAD~2".to_owned(),
    ));

    let (typed_value, typed_highlights) =
        state.overlays.picker.entries.with(&state.store, |entries| {
            let typed_entry = entries.first().expect("typed ref entry");
            (typed_entry.value.clone(), typed_entry.highlights.clone())
        });
    assert_eq!(typed_value, "HEAD~2");
    assert_eq!(typed_highlights, vec![(0, "HEAD~2".len())]);

    state.apply_action(crate::actions::OverlayAction::ConfirmOverlaySelection);
    assert_eq!(state.compare.left_ref.get(&state.store), "HEAD~2");
}

#[test]
fn ref_picker_uses_jj_refs_and_change_ids_without_git_workdir() {
    let mut state = AppState::default();
    let working_commit = "3e2d7a6e55221e519e3efb86e4f8fbb324980427".to_owned();
    let change_id = "xxyzvpwmsuxytmqltlzwzqpylvlqqyso".to_owned();

    state.repository.location.set(
        &state.store,
        Some(RepoLocation {
            kind: VcsKind::JJ,
            profile: crate::core::vcs::model::VCS_PROFILE_JJ,
            workspace_root: PathBuf::from("/repo"),
            store_root: Some(PathBuf::from("/repo/.jj")),
        }),
    );
    state.repository.refs.set(
        &state.store,
        vec![
            VcsRef {
                name: "@".to_owned(),
                kind: RefKind::WorkingCopy,
                target: RevisionId {
                    backend: VcsKind::JJ,
                    id: working_commit.clone(),
                },
                active: true,
                upstream: None,
                ahead_behind: None,
            },
            VcsRef {
                name: "main".to_owned(),
                kind: RefKind::Bookmark,
                target: RevisionId {
                    backend: VcsKind::JJ,
                    id: "a4c9f6e8b1d24036a78610a332e12ca25e97c315".to_owned(),
                },
                active: false,
                upstream: None,
                ahead_behind: None,
            },
        ],
    );
    state.repository.changes.set(
        &state.store,
        vec![VcsChange {
            revision: RevisionId {
                backend: VcsKind::JJ,
                id: working_commit,
            },
            change_id: Some(change_id.clone()),
            short_change_id: Some("xsvsonvs".to_owned()),
            short_change_id_prefix_len: Some(2),
            short_revision: "3e2d7a6e5522".to_owned(),
            summary: "Working copy".to_owned(),
            author_name: "ro".to_owned(),
            timestamp: 0,
            flags: ChangeFlags {
                current: true,
                working_copy: true,
                ..ChangeFlags::default()
            },
        }],
    );

    state.open_ref_picker(CompareField::Left);

    state.overlays.picker.entries.with(&state.store, |entries| {
        assert!(!entries.iter().any(|entry| entry.value == "@workdir"));

        let working_copy = entries
            .iter()
            .find(|entry| entry.value == "@")
            .expect("working copy ref");
        assert_eq!(
            working_copy.detail,
            "Working copy change \u{2022} current / xsvsonvs 3e2d7a6e5522"
        );

        let bookmark = entries
            .iter()
            .find(|entry| entry.value == "main")
            .expect("bookmark ref");
        assert_eq!(bookmark.detail, "Bookmark");

        let change = entries
            .iter()
            .find(|entry| entry.value == change_id)
            .expect("change id entry");
        assert_eq!(change.label, "xsvsonvs");
        assert!(change.highlights.is_empty());
        assert_eq!(
            change.label_style(),
            PickerLabelStyle::JjChangeId {
                prefix_len: 2,
                working_copy: true,
            }
        );
    });
}

#[test]
fn command_palette_uses_actual_match_indices_for_highlighting() {
    let mut state = AppState::default();
    state
        .overlays
        .command_palette
        .query
        .set(&state.store, "them".to_owned());

    state.rebuild_command_palette();

    let highlights = state
        .overlays
        .command_palette
        .entries
        .with(&state.store, |entries| {
            entries
                .iter()
                .find(|entry| entry.label == "Change Theme")
                .expect("Change Theme entry")
                .highlights
                .clone()
        });
    assert_eq!(highlights, vec![(7, 11)]);
}

#[test]
fn command_palette_surfaces_jj_operations_for_jj_repositories() {
    let mut state = AppState::default();
    state.repository.location.set(
        &state.store,
        Some(RepoLocation {
            kind: VcsKind::JJ,
            profile: crate::core::vcs::model::VCS_PROFILE_JJ,
            workspace_root: PathBuf::from("/repo"),
            store_root: Some(PathBuf::from("/repo/.jj")),
        }),
    );
    state
        .overlays
        .command_palette
        .query
        .set(&state.store, "jj".to_owned());

    state.rebuild_command_palette();

    let entries = state
        .overlays
        .command_palette
        .entries
        .with(&state.store, |entries| entries.clone());
    for operation in JjOperation::ALL {
        let label = format!("jj: {}", operation.label());
        let entry = entries
            .iter()
            .find(|entry| entry.label == label)
            .unwrap_or_else(|| panic!("missing {label} command"));
        assert!(matches!(
            entry.kind,
            super::PaletteEntryKind::Command(super::PaletteCommand::RunOperation(
                VcsOperation::Jj(found)
            )) if found == operation
        ));
    }

    let mut state = AppState::default();
    state
        .overlays
        .command_palette
        .query
        .set(&state.store, "jj".to_owned());
    state.rebuild_command_palette();
    let has_jj_operation = state
        .overlays
        .command_palette
        .entries
        .with(&state.store, |entries| {
            entries.iter().any(|entry| {
                JjOperation::ALL
                    .into_iter()
                    .any(|operation| entry.label == format!("jj: {}", operation.label()))
            })
        });
    assert!(!has_jj_operation);
}

#[test]
fn jj_operation_action_emits_repository_effect() {
    let mut state = AppState::default();
    let repo_path = PathBuf::from("/repo");
    let operation = VcsOperation::Jj(JjOperation::NewChange);
    state
        .compare
        .repo_path
        .set(&state.store, Some(repo_path.clone()));
    state.repository.location.set(
        &state.store,
        Some(RepoLocation {
            kind: VcsKind::JJ,
            profile: crate::core::vcs::model::VCS_PROFILE_JJ,
            workspace_root: repo_path.clone(),
            store_root: Some(repo_path.join(".jj")),
        }),
    );

    let effects = state.apply_action(crate::actions::RepositoryAction::RunOperation(
        operation.clone(),
    ));

    let [Effect::Repository(RepositoryEffect::RunOperation(request))] = effects.as_slice() else {
        panic!("expected RunOperation effect, got {effects:?}");
    };
    assert_eq!(request.repo_path, repo_path);
    assert_eq!(request.operation, operation);
}

#[test]
fn destructive_jj_palette_operation_requires_confirmation() {
    let mut state = AppState::default();
    let repo_path = PathBuf::from("/repo");
    let operation = VcsOperation::Jj(JjOperation::AbandonChange);
    state
        .compare
        .repo_path
        .set(&state.store, Some(repo_path.clone()));
    state.repository.location.set(
        &state.store,
        Some(RepoLocation {
            kind: VcsKind::JJ,
            profile: crate::core::vcs::model::VCS_PROFILE_JJ,
            workspace_root: repo_path.clone(),
            store_root: Some(repo_path.join(".jj")),
        }),
    );
    state
        .overlays
        .command_palette
        .query
        .set(&state.store, "abandon".to_owned());
    state.overlays.stack.update(&state.store, |stack| {
        stack.push(OverlayEntry {
            surface: OverlaySurface::CommandPalette,
            focus_return: None,
        });
    });
    state.rebuild_command_palette();

    let effects = state.apply_action(crate::actions::OverlayAction::ConfirmOverlaySelection);

    assert!(effects.is_empty());
    assert_eq!(state.overlays_top(), Some(OverlaySurface::Confirmation));
    assert_eq!(
        state.overlays.confirmation.action.get(&state.store),
        Some(crate::actions::RepositoryAction::RunOperation(operation.clone()).into())
    );

    let effects = state.apply_action(crate::actions::OverlayAction::ConfirmOverlaySelection);

    let [Effect::Repository(RepositoryEffect::RunOperation(request))] = effects.as_slice() else {
        panic!("expected RunOperation effect, got {effects:?}");
    };
    assert_eq!(request.repo_path, repo_path);
    assert_eq!(request.operation, operation);
    assert_eq!(state.overlays_top(), None);
}

#[test]
fn command_palette_surfaces_jj_rebase_destinations() {
    let mut state = AppState::default();
    state.repository.location.set(
        &state.store,
        Some(RepoLocation {
            kind: VcsKind::JJ,
            profile: crate::core::vcs::model::VCS_PROFILE_JJ,
            workspace_root: PathBuf::from("/repo"),
            store_root: Some(PathBuf::from("/repo/.jj")),
        }),
    );
    state.repository.refs.set(
        &state.store,
        vec![
            VcsRef {
                name: "@".to_owned(),
                kind: RefKind::WorkingCopy,
                target: RevisionId {
                    backend: VcsKind::JJ,
                    id: "current".to_owned(),
                },
                active: true,
                upstream: None,
                ahead_behind: None,
            },
            VcsRef {
                name: "main".to_owned(),
                kind: RefKind::Bookmark,
                target: RevisionId {
                    backend: VcsKind::JJ,
                    id: "main-revision".to_owned(),
                },
                active: false,
                upstream: None,
                ahead_behind: None,
            },
        ],
    );
    state
        .overlays
        .command_palette
        .query
        .set(&state.store, "rebase main".to_owned());

    state.rebuild_command_palette();

    let entry = state
        .overlays
        .command_palette
        .entries
        .with(&state.store, |entries| entries.first().cloned())
        .expect("rebase entry");
    assert_eq!(entry.label, "jj: Rebase @ Onto main");
    assert!(matches!(
        entry.kind,
        super::PaletteEntryKind::Command(super::PaletteCommand::RunOperation(
            VcsOperation::JjRebaseCurrentChangeOnto { ref destination }
        )) if destination == "main"
    ));
}

#[test]
fn command_palette_surfaces_jj_editable_changes() {
    let mut state = AppState::default();
    state.repository.location.set(
        &state.store,
        Some(RepoLocation {
            kind: VcsKind::JJ,
            profile: crate::core::vcs::model::VCS_PROFILE_JJ,
            workspace_root: PathBuf::from("/repo"),
            store_root: Some(PathBuf::from("/repo/.jj")),
        }),
    );
    state.repository.changes.set(
        &state.store,
        vec![
            VcsChange {
                revision: RevisionId {
                    backend: VcsKind::JJ,
                    id: "current-revision".to_owned(),
                },
                change_id: Some("current-change".to_owned()),
                short_change_id: Some("cur".to_owned()),
                short_change_id_prefix_len: Some(3),
                short_revision: "currev".to_owned(),
                summary: "current".to_owned(),
                author_name: "ro".to_owned(),
                timestamp: 0,
                flags: ChangeFlags {
                    current: true,
                    working_copy: true,
                    ..ChangeFlags::default()
                },
            },
            VcsChange {
                revision: RevisionId {
                    backend: VcsKind::JJ,
                    id: "target-revision".to_owned(),
                },
                change_id: Some("target-change".to_owned()),
                short_change_id: Some("tgt".to_owned()),
                short_change_id_prefix_len: Some(3),
                short_revision: "tgt123".to_owned(),
                summary: "target change".to_owned(),
                author_name: "ro".to_owned(),
                timestamp: 0,
                flags: ChangeFlags::default(),
            },
        ],
    );
    state
        .overlays
        .command_palette
        .query
        .set(&state.store, "edit tgt".to_owned());

    state.rebuild_command_palette();

    let entry = state
        .overlays
        .command_palette
        .entries
        .with(&state.store, |entries| entries.first().cloned())
        .expect("edit entry");
    assert_eq!(entry.label, "jj: Edit tgt");
    assert!(matches!(
        entry.kind,
        super::PaletteEntryKind::Command(super::PaletteCommand::RunOperation(
            VcsOperation::JjEditRevision {
                ref revision,
                ref label
            }
        )) if revision == "target-revision" && label == "tgt"
    ));
}

#[test]
fn command_palette_surfaces_jj_operation_log_restore_targets() {
    let mut state = AppState::default();
    state.repository.location.set(
        &state.store,
        Some(RepoLocation {
            kind: VcsKind::JJ,
            profile: crate::core::vcs::model::VCS_PROFILE_JJ,
            workspace_root: PathBuf::from("/repo"),
            store_root: Some(PathBuf::from("/repo/.jj")),
        }),
    );
    state.repository.operation_log.set(
        &state.store,
        vec![
            VcsOperationLogEntry {
                operation_id: "current-operation".to_owned(),
                short_operation_id: "current".to_owned(),
                user: "ro".to_owned(),
                time: "later".to_owned(),
                description: "snapshot working copy".to_owned(),
            },
            VcsOperationLogEntry {
                operation_id: "target-operation".to_owned(),
                short_operation_id: "target".to_owned(),
                user: "ro".to_owned(),
                time: "earlier".to_owned(),
                description: "describe change".to_owned(),
            },
        ],
    );
    state
        .overlays
        .command_palette
        .query
        .set(&state.store, "restore target".to_owned());

    state.rebuild_command_palette();

    let entries = state
        .overlays
        .command_palette
        .entries
        .with(&state.store, |entries| entries.clone());
    assert!(
        !entries
            .iter()
            .any(|entry| entry.label == "jj: Restore Operation current")
    );
    let entry = entries
        .iter()
        .find(|entry| entry.label == "jj: Restore Operation target")
        .expect("restore entry");
    assert_eq!(entry.detail, "describe change - ro - earlier");
    assert!(matches!(
        entry.kind,
        super::PaletteEntryKind::Command(super::PaletteCommand::RunOperation(
            VcsOperation::JjRestoreOperation {
                ref operation_id,
                ref label
            }
        )) if operation_id == "target-operation" && label == "target"
    ));
}

#[test]
fn sidebar_width_action_clamps_and_stores_manual_preference() {
    let mut state = AppState::default();

    state.apply_action(crate::actions::SettingsAction::SetSidebarWidthPx(40));
    assert_eq!(state.settings.sidebar_width_px, Some(179));

    state.apply_action(crate::actions::SettingsAction::SetSidebarWidthPx(420));
    assert_eq!(state.settings.sidebar_width_px, Some(420));
}

#[test]
fn ui_scale_actions_step_and_persist_within_bounds() {
    let mut state = AppState::default();

    let effects = state.apply_action(crate::actions::SettingsAction::IncreaseUiScale);
    assert_eq!(state.settings.ui_scale_pct, 110);
    assert_eq!(effects.len(), 1);

    for _ in 0..20 {
        state.apply_action(crate::actions::SettingsAction::IncreaseUiScale);
    }
    assert_eq!(state.settings.ui_scale_pct, 180);

    for _ in 0..20 {
        state.apply_action(crate::actions::SettingsAction::DecreaseUiScale);
    }
    assert_eq!(state.settings.ui_scale_pct, 70);
}

#[test]
fn avatar_url_sized_appends_or_replaces_s_param() {
    use super::avatar_url_sized;
    assert_eq!(
        avatar_url_sized("https://avatars.githubusercontent.com/u/1?v=4", 128),
        Some("https://avatars.githubusercontent.com/u/1?v=4&s=128".to_owned())
    );
    assert_eq!(
        avatar_url_sized("https://avatars.githubusercontent.com/u/1", 64),
        Some("https://avatars.githubusercontent.com/u/1?s=64".to_owned())
    );
    assert_eq!(
        avatar_url_sized("https://avatars.githubusercontent.com/u/1?s=40&v=4", 128),
        Some("https://avatars.githubusercontent.com/u/1?v=4&s=128".to_owned())
    );
    assert_eq!(avatar_url_sized("", 128), None);
}

#[test]
fn card_text_selection_slices_normalized_range() {
    let body = "the quick brown fox".to_owned();
    // Forward selection.
    let mut sel = CardTextSelection::new(7, body.clone(), 4);
    sel.focus = 9;
    assert_eq!(sel.normalized(), (4, 9));
    assert_eq!(sel.selected_text().as_deref(), Some("quick"));
    assert!(!sel.is_collapsed());

    // Reversed drag yields the same substring.
    let mut rev = CardTextSelection::new(7, body.clone(), 9);
    rev.focus = 4;
    assert_eq!(rev.normalized(), (4, 9));
    assert_eq!(rev.selected_text().as_deref(), Some("quick"));

    // Collapsed selection copies nothing.
    let collapsed = CardTextSelection::new(7, body.clone(), 4);
    assert!(collapsed.is_collapsed());
    assert_eq!(collapsed.selected_text(), None);

    // Out-of-range anchor is clamped at construction (no panic / no copy).
    let clamped = CardTextSelection::new(7, body, 999);
    assert!(clamped.is_collapsed());
}

#[test]
fn command_palette_detects_pr_url_and_emits_peek_effect() {
    let mut state = AppState::default();
    state.overlays.command_palette.query.set(
        &state.store,
        "https://github.com/foo/bar/pull/42".to_owned(),
    );

    let effects = state.rebuild_command_palette();

    // A peek effect was fired for the parsed key.
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::GitHub(GitHubEffect::PeekPullRequest {
            owner, repo, number, ..
        }) if owner == "foo" && repo == "bar" && *number == 42
    )));

    // Palette has the synthesized PR entry as the top row with key intact.
    let top = state
        .overlays
        .command_palette
        .entries
        .with(&state.store, |e| e.first().cloned())
        .expect("palette has at least one entry");
    assert!(matches!(
        top.kind,
        super::PaletteEntryKind::PullRequest((ref o, ref r, n))
        if o == "foo" && r == "bar" && n == 42
    ));

    // Cache entry is initialized to Loading.
    let cached = state.github.pull_request.cache.with(&state.store, |c| {
        c.get(&("foo".to_owned(), "bar".to_owned(), 42)).cloned()
    });
    let cached = cached.expect("cache entry");
    assert!(matches!(cached.meta, super::PrPeekMeta::Loading));
}

#[test]
fn pr_peeked_event_transitions_cache_meta_to_ready() {
    use crate::core::forge::github::PullRequestInfo;
    use crate::events::AppEvent;

    let mut state = AppState::default();
    state
        .overlays
        .command_palette
        .query
        .set(&state.store, "https://github.com/foo/bar/pull/7".to_owned());
    let _ = state.rebuild_command_palette();

    let info = PullRequestInfo {
        title: "Fix thing".to_owned(),
        state: "open".to_owned(),
        author_login: "alice".to_owned(),
        number: 7,
        additions: 12,
        deletions: 3,
        changed_files: 1,
        base_branch: "main".to_owned(),
        head_branch: "fix".to_owned(),
        base_sha: "a".to_owned(),
        head_sha: "b".to_owned(),
        base_repo_url: String::new(),
        head_repo_url: String::new(),
    };
    state.apply_event(AppEvent::from(GitHubEvent::PullRequestPeeked {
        owner: "foo".to_owned(),
        repo: "bar".to_owned(),
        number: 7,
        info: info.clone(),
    }));

    let meta = state.github.pull_request.cache.with(&state.store, |c| {
        c.get(&("foo".to_owned(), "bar".to_owned(), 7))
            .map(|e| e.meta.clone())
    });
    assert!(matches!(meta, Some(super::PrPeekMeta::Ready(_))));
}

// -----------------------------------------------------------------
// Compare progress — end-to-end through the event lifecycle
// -----------------------------------------------------------------

use super::{ComparePhase, CompareProgress, LoadingSubject};
use crate::events::{CompareFinished, RepositorySyncReason};

fn compare_ready_state() -> AppState {
    let state = AppState::default();
    state
        .compare
        .repo_path
        .set(&state.store, Some(PathBuf::from("/repo")));
    state.compare.left_ref.set(&state.store, "v5.0".to_owned());
    state.compare.right_ref.set(&state.store, "v5.1".to_owned());
    state.compare.mode.set(&state.store, CompareMode::TwoDot);
    state
}

#[test]
fn kickoff_compare_seeds_progress_with_labels_and_started_at() {
    let mut state = compare_ready_state();
    state.clock_ms = 1_000;
    let _ = state.kickoff_compare();

    let progress = state
        .workspace
        .compare_progress
        .with(&state.store, |p| p.clone())
        .expect("progress should be populated");
    match &progress.subject {
        LoadingSubject::Compare {
            left_label,
            right_label,
        } => {
            assert_eq!(left_label, "v5.0");
            assert_eq!(right_label, "v5.1");
        }
        other => panic!("expected Compare subject, got {other:?}"),
    }
    assert_eq!(progress.started_at_ms, 1_000);
    assert_eq!(progress.phase, ComparePhase::OpeningRepo);
    assert_eq!(progress.file_count_total, None);
    assert_eq!(
        state.workspace.mode.get(&state.store),
        WorkspaceMode::Loading,
        "viewport should flip to loading so the panel actually renders"
    );
}

#[test]
fn compare_progress_update_applies_only_when_generation_matches() {
    let mut state = compare_ready_state();
    let _ = state.kickoff_compare();
    let generation = state.workspace.compare_generation.get(&state.store);

    // Stale reporter — must be ignored.
    state.apply_event(AppEvent::from(CompareEvent::CompareProgressUpdate {
        generation: generation.wrapping_sub(1),
        phase: ComparePhase::EnumeratingChanges,
    }));
    assert_eq!(
        state
            .workspace
            .compare_progress
            .with(&state.store, |p| p.as_ref().unwrap().phase),
        ComparePhase::OpeningRepo,
        "stale generation must not advance the phase"
    );

    // Fresh reporter — applies.
    state.apply_event(AppEvent::from(CompareEvent::CompareProgressUpdate {
        generation,
        phase: ComparePhase::EnumeratingChanges,
    }));
    assert_eq!(
        state
            .workspace
            .compare_progress
            .with(&state.store, |p| p.as_ref().unwrap().phase),
        ComparePhase::EnumeratingChanges,
    );
}

#[test]
fn loading_files_phase_updates_counts_on_struct() {
    let mut state = compare_ready_state();
    let _ = state.kickoff_compare();
    let generation = state.workspace.compare_generation.get(&state.store);

    state.apply_event(AppEvent::from(CompareEvent::CompareProgressUpdate {
        generation,
        phase: ComparePhase::LoadingFiles {
            files_seen: 142,
            files_total: 3_891,
        },
    }));

    let progress = state
        .workspace
        .compare_progress
        .with(&state.store, |p| p.clone())
        .expect("progress exists");
    assert_eq!(progress.files_loaded, 142);
    assert_eq!(progress.file_count_total, Some(3_891));
    assert!(matches!(progress.phase, ComparePhase::LoadingFiles { .. }));
}

#[test]
fn kickoff_with_prior_state_reveals_loading_immediately() {
    let mut state = compare_ready_state();
    // Simulate a previously loaded compare (files present).
    state.workspace.files.set(
        &state.store,
        vec![FileListEntry {
            path: "old.rs".into(),
        }],
    );
    state.clock_ms = 10_000;

    let _ = state.kickoff_compare();
    let progress = state
        .workspace
        .compare_progress
        .with(&state.store, |p| p.clone())
        .expect("progress populated");
    assert_eq!(progress.started_at_ms, 10_000);
    assert_eq!(
        progress.reveal_at_ms, 10_000,
        "compare loading should be visible immediately"
    );
    assert_ne!(
        state.workspace.mode.get(&state.store),
        WorkspaceMode::Loading
    );
    // Prior files are preserved so fast compares don't cause a flash.
    assert_eq!(state.workspace.files.with(&state.store, |f| f.len()), 1);
}

#[test]
fn open_repository_seeds_repo_subject_progress() {
    let mut state = AppState::default();
    state.clock_ms = 500;

    let effects = state.open_repository(PathBuf::from("/tmp/linux"));

    let progress = state
        .workspace
        .compare_progress
        .with(&state.store, |p| p.clone())
        .expect("progress seeded for repo open");
    match &progress.subject {
        LoadingSubject::RepoOpen { name } => {
            assert_eq!(name, "linux");
        }
        other => panic!("expected RepoOpen subject, got {other:?}"),
    }
    assert_eq!(progress.phase, ComparePhase::OpeningRepo);
    assert_eq!(
        progress.reveal_at_ms,
        500 + super::COMPARE_REVEAL_DELAY_MS,
        "every repo open delays reveal so sub-threshold opens don't flash"
    );
    // Reporter generation is threaded through the SyncRepository effect
    // so the worker's phase events stamp the matching generation.
    let sync_gen = effects.iter().find_map(|eff| match eff {
        Effect::Repository(RepositoryEffect::SyncRepository {
            reporter_generation,
            ..
        }) => *reporter_generation,
        _ => None,
    });
    assert_eq!(sync_gen, Some(progress.generation));
}

#[test]
fn open_repository_with_prior_diff_delays_reveal() {
    let mut state = AppState::default();
    state.workspace.files.set(
        &state.store,
        vec![FileListEntry {
            path: "old.rs".into(),
        }],
    );
    state.clock_ms = 10_000;

    let _ = state.open_repository(PathBuf::from("/tmp/other"));

    let progress = state
        .workspace
        .compare_progress
        .with(&state.store, |p| p.clone())
        .expect("progress seeded");
    assert_eq!(
        progress.reveal_at_ms, 10_500,
        "re-open with prior diff delays reveal by COMPARE_REVEAL_DELAY_MS"
    );
}

#[test]
fn open_repository_resets_stale_compare_refs_before_snapshot() {
    let mut state = AppState::default();
    state.compare.left_ref.set(&state.store, "@-".to_owned());
    state.compare.right_ref.set(&state.store, "@".to_owned());
    state.compare.mode.set(&state.store, CompareMode::TwoDot);

    let path = PathBuf::from("/tmp/git-repo");
    let effects = state.open_repository(path.clone());

    assert_eq!(state.compare.left_ref.get(&state.store), "");
    assert_eq!(state.compare.right_ref.get(&state.store), "");
    assert_eq!(state.compare.mode.get(&state.store), CompareMode::default());
    let saved = effects.iter().find_map(|effect| match effect {
        Effect::Settings(SettingsEffect::SaveSettings(settings)) => settings.last_compare.as_ref(),
        _ => None,
    });
    let saved = saved.expect("open_repository should persist settings");
    assert_eq!(saved.repo_path.as_ref(), Some(&path));
    assert_eq!(saved.left_ref, "");
    assert_eq!(saved.right_ref, "");
}

#[test]
fn git_snapshot_after_jj_refs_uses_git_defaults() {
    let mut state = AppState::default();
    state.compare.left_ref.set(&state.store, "@-".to_owned());
    state.compare.right_ref.set(&state.store, "@".to_owned());
    state.compare.mode.set(&state.store, CompareMode::TwoDot);

    let path = PathBuf::from("/tmp/git-repo");
    let _ = state.open_repository(path.clone());
    state.apply_event(AppEvent::from(RepositoryEvent::RepositorySnapshotReady(
        crate::events::RepositorySnapshot::from_vcs_snapshot(
            crate::core::vcs::model::VcsSnapshot {
                location: RepoLocation {
                    kind: VcsKind::GIT,
                    profile: crate::core::vcs::model::VCS_PROFILE_GIT,
                    workspace_root: path,
                    store_root: None,
                },
                reason: RepositorySyncReason::Open,
                change_kind: None,
                capabilities: RepoCapabilities::git(),
                refs: Vec::new(),
                changes: Vec::new(),
                operation_log: Vec::new(),
                file_changes: Vec::new(),
            },
        ),
    )));

    let (left, right, mode) =
        crate::ui::vcs::profile(state.repository.location.get(&state.store).as_ref())
            .default_compare();
    assert_eq!(state.compare.left_ref.get(&state.store), left);
    assert_eq!(state.compare.right_ref.get(&state.store), right);
    assert_eq!(state.compare.mode.get(&state.store), mode);
}

#[test]
fn large_compare_stats_stream_offscreen_background_rows_after_visible_rows() {
    let state = compare_ready_state();
    state
        .workspace
        .source
        .set(&state.store, WorkspaceSource::Compare);
    state.workspace.mode.set(&state.store, WorkspaceMode::Ready);
    state
        .compare
        .repo_path
        .set(&state.store, Some(PathBuf::from("/repo")));
    state.file_list.row_height.set(&state.store, 36.0);
    state.file_list.gap.set(&state.store, 4.0);
    state.file_list.viewport_height.set(&state.store, 80.0);

    let summaries = (0..=super::COMPARE_STATS_VISIBLE_ONLY_FILE_LIMIT)
        .map(|index| {
            let path = format!("src/file-{index}.rs");
            let mut summary = CompareFileSummary::from_paths_status(
                Some(&path),
                Some(&path),
                carbon::FileStatus::Modified,
                true,
            );
            if index < 128 {
                summary.stats_deferred = false;
            }
            summary
        })
        .collect();
    state.workspace.compare_output.set(
        &state.store,
        Some(CompareOutput {
            file_summaries: summaries,
            ..CompareOutput::default()
        }),
    );

    let effect = state
        .next_compare_stats_hydration_effect()
        .expect("huge compares should keep streaming offscreen stats");

    match effect {
        Effect::Compare(CompareEffect::LoadFileStats(task)) => {
            assert_eq!(task.request.priority, CompareWorkPriority::Warmup);
            assert_eq!(task.request.files.first().map(|item| item.index), Some(128));
        }
        other => panic!("expected LoadFileStats effect, got {other:?}"),
    }
}

#[test]
fn large_compare_still_loads_exact_total_stats() {
    let mut state = compare_ready_state();
    state
        .workspace
        .source
        .set(&state.store, WorkspaceSource::Compare);
    state
        .compare
        .repo_path
        .set(&state.store, Some(PathBuf::from("/repo")));

    let summaries = (0..=super::COMPARE_STATS_VISIBLE_ONLY_FILE_LIMIT)
        .map(|index| {
            let path = format!("src/file-{index}.rs");
            CompareFileSummary::from_paths_status(
                Some(&path),
                Some(&path),
                carbon::FileStatus::Modified,
                true,
            )
        })
        .collect();
    state.workspace.compare_output.set(
        &state.store,
        Some(CompareOutput {
            file_summaries: summaries,
            ..CompareOutput::default()
        }),
    );

    let effect = state
        .start_compare_total_stats_if_needed()
        .expect("large deferred compares should request one bounded total-stats job");

    match effect {
        Effect::Compare(CompareEffect::LoadStats(task)) => {
            assert_eq!(task.request.priority, CompareWorkPriority::TotalStats);
            assert!(
                state
                    .workspace
                    .compare_total_stats_loading
                    .get(&state.store)
            );
        }
        other => panic!("expected LoadStats effect, got {other:?}"),
    }
}

#[test]
fn filtered_compare_stats_hydrates_filtered_visible_raw_indices() {
    let state = compare_ready_state();
    state
        .workspace
        .source
        .set(&state.store, WorkspaceSource::Compare);
    state.file_list.row_height.set(&state.store, 36.0);
    state.file_list.gap.set(&state.store, 4.0);
    state.file_list.viewport_height.set(&state.store, 80.0);
    state
        .file_list
        .filter
        .set(&state.store, "target-only".to_owned());

    let summaries = (0..50)
        .map(|index| {
            let path = if index == 40 {
                "src/target-only.rs".to_owned()
            } else {
                format!("src/file-{index}.rs")
            };
            CompareFileSummary::from_paths_status(
                Some(&path),
                Some(&path),
                carbon::FileStatus::Modified,
                true,
            )
        })
        .collect();
    state.workspace.compare_output.set(
        &state.store,
        Some(CompareOutput {
            file_summaries: summaries,
            ..CompareOutput::default()
        }),
    );

    let items = state.visible_compare_stats_hydration_items();

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].index, 40);
}

#[test]
fn tree_compare_stats_hydrates_visible_tree_file_indices() {
    let state = compare_ready_state();
    state
        .workspace
        .source
        .set(&state.store, WorkspaceSource::Compare);
    state
        .file_list
        .mode
        .set(&state.store, SidebarMode::TreeView);
    state
        .file_list
        .expanded_folders
        .set(&state.store, ["a".to_owned()].into_iter().collect());
    state.file_list.row_height.set(&state.store, 36.0);
    state.file_list.gap.set(&state.store, 4.0);
    state.file_list.viewport_height.set(&state.store, 80.0);

    let summaries = (0..50)
        .map(|index| {
            let path = if index == 40 {
                "a/target-visible.rs".to_owned()
            } else {
                format!("z/file-{index}.rs")
            };
            CompareFileSummary::from_paths_status(
                Some(&path),
                Some(&path),
                carbon::FileStatus::Modified,
                true,
            )
        })
        .collect();
    state.workspace.compare_output.set(
        &state.store,
        Some(CompareOutput {
            file_summaries: summaries,
            ..CompareOutput::default()
        }),
    );

    let items = state.visible_compare_stats_hydration_items();

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].index, 40);
}

#[test]
fn loaded_compare_stats_update_sidebar_meta() {
    let mut state = compare_ready_state();
    state
        .workspace
        .source
        .set(&state.store, WorkspaceSource::Compare);
    state
        .file_list
        .mode
        .set(&state.store, SidebarMode::TreeView);
    state.workspace.compare_output.set(
        &state.store,
        Some(CompareOutput {
            file_summaries: vec![CompareFileSummary::from_paths_status(
                None,
                Some("arch/arm64/boot/dts/mediatek/mt8183-kukui-jacuzzi-kenzo.dts"),
                carbon::FileStatus::Added,
                true,
            )],
            ..CompareOutput::default()
        }),
    );

    let effects = state.handle_compare_file_stats_ready(CompareFileStatsReady {
        generation: state.workspace.compare_generation.get(&state.store),
        stats: vec![CompareFileStat {
            index: 0,
            path: "arch/arm64/boot/dts/mediatek/mt8183-kukui-jacuzzi-kenzo.dts".to_owned(),
            additions: 13,
            deletions: 0,
        }],
        request_complete: false,
    });

    assert!(effects.is_empty());
    let meta = state.file_list_entry_meta(0);
    assert_eq!(meta.additions, 13);
    assert_eq!(meta.deletions, 0);
    assert!(
        !state.workspace.compare_output.with(&state.store, |output| {
            output
                .as_ref()
                .and_then(|output| output.file_summaries.first())
                .is_none_or(|summary| summary.stats_deferred)
        }),
        "loaded stats must clear the deferred marker used by sidebar rows",
    );
}

#[test]
fn expanding_tree_folder_starts_visible_stats_hydration() {
    let mut state = compare_ready_state();
    state
        .workspace
        .source
        .set(&state.store, WorkspaceSource::Compare);
    state
        .compare
        .repo_path
        .set(&state.store, Some(PathBuf::from("/repo")));
    state
        .file_list
        .mode
        .set(&state.store, SidebarMode::TreeView);
    state.file_list.row_height.set(&state.store, 36.0);
    state.file_list.gap.set(&state.store, 4.0);
    state.file_list.viewport_height.set(&state.store, 80.0);

    let summaries = (0..=super::COMPARE_STATS_VISIBLE_ONLY_FILE_LIMIT)
        .map(|index| {
            let path = if index == 40 {
                "a/target-visible.rs".to_owned()
            } else {
                format!("z/file-{index}.rs")
            };
            CompareFileSummary::from_paths_status(
                Some(&path),
                Some(&path),
                carbon::FileStatus::Modified,
                true,
            )
        })
        .collect();
    state.workspace.compare_output.set(
        &state.store,
        Some(CompareOutput {
            file_summaries: summaries,
            ..CompareOutput::default()
        }),
    );
    state.set_compare_stats_hydration(super::CompareStatsHydrationState::Running);

    let effects = state.apply_action(crate::actions::FileListAction::ToggleFolder("a".to_owned()));

    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::Compare(CompareEffect::LoadFileStats(task))
                if task.request.priority == CompareWorkPriority::VisibleSidebarStats
                    && task.request.files.iter().any(|item| item.index == 40)
        )
    }));
}

#[test]
fn compare_stats_ready_drains_history_when_hydration_has_no_visible_work() {
    let mut state = compare_ready_state();
    state
        .workspace
        .source
        .set(&state.store, WorkspaceSource::Compare);
    state.file_list.tab.set(&state.store, SidebarTab::Commits);
    state.workspace.compare_history_pending.set(
        &state.store,
        Some(crate::effects::CompareHistoryRequest {
            repo_path: PathBuf::from("/repo"),
            left_ref: "v5.0".to_owned(),
            right_ref: "v5.1".to_owned(),
        }),
    );
    let summaries = (0..=super::COMPARE_STATS_VISIBLE_ONLY_FILE_LIMIT)
        .map(|index| {
            let path = format!("src/file-{index}.rs");
            CompareFileSummary::from_paths_status(
                Some(&path),
                Some(&path),
                carbon::FileStatus::Modified,
                true,
            )
        })
        .collect();
    state.workspace.compare_output.set(
        &state.store,
        Some(CompareOutput {
            file_summaries: summaries,
            ..CompareOutput::default()
        }),
    );

    let effects = state.handle_compare_stats_ready(CompareStatsReady {
        generation: state.workspace.compare_generation.get(&state.store),
        additions: 0,
        deletions: 0,
    });

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Compare(CompareEffect::LoadHistory(_))))
    );
    assert!(
        state
            .workspace
            .compare_history_pending
            .get(&state.store)
            .is_none()
    );
}

#[test]
fn compare_file_stats_failure_does_not_retry_same_chunk() {
    let mut state = compare_ready_state();
    state
        .workspace
        .source
        .set(&state.store, WorkspaceSource::Compare);
    state.set_compare_stats_hydration(super::CompareStatsHydrationState::Running);
    state.workspace.compare_history_pending.set(
        &state.store,
        Some(crate::effects::CompareHistoryRequest {
            repo_path: PathBuf::from("/repo"),
            left_ref: "v5.0".to_owned(),
            right_ref: "v5.1".to_owned(),
        }),
    );
    state.workspace.compare_output.set(
        &state.store,
        Some(CompareOutput {
            file_summaries: vec![CompareFileSummary::from_paths_status(
                Some("src/file.rs"),
                Some("src/file.rs"),
                carbon::FileStatus::Modified,
                true,
            )],
            ..CompareOutput::default()
        }),
    );

    let effects = state.apply_event(AppEvent::from(CompareEvent::CompareFileStatsFailed {
        generation: state.workspace.compare_generation.get(&state.store),
        message: "backend failed".to_owned(),
    }));

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Compare(CompareEffect::LoadFileStats(_)))),
        "failed stats hydration should not immediately retry the same deferred chunk"
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Compare(CompareEffect::LoadHistory(_))))
    );
    assert!(
        state
            .workspace
            .compare_history_pending
            .get(&state.store)
            .is_none()
    );
    assert!(state.compare_stats_hydration_failed());
}

#[test]
fn repository_snapshot_ready_clears_repo_open_progress() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/linux");
    let _ = state.open_repository(path.clone());
    assert!(
        state
            .workspace
            .compare_progress
            .with(&state.store, |p| p.is_some())
    );

    state.apply_event(AppEvent::from(RepositoryEvent::RepositorySnapshotReady(
        crate::events::RepositorySnapshot::from_vcs_snapshot(
            crate::core::vcs::model::VcsSnapshot {
                location: RepoLocation {
                    kind: VcsKind::GIT,
                    profile: crate::core::vcs::model::VCS_PROFILE_GIT,
                    workspace_root: path,
                    store_root: None,
                },
                reason: RepositorySyncReason::Open,
                change_kind: None,
                capabilities: RepoCapabilities::git(),
                refs: Vec::new(),
                changes: Vec::new(),
                operation_log: Vec::new(),
                file_changes: Vec::new(),
            },
        ),
    )));

    assert!(
        state
            .workspace
            .compare_progress
            .with(&state.store, |p| p.is_none()),
        "snapshot-ready must tear down the repo-open progress panel"
    );
}

#[test]
fn kickoff_without_prior_state_reveals_loading_immediately() {
    let mut state = compare_ready_state();
    state.clock_ms = 5_000;

    let _ = state.kickoff_compare();
    let progress = state
        .workspace
        .compare_progress
        .with(&state.store, |p| p.clone())
        .expect("progress populated");
    assert_eq!(progress.started_at_ms, 5_000);
    assert_eq!(
        progress.reveal_at_ms, 5_000,
        "compare loading should be visible immediately"
    );
    // With no prior state to preserve, workspace_mode flips to Loading
    // up front so the editor/ready-hint stops rendering in the background.
    assert_eq!(
        state.workspace.mode.get(&state.store),
        WorkspaceMode::Loading
    );
}

#[test]
fn cancel_compare_bumps_generation_and_drops_stale_result() {
    let mut state = compare_ready_state();
    let _ = state.kickoff_compare();
    let generation = state.workspace.compare_generation.get(&state.store);

    let _ = state.cancel_compare();

    assert!(
        state
            .workspace
            .compare_progress
            .with(&state.store, |p| p.is_none()),
        "progress should be cleared after cancel"
    );
    let new_gen = state.workspace.compare_generation.get(&state.store);
    assert!(new_gen > generation, "generation should be bumped");
    assert_eq!(
        state.workspace.mode.get(&state.store),
        WorkspaceMode::Empty,
        "fresh-state cancel should revert the Loading flip"
    );

    // A stale CompareFinished arriving after cancel must be silently dropped.
    state.apply_event(AppEvent::from(CompareEvent::CompareFinished(
        CompareFinished {
            generation,
            request: vcs_compare_request(
                CompareMode::TwoDot,
                "v5.0".to_owned(),
                "v5.1".to_owned(),
                LayoutMode::Unified,
                RendererKind::Builtin,
            ),
            resolved_left: "deadbeef".to_owned(),
            resolved_right: "cafefeed".to_owned(),
            output: CompareOutput::default(),
            range_commits: Vec::new(),
        },
    )));
    assert_eq!(
        state.workspace.mode.get(&state.store),
        WorkspaceMode::Empty,
        "stale finished result must not promote workspace to Ready",
    );
    assert!(
        state
            .workspace
            .compare_progress
            .with(&state.store, |p| p.is_none()),
        "stale finished result must not re-seed progress",
    );
}

#[test]
fn cancel_compare_preserves_previous_diff_on_recompare() {
    let mut state = compare_ready_state();
    // Prior state: an existing file in the workspace.
    state.workspace.files.set(
        &state.store,
        vec![FileListEntry {
            path: "old.rs".into(),
        }],
    );
    state.workspace.mode.set(&state.store, WorkspaceMode::Ready);

    let _ = state.kickoff_compare();
    let _ = state.cancel_compare();

    assert!(
        state
            .workspace
            .compare_progress
            .with(&state.store, |p| p.is_none()),
        "progress cleared on cancel"
    );
    assert_eq!(
        state.workspace.mode.get(&state.store),
        WorkspaceMode::Ready,
        "previous workspace state is preserved on cancel — no blanking"
    );
    assert_eq!(
        state.workspace.files.with(&state.store, |f| f.len()),
        1,
        "prior file list must not be wiped by cancel"
    );
}

#[test]
fn compare_finished_advances_phase_and_records_file_count() {
    let mut state = compare_ready_state();
    let _ = state.kickoff_compare();
    let generation = state.workspace.compare_generation.get(&state.store);

    // Simulate a successful compare with 3 files.
    let files = ["a.rs", "b.rs", "c.rs"];
    let output = CompareOutput {
        carbon: carbon::DiffDocument {
            files: files
                .iter()
                .enumerate()
                .map(|(index, path)| carbon_summary_for_path(index, path))
                .collect(),
        },
        ..CompareOutput::default()
    };

    state.apply_event(AppEvent::from(CompareEvent::CompareFinished(
        CompareFinished {
            generation,
            request: vcs_compare_request(
                CompareMode::TwoDot,
                "v5.0".to_owned(),
                "v5.1".to_owned(),
                LayoutMode::Unified,
                RendererKind::Builtin,
            ),
            resolved_left: "deadbeef".to_owned(),
            resolved_right: "cafefeed".to_owned(),
            output,
            range_commits: Vec::new(),
        },
    )));

    // Small files load synchronously, so progress is already cleared by the
    // time handle_compare_finished returns. We at least know the workspace
    // is Ready and the compare file view is populated from CompareOutput.
    assert_eq!(state.workspace.mode.get(&state.store), WorkspaceMode::Ready,);
    assert_eq!(state.workspace_file_count(), 3);
}

#[test]
fn compare_failed_clears_progress_and_marks_workspace_empty() {
    let mut state = compare_ready_state();
    let _ = state.kickoff_compare();
    let generation = state.workspace.compare_generation.get(&state.store);

    state.apply_event(AppEvent::from(CompareEvent::CompareFailed {
        generation,
        message: "boom".to_owned(),
    }));

    assert_eq!(state.workspace.mode.get(&state.store), WorkspaceMode::Empty,);
    assert!(
        state
            .workspace
            .compare_progress
            .with(&state.store, |p| p.is_none()),
        "progress panel must tear down on compare failure",
    );
}

#[test]
fn compare_progress_label_does_not_panic_for_all_phases() {
    // Non-empty labels matter for the title-bar fallback. Cheap to
    // check exhaustively.
    let phases = [
        ComparePhase::OpeningRepo,
        ComparePhase::ResolvingRefs,
        ComparePhase::EnumeratingChanges,
        ComparePhase::LoadingFiles {
            files_seen: 142,
            files_total: 3_891,
        },
        ComparePhase::FetchingHistory,
        ComparePhase::PopulatingList,
        ComparePhase::RenderingFirstFile,
    ];
    for phase in phases {
        let label = phase.label();
        assert!(!label.is_empty());
    }
    // LoadingFiles label should interpolate counts.
    assert!(
        ComparePhase::LoadingFiles {
            files_seen: 142,
            files_total: 3_891,
        }
        .label()
        .contains("142"),
        "file counts must appear in the label"
    );

    let _ = CompareProgress {
        generation: 0,
        phase: ComparePhase::default(),
        subject: LoadingSubject::Compare {
            left_label: String::new(),
            right_label: String::new(),
        },
        started_at_ms: 0,
        reveal_at_ms: 0,
        file_count_total: None,
        files_loaded: 0,
    };
}
