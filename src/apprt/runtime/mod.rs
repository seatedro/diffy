pub mod commands;
pub mod delivery;
pub mod mutations;
pub mod navigation;
pub mod review;
pub mod viewport;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::thread;

use crate::apprt::services::{AppServices, ShellServices};
use crate::apprt::transport::{RuntimeEventSender, ScopedEventSender};
use crate::apprt::vcs_worker::VcsWorker;
use crate::apprt::watcher::RepoWatchWorker;
use crate::contracts::shared::WorkspaceEpoch;
use crate::contracts::shell::{Effect, ShellEffect, ShellEvent};
use crate::platform::startup::GitHubTokenStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RuntimeOptions {
    pub keyring_enabled: bool,
    pub github_token_store: GitHubTokenStore,
}

pub struct AppRuntime {
    events: RuntimeEventSender,
    shell: ShellServices,
    viewport: viewport::ViewportRuntime,
    navigation: navigation::NavigationRuntime,
    commands: commands::CommandsRuntime,
    review: review::ReviewRuntime,
    mutations: mutations::MutationsRuntime,
    delivery: delivery::DeliveryRuntime,
}

impl AppRuntime {
    pub fn new(services: AppServices, events: RuntimeEventSender, options: RuntimeOptions) -> Self {
        let vcs = VcsWorker::new(Arc::new(mutations::VcsEventRouter::new(events.clone())));
        let watcher = RepoWatchWorker::new(vcs.sender());
        Self {
            shell: services.shell,
            viewport: viewport::ViewportRuntime::new(services.viewport),
            navigation: navigation::NavigationRuntime::new(
                services.navigation,
                vcs.clone(),
                watcher,
            ),
            commands: commands::CommandsRuntime::new(services.commands),
            review: review::ReviewRuntime::new(services.review, options.github_token_store),
            mutations: mutations::MutationsRuntime::new(
                services.mutations,
                vcs,
                options.keyring_enabled,
            ),
            delivery: delivery::DeliveryRuntime::new(services.delivery),
            events,
        }
    }

    pub fn dispatch(&self, effect: Effect, workspace: WorkspaceEpoch) {
        let events = self.events.scoped(workspace);
        match effect {
            Effect::Shell(effect) => self.dispatch_shell(effect, events),
            Effect::Viewport(effect) => self.viewport.dispatch(effect, events),
            Effect::Navigation(effect) => self.navigation.dispatch(effect, events),
            Effect::Commands(effect) => self.commands.dispatch(effect, events),
            Effect::Review(effect) => self.review.dispatch(effect, events),
            Effect::Mutations(effect) => self.mutations.dispatch(effect, events),
            Effect::Delivery(effect) => self.delivery.dispatch(effect, events),
        }
    }

    pub fn dispatch_all(
        &self,
        effects: impl IntoIterator<Item = Effect>,
        workspace: WorkspaceEpoch,
    ) {
        for effect in effects {
            self.dispatch(effect, workspace);
        }
    }

    fn dispatch_shell(&self, effect: ShellEffect, events: ScopedEventSender) {
        let shell = self.shell;
        match effect {
            ShellEffect::OpenRepositoryDialog => {
                thread::spawn(move || {
                    events.send(ShellEvent::RepositoryDialogClosed {
                        path: shell.open_repository_dialog(),
                    });
                });
            }
            ShellEffect::OpenBrowser { url } => {
                thread::spawn(move || {
                    if let Err(error) = shell.open_browser(&url) {
                        events.send(ShellEvent::BrowserOpenFailed {
                            message: error.user_message(),
                        });
                    }
                });
            }
            ShellEffect::SetClipboard(text) => {
                thread::spawn(move || shell.set_clipboard(text));
            }
            ShellEffect::Focus(_) => {}
        }
    }
}
