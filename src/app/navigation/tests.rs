use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;

use quark_app::testing::{By, UiTestHarness};
use quark_app::{UiAdapter, UiApp, UiContext, ViewContext};
use quark_ui::FocusId;
use quark_ui::accessibility::AccessibilityRole as Role;
use quark_ui::element::AnyElement;
use quark_ui::text_input::{TextEditCommand, TextEditOutcome};

use super::NavigationController;
use crate::app::commands::fixtures::{self, REPO};
use crate::app::commands::harness::{self, effects, no_args, startup};
use crate::app::shell::Slot;
use crate::app::text_targets::{self, SIDEBAR, SIDEBAR_FILTER, TextCx, TextHost};
use crate::contracts::navigation::{
    NavigationAction, NavigationEffect, NavigationEvent, RepositorySyncReason, SelectedFile,
    SelectionOrigin, SidebarTab,
};
use crate::contracts::shared::{CompareGeneration, FileKey, SharedContext, WorkspaceEpoch};
use crate::contracts::shell::{
    Action, Controller, Effect, Intent, Reduction, ShellAction, ShellEffect,
};
use crate::contracts::viewport::pipeline::{CompareManifestSnapshot, FileStats};
use crate::core::compare::CompareMode;
use crate::core::vcs::model::VcsCompareSpec;
use crate::platform::persistence::Settings;

#[derive(Default)]
struct NavApp {
    nav: NavigationController,
    shared: SharedContext,
    intents: Vec<Intent>,
    effects: Vec<Effect>,
    clock: u64,
}

impl NavApp {
    fn reduce(&mut self, reduction: Reduction) {
        self.effects.extend(reduction.effects);
        let mut queue = VecDeque::from(reduction.intents);
        while let Some(intent) = queue.pop_front() {
            match &intent {
                Intent::PublishManifest(m) => self.shared.manifest = Some(m.clone()),
                Intent::SelectFile(f) => self.shared.selected_file = *f,
                Intent::RepositoryChanged(r) => self.shared.repository = Some(r.clone()),
                Intent::SettingsChanged(s) => self.shared.settings = s.clone(),
                Intent::ActivateWorkspace { .. } => {
                    self.shared.workspace = WorkspaceEpoch(self.shared.workspace.0 + 1)
                }
                _ => {}
            }
            let next = self.nav.apply_intent(&intent, &self.shared);
            self.effects.extend(next.effects);
            queue.extend(next.intents);
            self.intents.push(intent);
        }
    }

    fn inject(&mut self, intent: Intent) {
        self.reduce(Reduction::default().intent(intent));
    }

    fn event(&mut self, event: NavigationEvent) {
        let reduction = self.nav.reduce_event(event, &self.shared);
        self.reduce(reduction);
    }

    fn drain(&mut self, cx: &mut UiContext) {
        let focus = self.effects.iter().rev().find_map(|e| match e {
            Effect::Shell(ShellEffect::Focus(focus)) => Some(*focus),
            _ => None,
        });
        self.effects
            .retain(|e| !matches!(e, Effect::Shell(ShellEffect::Focus(_))));
        if let Some(focus) = focus {
            cx.set_focus(focus.map(text_targets::focus_id));
        }
    }
}

impl UiApp for NavApp {
    type Action = Action;
    type Message = ();

    fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
        let (width, height) = cx.frame.size();
        let slot = Slot {
            shared: &self.shared,
            width,
            height,
        };
        super::view(&mut self.nav, &slot, cx)
    }

    fn update(&mut self, action: Action, cx: &mut UiContext) {
        self.clock = cx.window.elapsed().as_millis() as u64;
        match action {
            Action::Navigation(action) => {
                let reduction = self.nav.reduce_action(action, &self.shared);
                self.reduce(reduction);
            }
            Action::Shell(ShellAction::FocusLogical(focus)) => {
                cx.set_focus(focus.map(text_targets::focus_id))
            }
            _ => {}
        }
        self.drain(cx);
    }

    fn edit_text(&mut self, target: FocusId, command: TextEditCommand) -> TextEditOutcome {
        let cx = TextCx {
            shared: &self.shared,
            now_ms: self.clock,
        };
        self.nav.edit_text(target, command, &cx).unwrap_or_default()
    }

    fn set_preedit(&mut self, target: FocusId, text: String, cursor: Option<(usize, usize)>) {
        self.nav.set_preedit(target, text, cursor);
    }
}

type Nav = UiTestHarness<NavApp>;

fn nav_harness() -> Nav {
    UiTestHarness::with_adapter(
        UiAdapter::new(NavApp::default(), "Files"),
        (320.0, 640.0),
        2.0,
    )
}

