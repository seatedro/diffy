#![cfg(feature = "capture")]

mod support;

use support::{
    assert_scene_snapshot, command_palette_state, compare_sheet_state, empty_state_with_recents,
    ready_state_with_files, repo_picker_state, toasts_state,
};

#[test]
fn capture_empty_state_snapshot() {
    let mut state = empty_state_with_recents();
    assert_scene_snapshot("empty_state", &mut state, 0x6059_6ce3_8120_d786);
}

#[test]
fn capture_ready_workspace_snapshot() {
    let mut state = ready_state_with_files(12);
    assert_scene_snapshot("ready_workspace", &mut state, 0xb45c_c9be_330c_531c);
}

#[test]
fn capture_compare_sheet_snapshot() {
    let mut state = compare_sheet_state();
    assert_scene_snapshot("compare_sheet", &mut state, 0x9837_c631_fdf9_d202);
}

#[test]
fn capture_repo_picker_snapshot() {
    let mut state = repo_picker_state(18);
    assert_scene_snapshot("repo_picker", &mut state, 0x565a_5ff1_e729_12ed);
}

#[test]
fn capture_command_palette_snapshot() {
    let mut state = command_palette_state(18);
    assert_scene_snapshot("command_palette", &mut state, 0x9966_6874_8abf_7622);
}

#[test]
fn capture_toasts_snapshot() {
    let mut state = toasts_state();
    assert_scene_snapshot("toasts", &mut state, 0xf238_9e42_2454_a263);
}
