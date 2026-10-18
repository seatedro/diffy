use std::rc::Rc;

use quark_app::testing::{By, UiTestHarness};
use quark_app::winit::keyboard::ModifiersState;
use quark_app::{InputEvent, KeyChord, KeyKind};
use quark_ui::accessibility::AccessibilityRole as Role;

use super::fixtures::*;
use crate::app::text_targets::{SEARCH, TEXT_COMPARE_LEFT, VIEWPORT};
use crate::contracts::navigation::{SelectedFile, SelectionOrigin};
use crate::contracts::shared::{CompareGeneration, FileKey, FileRevision, Side, SourceTarget};
use crate::contracts::shell::{Controller, Effect, Intent};
use crate::contracts::viewport::ViewportEffect;
use crate::contracts::viewport::interaction::{TextCompareView, ViewportInstall};
use crate::contracts::viewport::pipeline::{PipelineEffect, PreparedDiff};
use crate::core::compare::LayoutMode;

const SIZE: (f32, f32) = (900.0, 600.0);
const SHORT: (f32, f32) = (900.0, 240.0);

fn loaded(host: Host) -> UiTestHarness<Host> {
    loaded_at(host, SIZE)
}

fn loaded_at(host: Host, size: (f32, f32)) -> UiTestHarness<Host> {
    let mut ui = harness(host, size);
    for prepared in two_files() {
        ui.app_mut().install(ViewportInstall::File {
            prepared,
            remap: None,
        });
    }
    ui.frame();
    ui
}

fn focus_diff(ui: &mut UiTestHarness<Host>) {
    let list = ui.find(By::role_name(Role::List, "Diff"));
    ui.click(list.center());
    assert_eq!(ui.focus(), Some(VIEWPORT));
}

fn selections(ui: &UiTestHarness<Host>) -> Vec<SourceTarget> {
    ui.app()
        .intents
        .iter()
        .filter_map(|intent| match intent {
            Intent::SelectSource(Some(target)) => Some(target.clone()),
            _ => None,
        })
        .collect()
}

fn chord(ch: char, modifiers: ModifiersState) -> KeyChord {
    KeyChord {
        logical: KeyKind::Character(ch.to_string()),
        physical: None,
        modifiers,
        repeat: false,
    }
}

fn key_events(binding: &str) -> Vec<InputEvent> {
    let primary = if cfg!(target_os = "macos") {
        ModifiersState::SUPER
    } else {
        ModifiersState::CONTROL
    };
    let ch = binding.strip_prefix("mod+").and_then(|k| k.chars().next());
    let ch = ch.unwrap_or_else(|| panic!("unsupported binding {binding}"));
    vec![
        InputEvent::ModifiersChanged(primary),
        InputEvent::KeyPress(chord(ch, primary)),
        InputEvent::KeyRelease(chord(ch, primary)),
        InputEvent::ModifiersChanged(ModifiersState::empty()),
    ]
}

fn typed(text: &str) -> Vec<InputEvent> {
    text.chars()
        .flat_map(|ch| {
            let chord = chord(ch, ModifiersState::empty());
            [
                InputEvent::KeyPress(chord.clone()),
                InputEvent::TextInput(ch.to_string()),
                InputEvent::KeyRelease(chord),
            ]
        })
        .collect()
}

fn line_names(ui: &UiTestHarness<Host>) -> Vec<String> {
    ui.find_all(By::role(Role::ListItem))
        .into_iter()
        .filter_map(|n| n.name)
        .collect()
}

#[test]
fn empty_and_loading_states_name_the_region() {
    let mut ui = harness(Host::new(), SIZE);
    ui.find(By::role_name(Role::Main, "Diff"));
    assert!(ui.painted_text().contains("Nothing to compare"));

    ui.app_mut().install(ViewportInstall::Begin {
        generation: GENERATION,
    });
    ui.frame();
    assert!(ui.painted_text().contains("Loading diff"));

    ui.app_mut().install(ViewportInstall::Failed {
        generation: GENERATION,
        key: FileKey(1),
        revision: FileRevision(1),
        message: "git exited with status 128".to_owned(),
    });
    ui.frame();
    assert!(ui.painted_text().contains("git exited with status 128"));
}

