use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use quark::view;
use quark_app::platform::dialog::{DialogId, FileDialog, FileDialogKind};
use quark_app::{AppEvent as WindowEvent, InputEvent, UiApp, UiContext, UiSender, ViewContext};
use quark_components::tooltip::{TooltipSide, tooltip_layer};
use quark_components::{Axis, Pane, Split, SplitEvent, SplitState};
use quark_ui::FocusId;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::style::Styled;
use quark_ui::text_input::{TextEditCommand, TextEditOutcome};
use quark_ui::theme::Theme;
use winit::window::CursorIcon;

use crate::app::diff_slots::DiffSlots;
use crate::app::route::{self, Controllers, MAX_INTENTS_PER_TURN, Routed};
use crate::app::text_targets::{self, TextCx};
use crate::app::tokens::{self, Layout, Sp, Sz, TextStyle};
use crate::app::{commands, delivery, mutations, navigation, review, viewport};
use crate::contracts::commands::SettingsSection;
use crate::contracts::commands::{CommandsAction, LauncherFolder, SettingsSnapshot, ToastUpdate};
use crate::contracts::shared::{AccountEpoch, SharedContext, WorkspaceEpoch};
use crate::contracts::shell::{
    Action, AppMessage, AppStartup, AreaDirection, Controller, Effect, Intent, LogicalFocus,
    Reduction, ShellAction, ShellEffect, ShellEvent,
};
use crate::contracts::viewport::ViewportAction;
use crate::core::compare::RendererKind;
use crate::core::vcs::model::RefKind;

pub trait EffectDispatcher: 'static {
    fn attach(&mut self, _sender: UiSender<AppMessage>) {}

    fn syntax_wake_handled(&mut self) {}

    fn syntax_wake(&self) -> Option<Arc<dyn Fn() + Send + Sync>> {
        None
    }

    fn dispatch(&mut self, effect: Effect, workspace: WorkspaceEpoch);
}

pub struct Slot<'a> {
    pub shared: &'a SharedContext,
    pub width: f32,
    pub height: f32,
}

const COPIED_MS: u64 = 1500;
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug)]
struct ShellState {
    display_name: String,
    workspace_label: Option<String>,
    window_title_dirty: bool,
    debug_overlay: bool,
    sidebar: SplitState,
    repository_dialog: Option<DialogId>,
    scan_folder_dialog: Option<DialogId>,
    launcher_dialog: Option<(DialogId, LauncherFolder)>,
    copied_until: u64,
    area: Area,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Area {
    Title,
    PullRequest,
    Sidebar,
    #[default]
    Main,
    Commit,
    Status,
}

impl ShellState {
    fn new(display_name: String) -> Self {
        let theme = crate::app::theme::default_dark();
        let sidebar =
            Pane::fixed("Files", tokens::sidebar_default(&theme)).min(tokens::sidebar_min(&theme));
        let main = Pane::flex("Diff").min(tokens::main_surface_min(&theme));
        Self {
            display_name,
            workspace_label: None,
            window_title_dirty: false,
            debug_overlay: false,
            sidebar: SplitState::new(Axis::Horizontal, vec![sidebar, main]),
            repository_dialog: None,
            scan_folder_dialog: None,
            launcher_dialog: None,
            copied_until: 0,
            area: Area::Main,
        }
    }

    fn set_workspace(&mut self, path: &Path) {
        let label = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned();
        self.workspace_label = Some(label);
        self.window_title_dirty = true;
    }

    fn window_title(&self) -> String {
        match &self.workspace_label {
            Some(label) => format!("{label} - {}", self.display_name),
            None => self.display_name.clone(),
        }
    }
}

pub struct DiffyApp<D> {
    controllers: Controllers,
    shared: SharedContext,
    dispatcher: D,
    shell: ShellState,
    slots: DiffSlots,
    deferred: Vec<ShellEffect>,
    startup: Option<Rc<AppStartup>>,
    clock_ms: u64,
    energy_hud: crate::app::energy_hud::EnergyHud,
}

impl<D: EffectDispatcher> DiffyApp<D> {
    pub fn new(dispatcher: D, display_name: impl Into<String>) -> Self {
        Self {
            controllers: Controllers::default(),
            shared: SharedContext::default(),
            dispatcher,
            shell: ShellState::new(display_name.into()),
            slots: DiffSlots::default(),
            deferred: Vec::new(),
            startup: None,
            clock_ms: 0,
            energy_hud: Default::default(),
        }
    }

    pub fn with_startup(mut self, startup: AppStartup) -> Self {
        self.startup = Some(Rc::new(startup));
        self
    }

    pub fn shared(&self) -> &SharedContext {
        &self.shared
    }

