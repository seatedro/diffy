use std::path::PathBuf;

use quark::Rect;
use quark::scene::Primitive;
use quark_app::testing::{By, Node, UiTestHarness};
use quark_app::winit::window::Theme as SystemTheme;
use quark_ui::accessibility::AccessibilityRole as Role;
use quark_ui::theme::{Color, ThemeMode};

use crate::app::testing::*;
use crate::app::theme::ThemePair;
use crate::contracts::shared::WorkspaceEpoch;
use crate::contracts::shell::{AppMessage, ShellEvent};
use crate::core::themes::ThemeRegistry;

fn opened(path: &str) -> ShellEvent {
    ShellEvent::RepositoryDialogClosed {
        path: Some(PathBuf::from(path)),
    }
}

fn window(size: (f32, f32)) -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        width: size.0,
        height: size.1,
    }
}

fn contains(outer: Rect, inner: Rect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.width <= outer.x + outer.width + 0.5
        && inner.y + inner.height <= outer.y + outer.height + 0.5
}

fn below(upper: &Node, lower: &Node) -> bool {
    upper.bounds.y + upper.bounds.height <= lower.bounds.y + 0.5
}

fn regions(ui: &Harness) -> (Node, Option<Node>, Node, Node) {
    (
        ui.find(By::role_name(Role::TitleBar, "Title bar")),
        ui.try_find(By::role_name(Role::Navigation, "Files")),
        ui.find(By::role_name(Role::Main, "Diff")),
        ui.find(By::role_name(Role::Status, "Status")),
    )
}

fn background(ui: &Harness) -> Option<Color> {
    let (width, height) = ui.size();
    ui.scene()
        .primitives
        .iter()
        .find_map(|primitive| match primitive {
            Primitive::Rect(p) if p.rect.width == width && p.rect.height == height => Some(p.color),
            Primitive::RoundedRect(p) if p.rect.width == width && p.rect.height == height => {
                Some(p.color)
            }
            _ => None,
        })
}

#[test]
fn shell_renders_labeled_regions() {
    let ui = harness();

    assert!(
        ui.accessibility_tree()
            .starts_with(&format!("Window {TITLE:?}\n")),
        "root window is named {TITLE}"
    );
    let (title, files, diff, status) = regions(&ui);
    let files = files.expect("files region at the default size");
    assert_eq!(ui.find(By::test_id("title")), title);
    assert_eq!(ui.find(By::test_id("files")), files);
    assert_eq!(ui.find(By::test_id("diff")), diff);
    assert_eq!(ui.find(By::test_id("status")), status);

    assert_eq!(title.bounds.y, 0.0);
    assert!(below(&title, &files) && below(&title, &diff));
    assert!(below(&files, &status) && below(&diff, &status));
    assert!(files.bounds.x + files.bounds.width <= diff.bounds.x + 1.0);
    for node in [&title, &files, &diff, &status] {
        assert!(
            contains(window(SIZE), node.bounds),
            "{node} outside the window"
        );
    }
    assert!(contains(title.bounds, ui.find(By::name(TITLE)).bounds));
    assert!(ui.app().dispatcher().attached);
}

#[test]
fn small_windows_and_hidpi_keep_regions_inside_the_window() {
    let mut layouts = Vec::new();
    for (size, scale) in [
        ((480.0, 320.0), 1.0),
        ((480.0, 320.0), 2.0),
        ((200.0, 120.0), 3.0),
    ] {
        let ui = harness_at(size, scale);
        let (title, files, diff, status) = regions(&ui);
        assert!(files.is_none(), "sidebar hidden at {size:?}");
        assert!(below(&title, &diff) && below(&diff, &status));
        for node in [&title, &diff, &status] {
            assert!(
                contains(window(size), node.bounds),
                "{node} outside {size:?}"
            );
        }
        layouts.push([title.bounds, diff.bounds, status.bounds]);
    }
    assert_eq!(
        layouts[0], layouts[1],
        "logical layout ignores the device scale"
    );
}

#[test]
fn current_workspace_events_change_the_title_and_stale_ones_do_not() {
    let mut ui = harness();
    let main = ui.main_window();

    let frames = ui.frame_count();
    deliver(&mut ui, WorkspaceEpoch(7), opened("/fixtures/stale"));
    assert!(ui.try_find(By::name("stale")).is_none());
    assert_eq!(ui.app().shared().workspace, WorkspaceEpoch(0));
    assert_eq!(ui.frame_count(), frames, "a dropped event does not redraw");

    deliver_current(&mut ui, opened("/fixtures/alpha"));
    let title = ui.find(By::test_id("title"));
    assert!(contains(title.bounds, ui.find(By::name("alpha")).bounds));
    assert!(ui.frame_count() > frames);
    assert_eq!(ui.app().shared().workspace, WorkspaceEpoch(1));
    assert_eq!(ui.app().shared().revision, 1);
    assert_eq!(ui.window(main).title(), format!("alpha - {TITLE}"));

    let frames = ui.frame_count();
    deliver(&mut ui, WorkspaceEpoch(0), opened("/fixtures/beta"));
    assert!(ui.try_find(By::name("beta")).is_none());
    ui.find(By::name("alpha"));
    assert_eq!(ui.app().shared().workspace, WorkspaceEpoch(1));
    assert_eq!(ui.frame_count(), frames);
}

#[test]
fn syntax_wake_requests_a_frame() {
    let mut ui = harness();
    let frames = ui.frame_count();
    ui.send_message(AppMessage::SyntaxWake);
    assert_eq!(ui.frame_count(), frames + 1);
}

#[test]
fn theme_switch_repaints_with_the_other_palette() {
    let registry = ThemeRegistry::load();
    let name = registry
        .names()
        .find(|name| {
            registry
                .get(name)
                .is_some_and(|entry| entry.dark.is_dark && !entry.light.is_dark)
        })
        .map(str::to_owned)
        .expect("a dual theme");
    let themes = ThemePair::load(&registry, &name, 1.0);
    let mut ui = UiTestHarness::with_adapter(adapter(themes.clone(), None), SIZE, 1.0);
    assert_eq!(background(&ui), Some(themes.dark.colors.app_bg));

    let frames = ui.frame_count();
    ui.app_event(quark_app::AppEvent::ThemeChanged(SystemTheme::Light));

    assert!(ui.frame_count() > frames);
    assert_eq!(ui.theme().mode, ThemeMode::Light);
    assert_eq!(background(&ui), Some(themes.light.colors.app_bg));
    assert_ne!(themes.light.colors.app_bg, themes.dark.colors.app_bg);
}