#[test]
fn installed_files_render_with_identity_test_ids() {
    let ui = loaded(Host::new());
    ui.find(By::role_name(Role::List, "Diff"));
    ui.find(By::test_id("diffy.diff.file.1.1"));
    ui.find(By::test_id("diffy.diff.gap.1.1.0"));
    ui.find(By::test_id("diffy.diff.gap.1.1.1"));
    assert!(line_names(&ui).iter().any(|n| n == "changed ten"));
    ui.find(By::role_name(Role::Button, "Expand context"));
}

#[test]
fn split_layout_shows_both_sides() {
    let mut ui = loaded(Host::new());
    ui.click_node(By::role_name(Role::RadioButton, "Split"));
    let names = line_names(&ui);
    assert!(names.iter().any(|n| n == "line 10"));
    assert!(names.iter().any(|n| n == "changed ten"));
}

#[test]
fn hunk_keys_move_between_hunks_and_publish_their_source() {
    let mut ui = loaded(Host::new());
    focus_diff(&mut ui);
    ui.key("]");
    let first = ui.app().interaction.focused_hunk().expect("focused hunk");
    assert_eq!((first.source.file, first.hunk), (FileKey(1), 0));
    assert_eq!(first.source.revision, FileRevision(1));
    assert!(first.source.lines.contains(&10));
    ui.key("]");
    let second = ui.app().interaction.focused_hunk().expect("second hunk");
    assert_eq!((second.source.file, second.hunk), (FileKey(1), 1));
    ui.key("[");
    assert_eq!(ui.app().interaction.focused_hunk(), Some(first.clone()));
    assert_eq!(selections(&ui).last(), Some(&first.source));
}

#[test]
fn file_keys_reach_the_next_file_and_tell_the_sidebar() {
    let mut ui = loaded_at(Host::new(), SHORT);
    focus_diff(&mut ui);
    ui.key("n");
    ui.key("n");
    let told = ui.app().intents.iter().any(|intent| {
        matches!(
            intent,
            Intent::SelectFile(Some(SelectedFile {
                key: FileKey(2),
                origin: SelectionOrigin::Viewport
            }))
        )
    });
    assert!(told, "{:?}", ui.app().intents);
}

#[test]
fn sidebar_selection_reveals_the_file_without_echoing_back() {
    let mut ui = loaded_at(Host::new(), SHORT);
    ui.app_mut().intent(Intent::SelectFile(Some(SelectedFile {
        key: FileKey(2),
        origin: SelectionOrigin::Sidebar,
    })));
    ui.frame();
    assert_eq!(
        ui.app().interaction.diff().and_then(|d| d.top_file()),
        Some(FileKey(2))
    );
    ui.find(By::test_id("diffy.diff.file.2.1"));
    let echoed = ui
        .app()
        .intents
        .iter()
        .any(|intent| matches!(intent, Intent::SelectFile(Some(file)) if file.origin == SelectionOrigin::Viewport));
    assert!(!echoed);
}

#[test]
fn drag_selects_lines_publishes_a_source_target_and_copies() {
    let mut ui = loaded(Host::new());
    focus_diff(&mut ui);
    let from = ui.find(By::role_name(Role::ListItem, "line 8")).bounds;
    let to = ui.find(By::role_name(Role::ListItem, "line 9")).bounds;
    ui.drag(
        (from.x + 1.0, from.y + from.height / 2.0),
        (to.x + to.width - 1.0, to.y + to.height / 2.0),
    );
    let target = selections(&ui).last().cloned().expect("selection target");
    assert_eq!(target.file, FileKey(1));
    assert_eq!(target.side, Side::New);
    assert_eq!(target.lines, 8..10);
    ui.key("mod+c");
    assert_eq!(ui.clipboard_text().as_deref(), Some("line 8\nline 9"));
}

#[test]
fn find_in_diff_searches_and_steps_matches() {
    let mut ui = loaded(Host::new());
    focus_diff(&mut ui);
    ui.key("mod+f");
    assert_eq!(ui.focus(), Some(SEARCH));
    ui.find(By::role_name(Role::SearchInput, "Find in diff"));
    ui.type_text("changed");
    let summary = ui.app().interaction.search_summary();
    assert_eq!(summary.matches, 2);
    assert_eq!(summary.active, Some(0));
    assert!(ui.painted_text().contains("1 of 2"));
    ui.key("enter");
    assert_eq!(ui.app().interaction.search_summary().active, Some(1));
    ui.key("escape");
    assert_eq!(ui.focus(), Some(VIEWPORT));
    assert_eq!(ui.app().interaction.search_summary().matches, 0);
}