    pub fn controllers(&self) -> &Controllers {
        &self.controllers
    }

    pub fn dispatcher(&self) -> &D {
        &self.dispatcher
    }

    pub fn dispatcher_mut(&mut self) -> &mut D {
        &mut self.dispatcher
    }

    pub fn debug_overlay(&self) -> bool {
        self.shell.debug_overlay
    }

    fn copy(&mut self, text: String, cx: &mut UiContext) {
        cx.window.set_clipboard_text(&text);
        self.tick(cx);
        self.shell.copied_until = self.clock_ms + COPIED_MS;
        self.shared.copied = Some(text.into());
    }

    fn tick(&mut self, cx: &UiContext) {
        self.clock_ms = cx.window.elapsed().as_millis() as u64;
    }

    fn reduce(&mut self, reduction: Reduction) -> bool {
        let Reduction {
            effects,
            intents,
            mut changed,
        } = reduction;
        self.dispatch(effects);
        let mut queue = VecDeque::from(intents);
        let mut published = false;
        let mut applied = 0;
        while let Some(intent) = queue.pop_front() {
            if applied == MAX_INTENTS_PER_TURN {
                tracing::warn!(
                    dropped = queue.len() + 1,
                    "intent cascade exceeded its per-turn limit"
                );
                break;
            }
            applied += 1;
            published |= self.publish(&intent);
            let reduction = self.controllers.apply_intent(&intent, &self.shared);
            changed |= reduction.changed;
            self.dispatch(reduction.effects);
            queue.extend(reduction.intents);
        }
        if published {
            self.shared.revision += 1;
        }
        changed || published
    }

