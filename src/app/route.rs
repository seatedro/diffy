use std::rc::Rc;

use quark_components::SplitEvent;

use crate::app::commands::CommandsController;
use crate::app::delivery::DeliveryController;
use crate::app::mutations::MutationsController;
use crate::app::navigation::NavigationController;
use crate::app::review::ReviewController;
use crate::app::viewport::ViewportController;
use crate::contracts::commands::{CommandDescriptor, CommandSource, CommandsAction};
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{
    Action, AppEvent, Controller, Intent, Reduction, ShellAction, ShellEvent,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stream {
    Viewport,
    Navigation,
    Commands,
    Review,
    Mutations,
    Delivery,
}

pub const INTENT_ORDER: [Stream; 6] = [
    Stream::Navigation,
    Stream::Viewport,
    Stream::Mutations,
    Stream::Review,
    Stream::Delivery,
    Stream::Commands,
];

pub const MAX_INTENTS_PER_TURN: usize = 256;

pub enum Routed<S, R> {
    Shell(S),
    Stream(R),
}

#[derive(Debug, Default)]
pub struct Controllers {
    pub viewport: ViewportController,
    pub navigation: NavigationController,
    pub commands: CommandsController,
    pub review: ReviewController,
    pub mutations: MutationsController,
    pub delivery: DeliveryController,
}

impl Controllers {
    pub fn route_action(
        &mut self,
        action: Action,
        cx: &SharedContext,
    ) -> Routed<ShellAction, Reduction> {
        if matches!(action, Action::Commands(CommandsAction::OpenCommandPalette)) {
            let commands = self.external_commands(cx);
            let mut reduction = self
                .commands
                .reduce_action(CommandsAction::SetExternalCommands(commands), cx);
            reduction.merge(
                self.commands
                    .reduce_action(CommandsAction::OpenCommandPalette, cx),
            );
            return Routed::Stream(reduction);
        }
        Routed::Stream(match action {
            Action::Shell(action) => return Routed::Shell(action),
            Action::Viewport(action) => self.viewport.reduce_action(action, cx),
            Action::Navigation(action) => self.navigation.reduce_action(action, cx),
            Action::Commands(action) => self.commands.reduce_action(action, cx),
            Action::Review(action) => self.review.reduce_action(action, cx),
            Action::Mutations(action) => self.mutations.reduce_action(action, cx),
            Action::Delivery(action) => self.delivery.reduce_action(action, cx),
        })
    }

    fn external_commands(&self, cx: &SharedContext) -> Rc<[CommandDescriptor]> {
        let mut out = Vec::new();
        self.review.commands(cx, &mut out);
        out.into()
    }

    pub fn route_event(
        &mut self,
        event: AppEvent,
        cx: &SharedContext,
    ) -> Routed<ShellEvent, Reduction> {
        Routed::Stream(match event {
            AppEvent::Shell(event) => return Routed::Shell(event),
            AppEvent::Viewport(event) => self.viewport.reduce_event(event, cx),
            AppEvent::Navigation(event) => self.navigation.reduce_event(event, cx),
            AppEvent::Commands(event) => self.commands.reduce_event(event, cx),
            AppEvent::Review(event) => self.review.reduce_event(event, cx),
            AppEvent::Mutations(event) => self.mutations.reduce_event(event, cx),
            AppEvent::Delivery(event) => self.delivery.reduce_event(event, cx),
        })
    }

    pub fn apply_intent(&mut self, intent: &Intent, cx: &SharedContext) -> Reduction {
        let mut reduction = Reduction::default();
        for stream in INTENT_ORDER {
            reduction.merge(match stream {
                Stream::Viewport => self.viewport.apply_intent(intent, cx),
                Stream::Navigation => self.navigation.apply_intent(intent, cx),
                Stream::Commands => self.commands.apply_intent(intent, cx),
                Stream::Review => self.review.apply_intent(intent, cx),
                Stream::Mutations => self.mutations.apply_intent(intent, cx),
                Stream::Delivery => self.delivery.apply_intent(intent, cx),
            });
        }
        reduction
    }
}

pub fn sidebar_split(event: SplitEvent) -> quark_ui::Action {
    Action::Shell(ShellAction::ResizeSidebar(event)).into()
}

impl From<Action> for quark_ui::Action {
    fn from(action: Action) -> Self {
        quark_ui::Action::new(action)
    }
}

macro_rules! ui_action {
    ($($leaf:ty),* $(,)?) => {
        $(impl From<$leaf> for quark_ui::Action {
            fn from(action: $leaf) -> Self {
                quark_ui::Action::new(Action::from(action))
            }
        })*
    };
}

ui_action!(
    ShellAction,
    crate::contracts::viewport::ViewportAction,
    crate::contracts::navigation::NavigationAction,
    crate::contracts::commands::CommandsAction,
    crate::contracts::review::ReviewAction,
    crate::contracts::mutations::MutationsAction,
    crate::contracts::delivery::DeliveryAction,
);