fn open(ui: &mut Nav, files: Vec<crate::contracts::viewport::pipeline::ManifestFile>) {
    let app = ui.app_mut();
    app.inject(Intent::ActivateWorkspace {
        path: PathBuf::from(REPO),
    });
    app.event(fixtures::snapshot_ready(
        1,
        fixtures::snapshot(RepositorySyncReason::Open, false),
    ));
    app.inject(Intent::PublishManifest(fixtures::manifest(1, files)));
    ui.frame();
}

fn publish(ui: &mut Nav, manifest: Rc<CompareManifestSnapshot>) {
    ui.app_mut().inject(Intent::PublishManifest(manifest));
    ui.frame();
}

fn selected(ui: &Nav) -> Option<SelectedFile> {
    ui.app().shared.selected_file
}

fn file_node(key: u64) -> By {
    By::test_id(format!("file:{key}").as_str())
}

fn three_files() -> Vec<crate::contracts::viewport::pipeline::ManifestFile> {
    vec![
        fixtures::file(1, "src/a.rs"),
        fixtures::file(2, "src/b.rs"),
        fixtures::file(3, "docs/c.md"),
    ]
}

#[test]
fn cli_repository_activates_the_workspace_and_syncs_it() {
    let mut args = no_args();
    args.repo = Some(PathBuf::from(REPO));
    let mut ui = harness::harness_with(startup(args, Settings::default()));

    assert_eq!(ui.app().0.shared().workspace, WorkspaceEpoch(1));
    let main = ui.main_window();
    assert_eq!(ui.window(main).title(), "alpha - Diffy");
    let effects = effects(&ui);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Navigation(NavigationEffect::SyncRepository(r))
            if r.path == PathBuf::from(REPO) && r.reason == RepositorySyncReason::Open
    )));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Navigation(NavigationEffect::WatchRepository { path: Some(p) }) if p == &PathBuf::from(REPO)
    )));
    ui.find(By::name("Opening repository"));

    harness::deliver(
        &mut ui,
        fixtures::snapshot_ready(1, fixtures::snapshot(RepositorySyncReason::Open, true)),
    );
    let shared = ui.app().0.shared();
    assert!(shared.repository.is_some());
    let nav = &ui.app().0.controllers().navigation;
    assert!(
        nav.compare().working_copy,
        "no saved refs opens the working tree"
    );
    ui.find(By::role_name(Role::Tab, "History"));
    ui.find(By::role_name(Role::Tab, "Operation log"));
}

#[test]
fn startup_refs_request_a_compare_and_the_saved_selection_is_restored() {
    let mut args = no_args();
    args.repo = Some(PathBuf::from(REPO));
    args.left = Some("main".to_owned());
    args.right = Some("feature".to_owned());
    args.compare_mode = Some(CompareMode::TwoDot);
    args.file_path = Some("src/b.rs".to_owned());
    let startup = startup(args, Settings::default());

    let mut nav = NavigationController::default();
    let shared = SharedContext::default();
    let reduction = nav.apply_intent(&Intent::Startup(Rc::new(startup)), &shared);
    let [Intent::ActivateWorkspace { path }] = reduction.intents.as_slice() else {
        panic!("expected workspace activation, got {:?}", reduction.intents);
    };
    let activation = nav.apply_intent(&Intent::ActivateWorkspace { path: path.clone() }, &shared);
    let request = activation
        .intents
        .iter()
        .find_map(|i| match i {
            Intent::RequestCompare(c) => Some(c.clone()),
            _ => None,
        })
        .expect("bootstrap compare");
    assert_eq!(
        request.request.spec,
        VcsCompareSpec::Range {
            from: "main".to_owned(),
            to: "feature".to_owned()
        }
    );
    let opened = nav.reduce_event(
        fixtures::snapshot_ready(1, fixtures::snapshot(RepositorySyncReason::Open, false)),
        &shared,
    );
    assert!(
        !opened
            .intents
            .iter()
            .any(|i| matches!(i, Intent::RequestCompare(_))),
        "the bootstrap compare is not repeated"
    );
    let manifest = fixtures::manifest(1, three_files());
    let published = nav.apply_intent(&Intent::PublishManifest(manifest), &shared);
    assert!(published.intents.iter().any(|i| matches!(
        i,
        Intent::SelectFile(Some(SelectedFile {
            key: FileKey(2),
            origin: SelectionOrigin::Command
        }))
    )));
}

