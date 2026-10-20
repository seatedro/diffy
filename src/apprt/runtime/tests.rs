use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use super::{AppRuntime, RuntimeOptions};
use crate::apprt::runtime::commands::SETTINGS_SAVE_DEBOUNCE;
use crate::apprt::services::{AppServices, GitHubBackend};
use crate::apprt::transport::RuntimeEventSender;
use crate::contracts::commands::{CommandsEffect, CommandsEvent};
use crate::contracts::review::{
    GitHubRequest, GitHubResponse, PullRequestRef, ReviewEffect, ReviewEvent, ReviewScope,
};
use crate::contracts::shared::{AccountEpoch, CompareGeneration, JobId, WorkspaceEpoch};
use crate::contracts::shell::{AppEvent, AppMessage, EventEnvelope};
use crate::contracts::viewport::ViewportEvent;
use crate::contracts::viewport::pipeline::{PipelineEffect, PipelineEvent, TotalStatsRequest};
use crate::core::compare::{LayoutMode, RendererKind};
use crate::core::forge::github::PullRequestInfo;
use crate::core::vcs::model::{VcsCompareRequest, VcsCompareSpec};
use crate::platform::persistence::{Settings, SettingsStore};
use crate::platform::review_store::ReviewStore;

struct FakeGitHub;

impl GitHubBackend for FakeGitHub {
    fn execute(&self, request: GitHubRequest, emit: &mut dyn FnMut(GitHubResponse)) {
        match request {
            GitHubRequest::PeekPullRequest { pr, .. } => {
                let info = PullRequestInfo {
                    number: pr.number,
                    title: "Example".to_owned(),
                    ..PullRequestInfo::default()
                };
                emit(GitHubResponse::PullRequestPeeked { pr, info });
            }
            _ => emit(GitHubResponse::UserFetchFailed {
                message: "unexpected request".to_owned(),
            }),
        }
    }
}

struct Harness {
    runtime: AppRuntime,
    messages: Receiver<AppMessage>,
    store: SettingsStore,
    _dir: TempDir,
}

fn harness() -> Harness {
    let dir = TempDir::new().unwrap();
    let store = SettingsStore::new_in(dir.path());
    let review_store = ReviewStore::for_settings_store(&store);
    let services = AppServices::with_github(store.clone(), review_store, Arc::new(FakeGitHub));
    let (tx, messages) = mpsc::channel();
    let runtime = AppRuntime::new(
        services,
        RuntimeEventSender::from_channel(tx),
        RuntimeOptions::default(),
    );
    Harness {
        runtime,
        messages,
        store,
        _dir: dir,
    }
}

fn next_envelope(messages: &Receiver<AppMessage>) -> EventEnvelope {
    match messages.recv_timeout(Duration::from_secs(5)) {
        Ok(AppMessage::Runtime(envelope)) => envelope,
        Ok(AppMessage::SyntaxWake) => panic!("unexpected syntax wake"),
        Err(error) => panic!("no runtime event: {error}"),
    }
}

fn wait_for(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for condition");
}

#[test]
fn settings_saves_are_debounced_to_the_latest_snapshot() {
    let harness = harness();
    for name in ["One", "Two", "Three"] {
        let settings = Settings {
            theme_name: name.to_owned(),
            ..Settings::default()
        };
        harness.runtime.dispatch(
            CommandsEffect::SaveSettings(Box::new(settings)).into(),
            WorkspaceEpoch(4),
        );
    }

    wait_for(|| {
        harness
            .store
            .load()
            .is_ok_and(|saved| saved.theme_name == "Three")
    });
    thread::sleep(SETTINGS_SAVE_DEBOUNCE);

    let envelopes = harness.messages.try_iter().collect::<Vec<_>>();
    assert_eq!(envelopes.len(), 1);
    match &envelopes[0] {
        AppMessage::Runtime(envelope) => {
            assert_eq!(envelope.workspace, WorkspaceEpoch(4));
            assert!(matches!(
                envelope.event,
                AppEvent::Commands(CommandsEvent::SettingsSaved)
            ));
        }
        AppMessage::SyntaxWake => panic!("unexpected syntax wake"),
    }
}

#[test]
fn github_effects_report_through_the_backend_with_their_scope() {
    let harness = harness();
    let scope = ReviewScope {
        account: AccountEpoch(2),
        request: JobId(9),
    };
    let pr = PullRequestRef {
        owner: "octo".to_owned(),
        repo: "repo".to_owned(),
        number: 12,
    };
    harness.runtime.dispatch(
        ReviewEffect::GitHub {
            scope,
            request: Box::new(GitHubRequest::PeekPullRequest {
                pr: pr.clone(),
                token: None,
            }),
        }
        .into(),
        WorkspaceEpoch(3),
    );

    let envelope = next_envelope(&harness.messages);
    assert_eq!(envelope.workspace, WorkspaceEpoch(3));
    let AppEvent::Review(ReviewEvent::GitHub {
        scope: reported,
        response,
    }) = envelope.event
    else {
        panic!("unexpected event: {:?}", envelope.event);
    };
    assert_eq!(reported, scope);
    match *response {
        GitHubResponse::PullRequestPeeked { pr: peeked, info } => {
            assert_eq!(peeked, pr);
            assert_eq!(info.number, 12);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[test]
fn superseded_compare_generation_results_are_never_sent() {
    let harness = harness();
    let stats = |generation| {
        PipelineEffect::LoadTotalStats(Box::new(TotalStatsRequest {
            generation: CompareGeneration(generation),
            repo_path: PathBuf::from("/nonexistent/diffy-runtime-test"),
            request: VcsCompareRequest {
                spec: VcsCompareSpec::WorkingCopy,
                layout: LayoutMode::Unified,
                renderer: RendererKind::Builtin,
            },
        }))
    };

    harness.runtime.dispatch(
        PipelineEffect::SupersedeCompare(CompareGeneration(5)).into(),
        WorkspaceEpoch(1),
    );
    harness.runtime.dispatch(stats(4).into(), WorkspaceEpoch(1));
    harness.runtime.dispatch(stats(5).into(), WorkspaceEpoch(1));

    let envelope = next_envelope(&harness.messages);
    match envelope.event {
        AppEvent::Viewport(ViewportEvent::Pipeline(PipelineEvent::TotalStatsFailed {
            generation,
            ..
        })) => assert_eq!(generation, CompareGeneration(5)),
        other => panic!("unexpected event: {other:?}"),
    }
    thread::sleep(Duration::from_millis(100));
    assert!(harness.messages.try_recv().is_err());
}
