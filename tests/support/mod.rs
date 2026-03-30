use std::path::PathBuf;

use diffy::core::vcs::github::{DeviceFlowState, PullRequestInfo};
#[cfg(feature = "capture")]
use diffy::render::capture::{scene_to_png, scene_to_rgba};
use diffy::render::{Primitive, Rect, TextMetrics};
use diffy::ui::editor::element::EditorElement;
use diffy::ui::element::{ElementContext, ScrollActionBuilder};
use diffy::ui::shell::{UiFrame, build_ui_frame};
use diffy::ui::signals::SignalStore;
use diffy::ui::state::{
    AppState, AsyncStatus, CommandPaletteState, FileListEntry, FocusTarget, OverlayEntry,
    OverlayListState, OverlaySurface, PaletteCommand, PaletteEntry, PaletteEntryKind, PickerEntry,
    PickerKind, PickerState, Toast, ToastKind, WorkspaceMode,
};
use diffy::ui::theme::Theme;

pub const TEST_WIDTH: u32 = 1320;
pub const TEST_HEIGHT: u32 = 840;

pub fn render_frame(state: &mut AppState) -> UiFrame {
    render_frame_in(state, TEST_WIDTH as f32, TEST_HEIGHT as f32)
}

pub fn render_frame_in(state: &mut AppState, width: f32, height: f32) -> UiFrame {
    let theme = Theme::default_dark().with_ui_scale(state.ui_scale_factor());
    let mut font_system = diffy::fonts::new_font_system();
    let mut store = SignalStore::new();
    let mut cx = ElementContext::new(&theme, 1.0, &mut font_system, None, &mut store);
    let mut editor = EditorElement::default();

    build_ui_frame(
        state,
        &theme,
        &mut editor,
        TextMetrics::default(),
        width,
        height,
        &mut cx,
    )
}

pub fn empty_state_with_recents() -> AppState {
    let mut state = AppState::default();
    state.settings.recent_repos = vec![
        PathBuf::from("C:\\work\\diffy"),
        PathBuf::from("C:\\work\\zed"),
        PathBuf::from("C:\\work\\rust-analyzer"),
    ];
    state
}

pub fn ready_state_with_files(file_count: usize) -> AppState {
    let mut state = AppState::default();
    state.workspace_mode = WorkspaceMode::Ready;
    state.compare.repo_path = Some(PathBuf::from("C:\\work\\diffy"));
    state.compare.left_ref = "main".to_owned();
    state.compare.right_ref = "feature/native-ui".to_owned();
    state.compare.resolved_left = Some("abc1234".to_owned());
    state.compare.resolved_right = Some("def5678".to_owned());
    state.repository.status = AsyncStatus::Ready;
    state.workspace.files = (0..file_count)
        .map(|index| FileListEntry {
            path: format!("src/file_{index}.rs"),
            status: "M".to_owned(),
            additions: (index as i32 + 1) * 3,
            deletions: index as i32,
            is_binary: false,
        })
        .collect();
    if let Some(first) = state.workspace.files.first() {
        state.workspace.selected_file_index = Some(0);
        state.workspace.selected_file_path = Some(first.path.clone());
    }
    state.file_list.viewport_height = 180.0;
    state
}

pub fn compare_sheet_state() -> AppState {
    let mut state = ready_state_with_files(6);
    state.overlays.stack.push(OverlayEntry {
        surface: OverlaySurface::CompareSheet,
        focus_return: Some(FocusTarget::TitleBar),
    });
    state.focus.current = Some(FocusTarget::CompareLeftRef);
    state
}

pub fn repo_picker_state(entry_count: usize) -> AppState {
    let mut state = AppState::default();
    state.compare.repo_path = Some(PathBuf::from("C:\\work\\diffy"));
    state.overlays.stack.push(OverlayEntry {
        surface: OverlaySurface::RepoPicker,
        focus_return: Some(FocusTarget::TitleBar),
    });
    state.overlays.picker = PickerState {
        kind: PickerKind::Repository,
        query: "diff".to_owned(),
        entries: (0..entry_count)
            .map(|index| PickerEntry {
                label: format!("repo-{index}"),
                detail: format!("C:\\work\\repo-{index}"),
                value: format!("C:\\work\\repo-{index}"),
            })
            .collect(),
        selected_index: entry_count.saturating_sub(1).min(2),
        list: OverlayListState {
            viewport_height_px: 204,
            ..OverlayListState::default()
        },
    };
    state.focus.current = Some(FocusTarget::PickerInput);
    state
}

pub fn command_palette_state(entry_count: usize) -> AppState {
    let mut state = ready_state_with_files(8);
    state.overlays.stack.push(OverlayEntry {
        surface: OverlaySurface::CommandPalette,
        focus_return: Some(FocusTarget::TitleBar),
    });
    state.overlays.command_palette = CommandPaletteState {
        query: "open".to_owned(),
        entries: (0..entry_count)
            .map(|index| PaletteEntry {
                label: format!("Open item {index}"),
                detail: format!("detail-{index}"),
                kind: PaletteEntryKind::Command(if index % 2 == 0 {
                    PaletteCommand::OpenCompareSheet
                } else {
                    PaletteCommand::FocusViewport
                }),
            })
            .collect(),
        selected_index: entry_count.saturating_sub(1).min(3),
        list: OverlayListState {
            viewport_height_px: 288,
            ..OverlayListState::default()
        },
    };
    state.focus.current = Some(FocusTarget::CommandPaletteInput);
    state
}