#[test]
fn pull_request_startup_leaves_the_compare_to_review() {
    let mut args = no_args();
    args.repo = Some(PathBuf::from(REPO));
    args.open_pr = Some("https://github.com/o/r/pull/1".to_owned());
    let mut nav = NavigationController::default();
    let shared = SharedContext::default();
    nav.apply_intent(
        &Intent::Startup(Rc::new(startup(args, Settings::default()))),
        &shared,
    );
    nav.apply_intent(
        &Intent::ActivateWorkspace {
            path: PathBuf::from(REPO),
        },
        &shared,
    );
    let opened = nav.reduce_event(
        fixtures::snapshot_ready(1, fixtures::snapshot(RepositorySyncReason::Open, false)),
        &shared,
    );
    assert!(
        opened
            .intents
            .iter()
            .any(|i| matches!(i, Intent::RepositoryChanged(_)))
    );
    assert!(
        !opened
            .intents
            .iter()
            .any(|i| matches!(i, Intent::RequestCompare(_)))
    );
}

#[test]
fn manifest_rows_render_with_stats_and_pending_never_reads_as_zero() {
    let mut ui = nav_harness();
    let mut files = three_files();
    files[1].stats = FileStats::Known {
        additions: 3,
        deletions: 0,
    };
    open(&mut ui, files);

    ui.find(By::role_name(Role::List, "Changed files"));
    let row = ui.find(file_node(1));
    assert_eq!(row.name.as_deref(), Some("src/a.rs"));
    let painted = ui.painted_text();
    assert!(painted.contains("+3"));
    assert!(!painted.contains("+0") && !painted.contains("-0"));
    assert_eq!(selected(&ui).map(|s| s.key), Some(FileKey(1)));

    ui.click_node(file_node(3));
    assert_eq!(
        selected(&ui),
        Some(SelectedFile {
            key: FileKey(3),
            origin: SelectionOrigin::Sidebar
        })
    );
}

#[test]
fn selection_survives_reorder_filter_and_rename() {
    let mut ui = nav_harness();
    open(&mut ui, three_files());
    ui.click_node(file_node(2));

    let mut reordered = three_files();
    reordered.reverse();
    publish(&mut ui, fixtures::manifest(2, reordered));
    assert_eq!(ui.app().nav.selected_key(), Some(FileKey(2)));

    ui.click_node(By::role_name(Role::SearchInput, "Filter files"));
    ui.type_text("docs");
    assert!(ui.try_find(file_node(2)).is_none());
    assert_eq!(ui.app().nav.selected_key(), Some(FileKey(2)));
    ui.key("escape");
    assert_eq!(ui.app().nav.filter().text(), "");
    ui.find(file_node(2));

    publish(
        &mut ui,
        fixtures::manifest(
            3,
            vec![
                fixtures::file(11, "src/a.rs"),
                fixtures::renamed(12, "src/b.rs", "src/renamed.rs"),
            ],
        ),
    );
    assert_eq!(ui.app().nav.selected_key(), Some(FileKey(12)));
    assert_eq!(selected(&ui).map(|s| s.key), Some(FileKey(12)));
}

#[test]
fn filter_escape_clears_and_returns_focus_and_tab_moves_on() {
    let mut ui = nav_harness();
    open(&mut ui, three_files());
    let field = By::role_name(Role::SearchInput, "Filter files");

    ui.click_node(field.clone());
    assert_eq!(ui.focus(), Some(SIDEBAR_FILTER));
    ui.type_text("src");
    ui.ime_preedit("ü", Some((0, 2)));
    ui.ime_commit("ü");
    assert_eq!(ui.app().nav.filter().text(), "srcü");
    ui.find(By::name("No matching files"));

    ui.key("escape");
    assert_eq!(ui.app().nav.filter().text(), "");
    assert_eq!(ui.focus(), Some(SIDEBAR));

    ui.click_node(field.clone());
    ui.key("escape");
    assert_eq!(
        ui.focus(),
        Some(SIDEBAR),
        "an empty filter blurs to the list"
    );

    ui.click_node(field);
    ui.key("tab");
    assert_ne!(ui.focus(), Some(SIDEBAR_FILTER));
}

#[test]
fn keyboard_moves_the_selection_through_visible_rows() {
    let mut ui = nav_harness();
    open(&mut ui, three_files());
    ui.click_node(file_node(1));
    ui.key("arrowdown");
    assert_eq!(ui.app().nav.selected_key(), Some(FileKey(2)));
    ui.key("arrowdown");
    ui.key("arrowdown");
    assert_eq!(ui.app().nav.selected_key(), Some(FileKey(3)));
    ui.key("arrowup");
    assert_eq!(ui.app().nav.selected_key(), Some(FileKey(2)));
}

#[test]
fn tree_mode_groups_folders_and_reveals_the_selection() {
    let mut ui = nav_harness();
    open(&mut ui, three_files());
    ui.click_node(file_node(2));
    ui.click_node(By::role_name(Role::Button, "Show file tree"));
    ui.find(By::role_name(Role::Tree, "Changed files"));
    let src = ui.find(By::role_name(Role::TreeItem, "src"));
    assert!(ui.try_find(By::role_name(Role::TreeItem, "docs")).is_some());
    ui.find(file_node(2));
    assert!(
        ui.try_find(file_node(3)).is_none(),
        "collapsed folder hides its files"
    );
    ui.click_node(By::role_name(Role::TreeItem, "docs"));
    ui.find(file_node(3));
    assert!(src.bounds.y < ui.find(file_node(2)).bounds.y);
}