#[test]
fn typing_right_after_opening_find_reaches_the_field() {
    let mut ui = loaded(Host::new());
    focus_diff(&mut ui);
    let closed = ui.find(By::role_name(Role::SearchInput, "Find in diff"));
    assert!(closed.bounds.height <= 0.0, "{:?}", closed.bounds);
    let top = ui.app().interaction.diff().map(|d| d.scroll_offset());
    ui.send_events(key_events("mod+f").into_iter().chain(typed("ud1")));
    assert_eq!(ui.focus(), Some(SEARCH));
    assert_eq!(ui.app().interaction.search().field.text(), "ud1");
    assert_eq!(ui.app().interaction.diff().map(|d| d.scroll_offset()), top);
    assert_eq!(ui.app().interaction.layout(), LayoutMode::Unified);
    let field = ui.find(By::role_name(Role::SearchInput, "Find in diff"));
    assert!(field.bounds.width > SIZE.0 / 3.0, "{}", field.bounds.width);
}

#[test]
fn workspace_shortcuts_drive_the_focused_diff() {
    let mut ui = loaded(Host::new());
    focus_diff(&mut ui);
    ui.key("2");
    assert_eq!(ui.app().interaction.layout(), LayoutMode::Split);
    ui.key("1");
    assert_eq!(ui.app().interaction.layout(), LayoutMode::Unified);
}

#[test]
fn layout_survives_unrelated_settings_changes() {
    let mut ui = loaded(Host::new());
    ui.app_mut().update_settings(|_| {});
    ui.click_node(By::role_name(Role::RadioButton, "Split"));
    assert_eq!(ui.app().interaction.layout(), LayoutMode::Split);
    ui.app_mut().update_settings(|s| s.ui_scale_pct = 110);
    assert_eq!(ui.app().interaction.layout(), LayoutMode::Split);
    ui.app_mut()
        .update_settings(|s| s.viewport.layout = LayoutMode::Split);
    ui.app_mut()
        .update_settings(|s| s.viewport.layout = LayoutMode::Unified);
    assert_eq!(ui.app().interaction.layout(), LayoutMode::Unified);
}

#[test]
fn search_follows_new_revisions_of_a_file() {
    let mut ui = loaded(Host::new());
    focus_diff(&mut ui);
    ui.key("mod+f");
    ui.type_text("needle");
    assert_eq!(ui.app().interaction.search_summary().matches, 1);
    let base = numbered(0..60);
    ui.app_mut().install(ViewportInstall::File {
        prepared: prepared(2, 2, "b.rs", &base, &base.replace("line 7\n", "seven\n")),
        remap: None,
    });
    ui.frame();
    assert_eq!(ui.app().interaction.search_summary().matches, 0);
}

#[test]
fn stale_installs_are_ignored() {
    let mut ui = loaded(Host::new());
    ui.app_mut().install(ViewportInstall::Begin {
        generation: CompareGeneration(3),
    });
    let base = numbered(0..5);
    ui.app_mut().install(ViewportInstall::File {
        prepared: prepared(9, 1, "old.rs", &base, "x\n"),
        remap: None,
    });
    ui.frame();
    assert!(ui.try_find(By::test_id("diffy.diff.file.9.1")).is_none());
    assert!(ui.painted_text().contains("Loading diff"));
}

#[test]
fn wrap_and_wrap_column_rewrap_long_lines() {
    let long = format!("{}\n", "word ".repeat(80));
    let mut ui = harness(Host::new(), SIZE);
    ui.app_mut().install(ViewportInstall::File {
        prepared: prepared(1, 1, "long.txt", "short\n", &long),
        remap: None,
    });
    ui.frame();
    let height = |ui: &UiTestHarness<Host>| {
        ui.app()
            .interaction
            .diff()
            .map_or(0.0, |d| d.content_height())
    };
    let unwrapped = height(&ui);
    ui.click_node(By::role_name(Role::Switch, "Wrap"));
    let wrapped = height(&ui);
    assert!(wrapped > unwrapped, "{wrapped} > {unwrapped}");
    assert!(ui.accessibility_tree().contains("Wrap"));
    ui.app_mut()
        .update_settings(|s| s.viewport.wrap_column = 20);
    ui.frame();
    assert!(height(&ui) > wrapped);
}

