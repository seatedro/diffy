use std::path::PathBuf;
use std::sync::Arc;

use diffy_syntax::{Catalog, GrammarState, GrammarStatus};
use quark::view;
use quark_app::testing::{By, UiTestHarness};
use quark_app::{UiApp, UiContext, ViewContext};
use quark_ui::accessibility::AccessibilityRole as Role;
use quark_ui::element::*;
use quark_ui::style::Styled;

use super::LANGUAGES_TOGGLE;
use super::state::{DeliveryController, UPDATE_POLL_INTERVAL, UpdateState, grammar_languages};
use crate::app::shell::Slot;
use crate::app::testing::*;
use crate::contracts::commands::{
    CommandDescriptor, CommandSource, CommandsAction, SettingsSnapshot,
};
use crate::contracts::delivery::{DeliveryEffect, DeliveryEvent, GrammarAvailability};
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Action, Controller, Effect, Intent};
use crate::core::update::{AvailableUpdate, StagedUpdate, UpdateFormat, UpdatePlatform};

fn staged() -> StagedUpdate {
    StagedUpdate {
        update: AvailableUpdate {
            version: "9.9.9".to_owned(),
            channel: "stable".to_owned(),
            notes: None,
            pub_date: None,
            platform: "macos-aarch64".to_owned(),
            artifact: UpdatePlatform {
                format: UpdateFormat::Dmg,
                url: "https://example.com/a.dmg".to_owned(),
                signature: String::new(),
                sha256: String::new(),
                size: None,
            },
        },
        artifact_path: PathBuf::from("a.dmg"),
    }
}

fn grammars(states: &[(&str, GrammarState)]) -> DeliveryEvent {
    DeliveryEvent::GrammarsChanged(Arc::new(Catalog {
        statuses: states
            .iter()
            .map(|(language, state)| GrammarStatus {
                language: (*language).to_owned(),
                state: state.clone(),
            })
            .collect(),
        tags: states
            .iter()
            .map(|(language, _)| ((*language).to_owned(), (*language).to_owned()))
            .collect(),
        languages: states.len(),
    }))
}

fn delivery_effects(ui: &Harness) -> Vec<DeliveryEffect> {
    ui.app()
        .dispatcher()
        .effects
        .iter()
        .filter_map(|(effect, _)| match effect {
            Effect::Delivery(effect) => Some(effect.clone()),
            _ => None,
        })
        .collect()
}

fn startup_effects(updates_supported: bool, auto_update: bool) -> Vec<DeliveryEffect> {
    let mut delivery = DeliveryController::new(updates_supported);
    let settings = crate::platform::persistence::Settings {
        auto_update,
        ..Default::default()
    };
    let startup = crate::contracts::shell::AppStartup {
        options: crate::platform::startup::StartupOptions::from_parts(
            clap::Parser::parse_from(["diffy"]),
            None,
            String::new(),
            false,
        ),
        settings,
    };
    delivery
        .apply_intent(&Intent::Startup(startup.into()), &SharedContext::default())
        .effects
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::Delivery(effect) => Some(effect),
            _ => None,
        })
        .collect()
}

#[test]
fn startup_watches_grammars_and_checks_silently_per_settings() {
    let common = [
        "bash",
        "c",
        "cpp",
        "go",
        "javascript",
        "json",
        "python",
        "rust",
        "toml",
        "typescript",
        "tsx",
    ]
    .map(String::from)
    .to_vec();
    let base = vec![
        DeliveryEffect::WatchGrammars,
        DeliveryEffect::EnsureGrammars { languages: common },
    ];
    let mut checking = base.clone();
    checking.push(DeliveryEffect::CheckForUpdates { silent: true });
    checking.push(DeliveryEffect::ScheduleUpdatePoll {
        generation: 1,
        after: UPDATE_POLL_INTERVAL,
    });

    assert_eq!(startup_effects(true, true), checking);
    assert_eq!(startup_effects(true, false), base);
    assert_eq!(startup_effects(false, true), base);
}

#[test]
fn paths_request_their_extension_tags() {
    let cases = [
        ("src/app.js", vec!["js"]),
        ("src/View.JSX", vec!["jsx"]),
        ("flake.nix", vec!["nix"]),
        ("Dockerfile", vec!["dockerfile"]),
        ("src/", vec![]),
    ];
    for (path, expected) in cases {
        assert_eq!(
            grammar_languages(path).collect::<Vec<_>>(),
            expected,
            "{path}"
        );
    }
}

