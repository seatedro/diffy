use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use quark::view;
use quark_app::platform::dialog::{DialogId, FileDialog, FileDialogKind};
use quark_app::{AppEvent as WindowEvent, InputEvent, UiApp, UiContext, UiSender, ViewContext};
use quark_components::{Axis, Pane, Split, SplitState};
use quark_ui::FocusId;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::style::Styled;
use quark_ui::text_input::{TextEditCommand, TextEditOutcome};
use quark_ui::theme::Theme;

use crate::app::diff_slots::DiffSlots;
use crate::app::route::{self, Controllers, MAX_INTENTS_PER_TURN, Routed};
use crate::app::text_targets::{self, TextCx};
use crate::app::tokens::{self, Sp, TextStyle};
use crate::app::{commands, delivery, mutations, navigation, review, viewport};
use crate::contracts::commands::ToastUpdate;
use crate::contracts::shared::{AccountEpoch, SharedContext, WorkspaceEpoch};
use crate::contracts::shell::{
    Action, AppMessage, AppStartup, Effect, Intent, Reduction, ShellAction, ShellEffect, ShellEvent,
};

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

#[derive(Debug)]
struct ShellState {
    display_name: String,
    workspace_label: Option<String>,
    window_title_dirty: bool,
    debug_overlay: bool,
    sidebar: SplitState,
    repository_dialog: Option<DialogId>,
}

impl ShellState {
    fn new(display_name: String) -> Self {
        let theme = Theme::default_dark();
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
                self.shell.set_workspace(path);
                true
            }
            Intent::PublishManifest(manifest) => {
                shared.manifest = Some(manifest.clone());
                true
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
                shared.settings = settings.clone();
                true
            }
            Intent::Startup(_)
            | Intent::OpenPullRequest { .. }
            | Intent::RequestAnnotation(_)
            | Intent::OpenTextCompare
            | Intent::RequestCompare(_)
            | Intent::InvalidateRepository
            | Intent::GrammarAvailable(_)
            | Intent::RequestConfirmation(_)
            | Intent::Toast(_) => false,
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
                ShellEffect::SetClipboard(text) => cx.window.set_clipboard_text(&text),
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
    }

    fn shell_action(&mut self, action: ShellAction, cx: &mut UiContext) {
        match action {
            ShellAction::FocusLogical(focus) => cx.set_focus(focus.map(text_targets::focus_id)),
            ShellAction::CopyText(text) => cx.window.set_clipboard_text(&text),
            ShellAction::ToggleDebugOverlay => self.shell.debug_overlay = !self.shell.debug_overlay,
            ShellAction::Minimize => cx.window.minimize(),
            ShellAction::ToggleMaximize => cx.window.toggle_maximized(),
            ShellAction::Close => {
                if let Some(window) = cx.window.window_handle() {
                    cx.window.close_window(window);
                }
            }
            ShellAction::ResizeSidebar(event) => {
                self.shell.sidebar.apply(event, self.clock_ms);
                cx.window.request_redraw();
            }
            ShellAction::Bootstrap | ShellAction::Noop => {}
        }
    }

    fn shell_event(&mut self, event: ShellEvent) -> Reduction {
        match event {
            ShellEvent::RepositoryDialogClosed { path: Some(path) } => {
                Reduction::default().intent(Intent::ActivateWorkspace { path })
            }
            ShellEvent::RepositoryDialogClosed { path: None } => Reduction::default(),
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
                <div class="flex-1 flex-row items-center" min_w={0.0}>
                    <text
                        size={TextStyle::Body.font_size(theme)}
                        class="truncate font-semibold"
                        color={tc.text_strong}
                    >
                        {label}
                    </text>
                </div>
                {review}
            </div>
        }
    }

    fn body(&mut self, width: f32, height: f32, cx: &mut ViewContext) -> AnyElement {
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
            navigation::view(&mut self.controllers.navigation, &slot, cx)
        });
        let decorator = self.slots.decorator(
            self.controllers.review.presentation(),
            self.controllers.mutations.presentation(),
            tokens::ui_scale(theme),
        );
        let slot = Slot {
            shared: &self.shared,
            width: main_w,
            height,
        };
        let main = viewport::interaction::view_shared(
            &mut self.controllers.viewport.interaction,
            &slot,
            decorator,
            cx,
        );
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
                <div class="flex-1 flex-row items-center" min_w={0.0}>{compare}</div>
                {changes}
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
            self.controllers.viewport.interaction.set_syntax_wake(wake);
        }
        if let Some(startup) = self.startup.take() {
            self.reduce(Reduction::default().intent(Intent::Startup(startup)));
            self.flush(cx);
        }
    }

    fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
        self.clock_ms = cx.frame.elapsed().as_millis() as u64;
        let theme = cx.theme;
        let (width, height) = cx.frame.size();
        let title_h = theme.metrics.title_bar_height.min(height);
        let status_h = theme
            .metrics
            .status_bar_height
            .min((height - title_h).max(0.0));
        let body_h = (height - title_h - status_h).max(0.0);
        let title = self.title_bar(width, title_h, cx);
        let body = self.body(width, body_h, cx);
        let status = self.status_bar(width, status_h, cx);
        let slot = Slot {
            shared: &self.shared,
            width,
            height,
        };
        let mutation_panel = mutations::panel(&mut self.controllers.mutations, &slot, cx);
        let review_overlay = review::overlay(&self.controllers.review, &slot, cx);
        let overlays = commands::overlays(&mut self.controllers.commands, &slot, None, cx);
        let languages = delivery::overlay(&mut self.controllers.delivery, &slot, cx);
        let toasts = commands::toasts(&mut self.controllers.commands, &slot, cx);
        view! {
            <div w={width} h={height} class="flex-col relative overflow-hidden" bg={theme.colors.app_bg}>
                {title}
                {body}
                {status}
                {?mutation_panel}
                {?review_overlay}
                {?overlays}
                {?languages}
                {?toasts}
            </div>
        }
    }

    fn update(&mut self, action: Action, cx: &mut UiContext) {
        self.tick(cx);
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
                if envelope.workspace != self.shared.workspace {
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
        if let WindowEvent::FileDialogClosed { id, paths } = event
            && self.shell.repository_dialog == Some(id)
        {
            self.shell.repository_dialog = None;
            let path = paths.into_iter().next();
            let changed = self.deliver(ShellEvent::RepositoryDialogClosed { path }.into());
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
        let Some(action) = commands::key_action(&self.controllers, event) else {
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