    fn dispatch(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Shell(
                    effect @ (ShellEffect::OpenRepositoryDialog
                    | ShellEffect::OpenScanFolderDialog
                    | ShellEffect::OpenLauncherDialog(_)
                    | ShellEffect::SetClipboard(_)
                    | ShellEffect::Focus(_)),
                ) => self.deferred.push(effect),
                effect => self.dispatcher.dispatch(effect, self.shared.workspace),
            }
        }
    }

    fn publish(&mut self, intent: &Intent) -> bool {
        let shared = &mut self.shared;
        match intent {
            Intent::ActivateWorkspace { path } => {
                shared.workspace = WorkspaceEpoch(shared.workspace.0 + 1);
                shared.repository = None;
                shared.manifest = None;
                shared.selected_file = None;
                shared.selection = None;
                shared.review = None;
                shared.symbols = None;
                shared.document = None;
                self.shell.set_workspace(path);
                true
            }
            Intent::PublishManifest(manifest) => {
                shared.manifest = Some(manifest.clone());
                true
            }
            Intent::PublishSymbols(index) => {
                shared.symbols = index.clone();
                true
            }
            Intent::PublishDocument(document) => {
                let changed = shared.document != *document;
                shared.document = document.clone();
                changed
            }
            Intent::SelectFile(file) => {
                let changed = shared.selected_file != *file;
                shared.selected_file = file.clone();
                changed
            }
            Intent::SelectSource(target) => {
                let changed = shared.selection != *target;
                shared.selection = target.clone();
                changed
            }
            Intent::RepositoryChanged(repository) => {
                shared.repository = Some(repository.clone());
                true
            }
            Intent::SetReviewTarget(review) => {
                shared.review = review.clone();
                true
            }
            Intent::AccountChanged => {
                shared.account = AccountEpoch(shared.account.0 + 1);
                true
            }
            Intent::SettingsChanged(settings) => {
                if let Some(width) = settings.settings.sidebar_width_px {
                    self.shell.sidebar.set_size(0, width as f32);
                }
                shared.settings = settings.clone();
                true
            }
            Intent::Startup(startup) => {
                let mut settings = startup.settings.clone();
                commands::normalize_settings(&mut settings);
                if let Some(width) = settings.sidebar_width_px {
                    self.shell.sidebar.set_size(0, width as f32);
                }
                shared.settings = Rc::new(SettingsSnapshot {
                    revision: shared.settings.revision + 1,
                    settings,
                });
                true
            }
            Intent::OpenPullRequest { .. }
            | Intent::FocusReviewCard(_)
            | Intent::RequestAnnotation(_)
            | Intent::OpenTextCompare
            | Intent::RequestCompare(_)
            | Intent::InvalidateRepository
            | Intent::GrammarAvailable(_)
            | Intent::ChooseDefinition(_)
            | Intent::RequestConfirmation(_)
            | Intent::Toast(_)
            | Intent::RankFiles(_)
            | Intent::RankFilesUnavailable { .. } => false,
        }
    }

    fn flush(&mut self, cx: &mut UiContext) {
        for _ in 0..16 {
            let requests = commands::take_requests(&self.controllers);
            if requests.is_empty() {
                break;
            }
            cx.window.request_redraw();
            for action in commands::apply_platform(requests, cx) {
                match self.controllers.route_action(action, &self.shared) {
                    Routed::Shell(action) => self.shell_action(action, cx),
                    Routed::Stream(reduction) => {
                        self.reduce(reduction);
                    }
                }
            }
        }
        for effect in std::mem::take(&mut self.deferred) {
            match effect {
                ShellEffect::OpenRepositoryDialog => {
                    self.shell.repository_dialog = Some(cx.window.file_dialog(FileDialog {
                        kind: FileDialogKind::OpenFolder,
                        title: Some("Open Repository".to_owned()),
                        ..FileDialog::default()
                    }));
                }
                ShellEffect::OpenScanFolderDialog => {
                    self.shell.scan_folder_dialog = Some(cx.window.file_dialog(FileDialog {
                        kind: FileDialogKind::OpenFolder,
                        title: Some("Add Folder".to_owned()),
                        ..FileDialog::default()
                    }));
                }
                ShellEffect::OpenLauncherDialog(folder) => {
                    let title = match folder {
                        LauncherFolder::LocateClone { .. } => "Locate Clone",
                        LauncherFolder::CloneInto { .. } => "Clone Into",
                    };
                    let id = cx.window.file_dialog(FileDialog {
                        kind: FileDialogKind::OpenFolder,
                        title: Some(title.to_owned()),
                        ..FileDialog::default()
                    });
                    self.shell.launcher_dialog = Some((id, folder));
                }
                ShellEffect::SetClipboard(text) => self.copy(text, cx),
                ShellEffect::Focus(focus) => cx.set_focus(focus.map(text_targets::focus_id)),
                effect => self
                    .dispatcher
                    .dispatch(Effect::Shell(effect), self.shared.workspace),
            }
        }
        if self.shell.window_title_dirty {
            self.shell.window_title_dirty = false;
            cx.window.set_title(&self.shell.window_title());
        }
        self.ensure_focus(cx);
    }

    fn ensure_focus(&mut self, cx: &mut UiContext) {
        if cx.focus().is_none()
            && self.shared.repository.is_some()
            && self.controllers.commands.top_layer().is_none()
            && !self.controllers.commands.settings_open()
            && !self.controllers.review.is_modal_open()
        {
            cx.set_focus(Some(text_targets::focus_id(self.area_focus(Area::Main))));
        }
    }

    fn shell_action(&mut self, action: ShellAction, cx: &mut UiContext) {
        match action {
            ShellAction::FocusLogical(focus) => cx.set_focus(focus.map(text_targets::focus_id)),
            ShellAction::StartJump => {
                cx.start_jump_hints();
            }
            ShellAction::FocusArea(direction) => {
                let from = self.area_of(cx.focus());
                if let Some(to) = self.area_toward(from, direction) {
                    self.shell.area = to;
                    cx.set_focus(Some(text_targets::focus_id(self.area_focus(to))));
                }
            }
            ShellAction::CopyText(text) => self.copy(text, cx),
            ShellAction::ToggleDebugOverlay => {
                self.shell.debug_overlay = !self.shell.debug_overlay;
                self.energy_hud = Default::default();
            }
            ShellAction::Minimize => cx.window.minimize(),
            ShellAction::ToggleMaximize => cx.window.toggle_maximized(),
            ShellAction::Close => {
                if let Some(window) = cx.window.window_handle() {
                    cx.window.close_window(window);
                }
            }
            ShellAction::CloseTabOrWindow => match self.controllers.viewport.files.active() {
                Some(tab) => self.update(
                    crate::contracts::viewport::FilesAction::Close(tab).into(),
                    cx,
                ),
                None => self.shell_action(ShellAction::Close, cx),
            },
            ShellAction::ResizeSidebar(event) => {
                let commit = matches!(event, SplitEvent::Release { .. } | SplitEvent::Nudge { .. });
                self.shell.sidebar.apply(event, self.clock_ms);
                if commit && !self.shell.sidebar.is_collapsed(0) {
                    let width = self.shell.sidebar.size(0).round() as u32;
                    let action = Action::from(CommandsAction::SetSidebarWidth(width));
                    if let Routed::Stream(reduction) =
                        self.controllers.route_action(action, &self.shared)
                    {
                        self.reduce(reduction);
                    }
                }
                cx.window.request_redraw();
            }
            ShellAction::Bootstrap | ShellAction::Noop => {}
        }
    }

    fn area_of(&self, focus: Option<FocusId>) -> Area {
        use text_targets as t;
        match focus {
            Some(t::SIDEBAR | t::SIDEBAR_FILTER) => Area::Sidebar,
            Some(
                t::VIEWPORT
                | t::SEARCH
                | t::REVIEW_OVERVIEW
                | t::REVIEW_COMPOSER
                | t::TEXT_COMPARE_LEFT
                | t::TEXT_COMPARE_RIGHT,
            ) => Area::Main,
            Some(t::COMMIT_MESSAGE) => Area::Commit,
            Some(t::TITLE_BAR | t::REVIEW_ACCOUNT) => Area::Title,
            Some(t::PULL_REQUEST_BAR) => Area::PullRequest,
            Some(id) if id == text_targets::focus_id(LogicalFocus::StatusBar) => Area::Status,
            _ => self.shell.area,
        }
    }

    fn area_toward(&self, from: Area, direction: AreaDirection) -> Option<Area> {
        let pull_request = self.controllers.review.active_pull_request().is_some();
        let sidebar = self.controllers.navigation.visible() && self.shared.repository.is_some();
        let commit = sidebar
            && mutations::working_tree(&self.shared)
            && self
                .shared
                .repository
                .as_ref()
                .is_some_and(|repo| repo.snapshot.capabilities.create_commit);
        let top = if pull_request {
            Area::PullRequest
        } else {
            Area::Title
        };
        use AreaDirection as D;
        let to = match (from, direction) {
            (Area::Main, D::Left) | (Area::Commit, D::Up) if sidebar => Area::Sidebar,
            (Area::Sidebar | Area::Commit, D::Right) => Area::Main,
            (Area::Main | Area::Sidebar, D::Up) => top,
            (Area::PullRequest, D::Up) => Area::Title,
            (Area::Title, D::Down) if pull_request => Area::PullRequest,
            (Area::Title | Area::PullRequest, D::Down) => Area::Main,
            (Area::Sidebar, D::Down) if commit => Area::Commit,
            (Area::Sidebar | Area::Main | Area::Commit, D::Down) => Area::Status,
            (Area::Status, D::Up) => Area::Main,
            _ => return None,
        };
        Some(to)
    }

    fn area_focus(&self, area: Area) -> LogicalFocus {
        match area {
            Area::Title => LogicalFocus::TitleBar,
            Area::PullRequest => LogicalFocus::PullRequestBar,
            Area::Sidebar => LogicalFocus::Sidebar,
            Area::Main if self.controllers.review.overview_open() => LogicalFocus::ReviewOverview,
            Area::Main => LogicalFocus::Viewport,
            Area::Commit => LogicalFocus::CommitMessage,
            Area::Status => LogicalFocus::StatusBar,
        }
    }

    fn shell_event(&mut self, event: ShellEvent) -> Reduction {
        match event {
            ShellEvent::RepositoryDialogClosed { path: Some(path) } => {
                Reduction::default().intent(Intent::ActivateWorkspace { path })
            }
            ShellEvent::RepositoryDialogClosed { path: None } => Reduction::default(),
            ShellEvent::ScanFolderDialogClosed { path: Some(path) } => Controller::reduce_action(
                &mut self.controllers.commands,
                CommandsAction::AddRepoScanRoot(path),
                &self.shared,
            ),
            ShellEvent::ScanFolderDialogClosed { path: None } => Reduction::default(),
            ShellEvent::LauncherDialogClosed {
                folder,
                path: Some(path),
            } => Controller::reduce_action(
                &mut self.controllers.commands,
                CommandsAction::LauncherFolderChosen { folder, path },
                &self.shared,
            ),
            ShellEvent::LauncherDialogClosed { path: None, .. } => Reduction::default(),
            ShellEvent::BrowserOpenFailed { message } => {
                Reduction::default().intent(Intent::Toast(ToastUpdate::error(message)))
            }
        }
    }

    fn deliver(&mut self, event: crate::contracts::shell::AppEvent) -> bool {
        let reduction = match self.controllers.route_event(event, &self.shared) {
            Routed::Shell(event) => self.shell_event(event),
            Routed::Stream(reduction) => reduction,
        };
        self.reduce(reduction)
    }

    fn title_bar(&mut self, width: f32, height: f32, cx: &mut ViewContext) -> AnyElement {
        let theme = cx.theme;
        let tc = &theme.colors;
        let scale = tokens::ui_scale(theme);
        let label = self
            .shell
            .workspace_label
            .as_deref()
            .unwrap_or(&self.shell.display_name);
        let slot = Slot {
            shared: &self.shared,
            width,
            height,
        };
        let review = review::view(&mut self.controllers.review, &slot, cx);
        let editing = self.controllers.commands.editing_ref();
        let nav = &self.controllers.navigation;
        let repo = match nav.repo_path() {
            Some(_) => navigation::repo_chip(nav, theme),
            None => view! {
                <text size={TextStyle::Body.font_size(theme)} class="truncate font-semibold" color={tc.text_strong}>
                    {label}
                </text>
            },
        };
        let compare = navigation::compare_bar(nav, editing, theme);
        view! {
            <div
                w={width}
                h={height}
                window_drag_region
                class="flex-row items-center shrink-0"
                pl={tokens::title_leading_inset(theme)}
                pr={tokens::scaled(Sp::MD, scale)}
                gap={tokens::scaled(Sp::SM, scale)}
                bg={tc.title_bar_background}
                border_b={tc.border_variant}
                accessibility_role={AccessibilityRole::TitleBar}
                aria-label="Title bar"
                test_id="title"
            >
                <div class="flex-1 flex-row items-center" min_w={0.0}>{repo}</div>
                {?compare}
                <div class="flex-1 flex-row items-center justify-end overflow-hidden" min_w={0.0} gap={tokens::scaled(Sp::SM, scale)}>{review}</div>
            </div>
        }
    }

    fn body(&mut self, width: f32, height: f32, cx: &mut ViewContext) -> AnyElement {
        if self.controllers.commands.launcher_active()
            && !self.controllers.viewport.interaction.text_compare().active
            && !self.controllers.review.overview_open()
        {
            let slot = Slot {
                shared: &self.shared,
                width,
                height,
            };
            let signed_in = self.controllers.review.signed_in();
            if let Some(launcher) =
                commands::launcher(&mut self.controllers.commands, &slot, signed_in, cx)
            {
                return launcher;
            }
        }
        let theme = cx.theme;
        let fits = tokens::sidebar_fits(theme, width) && self.controllers.navigation.visible();
        let (files_w, main_w) = match self.shell.sidebar.resolve(width).as_slice() {
            [files, main] if fits => (*files, *main),
            _ => (0.0, width),
        };
        let files = fits.then(|| {
            let slot = Slot {
                shared: &self.shared,
                width: files_w,
                height,
            };
            navigation::view(
                &mut self.controllers.navigation,
                &self.controllers.review,
                &self.controllers.viewport,
                &slot,
                mutations::commit_box(&mut self.controllers.mutations, &slot, cx),
                cx,
            )
        });
        let presentation = self.controllers.review.presentation();
        self.controllers
            .viewport
            .interaction
            .sync_review(&presentation);
        let banner = self.controllers.navigation.decorator(
            &self.controllers.review,
            self.shared.copied.as_deref(),
            theme,
        );
        let decorator = self.slots.decorator(
            presentation,
            self.controllers.mutations.presentation(),
            banner,
            tokens::ui_scale(theme),
        );
        let strip = viewport::files::tab_strip(&self.controllers.viewport.files, main_w, cx);
        let strip_h = if strip.is_some() {
            tokens::scaled(Sz::SIDEBAR_LIST_OFFSET, tokens::ui_scale(theme))
        } else {
            0.0
        };
        let body_h = (height - strip_h).max(0.0);
        let slot = Slot {
            shared: &self.shared,
            width: main_w,
            height: body_h,
        };
        let overview = self.controllers.review.overview_open();
        let file = if overview {
            None
        } else {
            viewport::files::file_view(&mut self.controllers.viewport.files, main_w, body_h, cx)
        };
        let showing_file = overview || file.is_some();
        let main = match (
            file,
            navigation::merge_base_empty(&self.controllers.navigation, theme),
        ) {
            _ if overview => review::overview(&self.controllers.review, &slot, cx),
            (Some(file), _) => file,
            (None, Some(empty)) => empty,
            (None, None) => viewport::interaction::view_shared(
                &mut self.controllers.viewport.interaction,
                &slot,
                decorator,
                cx,
            ),
        };
        let bar = (!showing_file)
            .then(|| mutations::selection_bar(&mut self.controllers.mutations, &slot, cx))
            .flatten();
        let main = view! {
            <div w={main_w} h={height} class="flex-col">
                {?strip}
                <div w={main_w} h={body_h} class="relative">{main}{?bar}</div>
            </div>
        };
        let content = match files {
            Some(files) => Split::new(
                "diffy.sidebar",
                &self.shell.sidebar,
                width,
                route::sidebar_split,
            )
            .child(files)
            .child(main)
            .build(theme),
            None => main,
        };
        view! { <div w={width} h={height} class="flex-row">{content}</div> }
    }

    fn status_bar(&mut self, width: f32, height: f32, cx: &mut ViewContext) -> AnyElement {
        let theme = cx.theme;
        let tc = &theme.colors;
        let scale = tokens::ui_scale(theme);
        let slot = Slot {
            shared: &self.shared,
            width,
            height,
        };
        let compare = viewport::pipeline::view(&mut self.controllers.viewport.pipeline, &slot, cx);
        let changes = mutations::view(&mut self.controllers.mutations, &slot, cx);
        let delivery = delivery::view(&mut self.controllers.delivery, &slot, cx);
        let viewing = navigation::viewing_status(&self.controllers.navigation, theme);
        let small = TextStyle::BodySmall.font_size(theme);
        let sync = mutations::sync_controls(&self.controllers.mutations, &slot, theme);
        let branch = branch_status(&self.shared, sync, theme);
        let settings = mutations::render_control(
            mutations::Control::new(
                text_targets::STATUS_SETTINGS,
                "Settings",
                CommandsAction::OpenSettings,
                mutations::Tone::Ghost,
            )
            .icon_only(quark_ui::icons::lucide::SETTINGS),
            &mutations::Chrome::from_theme(theme).fit(height),
        );
        let engine = match self
            .shared
            .settings
            .settings
            .last_compare
            .as_ref()
            .map(|compare| compare.renderer)
        {
            Some(RendererKind::Semantic) => "Semantic",
            _ => "Git diff",
        };
        let indexing = self
            .controllers
            .viewport
            .symbols
            .progress()
            .map(|(done, total)| {
                let percent = done * 100 / total.max(1);
                view! {
                    <text size={small} color={tc.text_muted}>
                        {format!("Indexing symbols {percent}%")}
                    </text>
                }
            });
        view! {
            <div
                w={width}
                h={height}
                class="flex-row items-center shrink-0 overflow-hidden"
                px={tokens::scaled(Sp::MD, scale)}
                gap={tokens::scaled(Sp::SM, scale)}
                bg={tc.status_bar_background}
                border_t={tc.border_variant}
                accessibility_role={AccessibilityRole::Status}
                aria-label="Status"
                test_id="status"
            >
                {settings}
                {?branch}
                <div class="flex-1 flex-row items-center" min_w={0.0}>{compare}</div>
                {?viewing}
                {changes}
                {?indexing}
                <text size={small} color={tc.text_muted}>{engine}</text>
                {delivery}
            </div>
        }
    }
}