#[test]
fn installed_grammars_publish_availability_once_per_change() {
    let mut delivery = DeliveryController::default();
    let cx = SharedContext::default();
    let mut available = |event| {
        delivery
            .reduce_event(event, &cx)
            .intents
            .into_iter()
            .filter_map(|intent| match intent {
                Intent::GrammarAvailable(availability) => Some(availability),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    let first = available(grammars(&[("rust", GrammarState::Installed)]));
    let repeat = available(grammars(&[("rust", GrammarState::Ready)]));
    let downloading = available(grammars(&[
        ("rust", GrammarState::Ready),
        ("go", GrammarState::Downloading),
    ]));

    assert_eq!(
        first,
        vec![GrammarAvailability {
            revision: 1,
            tags: Arc::from(["rust".to_owned()]),
        }]
    );
    assert!(repeat.is_empty() && downloading.is_empty());
}

struct Host {
    delivery: DeliveryController,
    shared: SharedContext,
    effects: Vec<DeliveryEffect>,
    commands: Vec<CommandsAction>,
    settings: bool,
}

impl UiApp for Host {
    type Action = Action;
    type Message = DeliveryEvent;

    fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
        let (width, height) = cx.frame.size();
        let status_h = cx.theme.metrics.status_bar_height;
        let slot = Slot {
            shared: &self.shared,
            width,
            height: status_h,
        };
        if self.settings {
            let page = super::updates_settings(&self.delivery, &self.shared.settings, width, cx);
            return view! { <div w={width} h={height}>{page}</div> };
        }
        let status = super::view(&mut self.delivery, &slot, cx);
        let overlay = super::overlay(&mut self.delivery, &slot, cx);
        view! {
            <div w={width} h={height} class="flex-col relative overflow-hidden">
                <div class="flex-1" />
                <div h={status_h} class="flex-row items-center overflow-hidden">{status}</div>
                {?overlay}
            </div>
        }
    }

    fn message(&mut self, event: DeliveryEvent, cx: &mut UiContext) {
        self.delivery.reduce_event(event, &self.shared);
        cx.window.request_redraw();
    }

    fn update(&mut self, action: Action, _cx: &mut UiContext) {
        if let Action::Commands(action) = &action {
            self.commands.push(action.clone());
        }
        if let Action::Delivery(action) = action {
            let reduction = self.delivery.reduce_action(action, &self.shared);
            self.effects.extend(
                reduction
                    .effects
                    .into_iter()
                    .filter_map(|effect| match effect {
                        Effect::Delivery(effect) => Some(effect),
                        _ => None,
                    }),
            );
        }
    }
}

fn host() -> UiTestHarness<Host> {
    host_showing(false)
}

fn host_showing(settings: bool) -> UiTestHarness<Host> {
    UiTestHarness::new(
        Host {
            delivery: DeliveryController::default(),
            shared: SharedContext::default(),
            effects: Vec::new(),
            commands: Vec::new(),
            settings,
        },
        (720.0, 560.0),
        2.0,
    )
}

fn event(ui: &mut UiTestHarness<Host>, event: DeliveryEvent) {
    ui.send_message(event);
}

#[test]
fn failed_language_offers_retry_download() {
    let mut ui = host();
    event(
        &mut ui,
        grammars(&[
            ("go", GrammarState::Ready),
            ("rust", GrammarState::Failed("HTTP 404".to_owned())),
        ]),
    );

    assert!(ui.try_find(By::name("Retry download")).is_none());
    ui.click_node(By::role_name(Role::Button, "Syntax languages"));
    let panel = ui.find(By::test_id("delivery.languages"));
    let status = ui.find(By::role_name(Role::Button, "Syntax languages"));
    let row = ui.find(By::test_id("delivery.language.rust"));
    let retry = ui.find(By::role_name(Role::Button, "Retry download"));
    assert!(panel.bounds.y + panel.bounds.height <= status.bounds.y);
    assert!(retry.bounds.y >= row.bounds.y && retry.bounds.y < row.bounds.y + row.bounds.height);
    ui.click_node(By::role_name(Role::Button, "Retry download"));

    assert_eq!(
        ui.app().effects,
        vec![DeliveryEffect::RetryGrammar {
            language: "rust".to_owned()
        }]
    );
    assert!(
        ui.try_find(By::role_name(Role::Button, "Retry download"))
            .is_none()
    );
    assert!(ui.painted_text().contains("Downloading"));
}

#[test]
fn check_for_updates_disables_while_checking_and_failure_keeps_focus() {
    let mut ui = host();
    ui.click_node(By::role_name(Role::Button, "Syntax languages"));
    ui.click_node(By::role_name(Role::Button, "Check for updates"));

    assert_eq!(
        ui.app().effects,
        vec![DeliveryEffect::CheckForUpdates { silent: false }]
    );
    assert!(matches!(ui.app().delivery.update(), UpdateState::Checking));
    ui.click_node(By::role_name(Role::Button, "Check for updates"));
    assert_eq!(ui.app().effects.len(), 1);

    let focus = ui.focus();
    event(
        &mut ui,
        DeliveryEvent::UpdateCheckFailed {
            message: "offline".to_owned(),
            silent: false,
        },
    );

    assert_eq!(ui.focus(), focus);
    assert!(matches!(ui.app().delivery.update(), UpdateState::Failed(_)));
    assert!(ui.painted_text().contains("offline"));
    assert!(
        ui.find(By::role_name(Role::Button, "Check for updates"))
            .bounds
            .width
            > 0.0
    );
}

#[test]
fn staged_update_offers_restart_in_the_status_bar() {
    let mut ui = harness();
    deliver_current(
        &mut ui,
        DeliveryEvent::UpdateStaged {
            staged: Box::new(staged()),
            silent: true,
        },
    );

    let status = ui.find(By::test_id("status"));
    let restart = ui.find(By::role_name(Role::Button, "Restart to update"));
    assert!(restart.bounds.y >= status.bounds.y);
    ui.click_node(By::role_name(Role::Button, "Restart to update"));

    assert_eq!(
        delivery_effects(&ui),
        vec![DeliveryEffect::ApplyStagedUpdate(Box::new(staged()))]
    );
    assert!(
        ui.try_find(By::role_name(Role::Button, "Restart to update"))
            .is_none()
    );
}

fn reduce_effects(reduction: crate::contracts::shell::Reduction) -> Vec<DeliveryEffect> {
    reduction
        .effects
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::Delivery(effect) => Some(effect),
            _ => None,
        })
        .collect()
}

