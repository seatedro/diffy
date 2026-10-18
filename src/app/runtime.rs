use std::sync::Arc;

use quark_app::UiSender;

use crate::app::shell::EffectDispatcher;
use crate::apprt::runtime::{AppRuntime, RuntimeOptions};
use crate::apprt::services::AppServices;
use crate::apprt::transport::{RuntimeEventSender, SyntaxWaker};
use crate::contracts::shared::WorkspaceEpoch;
use crate::contracts::shell::{AppMessage, Effect};

pub struct RuntimeDispatcher {
    services: Option<AppServices>,
    options: RuntimeOptions,
    runtime: Option<AppRuntime>,
    syntax: Option<SyntaxWaker>,
    pending: Vec<(Effect, WorkspaceEpoch)>,
}

impl RuntimeDispatcher {
    pub fn new(services: AppServices, options: RuntimeOptions) -> Self {
        Self {
            services: Some(services),
            options,
            runtime: None,
            syntax: None,
            pending: Vec::new(),
        }
    }

    pub fn syntax_waker(&self) -> Option<SyntaxWaker> {
        self.syntax.clone()
    }
}

impl EffectDispatcher for RuntimeDispatcher {
    fn attach(&mut self, sender: UiSender<AppMessage>) {
        let Some(services) = self.services.take() else {
            return;
        };
        let events = RuntimeEventSender::from_ui(sender);
        self.syntax = Some(events.syntax_waker());
        let runtime = AppRuntime::new(services, events, self.options);
        for (effect, workspace) in self.pending.drain(..) {
            runtime.dispatch(effect, workspace);
        }
        self.runtime = Some(runtime);
    }

    fn dispatch(&mut self, effect: Effect, workspace: WorkspaceEpoch) {
        match &self.runtime {
            Some(runtime) => runtime.dispatch(effect, workspace),
            None => self.pending.push((effect, workspace)),
        }
    }

    fn syntax_wake(&self) -> Option<Arc<dyn Fn() + Send + Sync>> {
        self.syntax.clone().map(|waker| {
            Arc::new(move || {
                waker.wake();
            }) as Arc<dyn Fn() + Send + Sync>
        })
    }

    fn syntax_wake_handled(&mut self) {
        if let Some(syntax) = &self.syntax {
            syntax.acknowledge();
        }
    }
}