pub fn empty_state(title: &str, detail: &str, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    view! {
        <div
            class="w-full h-full flex-col items-center justify-center"
            gap={tokens::scaled(Sp::XS, scale)}
            p={tokens::scaled(Sp::LG, scale)}
        >
            <text size={TextStyle::Body.font_size(theme)} class="font-medium" color={tc.text}>
                {title}
            </text>
            <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>
                {detail}
            </text>
        </div>
    }
}

impl<D: EffectDispatcher> DiffyApp<D> {
    fn hover_event(&mut self, event: &InputEvent, cx: &mut UiContext) {
        let viewport = &mut self.controllers.viewport;
        let (pointer, primary) = match event {
            InputEvent::PointerMoved { x, y } => (Some((*x, *y)), viewport.hover.primary),
            InputEvent::PointerLeft => (None, viewport.hover.primary),
            InputEvent::ModifiersChanged(modifiers) => {
                let primary = if cfg!(target_os = "macos") {
                    modifiers.super_key()
                } else {
                    modifiers.control_key()
                };
                (viewport.hover.pointer, primary)
            }
            InputEvent::Wheel { .. } | InputEvent::KeyPress(_) => {
                if viewport.dismiss_hover() {
                    cx.window.request_redraw();
                }
                return;
            }
            InputEvent::PointerButton {
                state: winit::event::ElementState::Pressed,
                ..
            } => {
                if viewport.hover.dismiss() {
                    cx.window.request_redraw();
                }
                return;
            }
            _ => return,
        };
        let pointer = pointer.filter(|_| !self.controllers.commands.modal_open());
        let linked = self.controllers.viewport.link_active();
        let reduction = self
            .controllers
            .viewport
            .hover(pointer, primary, &self.shared);
        if self.reduce(reduction) {
            cx.window.request_redraw();
        }
        if self.controllers.viewport.link_active() {
            cx.window.set_cursor(CursorIcon::Pointer);
        } else if linked {
            cx.window.set_cursor(CursorIcon::Text);
        }
    }
}