fn settings(auto_update: bool) -> Intent {
    let mut snapshot = SettingsSnapshot::default();
    snapshot.settings.auto_update = auto_update;
    Intent::SettingsChanged(snapshot.into())
}

#[test]
fn update_polling_follows_settings_and_drops_superseded_timers() {
    let cx = SharedContext::default();
    let mut delivery = DeliveryController::new(true);
    let poll = |generation| DeliveryEffect::ScheduleUpdatePoll {
        generation,
        after: UPDATE_POLL_INTERVAL,
    };
    let check = DeliveryEffect::CheckForUpdates { silent: true };

    let enabled = reduce_effects(delivery.apply_intent(&settings(true), &cx));
    let stale =
        reduce_effects(delivery.reduce_event(DeliveryEvent::UpdatePollDue { generation: 0 }, &cx));
    let due =
        reduce_effects(delivery.reduce_event(DeliveryEvent::UpdatePollDue { generation: 1 }, &cx));
    delivery.reduce_event(DeliveryEvent::UpdateNotAvailable { silent: true }, &cx);
    let disabled = reduce_effects(delivery.apply_intent(&settings(false), &cx));
    let after_disable =
        reduce_effects(delivery.reduce_event(DeliveryEvent::UpdatePollDue { generation: 2 }, &cx));

    assert_eq!(enabled, vec![check.clone(), poll(1)]);
    assert!(stale.is_empty());
    assert_eq!(due, vec![check, poll(2)]);
    assert!(disabled.is_empty() && after_disable.is_empty());
}

#[test]
fn staged_update_stops_polling() {
    let cx = SharedContext::default();
    let mut delivery = DeliveryController::new(true);
    delivery.apply_intent(&settings(true), &cx);
    delivery.reduce_event(
        DeliveryEvent::UpdateStaged {
            staged: Box::new(staged()),
            silent: true,
        },
        &cx,
    );

    assert!(
        reduce_effects(delivery.reduce_event(DeliveryEvent::UpdatePollDue { generation: 1 }, &cx))
            .is_empty()
    );
}