#[test]
fn expand_context_reveals_hidden_lines() {
    let mut ui = loaded(Host::new());
    let before = ui
        .app()
        .interaction
        .diff()
        .map_or(0.0, |d| d.content_height());
    ui.click_node(By::role_name(Role::Button, "Expand context"));
    let after = ui
        .app()
        .interaction
        .diff()
        .map_or(0.0, |d| d.content_height());
    assert!(after > before);
}

#[test]
fn small_window_keeps_the_view_inside_its_bounds() {
    let mut ui = harness(Host::new(), (320.0, 200.0));
    for prepared in two_files() {
        ui.app_mut().install(ViewportInstall::File {
            prepared,
            remap: None,
        });
    }
    ui.frame();
    let main = ui.find(By::role_name(Role::Main, "Diff"));
    let list = ui.find(By::role_name(Role::List, "Diff"));
    assert!(list.bounds.y + list.bounds.height <= main.bounds.y + main.bounds.height + 0.5);
    ui.resize(240.0, 120.0);
    ui.frame();
    ui.find(By::role_name(Role::List, "Diff"));
}

#[test]
fn decorator_receives_file_and_hunk_identity() {
    let recorder = Rc::new(Recorder {
        revision: 1,
        marked: Some((FileKey(1), 10)),
        ..Recorder::default()
    });
    let ui = loaded(Host::with_decorator(recorder.clone()));
    let separators = recorder.separators.borrow();
    let hunk = separators
        .iter()
        .find(|s| s.hunk.as_ref().is_some_and(|h| h.0 == 0))
        .expect("first hunk separator");
    assert_eq!(hunk.file, Some((FileKey(1), 1)));
    let (_, old, new) = hunk.hunk.clone().expect("hunk source");
    assert!(old.contains(&10) && new.contains(&10));
    assert!(recorder.headers.borrow().contains(&Some((FileKey(1), 1))));
    drop(separators);
    drop(ui);
}

#[test]
fn hunk_targets_follow_the_hunk_source() {
    let ui = loaded(Host::new());
    let sources = ui.app().interaction.sources().clone();
    let unit = sources.unit_of(FileKey(1)).expect("unit");
    let target = super::hunk_target(&sources.hunk_at(unit, 1).expect("hunk"));
    assert_eq!(target.hunk, 1);
    assert_eq!(target.source.file, FileKey(1));
    assert!(target.source.lines.contains(&40));
}

#[test]
fn text_compare_edits_requests_and_installs_only_the_current_result() {
    use crate::contracts::viewport::interaction::InteractionAction as A;
    let mut ui = harness(Host::new(), SIZE);
    let edit = A::SetTextCompareView(TextCompareView::Edit);
    let reduction = ui
        .app_mut()
        .interaction
        .reduce_action(edit, &Default::default());
    assert!(reduction.changed);
    ui.frame();
    ui.click_node(By::test_id("text-compare.left"));
    ui.type_text("alpha");
    assert_eq!(ui.focus(), Some(TEXT_COMPARE_LEFT));
    ui.click_node(By::role_name(Role::Button, "Swap sides"));
    assert_eq!(ui.app().interaction.text_compare().right.text(), "alpha");
    ui.click_node(By::role_name(Role::Button, "Compare"));
    let request = ui
        .app()
        .effects
        .iter()
        .rev()
        .find_map(|effect| match effect {
            Effect::Viewport(ViewportEffect::Pipeline(PipelineEffect::RunTextCompare(request))) => {
                Some(request.clone())
            }
            _ => None,
        });
    let request = request.expect("text compare request");
    assert_eq!(&*request.right_text, "alpha");
    let stale = prepared_at(
        CompareGeneration(request.generation.0 + 5),
        1,
        1,
        "text.txt",
        "",
        "alpha",
    );
    ui.app_mut()
        .install(ViewportInstall::TextCompare { prepared: stale });
    ui.frame();
    assert_eq!(
        ui.app().interaction.text_compare().view,
        TextCompareView::Edit
    );
    let current = prepared_at(request.generation, 1, 1, "text.txt", "", "alpha");
    ui.app_mut()
        .install(ViewportInstall::TextCompare { prepared: current });
    ui.frame();
    assert_eq!(
        ui.app().interaction.text_compare().view,
        TextCompareView::Diff
    );
    assert!(line_names(&ui).iter().any(|n| n == "alpha"));
}

