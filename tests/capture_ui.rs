use std::path::PathBuf;

use diffy::render::capture::scene_to_png;
use diffy::render::TextMetrics;
use diffy::ui::diff_viewport::runtime::DiffViewportRuntime;
use diffy::ui::element::*;
use diffy::ui::shell::build_ui_frame;
use diffy::ui::signals::SignalStore;
use diffy::ui::state::{
    AppState, FileListEntry, OverlayEntry, OverlaySurface, ToastKind, WorkspaceMode,
};
use diffy::ui::theme::Theme;

// ---------------------------------------------------------------------------
// Shared render helper
// ---------------------------------------------------------------------------

fn capture_frame(name: &str, width: u32, height: u32, state: &mut AppState) {
    let theme = Theme::default_dark();
    let mut font_system = diffy::fonts::new_font_system();
    let mut store = SignalStore::new();
    let mut cx = ElementContext::new(&theme, 1.0, &mut font_system, None, &mut store);
    let mut viewport_runtime = DiffViewportRuntime::default();
    let text_metrics = TextMetrics::default();

    let frame = build_ui_frame(
        state,
        &theme,
        &mut viewport_runtime,
        text_metrics,
        width as f32,
        height as f32,
        &mut cx,
    );

    let dir = std::path::Path::new("target/captures");
    std::fs::create_dir_all(dir).ok();
    let path = dir.join(format!("{name}.png"));
    scene_to_png(&frame.scene, width, height, &path);
    eprintln!("captured: {}", path.display());
}

// ---------------------------------------------------------------------------
// Captures
// ---------------------------------------------------------------------------

/// Empty state — no repo, shows welcome card with recent repos.
#[test]
fn capture_empty_state() {
    let mut state = AppState::default();
    state.settings.recent_repos = vec![
        PathBuf::from("C:\\work\\diffy"),
        PathBuf::from("C:\\work\\react"),
        PathBuf::from("C:\\work\\linear-app"),
        PathBuf::from("C:\\work\\rustls"),
    ];

    capture_frame("empty_state", 1320, 840, &mut state);
}

/// File list with selection — 7 files, third selected, status ready.
#[test]
fn capture_file_list() {
    let mut state = AppState::default();
    state.workspace_mode = WorkspaceMode::Ready;
    state.compare.repo_path = Some(PathBuf::from("C:\\work\\diffy"));
    state.compare.left_ref = "main".to_owned();
    state.compare.right_ref = "feature/native-ui".to_owned();
    state.compare.resolved_left = Some("abc1234".to_owned());
    state.compare.resolved_right = Some("def5678".to_owned());
    state.repository.status = diffy::ui::state::AsyncStatus::Ready;

    state.workspace.files = vec![
        FileListEntry {
            path: "src/main.rs".into(),
            status: "M".into(),
            additions: 42,
            deletions: 8,
            is_binary: false,
        },
        FileListEntry {
            path: "src/lib.rs".into(),
            status: "M".into(),
            additions: 156,
            deletions: 23,
            is_binary: false,
        },
        FileListEntry {
            path: "src/render/renderer.rs".into(),
            status: "M".into(),
            additions: 384,
            deletions: 12,
            is_binary: false,
        },
        FileListEntry {
            path: "src/ui/element.rs".into(),
            status: "M".into(),
            additions: 221,
            deletions: 0,
            is_binary: false,
        },
        FileListEntry {
            path: "src/ui/shell.rs".into(),
            status: "M".into(),
            additions: 861,
            deletions: 842,
            is_binary: false,
        },
        FileListEntry {
            path: "Cargo.toml".into(),
            status: "M".into(),
            additions: 3,
            deletions: 0,
            is_binary: false,
        },
        FileListEntry {
            path: "README.md".into(),
            status: "M".into(),
            additions: 12,
            deletions: 4,
            is_binary: false,
        },
    ];
    state.workspace.selected_file_index = Some(2);
    state.workspace.selected_file_path = Some("src/render/renderer.rs".into());

    capture_frame("file_list", 1320, 840, &mut state);
}

/// Compare sheet modal overlay on top of empty state.
#[test]
fn capture_modal_overlay() {
    let mut state = AppState::default();
    state.compare.repo_path = Some(PathBuf::from("C:\\work\\diffy"));
    state.overlays.stack.push(OverlayEntry {
        surface: OverlaySurface::CompareSheet,
        focus_return: None,
    });

    capture_frame("modal_overlay", 1320, 840, &mut state);
}

/// Toast notifications stacked in bottom-right.
#[test]
fn capture_toasts() {
    let mut state = AppState::default();
    state.workspace_mode = WorkspaceMode::Ready;
    state.compare.repo_path = Some(PathBuf::from("C:\\work\\diffy"));
    state.compare.resolved_left = Some("main".to_owned());
    state.compare.resolved_right = Some("feature".to_owned());
    state.repository.status = diffy::ui::state::AsyncStatus::Ready;
    state.workspace.files = vec![
        FileListEntry {
            path: "src/main.rs".into(),
            status: "M".into(),
            additions: 10,
            deletions: 2,
            is_binary: false,
        },
    ];
    state.workspace.selected_file_index = Some(0);
    state.workspace.selected_file_path = Some("src/main.rs".into());

    state.toasts.push(diffy::ui::state::Toast {
        id: 1,
        kind: ToastKind::Info,
        message: "Compare completed in 142ms".into(),
        created_at_ms: 0,
        hovered: false,
    });
    state.toasts.push(diffy::ui::state::Toast {
        id: 2,
        kind: ToastKind::Error,
        message: "Failed to resolve ref 'origin/old-branch'".into(),
        created_at_ms: 0,
        hovered: false,
    });

    capture_frame("toasts", 1320, 840, &mut state);
}