pub fn pull_request_modal_state() -> AppState {
    let mut state = ready_state_with_files(3);
    state.overlays.stack.push(OverlayEntry {
        surface: OverlaySurface::PullRequestModal,
        focus_return: Some(FocusTarget::TitleBar),
    });
    state.focus.current = Some(FocusTarget::PullRequestInput);
    state.github.pull_request.url_input = "https://github.com/owner/repo/pull/42".to_owned();
    state.github.pull_request.info = Some(PullRequestInfo {
        title: "Improve scroll plumbing".to_owned(),
        state: "open".to_owned(),
        author_login: "ro".to_owned(),
        number: 42,
        additions: 12,
        deletions: 3,
        changed_files: 2,
        base_branch: "main".to_owned(),
        head_branch: "feature/native-ui".to_owned(),
        base_sha: "abc".to_owned(),
        head_sha: "def".to_owned(),
        base_repo_url: "https://github.com/owner/repo.git".to_owned(),
        head_repo_url: "https://github.com/owner/repo.git".to_owned(),
    });
    state
}

pub fn auth_modal_state(with_device_flow: bool) -> AppState {
    let mut state = AppState::default();
    state.overlays.stack.push(OverlayEntry {
        surface: OverlaySurface::GitHubAuthModal,
        focus_return: Some(FocusTarget::TitleBar),
    });
    state.focus.current = Some(FocusTarget::AuthPrimaryAction);
    state.github.auth.token_present = false;
    state.github.auth.device_flow = with_device_flow.then_some(DeviceFlowState {
        device_code: "device-code".to_owned(),
        user_code: "ABCD-EFGH".to_owned(),
        verification_uri: "https://github.com/login/device".to_owned(),
        interval: 5,
    });
    state
}

pub fn toasts_state() -> AppState {
    let mut state = ready_state_with_files(2);
    state.toasts = vec![
        Toast {
            id: 1,
            kind: ToastKind::Info,
            message: "Compare completed in 142ms".to_owned(),
            created_at_ms: 0,
            hovered: false,
        },
        Toast {
            id: 2,
            kind: ToastKind::Error,
            message: "Failed to resolve ref 'origin/old-branch'".to_owned(),
            created_at_ms: 0,
            hovered: false,
        },
    ];
    state
}

pub fn scene_contains_text(frame: &UiFrame, needle: &str) -> bool {
    frame
        .scene
        .primitives
        .iter()
        .any(|primitive| match primitive {
            Primitive::TextRun(text) => text.text.contains(needle),
            Primitive::RichTextRun(text) => {
                text.spans.iter().any(|span| span.text.contains(needle))
            }
            _ => false,
        })
}

pub fn scene_text_rect(frame: &UiFrame, needle: &str) -> Option<Rect> {
    frame
        .scene
        .primitives
        .iter()
        .find_map(|primitive| match primitive {
            Primitive::TextRun(text) if text.text.contains(needle) => Some(text.rect),
            Primitive::RichTextRun(text)
                if text.spans.iter().any(|span| span.text.contains(needle)) =>
            {
                Some(text.rect)
            }
            _ => None,
        })
}

pub fn largest_rounded_rect(frame: &UiFrame) -> Option<Rect> {
    frame
        .scene
        .primitives
        .iter()
        .fold(None, |largest, primitive| {
            let Some(rect) = (match primitive {
                Primitive::RoundedRect(rounded) => Some(rounded.rect),
                _ => None,
            }) else {
                return largest;
            };

            match largest {
                Some(current) if current.width * current.height >= rect.width * rect.height => {
                    Some(current)
                }
                _ => Some(rect),
            }
        })
}

pub fn has_hit(frame: &UiFrame, predicate: impl Fn(&diffy::ui::actions::Action) -> bool) -> bool {
    frame.hits.iter().any(|hit| predicate(&hit.action))
}

pub fn count_hits(
    frame: &UiFrame,
    predicate: impl Fn(&diffy::ui::actions::Action) -> bool,
) -> usize {
    frame
        .hits
        .iter()
        .filter(|hit| predicate(&hit.action))
        .count()
}

pub fn has_text_input_for(frame: &UiFrame, target: FocusTarget) -> bool {
    frame
        .text_input_hit_areas
        .iter()
        .any(|area| area.focus_target == target)
}

pub fn has_scroll_region(
    frame: &UiFrame,
    predicate: impl Fn(&ScrollActionBuilder) -> bool,
) -> bool {
    frame
        .scroll_regions
        .iter()
        .any(|region| predicate(&region.action_builder))
}

#[cfg(feature = "capture")]
pub fn frame_fingerprint(frame: &UiFrame) -> u64 {
    let rgba = scene_to_rgba(&frame.scene, TEST_WIDTH, TEST_HEIGHT);
    fnv1a64(&rgba)
}

#[cfg(feature = "capture")]
pub fn assert_scene_snapshot(name: &str, state: &mut AppState, expected: u64) {
    let frame = render_frame(state);
    let actual = frame_fingerprint(&frame);
    if actual != expected {
        let dir = std::path::Path::new("target/captures");
        std::fs::create_dir_all(dir).ok();
        let path = dir.join(format!("{name}.actual.png"));
        scene_to_png(&frame.scene, TEST_WIDTH, TEST_HEIGHT, &path);
        panic!(
            "snapshot fingerprint mismatch for {name}: expected {expected:#016x}, got {actual:#016x}; wrote {}",
            path.display()
        );
    }
}

#[cfg(feature = "capture")]
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