#[test]
fn viewed_markers_toggle_by_path() {
    let mut ui = nav_harness();
    open(&mut ui, three_files());
    let first = ui.find_all(By::role_name(Role::CheckBox, "Mark viewed"));
    assert_eq!(first.len(), 3);
    ui.click(first[0].center());
    assert!(ui.app().nav.is_viewed("src/a.rs"));
    ui.find(By::role_name(Role::CheckBox, "Mark unviewed"));
}

#[test]
fn large_manifests_only_build_visible_rows() {
    let mut ui = nav_harness();
    let files = (0..20_000)
        .map(|i| fixtures::file(i + 1, &format!("dir{}/file{i}.rs", i % 50)))
        .collect();
    open(&mut ui, files);
    let rows = ui.find_all(By::role(Role::ListItem));
    assert!(
        !rows.is_empty() && rows.len() < 80,
        "{} rows built",
        rows.len()
    );
    let list = ui.find(By::role_name(Role::List, "Changed files"));
    ui.pointer_move(list.center());
    ui.wheel(0.0, 20.0);
    let scrolled = ui.app().nav.scroll_px();
    assert!(scrolled > 0.0);
    ui.find(file_node((scrolled / list_row_height(&ui)) as u64 + 2));
}

fn list_row_height(ui: &Nav) -> f32 {
    ui.theme().metrics.ui_row_height.round()
}

#[test]
fn history_tab_loads_range_commits_and_drills_into_one() {
    let mut ui = nav_harness();
    open(&mut ui, three_files());
    ui.app_mut().update_compare("main", "feature");
    ui.click_node(By::role_name(Role::Tab, "History"));
    assert_eq!(ui.app().nav.tab(), SidebarTab::Commits);
    let generation = ui
        .app()
        .effects
        .iter()
        .find_map(|e| match e {
            Effect::Navigation(NavigationEffect::LoadHistory(r)) => Some(r.generation),
            _ => None,
        })
        .expect("history request");
    ui.find(By::name("Loading history"));

    ui.app_mut().event(NavigationEvent::HistoryReady {
        generation: CompareGeneration(generation.0 + 5),
        commits: vec![fixtures::change("x9", "Stale")].into(),
    });
    ui.frame();
    ui.find(By::name("Loading history"));

    ui.app_mut().event(NavigationEvent::HistoryReady {
        generation,
        commits: vec![
            fixtures::change("d4", "Fix parser"),
            fixtures::change("e5", "Add tests"),
        ]
        .into(),
    });
    ui.frame();
    ui.click_node(By::role_name(Role::ListItem, "Add tests"));
    let compare = ui.app().nav.compare().clone();
    assert_eq!(compare.mode, CompareMode::SingleCommit);
    assert_eq!(compare.left, "e5aaaa");
    assert!(ui.app().intents.iter().any(|i| matches!(
        i,
        Intent::RequestCompare(c) if c.request.spec == VcsCompareSpec::Change { revision: "e5aaaa".to_owned() }
    )));

    ui.click_node(By::role_name(Role::Button, "Back to comparison"));
    assert_eq!(ui.app().nav.compare().mode, CompareMode::TwoDot);
    assert_eq!(ui.app().nav.compare().left, "main");
}

#[test]
fn viewport_selection_does_not_bounce_back() {
    let mut nav = NavigationController::default();
    let shared = SharedContext::default();
    nav.apply_intent(
        &Intent::PublishManifest(fixtures::manifest(1, three_files())),
        &shared,
    );
    let reduction = nav.apply_intent(
        &Intent::SelectFile(Some(SelectedFile {
            key: FileKey(3),
            origin: SelectionOrigin::Viewport,
        })),
        &shared,
    );
    assert!(reduction.intents.is_empty());
    assert_eq!(nav.selected_key(), Some(FileKey(3)));
}

impl NavApp {
    fn update_compare(&mut self, left: &str, right: &str) {
        for action in [
            NavigationAction::SetRef {
                field: crate::contracts::navigation::CompareField::Left,
                value: left.to_owned(),
            },
            NavigationAction::SetRef {
                field: crate::contracts::navigation::CompareField::Right,
                value: right.to_owned(),
            },
            NavigationAction::SetCompareMode(CompareMode::TwoDot),
        ] {
            let reduction = self.nav.reduce_action(action, &self.shared);
            self.reduce(reduction);
        }
    }
}