impl<D: EffectDispatcher> UiApp for DiffyApp<D> {
    type Action = Action;
    type Message = AppMessage;

    fn init(&mut self, cx: &mut UiContext) {
        self.tick(cx);
        self.dispatcher.attach(cx.sender::<AppMessage>());
        self.controllers
            .review
            .set_syntax_wake(self.dispatcher.syntax_wake());
        if let Some(wake) = self.dispatcher.syntax_wake() {
            self.controllers
                .viewport
                .files
                .set_syntax_wake(wake.clone());
            self.controllers.viewport.interaction.set_syntax_wake(wake);
        }
        if let Some(startup) = self.startup.take() {
            self.reduce(Reduction::default().intent(Intent::Startup(startup)));
            self.flush(cx);
        }
    }

    fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
        let _work = crate::platform::energy::work(crate::platform::energy::Kind::Render);
        self.clock_ms = cx.frame.elapsed().as_millis() as u64;
        if self.shared.copied.is_some() {
            match self.shell.copied_until.checked_sub(self.clock_ms) {
                Some(left) if left > 0 => cx.frame.request_frame_in(Duration::from_millis(left)),
                _ => self.shared.copied = None,
            }
        }
        let theme = cx.theme;
        let (width, height) = cx.frame.size();
        let title_h = theme.metrics.title_bar_height.min(height);
        let status_h = theme
            .metrics
            .status_bar_height
            .min((height - title_h).max(0.0));
        let pr_h = review::pull_request_bar_height(&self.controllers.review, theme)
            .min((height - title_h - status_h).max(0.0));
        let body_h = (height - title_h - pr_h - status_h).max(0.0);
        let title = self.title_bar(width, title_h, cx);
        let pr_bar = review::pull_request_bar(&self.controllers.review, width, pr_h, cx.theme);
        let body = self.body(width, body_h, cx);
        let status = self.status_bar(width, status_h, cx);
        let slot = Slot {
            shared: &self.shared,
            width,
            height,
        };
        let mutation_panel = mutations::panel(&mut self.controllers.mutations, &slot, cx);
        let review_overlay = review::overlay(&self.controllers.review, &slot, cx);
        let external = commands::settings_slot(&self.controllers.commands, &slot, cx.theme).map(
            |(section, width)| match section {
                SettingsSection::Clankers => mutations::clankers_settings(
                    &mut self.controllers.mutations,
                    &self.shared.settings,
                    width,
                    cx,
                ),
                SettingsSection::Accounts => {
                    review::accounts_settings(&self.controllers.review, cx.theme)
                }
                _ => delivery::updates_settings(
                    &self.controllers.delivery,
                    &self.shared.settings,
                    width,
                    cx,
                ),
            },
        );
        let hover = self.controllers.viewport.hover.card((width, height), cx);
        let overlays = commands::overlays(&mut self.controllers.commands, &slot, external, cx);
        let languages = delivery::overlay(&mut self.controllers.delivery, &slot, cx);
        let toasts = commands::toasts(&mut self.controllers.commands, &slot, cx);
        let leader = commands::leader_menu(&self.controllers.commands, &slot, cx);
        let tooltip = tooltip(cx, width, height);
        let energy = self.shell.debug_overlay.then(|| self.energy_hud.view(cx));
        view! {
            <div w={width} h={height} class="flex-col relative overflow-hidden" bg={theme.colors.app_bg}>
                {title}
                {?pr_bar}
                {body}
                {status}
                {?mutation_panel}
                {?review_overlay}
                {?hover}
                {?overlays}
                {?languages}
                {?toasts}
                {?leader}
                {?tooltip}
                {?energy}
            </div>
        }
    }

    fn update(&mut self, action: Action, cx: &mut UiContext) {
        self.tick(cx);
        if matches!(action, Action::Viewport(ViewportAction::Interaction(_))) {
            let modifiers = cx.window.modifiers();
            self.controllers.viewport.interaction.set_modifiers(
                modifiers.shift_key(),
                modifiers.super_key() || modifiers.control_key(),
            );
        }
        match self.controllers.route_action(action, &self.shared) {
            Routed::Shell(action) => self.shell_action(action, cx),
            Routed::Stream(reduction) => {
                self.reduce(reduction);
            }
        }
        self.flush(cx);
    }

    fn message(&mut self, message: AppMessage, cx: &mut UiContext) {
        self.tick(cx);
        match message {
            AppMessage::Runtime(envelope) => {
                if envelope.workspace != self.shared.workspace
                    && !envelope.event.outlives_workspace()
                {
                    tracing::trace!(
                        event = ?envelope.event,
                        stale = envelope.workspace.0,
                        current = self.shared.workspace.0,
                        "dropped stale runtime event"
                    );
                    return;
                }
                let changed = self.deliver(envelope.event);
                self.flush(cx);
                if changed {
                    cx.window.request_redraw();
                }
            }
            AppMessage::SyntaxWake => {
                self.dispatcher.syntax_wake_handled();
                cx.window.request_redraw();
            }
        }
    }

    fn app_event(&mut self, event: WindowEvent, cx: &mut UiContext) {
        self.tick(cx);
        if let WindowEvent::Menu(id) = &event
            && let Some(action) = commands::menu_action(&self.controllers, id)
        {
            self.update(action, cx);
            cx.window.request_redraw();
            return;
        }
        if let WindowEvent::FileDialogClosed { id, paths } = event {
            let path = paths.into_iter().next();
            let event = if self.shell.repository_dialog == Some(id) {
                self.shell.repository_dialog = None;
                ShellEvent::RepositoryDialogClosed { path }
            } else if self.shell.scan_folder_dialog == Some(id) {
                self.shell.scan_folder_dialog = None;
                ShellEvent::ScanFolderDialogClosed { path }
            } else if let Some((_, folder)) = self
                .shell
                .launcher_dialog
                .take_if(|(dialog, _)| *dialog == id)
            {
                ShellEvent::LauncherDialogClosed { folder, path }
            } else {
                return;
            };
            let changed = self.deliver(event.into());
            self.flush(cx);
            if changed {
                cx.window.request_redraw();
            }
        }
    }

    fn edit_text(&mut self, target: FocusId, command: TextEditCommand) -> TextEditOutcome {
        let cx = TextCx {
            shared: &self.shared,
            now_ms: self.clock_ms,
        };
        let outcome = self
            .controllers
            .edit_text(target, command, &cx)
            .unwrap_or_default();
        let reduction = self.controllers.after_edit(target, &self.shared);
        self.reduce(reduction);
        outcome
    }

    fn set_preedit(&mut self, target: FocusId, text: String, cursor: Option<(usize, usize)>) {
        self.controllers.set_preedit(target, text, cursor);
    }

    fn event(&mut self, event: &InputEvent, cx: &mut UiContext) -> bool {
        self.tick(cx);
        if matches!(event, InputEvent::KeyPress(_)) {
            self.ensure_focus(cx);
        }
        self.hover_event(event, cx);
        let Some(action) = commands::key_action(&self.controllers, event, cx.focus()) else {
            return false;
        };
        self.update(action, cx);
        cx.window.request_redraw();
        true
    }

    fn set_text_value(&mut self, target: FocusId, value: String, cx: &mut UiContext) {
        self.tick(cx);
        let text_cx = TextCx {
            shared: &self.shared,
            now_ms: self.clock_ms,
        };
        self.controllers.set_text_value(target, value, &text_cx);
        let reduction = self.controllers.after_edit(target, &self.shared);
        if self.reduce(reduction) {
            cx.window.request_redraw();
        }
        self.flush(cx);
    }
}