#[test]
#[ignore = "timing; run in release with --ignored --nocapture"]
fn large_diff_frame_and_handler_times() {
    use std::time::{Duration, Instant};
    let old: String = (0..100_000)
        .map(|i| format!("let value_{i} = compute({i});\n"))
        .collect();
    let new: String = old
        .lines()
        .enumerate()
        .map(|(i, line)| {
            if i % 50 == 0 {
                format!("{line} // edited\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    let built = Instant::now();
    let file = prepared(1, 1, "large.rs", &old, &new);
    let diffed = built.elapsed();
    let mut ui = harness(Host::new(), (1280.0, 800.0));
    let installed = Instant::now();
    ui.app_mut().install(ViewportInstall::File {
        prepared: file,
        remap: None,
    });
    let install = installed.elapsed();
    let painted = Instant::now();
    ui.frame();
    let first = painted.elapsed();
    focus_diff(&mut ui);
    let mut frames = Vec::new();
    for _ in 0..200 {
        let at = Instant::now();
        ui.pointer_move((600.0, 400.0));
        ui.wheel(0.0, 120.0);
        frames.push(at.elapsed());
    }
    let mut hunks = Vec::new();
    for _ in 0..200 {
        let at = Instant::now();
        ui.key("]");
        hunks.push(at.elapsed());
    }
    let searched = Instant::now();
    ui.key("mod+f");
    ui.type_text("edited");
    let search = searched.elapsed();
    let settled = Instant::now();
    ui.frame();
    let idle = settled.elapsed();
    let stats = |mut v: Vec<Duration>| {
        v.sort();
        let avg = v.iter().sum::<Duration>() / v.len() as u32;
        (avg, v[v.len() * 99 / 100], v[v.len() - 1])
    };
    eprintln!(
        "100k lines: diff {diffed:.2?}, install {install:.2?}, first frame {first:.2?}, \
         wheel frame avg/p99/max {:.2?}, next-hunk handler+frame avg/p99/max {:.2?}, \
         search typing {search:.2?} ({} matches), repeated frame {idle:.2?}",
        stats(frames),
        stats(hunks),
        ui.app().interaction.search_summary().matches,
    );
}

#[test]
fn open_text_compare_intent_enters_the_editors() {
    let mut ui = loaded(Host::new());
    ui.app_mut().intent(Intent::OpenTextCompare);
    ui.frame();
    assert!(ui.app().interaction.text_compare().active);
    assert_eq!(
        ui.app().interaction.text_compare().view,
        TextCompareView::Edit
    );
    ui.find(By::test_id("text-compare"));
}

#[test]
fn annotate_requests_a_review_annotation_at_the_line() {
    let mut ui = loaded(Host::new());
    let event = diffy_ui::DiffEvent::Annotate {
        file: 0,
        side: Side::New,
        line: 10,
    };
    let action = crate::contracts::viewport::interaction::InteractionAction::Diff(
        crate::contracts::viewport::interaction::DiffInput(event),
    );
    let reduction = ui
        .app_mut()
        .interaction
        .reduce_action(action, &Default::default());
    let requested: Vec<_> = reduction
        .intents
        .iter()
        .filter_map(|intent| match intent {
            Intent::RequestAnnotation(target) => Some(target.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        requested,
        [SourceTarget {
            file: FileKey(1),
            revision: FileRevision(1),
            side: Side::New,
            lines: 10..11,
        }]
    );
}

#[test]
fn single_file_mode_shows_the_selected_file_and_steps_files() {
    let mut host = Host::new();
    host.update_settings(|s| s.continuous_scroll = false);
    let mut ui = loaded(host);
    assert!(ui.try_find(By::test_id("diffy.diff.file.1.1")).is_some());
    assert!(ui.try_find(By::test_id("diffy.diff.file.2.1")).is_none());

    ui.app_mut().intent(Intent::SelectFile(Some(SelectedFile {
        key: FileKey(2),
        origin: SelectionOrigin::Sidebar,
    })));
    ui.frame();
    assert!(ui.try_find(By::test_id("diffy.diff.file.1.1")).is_none());
    assert!(ui.try_find(By::test_id("diffy.diff.file.2.1")).is_some());

    focus_diff(&mut ui);
    ui.key("shift+n");
    assert!(ui.try_find(By::test_id("diffy.diff.file.1.1")).is_some());
    assert_eq!(
        ui.app().shared.selected_file.map(|s| s.key),
        Some(FileKey(1))
    );

    ui.app_mut().update_settings(|s| s.continuous_scroll = true);
    ui.frame();
    assert!(ui.try_find(By::test_id("diffy.diff.file.1.1")).is_some());
    assert!(ui.try_find(By::test_id("diffy.diff.file.2.1")).is_some());
}

#[test]
fn placeholders_let_order_name_unloaded_files() {
    use diffy_diff::{FileMeta, Placeholder, Revision, SlotState};
    use std::sync::Arc;

    use crate::contracts::viewport::pipeline::SlotUpdate;

    let mut ui = loaded(Host::new());
    let host = ui.app_mut();
    let shared = host.shared.clone();
    host.interaction.install_slot(
        SlotUpdate::Placeholder {
            key: FileKey(3),
            placeholder: Placeholder::new(Revision(1), SlotState::Loading, 12, FileMeta::default()),
        },
        &shared,
    );
    let reduction = host.interaction.install(
        ViewportInstall::Order {
            generation: GENERATION,
            files: Arc::from([FileKey(3), FileKey(1), FileKey(2)]),
        },
        &shared,
    );
    assert!(reduction.changed);
    let order = ui
        .app()
        .interaction
        .diff()
        .map(|d| d.session().order().to_vec());
    assert_eq!(order, Some(vec![FileKey(3), FileKey(1), FileKey(2)]));
}

#[test]
fn expanding_a_patch_only_file_requests_its_sources() {
    use diffy_diff::{FileDiffSnapshot, Revision, parse_unified};

    use crate::contracts::viewport::interaction::{ContextDirection, InteractionAction};

    let patch = "--- a/c.rs\n+++ b/c.rs\n@@ -10,3 +10,3 @@\n one\n-two\n+deux\n three\n";
    let doc = parse_unified(patch).unwrap();
    let key = FileKey(7);
    let snapshot = FileDiffSnapshot::new(key, Revision(1), std::sync::Arc::new(doc)).unwrap();
    let prepared = PreparedDiff::new(key, GENERATION, FileRevision(1), snapshot);
    let mut ui = harness(Host::new(), SIZE);
    ui.app_mut().install(ViewportInstall::File {
        prepared,
        remap: None,
    });
    ui.frame();
    let interaction = &mut ui.app_mut().interaction;
    interaction.reduce_action(
        InteractionAction::ExpandContext {
            file: key,
            hunk: 0,
            direction: ContextDirection::Above,
            amount: 20,
        },
        &Default::default(),
    );
    assert_eq!(
        interaction.take_source_requests(),
        [(key, diffy_diff::SourcePurpose::ExpandContext)]
    );
    assert!(interaction.take_source_requests().is_empty());
}

#[test]
fn scrolled_file_keeps_its_header_pinned() {
    let mut ui = harness(Host::new(), SHORT);
    ui.app_mut().install(ViewportInstall::File {
        prepared: prepared(1, 1, "long.rs", "", &numbered(0..200)),
        remap: None,
    });
    ui.frame();
    focus_diff(&mut ui);
    ui.key("shift+g");
    ui.frame();
    let header = ui.find(By::test_id("diffy.diff.file.1.1"));
    let list = ui.find(By::role_name(Role::List, "Diff"));
    assert!(
        (header.bounds.y - list.bounds.y).abs() < 1.0,
        "{header:?} {list:?}"
    );
}
