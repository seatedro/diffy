use crate::contracts::commands::{
    CommandsAction, CommandsEffect, CommandsEvent, ConfirmationRequest, SettingsSnapshot,
    ToastUpdate,
};
use crate::contracts::delivery::{
    DeliveryAction, DeliveryEffect, DeliveryEvent, GrammarAvailability,
};
use crate::contracts::mutations::{MutationsAction, MutationsEffect, MutationsEvent};
use crate::contracts::navigation::{
    NavigationAction, NavigationEffect, NavigationEvent, RepositoryContext,
};
use crate::contracts::review::{ReviewAction, ReviewContext, ReviewEffect, ReviewEvent};
use crate::contracts::shared::{SharedContext, SourceTarget, WorkspaceEpoch};
use crate::contracts::viewport::{
    CompareIntent, CompareManifestSnapshot, ViewportAction, ViewportEffect, ViewportEvent,
};
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub enum ShellAction {
    Bootstrap,
    ResizeSidebar(quark_components::SplitEvent),
    FocusLogical(Option<crate::contracts::shell::LogicalFocus>),
    CopyText(String),
    ToggleDebugOverlay,
    Minimize,
    ToggleMaximize,
    Close,
    Noop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogicalFocus {
    Viewport,
    Sidebar,
    SidebarFilter,
    Search,
    Palette,
    Picker,
    Settings,
    CommitMessage,
    ReviewComposer,
    TextCompareLeft,
    TextCompareRight,
    SettingsOpenAiKey,
    SettingsAnthropicKey,
    SettingsSteeringPrompt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEffect {
    OpenRepositoryDialog,
    OpenBrowser { url: String },
    SetClipboard(String),
    Focus(Option<LogicalFocus>),
}

#[derive(Debug, Clone)]
pub enum ShellEvent {
    RepositoryDialogClosed { path: Option<PathBuf> },
    BrowserOpenFailed { message: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Shell(ShellAction),
    Viewport(ViewportAction),
    Navigation(NavigationAction),
    Commands(CommandsAction),
    Review(ReviewAction),
    Mutations(MutationsAction),
    Delivery(DeliveryAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Shell(ShellEffect),
    Viewport(ViewportEffect),
    Navigation(NavigationEffect),
    Commands(CommandsEffect),
    Review(ReviewEffect),
    Mutations(MutationsEffect),
    Delivery(DeliveryEffect),
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    Shell(ShellEvent),
    Viewport(ViewportEvent),
    Navigation(NavigationEvent),
    Commands(CommandsEvent),
    Review(ReviewEvent),
    Mutations(MutationsEvent),
    Delivery(DeliveryEvent),
}

#[derive(Debug, Clone)]
pub struct EventEnvelope {
    pub workspace: WorkspaceEpoch,
    pub event: AppEvent,
}

#[derive(Debug)]
pub enum AppMessage {
    Runtime(EventEnvelope),
    SyntaxWake,
}

#[derive(Debug)]
pub struct AppStartup {
    pub options: crate::platform::startup::StartupOptions,
    pub settings: crate::platform::persistence::Settings,
}

#[derive(Debug, Clone)]
pub enum Intent {
    Startup(Rc<AppStartup>),
    SelectFile(Option<crate::contracts::navigation::SelectedFile>),
    ActivateWorkspace { path: PathBuf },
    RequestCompare(CompareIntent),
    PublishManifest(Rc<CompareManifestSnapshot>),
    SelectSource(Option<SourceTarget>),
    RepositoryChanged(Rc<RepositoryContext>),
    InvalidateRepository,
    SetReviewTarget(Option<Rc<ReviewContext>>),
    AccountChanged,
    GrammarAvailable(GrammarAvailability),
    SettingsChanged(Rc<SettingsSnapshot>),
    RequestConfirmation(ConfirmationRequest),
    Toast(ToastUpdate),
}

#[derive(Debug, Default)]
pub struct Reduction {
    pub effects: Vec<Effect>,
    pub intents: Vec<Intent>,
    pub changed: bool,
}

impl Reduction {
    pub fn changed() -> Self {
        Self {
            changed: true,
            ..Self::default()
        }
    }

    pub fn effect(mut self, effect: impl Into<Effect>) -> Self {
        self.effects.push(effect.into());
        self
    }

    pub fn intent(mut self, intent: Intent) -> Self {
        self.intents.push(intent);
        self
    }

    pub fn merge(&mut self, other: Reduction) {
        self.effects.extend(other.effects);
        self.intents.extend(other.intents);
        self.changed |= other.changed;
    }
}

pub trait Controller {
    type Action;
    type Event;

    fn reduce_action(&mut self, action: Self::Action, cx: &SharedContext) -> Reduction;

    fn reduce_event(&mut self, event: Self::Event, cx: &SharedContext) -> Reduction;

    fn apply_intent(&mut self, _intent: &Intent, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }
}

macro_rules! route {
    ($root:ident, $($variant:ident($ty:ty)),* $(,)?) => {
        $(impl From<$ty> for $root {
            fn from(value: $ty) -> Self {
                Self::$variant(value)
            }
        })*
    };
}

route!(
    Action,
    Shell(ShellAction),
    Viewport(ViewportAction),
    Navigation(NavigationAction),
    Commands(CommandsAction),
    Review(ReviewAction),
    Mutations(MutationsAction),
    Delivery(DeliveryAction)
);
route!(
    Effect,
    Shell(ShellEffect),
    Viewport(ViewportEffect),
    Navigation(NavigationEffect),
    Commands(CommandsEffect),
    Review(ReviewEffect),
    Mutations(MutationsEffect),
    Delivery(DeliveryEffect)
);
route!(
    AppEvent,
    Shell(ShellEvent),
    Viewport(ViewportEvent),
    Navigation(NavigationEvent),
    Commands(CommandsEvent),
    Review(ReviewEvent),
    Mutations(MutationsEvent),
    Delivery(DeliveryEvent)
);