#[test]
fn commands_follow_update_and_grammar_state() {
    let cx = SharedContext::default();
    let mut delivery = DeliveryController::new(true);
    let titles = |delivery: &DeliveryController| {
        let mut out: Vec<CommandDescriptor> = Vec::new();
        delivery.commands(&cx, &mut out);
        out.into_iter()
            .map(|command| (command.title, command.detail, command.enabled))
            .collect::<Vec<_>>()
    };

    let idle = titles(&delivery);
    delivery.reduce_event(
        grammars(&[("python", GrammarState::Failed("no".to_owned()))]),
        &cx,
    );
    delivery.reduce_event(
        DeliveryEvent::UpdateStaged {
            staged: Box::new(staged()),
            silent: true,
        },
        &cx,
    );
    let staged_titles = titles(&delivery);

    assert_eq!(
        idle,
        vec![
            ("Check for updates".to_owned(), None, true),
            ("Syntax languages".to_owned(), None, true),
        ]
    );
    assert_eq!(
        staged_titles,
        vec![
            ("Check for updates".to_owned(), None, false),
            (
                "Restart to update".to_owned(),
                Some("Diffy 9.9.9".to_owned()),
                true
            ),
            (
                "Syntax languages".to_owned(),
                Some("1 failed to download or load".to_owned()),
                true
            ),
        ]
    );
}

#[test]
fn languages_panel_closes_on_escape_and_outside_click_keeping_focus_on_the_toggle() {
    let mut ui = host();
    ui.click_node(By::role_name(Role::Button, "Syntax languages"));
    assert_eq!(ui.focus(), Some(LANGUAGES_TOGGLE));
    let panel = ui.find(By::test_id("delivery.languages"));

    ui.click((panel.bounds.x + 4.0, panel.bounds.y + 4.0));
    assert!(ui.app().delivery.languages_open());
    assert_eq!(ui.focus(), Some(LANGUAGES_TOGGLE));
    ui.key("escape");
    assert!(!ui.app().delivery.languages_open());
    assert!(ui.try_find(By::test_id("delivery.languages")).is_none());
    assert_eq!(ui.focus(), Some(LANGUAGES_TOGGLE));

    ui.key("enter");
    assert!(ui.app().delivery.languages_open());
    ui.click((8.0, 8.0));
    assert!(!ui.app().delivery.languages_open());
    assert_eq!(ui.focus(), Some(LANGUAGES_TOGGLE));
    assert!(ui.app().effects.is_empty());
}

#[test]
fn languages_panel_has_a_title_and_reports_each_state() {
    let mut ui = host();
    event(
        &mut ui,
        grammars(&[
            ("rust", GrammarState::Ready),
            ("go", GrammarState::Installed),
            ("zig", GrammarState::Downloading),
        ]),
    );
    ui.click_node(By::role_name(Role::Button, "Syntax languages"));
    let text = ui.painted_text();
    let state = |language: &str| {
        let row = ui.find(By::test_id(format!("delivery.language.{language}")));
        ui.painted_texts()
            .into_iter()
            .filter(|painted| {
                painted.bounds.y >= row.bounds.y
                    && painted.bounds.y < row.bounds.y + row.bounds.height
            })
            .map(|painted| painted.text)
            .collect::<Vec<_>>()
            .join(" ")
    };

    assert!(text.contains("Syntax languages"));
    assert_eq!(state("rust"), "Rust Ready");
    assert_eq!(state("go"), "Go Installed");
    assert_eq!(state("zig"), "Zig Downloading");
    assert!(text.contains("3 languages available"));
}

#[test]
fn updates_settings_toggle_check_and_restart() {
    let mut ui = host_showing(true);
    ui.find(By::test_id("settings.updates"));
    ui.click_node(By::role_name(Role::Button, "Check for updates"));
    ui.click_node(By::role_name(Role::Switch, "Automatic updates"));
    event(
        &mut ui,
        DeliveryEvent::UpdateStaged {
            staged: Box::new(staged()),
            silent: true,
        },
    );
    ui.click_node(By::role_name(Role::Button, "Restart to update"));

    assert_eq!(
        ui.app().effects,
        vec![
            DeliveryEffect::CheckForUpdates { silent: false },
            DeliveryEffect::ApplyStagedUpdate(Box::new(staged())),
        ]
    );
    assert_eq!(ui.app().commands, vec![CommandsAction::ToggleAutoUpdate]);
    assert!(ui.painted_text().contains(crate::APP_VERSION));
}
