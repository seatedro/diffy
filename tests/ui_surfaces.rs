mod support;

use diffy::ui::actions::Action;
use diffy::ui::element::ScrollActionBuilder;
use diffy::ui::state::FocusTarget;

use support::{
    auth_modal_state, command_palette_state, compare_sheet_state, count_hits,
    empty_state_with_recents, has_hit, has_scroll_region, has_text_input_for, largest_rounded_rect,
    pull_request_modal_state, ready_state_with_files, render_frame, render_frame_in,
    repo_picker_state, scene_contains_text, scene_text_rect, toasts_state,
};

#[test]
fn empty_state_renders_primary_surfaces() {
    let mut state = empty_state_with_recents();
    let frame = render_frame(&mut state);

    assert!(scene_contains_text(&frame, "Start a new compare"));
    assert!(scene_contains_text(&frame, "Recent repositories"));
    assert!(scene_contains_text(&frame, "idle"));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::OpenCompareSheet
    )));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::OpenRepository(_)
    )));
    assert!(frame.viewport_rect.is_none());
}

#[test]
fn ready_workspace_wires_titlebar_sidebar_viewport_and_status_bar() {
    let mut state = ready_state_with_files(18);
    let frame = render_frame(&mut state);

    assert!(scene_contains_text(&frame, "src/file_0.rs"));
    assert!(scene_contains_text(&frame, "ready"));
    assert!(frame.file_list_rect.is_some());
    assert!(frame.sidebar_resize_handle_rect.is_some());
    assert!(frame.viewport_rect.is_some());
    assert!(has_scroll_region(&frame, |builder| matches!(
        builder,
        ScrollActionBuilder::FileList
    )));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::SelectFile(0)
    )));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::OpenCompareSheet
    )));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::OpenPullRequestModal
    )));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::ToggleWrap
    )));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::ToggleThemeMode
    )));
}

#[test]
fn overflowing_sidebar_registers_a_file_list_scrollbar_track() {
    let mut state = ready_state_with_files(32);
    let frame = render_frame(&mut state);

    assert!(
        frame
            .scrollbar_tracks
            .iter()
            .any(|track| matches!(track.action_builder, ScrollActionBuilder::FileList))
    );
}

#[test]
fn sidebar_expands_for_long_file_names_when_not_manually_resized() {
    let mut state = ready_state_with_files(3);
    let long_path = "src/features/worktree/native/sidebar/this_is_a_deliberately_extremely_long_filename_for_layout_regression_checks.rs".to_owned();
    state.workspace.files[1].path = long_path.clone();
    let frame = render_frame(&mut state);

    assert!(scene_contains_text(&frame, &long_path));
    assert!(frame.file_list_rect.is_some_and(|rect| rect.width > 280.0));
}

#[test]
fn compare_sheet_exposes_backdrop_and_controls() {
    let mut state = compare_sheet_state();
    let frame = render_frame(&mut state);

    assert!(scene_contains_text(&frame, "Compare Setup"));
    assert!(scene_contains_text(&frame, "Start Compare"));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::CloseOverlay
    )));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::OpenRepoPicker
    )));
    assert!(has_text_input_for(&frame, FocusTarget::CompareLeftRef));
    assert!(has_text_input_for(&frame, FocusTarget::CompareRightRef));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::StartCompare
    )));
}

#[test]
fn compare_sheet_reflows_inside_panel_at_high_scale() {
    let mut state = compare_sheet_state();
    state.settings.ui_scale_pct = 170;
    state.overlays.compare_sheet.validation_message = Some(
        "Git error: revspec 'native' not found; class=Reference (4); code=NotFound (-3)".into(),
    );
    let frame = render_frame_in(&mut state, 840.0, 620.0);
    let panel = largest_rounded_rect(&frame).expect("modal panel");

    for needle in [
        "Compare Setup",
        "Three Dot",
        "Difftastic",
        "Start Compare",
        "Git error:",
    ] {
        let rect =
            scene_text_rect(&frame, needle).unwrap_or_else(|| panic!("missing text {needle}"));
        assert!(
            rect.right() <= panel.right() + 0.5,
            "{needle} overflowed the modal"
        );
    }
}

#[test]
fn repo_picker_exposes_input_entries_and_scroll_surface() {
    let mut state = repo_picker_state(24);
    let frame = render_frame(&mut state);

    assert!(scene_contains_text(&frame, "Repository Picker"));
    assert!(has_text_input_for(&frame, FocusTarget::PickerInput));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::SelectOverlayEntry(0)
    )));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::OpenRepositoryDialog
    )));
    assert!(has_scroll_region(&frame, |builder| match builder {
        ScrollActionBuilder::Custom(build) => {
            matches!(build(1), Action::ScrollActiveOverlayListPx(1))
        }
        _ => false,
    }));
}

#[test]
fn command_palette_exposes_input_entries_and_scroll_surface() {
    let mut state = command_palette_state(30);
    let frame = render_frame(&mut state);

    assert!(scene_contains_text(&frame, "Command Palette"));
    assert!(has_text_input_for(&frame, FocusTarget::CommandPaletteInput));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::SelectOverlayEntry(_)
    )));
    assert!(has_scroll_region(&frame, |builder| match builder {
        ScrollActionBuilder::Custom(build) => {
            matches!(build(1), Action::ScrollActiveOverlayListPx(1))
        }
        _ => false,
    }));
}

#[test]
fn pull_request_modal_exposes_input_and_actions() {
    let mut state = pull_request_modal_state();
    let frame = render_frame(&mut state);

    assert!(scene_contains_text(&frame, "GitHub Pull Request"));
    assert!(scene_contains_text(&frame, "Improve scroll plumbing"));
    assert!(has_text_input_for(&frame, FocusTarget::PullRequestInput));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::SubmitPullRequest
    )));
    assert!(has_hit(&frame, |action| matches!(
        action,
        Action::UsePullRequestCompare
    )));
}

#[test]
fn auth_modal_switches_primary_action_based_on_device_flow_state() {
    let mut idle_state = auth_modal_state(false);
    let idle_frame = render_frame(&mut idle_state);
    assert!(scene_contains_text(&idle_frame, "Not authenticated"));
    assert!(has_hit(&idle_frame, |action| matches!(
        action,
        Action::StartGitHubDeviceFlow
    )));

    let mut flow_state = auth_modal_state(true);
    let flow_frame = render_frame(&mut flow_state);
    assert!(scene_contains_text(&flow_frame, "User code: ABCD-EFGH"));
    assert!(has_hit(&flow_frame, |action| matches!(
        action,
        Action::OpenDeviceFlowBrowser
    )));
}

#[test]
fn toast_layer_registers_one_dismiss_hit_per_toast() {
    let mut state = toasts_state();
    let frame = render_frame(&mut state);

    assert!(scene_contains_text(&frame, "Compare completed in 142ms"));
    assert!(scene_contains_text(&frame, "Failed to resolve ref"));
    assert_eq!(
        count_hits(&frame, |action| matches!(action, Action::DismissToast(_))),
        2
    );
}