fn branch_status(
    shared: &SharedContext,
    sync: Option<AnyElement>,
    theme: &Theme,
) -> Option<AnyElement> {
    let repository = shared.repository.as_ref()?;
    let active = repository.snapshot.refs.iter().find(|reference| {
        reference.active && matches!(reference.kind, RefKind::Branch | RefKind::Bookmark)
    })?;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    Some(view! {
        <div
            class="flex-row items-center shrink-0"
            gap={tokens::scaled(Sp::XS, scale)}
            aria-label={format!("Branch {}", active.name)}
            test_id="status.branch"
        >
            <div
                w={tokens::scaled(Layout::STATUS_DOT, scale)}
                h={tokens::scaled(Layout::STATUS_DOT, scale)}
                rounded={tokens::scaled(Layout::STATUS_DOT, scale)}
                bg={tc.line_add_text}
            />
            <text size={small} color={tc.text}>{active.name.clone()}</text>
            {?sync}
        </div>
    })
}

fn tooltip(cx: &mut ViewContext, width: f32, height: f32) -> Option<AnyElement> {
    let hovered = cx.hovered_tooltip()?.clone();
    let now = cx.frame.elapsed();
    let due = hovered.since + TOOLTIP_DELAY;
    if now < due {
        cx.frame.request_frame_in(due - now);
        return None;
    }
    let theme = cx.theme;
    let metrics = &theme.metrics;
    let size = theme
        .components
        .tooltip
        .font_size
        .map_or(metrics.ui_small_font_size, |s| s * metrics.ui_scale());
    let box_h = size * 2.4;
    let box_w = hovered.text.chars().count() as f32 * size * 0.6 + size * 2.0;
    let x = hovered.pointer.0.min(width - box_w).max(0.0);
    let below = hovered.bounds.y + hovered.bounds.height;
    let (y, side) = if below + box_h > height {
        (hovered.bounds.y - box_h, TooltipSide::Top)
    } else {
        (below, TooltipSide::Bottom)
    };
    Some(tooltip_layer(&hovered.text, x, y, side, theme))
}
