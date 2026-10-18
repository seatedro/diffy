use std::collections::HashMap;
use std::mem::Discriminant;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use quark_app::UiSender;

use crate::contracts::shared::{JobId, WorkspaceEpoch};
use crate::contracts::shell::{AppEvent, AppMessage, EventEnvelope};
use crate::core::compare::ComparePhase;

pub trait MessageSink: Send + Sync + 'static {
    fn deliver(&self, message: AppMessage) -> bool;
}

impl MessageSink for UiSender<AppMessage> {
    fn deliver(&self, message: AppMessage) -> bool {
        self.send(message)
    }
}

impl MessageSink for Sender<AppMessage> {
    fn deliver(&self, message: AppMessage) -> bool {
        self.send(message).is_ok()
    }
}

#[derive(Clone)]
pub struct RuntimeEventSender {
    sink: Arc<dyn MessageSink>,
}

impl RuntimeEventSender {
    pub fn new(sink: impl MessageSink) -> Self {
        Self {
            sink: Arc::new(sink),
        }
    }

    pub fn from_ui(sender: UiSender<AppMessage>) -> Self {
        Self::new(sender)
    }

    pub fn from_channel(sender: Sender<AppMessage>) -> Self {
        Self::new(sender)
    }

    pub fn send(&self, workspace: WorkspaceEpoch, event: impl Into<AppEvent>) -> bool {
        self.sink.deliver(AppMessage::Runtime(EventEnvelope {
            workspace,
            event: event.into(),
        }))
    }

    pub fn syntax_waker(&self) -> SyntaxWaker {
        SyntaxWaker {
            sink: Arc::clone(&self.sink),
            pending: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn scoped(&self, workspace: WorkspaceEpoch) -> ScopedEventSender {
        ScopedEventSender {
            sender: self.clone(),
            workspace,
        }
    }
}

#[derive(Clone)]
pub struct SyntaxWaker {
    sink: Arc<dyn MessageSink>,
    pending: Arc<AtomicBool>,
}

impl SyntaxWaker {
    pub fn wake(&self) -> bool {
        if self.pending.swap(true, Ordering::AcqRel) {
            return true;
        }
        let delivered = self.sink.deliver(AppMessage::SyntaxWake);
        if !delivered {
            self.pending.store(false, Ordering::Release);
        }
        delivered
    }

    pub fn acknowledge(&self) {
        self.pending.store(false, Ordering::Release);
    }
}

#[derive(Clone)]
pub struct ScopedEventSender {
    sender: RuntimeEventSender,
    workspace: WorkspaceEpoch,
}

impl ScopedEventSender {
    pub fn workspace(&self) -> WorkspaceEpoch {
        self.workspace
    }

    pub fn send(&self, event: impl Into<AppEvent>) -> bool {
        self.sender.send(self.workspace, event)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProgressKey {
    Compare {
        generation: u64,
        phase: Discriminant<ComparePhase>,
    },
    Fetch(JobId),
    Push(JobId),
}

impl ProgressKey {
    pub fn compare(generation: u64, phase: &ComparePhase) -> Self {
        Self::Compare {
            generation,
            phase: std::mem::discriminant(phase),
        }
    }
}

pub struct ProgressGate {
    interval: Duration,
    last: Mutex<HashMap<ProgressKey, Instant>>,
}

impl ProgressGate {
    pub const DEFAULT_INTERVAL: Duration = Duration::from_millis(33);
    const MAX_KEYS: usize = 64;

    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: Mutex::new(HashMap::new()),
        }
    }

    pub fn admit(&self, key: ProgressKey, now: Instant) -> bool {
        let Ok(mut last) = self.last.lock() else {
            return true;
        };
        if let Some(previous) = last.get(&key)
            && now.saturating_duration_since(*previous) < self.interval
        {
            return false;
        }
        if last.len() >= Self::MAX_KEYS {
            last.clear();
        }
        last.insert(key, now);
        true
    }

    pub fn finish(&self, key: ProgressKey) {
        if let Ok(mut last) = self.last.lock() {
            last.remove(&key);
        }
    }
}

impl Default for ProgressGate {
    fn default() -> Self {
        Self::new(Self::DEFAULT_INTERVAL)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::contracts::commands::CommandsEvent;

    #[test]
    fn channel_sink_wraps_events_with_the_captured_workspace() {
        let (tx, rx) = mpsc::channel();
        let sender = RuntimeEventSender::from_channel(tx);
        let scoped = sender.scoped(WorkspaceEpoch(7));

        assert!(scoped.send(CommandsEvent::SettingsSaved));

        match rx.try_recv() {
            Ok(AppMessage::Runtime(envelope)) => {
                assert_eq!(envelope.workspace, WorkspaceEpoch(7));
                assert!(matches!(
                    envelope.event,
                    AppEvent::Commands(CommandsEvent::SettingsSaved)
                ));
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn syntax_wakes_coalesce_until_acknowledged() {
        let (tx, rx) = mpsc::channel();
        let waker = RuntimeEventSender::from_channel(tx).syntax_waker();

        assert!(waker.wake());
        assert!(waker.wake());
        assert!(matches!(rx.try_recv(), Ok(AppMessage::SyntaxWake)));
        assert!(rx.try_recv().is_err());

        waker.acknowledge();
        assert!(waker.wake());
        assert!(matches!(rx.try_recv(), Ok(AppMessage::SyntaxWake)));
    }

    #[test]
    fn channel_sink_reports_a_closed_app() {
        let (tx, rx) = mpsc::channel();
        drop(rx);
        let sender = RuntimeEventSender::from_channel(tx);
        assert!(!sender.send(WorkspaceEpoch(1), CommandsEvent::SettingsSaved));
    }

    #[test]
    fn progress_gate_coalesces_same_key_and_passes_new_phases() {
        let gate = ProgressGate::new(Duration::from_millis(50));
        let start = Instant::now();
        let loading = ComparePhase::LoadingFiles {
            files_seen: 1,
            files_total: 10,
        };
        let key = ProgressKey::compare(3, &loading);

        assert!(gate.admit(key, start));
        assert!(!gate.admit(key, start + Duration::from_millis(10)));
        assert!(gate.admit(
            ProgressKey::compare(3, &ComparePhase::PopulatingList),
            start + Duration::from_millis(10)
        ));
        assert!(gate.admit(key, start + Duration::from_millis(60)));

        gate.finish(key);
        assert!(gate.admit(key, start + Duration::from_millis(61)));
    }

    #[test]
    fn runtime_messages_are_sendable() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<AppMessage>();
        assert_send::<RuntimeEventSender>();
        assert_sync::<RuntimeEventSender>();
    }
}
