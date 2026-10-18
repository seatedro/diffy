use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use quark_app::testing::{By, UiTestHarness};
use quark_app::{UiApp, UiContext, ViewContext};
use quark_ui::FocusId;
use quark_ui::accessibility::AccessibilityRole as Role;
use quark_ui::element::*;
use quark_ui::style::Styled;
use quark_ui::text_input::{TextEditCommand, TextEditOutcome};

use super::presentation::action_buttons;
use super::{MutationsController, panel, view};
use crate::app::shell::Slot;
use crate::app::text_targets::{TextCx, TextHost};
use crate::contracts::commands::ToastUpdate;
use crate::contracts::mutations::{
    ConfirmedMutation, MutationSelection, MutationSource, MutationSourceKey, MutationsAction,
    MutationsEffect, MutationsEvent, StagedDigest,
};
use crate::contracts::navigation::{
    RepositoryContext, RepositoryRevision, RepositorySnapshot, RepositorySyncReason,
};
use crate::contracts::shared::{FileKey, FileRevision, SharedContext, Side, SourceTarget};
use crate::contracts::shell::{Action, Controller, Effect, Intent, Reduction};
use crate::contracts::viewport::CompareManifestSnapshot;
use crate::contracts::viewport::pipeline::ManifestFile;
use crate::core::compare::stats::CompareFilePaths;
use crate::core::vcs::model::{
    ChangeBucket, FileChange, FileChangeStatus, FileOperation, PublishAction, PublishActionKind,
    PublishPlan, RefKind, RepoCapabilities, RepoLocation, RevisionId, VcsKind, VcsRef,
};
use crate::core::vcs::mutation::MutationMaterial;
use crate::core::vcs::source::SourceIdentity;
use crate::platform::secrets::AiKeyKind;

const FILE: FileKey = FileKey(1);
const REVISION: FileRevision = FileRevision(4);

fn change(bucket: ChangeBucket) -> FileChange {
    FileChange {
        path: "f.txt".to_owned(),
        old_path: None,
        status: FileChangeStatus::Modified,
        bucket,
    }
}

fn repository(
    revision: u64,
    buckets: &[ChangeBucket],
    capabilities: RepoCapabilities,
) -> Rc<RepositoryContext> {
    Rc::new(RepositoryContext {
        revision: RepositoryRevision(revision),
        snapshot: Arc::new(RepositorySnapshot {
            location: RepoLocation {
                kind: VcsKind::GIT,
                profile: "git",
                workspace_root: PathBuf::from("/repo"),
                store_root: None,
            },
            path: PathBuf::from("/repo"),
            reason: RepositorySyncReason::Rescan,
            change_kind: None,
            capabilities,
            refs: vec![VcsRef {
                name: "main".to_owned(),
                kind: RefKind::Branch,
                target: RevisionId::git("abc"),
                active: true,
                upstream: Some("origin/main".to_owned()),
                ahead_behind: None,
            }],
            changes: Vec::new(),
            operation_log: Vec::new(),
            file_changes: buckets.iter().map(|bucket| change(*bucket)).collect(),
            publish_plan: None,
        }),
    })
}

fn manifest(revision: FileRevision) -> Rc<CompareManifestSnapshot> {
    Rc::new(CompareManifestSnapshot {
        files: Arc::from(vec![ManifestFile::new(
            FILE,
            revision,
            CompareFilePaths::from_paths(Some("f.txt"), Some("f.txt")),
            diffy_diff::FileStatus::Modified,
        )]),
        ..CompareManifestSnapshot::default()
    })
}

fn shared() -> SharedContext {
    SharedContext {
        repository: Some(repository(
            1,
            &[ChangeBucket::Unstaged, ChangeBucket::Staged],
            RepoCapabilities::git(),
        )),
        manifest: Some(manifest(REVISION)),
        ..SharedContext::default()
    }
}

fn target(side: Side, lines: std::ops::Range<u32>) -> SourceTarget {
    SourceTarget {
        file: FILE,
        revision: REVISION,
        side,
        lines,
    }
}

fn hunk() -> crate::contracts::shared::HunkTarget {
    crate::contracts::shared::HunkTarget {
        source: target(Side::New, 0..4),
        hunk: 0,
    }
}

fn mutation_effects(reduction: &Reduction) -> Vec<MutationsEffect> {
    reduction
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Mutations(effect) => Some(effect.clone()),
            _ => None,
        })
        .collect()
}

fn source_for(key: MutationSourceKey) -> Arc<MutationSource> {
    Arc::new(MutationSource::new(
        key,
        MutationMaterial {
            patch: Arc::from(&b"--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n+b\n"[..]),
            identity: SourceIdentity::default(),
        },
    ))
}

#[test]
fn hunk_operations_load_exact_sources_lazily_then_apply() {
    let cx = shared();
    let mut controller = MutationsController::default();
    let reduction = controller.reduce_action(MutationsAction::StageHunkAt(hunk()), &cx);
    let effects1 = mutation_effects(&reduction);
    let [MutationsEffect::LoadMutationSource(key)] = effects1.as_slice() else {
        panic!("expected a source load, got {:?}", reduction.effects);
    };
    assert_eq!(key.change.bucket, ChangeBucket::Unstaged);
    assert_eq!((key.file, key.revision), (FILE, REVISION));
    assert!(controller.busy());

    let ready = controller.reduce_event(
        MutationsEvent::MutationSourceReady {
            source: source_for(key.clone()),
        },
        &cx,
    );
    let effects2 = mutation_effects(&ready);
    let [MutationsEffect::ApplyPatch(request)] = effects2.as_slice() else {
        panic!("expected a patch, got {:?}", ready.effects);
    };
    assert_eq!(request.operation, FileOperation::Stage);
    assert_eq!(request.selection, MutationSelection::Hunks(vec![hunk()]));

    let duplicate = controller.reduce_event(
        MutationsEvent::MutationSourceReady {
            source: source_for(key.clone()),
        },
        &cx,
    );
    assert!(duplicate.effects.is_empty());

    controller.reduce_event(
        MutationsEvent::FileOperationApplied {
            job: request.job,
            path: PathBuf::from("/repo"),
        },
        &cx,
    );
    assert!(!controller.busy());
    let unstage = controller.reduce_action(MutationsAction::UnstageHunkAt(hunk()), &cx);
    let effects3 = mutation_effects(&unstage);
    let [MutationsEffect::LoadMutationSource(key)] = effects3.as_slice() else {
        panic!("expected a source load");
    };
    assert_eq!(key.change.bucket, ChangeBucket::Staged);
}

#[test]
fn stale_targets_and_sources_are_rejected() {
    let mut cx = shared();
    let mut controller = MutationsController::default();
    let stale = SourceTarget {
        revision: FileRevision(3),
        ..target(Side::New, 0..1)
    };
    let reduction = controller.reduce_action(
        MutationsAction::StageHunkAt(crate::contracts::shared::HunkTarget {
            source: stale,
            hunk: 0,
        }),
        &cx,
    );
    assert!(reduction.effects.is_empty());
    assert!(matches!(
        reduction.intents.as_slice(),
        [Intent::Toast(ToastUpdate::Show { .. })]
    ));

    let reduction = controller.reduce_action(MutationsAction::StageHunkAt(hunk()), &cx);
    let effects4 = mutation_effects(&reduction);
    let [MutationsEffect::LoadMutationSource(key)] = effects4.as_slice() else {
        panic!("expected a source load");
    };
    let repo = repository(2, &[ChangeBucket::Unstaged], RepoCapabilities::git());
    cx.repository = Some(repo.clone());
    let changed = controller.apply_intent(&Intent::RepositoryChanged(repo), &cx);
    assert!(!changed.intents.is_empty());
    let late = controller.reduce_event(
        MutationsEvent::MutationSourceReady {
            source: source_for(key.clone()),
        },
        &cx,
    );
    assert!(late.effects.is_empty());
    assert!(!controller.busy());
}

#[test]
fn discards_need_confirmation_through_the_commands_host() {
    let cx = shared();
    let mut controller = MutationsController::default();
    let reduction = controller.reduce_action(MutationsAction::DiscardHunkAt(hunk()), &cx);
    assert!(reduction.effects.is_empty());
    let [Intent::RequestConfirmation(request)] = reduction.intents.as_slice() else {
        panic!("expected a confirmation, got {:?}", reduction.intents);
    };
    assert!(request.destructive);
    assert_eq!(
        *request.confirm,
        Action::Mutations(MutationsAction::ApplyConfirmed(
            ConfirmedMutation::Selection(MutationSelection::Hunks(vec![hunk()]))
        ))
    );
    let Action::Mutations(confirmed) = *request.confirm.clone() else {
        unreachable!()
    };
    let reduction = controller.reduce_action(confirmed, &cx);
    let effects5 = mutation_effects(&reduction);
    let [MutationsEffect::LoadMutationSource(key)] = effects5.as_slice() else {
        panic!("expected a source load");
    };
    assert_eq!(key.change.bucket, ChangeBucket::Unstaged);
}

#[test]
fn selected_lines_are_separate_from_the_text_selection() {
    let mut cx = shared();
    cx.selection = Some(target(Side::New, 7..9));
    let mut controller = MutationsController::default();
    controller.reduce_action(
        MutationsAction::ToggleLineSelection(target(Side::New, 2..3)),
        &cx,
    );
    controller.reduce_action(
        MutationsAction::ExtendLineSelection(target(Side::New, 5..6)),
        &cx,
    );
    controller.reduce_action(
        MutationsAction::ToggleLineSelection(target(Side::Old, 1..2)),
        &cx,
    );
    controller.reduce_action(
        MutationsAction::ToggleLineSelection(target(Side::New, 4..5)),
        &cx,
    );
    assert_eq!(controller.selected_lines().len(), 4);
    assert_eq!(cx.selection, Some(target(Side::New, 7..9)));

    let reduction = controller.reduce_action(MutationsAction::UnstageSelectedLines, &cx);
    let effects6 = mutation_effects(&reduction);
    let [MutationsEffect::LoadMutationSource(key)] = effects6.as_slice() else {
        panic!("expected a source load");
    };
    assert!(controller.selected_lines().is_empty());
    let ready = controller.reduce_event(
        MutationsEvent::MutationSourceReady {
            source: source_for(key.clone()),
        },
        &cx,
    );
    let effects7 = mutation_effects(&ready);
    let [MutationsEffect::ApplyPatch(request)] = effects7.as_slice() else {
        panic!("expected a patch");
    };
    assert_eq!(
        request.selection,
        MutationSelection::Lines(vec![
            target(Side::Old, 1..2),
            target(Side::New, 2..4),
            target(Side::New, 5..6),
        ])
    );
    assert_eq!(request.operation, FileOperation::Unstage);
}

#[test]
fn manifest_republish_drops_selection_for_old_revisions() {
    let mut cx = shared();
    let mut controller = MutationsController::default();
    controller.reduce_action(
        MutationsAction::ToggleLineSelection(target(Side::New, 0..2)),
        &cx,
    );
    let next = manifest(FileRevision(5));
    cx.manifest = Some(next.clone());
    controller.apply_intent(&Intent::PublishManifest(next), &cx);
    assert!(controller.selected_lines().is_empty());
}

fn ai_ready(controller: &mut MutationsController, cx: &SharedContext) {
    controller.reduce_action(
        MutationsAction::SetAiKey {
            kind: AiKeyKind::Anthropic,
            value: crate::contracts::review::Secret::new("key"),
        },
        cx,
    );
}

fn chunk(generation: u64, text: &str) -> MutationsEvent {
    MutationsEvent::CommitMessageChunk {
        generation,
        repository: RepositoryRevision(1),
        chunk: text.to_owned(),
    }
}

#[test]
fn ai_stream_keeps_user_edits_and_drops_stale_chunks() {
    let mut cx = shared();
    let mut controller = MutationsController::default();
    ai_ready(&mut controller, &cx);
    let first = controller.reduce_action(MutationsAction::GenerateCommitMessage, &cx);
    let effects8 = mutation_effects(&first);
    let [MutationsEffect::GenerateCommitMessage(request)] = effects8.as_slice() else {
        panic!("expected a generation");
    };
    let generation = request.generation;
    controller.reduce_event(chunk(generation, "Fix "), &cx);
    let text_cx = TextCx {
        shared: &cx,
        now_ms: 10,
    };
    controller.edit_text(
        crate::app::text_targets::COMMIT_MESSAGE,
        TextEditCommand::CursorHome,
        &text_cx,
    );
    controller.edit_text(
        crate::app::text_targets::COMMIT_MESSAGE,
        TextEditCommand::InsertText("ui: ".to_owned()),
        &text_cx,
    );
    controller.reduce_event(chunk(generation, "parser"), &cx);
    assert_eq!(controller.commit_text(), "ui: Fix parser");

    controller.reduce_event(chunk(generation.wrapping_sub(1), " stale"), &cx);
    assert_eq!(controller.commit_text(), "ui: Fix parser");

    controller.reduce_event(started(generation, 7), &cx);
    let same = repository(2, &[ChangeBucket::Staged], RepoCapabilities::git());
    cx.repository = Some(same.clone());
    let check = controller.apply_intent(&Intent::RepositoryChanged(same), &cx);
    assert!(matches!(
        mutation_effects(&check).as_slice(),
        [MutationsEffect::CheckStagedDiff {
            has_staged: true,
            ..
        }]
    ));
    controller.reduce_event(chunk(generation, " held"), &cx);
    assert_eq!(controller.commit_text(), "ui: Fix parser");
    controller.reduce_event(checked(generation, Some(7)), &cx);
    assert_eq!(controller.commit_text(), "ui: Fix parser held");

    let moved = repository(3, &[ChangeBucket::Staged], RepoCapabilities::git());
    cx.repository = Some(moved.clone());
    controller.apply_intent(&Intent::RepositoryChanged(moved), &cx);
    controller.reduce_event(chunk(generation, " dropped"), &cx);
    let stopped = controller.reduce_event(checked(generation, Some(8)), &cx);
    assert!(!stopped.intents.is_empty());
    controller.reduce_event(chunk(generation, " after"), &cx);
    assert_eq!(controller.commit_text(), "ui: Fix parser held");
    assert!(!controller.ai().active);

    cx.repository = Some(repository(
        3,
        &[ChangeBucket::Staged],
        RepoCapabilities::git(),
    ));
    let second = controller.reduce_action(MutationsAction::GenerateCommitMessage, &cx);
    let effects9 = mutation_effects(&second);
    let [MutationsEffect::GenerateCommitMessage(request)] = effects9.as_slice() else {
        panic!("expected a second generation");
    };
    assert_eq!(
        request.subject_override.as_deref(),
        Some("ui: Fix parser held")
    );
    controller.reduce_event(chunk(generation, " old"), &cx);
    controller.reduce_event(chunk(request.generation, "body"), &cx);
    assert_eq!(controller.commit_text(), "ui: Fix parser held\nbody");
}

fn started(generation: u64, digest: u64) -> MutationsEvent {
    MutationsEvent::CommitMessageStarted {
        generation,
        staged: StagedDigest(digest),
    }
}

fn checked(generation: u64, digest: Option<u64>) -> MutationsEvent {
    MutationsEvent::StagedDiffChecked {
        generation,
        staged: digest.map(StagedDigest),
    }
}

#[test]
fn staged_checks_that_beat_the_start_event_still_count() {
    let mut cx = shared();
    let mut controller = MutationsController::default();
    ai_ready(&mut controller, &cx);
    let first = controller.reduce_action(MutationsAction::GenerateCommitMessage, &cx);
    let effects = mutation_effects(&first);
    let [MutationsEffect::GenerateCommitMessage(request)] = effects.as_slice() else {
        panic!("expected a generation");
    };
    let generation = request.generation;
    let repo = repository(2, &[ChangeBucket::Staged], RepoCapabilities::git());
    cx.repository = Some(repo.clone());
    controller.apply_intent(&Intent::RepositoryChanged(repo), &cx);
    controller.reduce_event(checked(generation, Some(5)), &cx);
    controller.reduce_event(started(generation, 4), &cx);
    assert!(!controller.ai().active);
    controller.reduce_event(chunk(generation, "late"), &cx);
    assert_eq!(controller.commit_text(), "");
}

#[test]
fn provider_preference_and_saved_prompt_steer_generation() {
    let cx = shared();
    let mut controller = MutationsController::default();
    ai_ready(&mut controller, &cx);
    controller.reduce_action(
        MutationsAction::SetAiProvider(Some(crate::ai::Provider::OpenAi)),
        &cx,
    );
    let missing = controller.reduce_action(MutationsAction::GenerateCommitMessage, &cx);
    assert!(missing.effects.is_empty());
    controller.reduce_action(MutationsAction::SetAiProvider(None), &cx);
    controller
        .steering_editor_mut()
        .set_text("Use imperative mood");
    controller.reduce_action(MutationsAction::SaveSteeringPrompt, &cx);
    let reduction = controller.reduce_action(MutationsAction::GenerateCommitMessage, &cx);
    let effects = mutation_effects(&reduction);
    let [MutationsEffect::GenerateCommitMessage(request)] = effects.as_slice() else {
        panic!("expected a generation");
    };
    assert_eq!(request.provider, crate::ai::Provider::Anthropic);
    assert_eq!(request.steering_prompt, "Use imperative mood");
}

#[test]
fn commits_need_a_message_and_committable_changes() {
    let cx = shared();
    let mut controller = MutationsController::default();
    let empty = controller.reduce_action(MutationsAction::SubmitCommit, &cx);
    assert!(empty.effects.is_empty());
    controller.commit_editor_mut().set_text("Add parser");
    let reduction = controller.reduce_action(MutationsAction::SubmitCommit, &cx);
    let commit_effects = mutation_effects(&reduction);
    let [MutationsEffect::CreateCommit { job, message, .. }] = commit_effects.as_slice() else {
        panic!("expected a commit");
    };
    assert_eq!(message, "Add parser");
    controller.reduce_event(
        MutationsEvent::CommitCreated {
            job: *job,
            path: PathBuf::from("/repo"),
        },
        &cx,
    );
    assert_eq!(controller.commit_text(), "");
}

#[test]
fn jj_operations_confirm_and_report_progress() {
    use crate::core::vcs::jj::service::jj_capabilities;
    use crate::core::vcs::model::{JjOperation, VcsOperation};
    let cx = SharedContext {
        repository: Some(repository(
            1,
            &[ChangeBucket::WorkingCopy],
            jj_capabilities(true),
        )),
        ..SharedContext::default()
    };
    let mut controller = MutationsController::default();
    let operation = VcsOperation::Jj(JjOperation::AbandonChange);
    let reduction = controller.reduce_action(MutationsAction::RunOperation(operation.clone()), &cx);
    assert!(matches!(
        reduction.intents.as_slice(),
        [Intent::RequestConfirmation(_)]
    ));
    let reduction =
        controller.reduce_action(MutationsAction::RunConfirmedOperation(operation), &cx);
    assert!(matches!(
        mutation_effects(&reduction).as_slice(),
        [MutationsEffect::RunOperation { .. }]
    ));
    assert!(matches!(
        reduction.intents.as_slice(),
        [Intent::Toast(ToastUpdate::Progress { .. })]
    ));
    let stage = controller.reduce_action(MutationsAction::StageHunkAt(hunk()), &cx);
    assert!(stage.effects.is_empty());
}

struct Host {
    controller: MutationsController,
    shared: SharedContext,
    reductions: Vec<Reduction>,
    now_ms: u64,
}

enum HostMessage {
    Action(MutationsAction),
    Event(MutationsEvent),
}

impl Host {
    fn new(shared: SharedContext) -> Self {
        let mut controller = MutationsController::default();
        if let Some(repo) = shared.repository.clone() {
            controller.apply_intent(&Intent::RepositoryChanged(repo), &shared);
        }
        Self {
            controller,
            shared,
            reductions: Vec::new(),
            now_ms: 0,
        }
    }

    fn effects(&self) -> Vec<MutationsEffect> {
        self.reductions.iter().flat_map(mutation_effects).collect()
    }

    fn intents(&self) -> Vec<&Intent> {
        self.reductions.iter().flat_map(|r| &r.intents).collect()
    }
}

impl UiApp for Host {
    type Action = Action;
    type Message = HostMessage;

    fn message(&mut self, message: HostMessage, cx: &mut UiContext) {
        let reduction = match message {
            HostMessage::Action(action) => self.controller.reduce_action(action, &self.shared),
            HostMessage::Event(event) => self.controller.reduce_event(event, &self.shared),
        };
        self.reductions.push(reduction);
        cx.window.request_redraw();
    }

    fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
        let (width, height) = cx.frame.size();
        let status_h = cx.theme.metrics.status_bar_height;
        let slot = Slot {
            shared: &self.shared,
            width,
            height: status_h,
        };
        let status = view(&mut self.controller, &slot, cx);
        let panel = panel(&mut self.controller, &slot, cx);
        let presentation = self.controller.presentation();
        let hunk = action_buttons(presentation.hunk_actions(&hunk()), 4.0);
        quark::view! {
            <div w={width} h={height} class="flex-col relative">
                <div class="flex-1">{hunk}</div>
                {status}
                {?panel}
            </div>
        }
    }

    fn update(&mut self, action: Action, _cx: &mut UiContext) {
        if let Action::Mutations(action) = action {
            let reduction = self.controller.reduce_action(action, &self.shared);
            self.reductions.push(reduction);
        }
    }

    fn edit_text(&mut self, target: FocusId, command: TextEditCommand) -> TextEditOutcome {
        self.now_ms += 1;
        let cx = TextCx {
            shared: &self.shared,
            now_ms: self.now_ms,
        };
        self.controller
            .edit_text(target, command, &cx)
            .unwrap_or_default()
    }

    fn set_preedit(&mut self, target: FocusId, text: String, cursor: Option<(usize, usize)>) {
        self.controller.set_preedit(target, text, cursor);
    }
}

fn harness() -> UiTestHarness<Host> {
    UiTestHarness::new(Host::new(shared()), (900.0, 640.0), 2.0)
}

#[test]
fn stage_hunk_control_requests_the_exact_source() {
    let mut ui = harness();
    ui.click_node(By::role_name(Role::Button, "Stage hunk"));
    assert!(matches!(
        ui.app().effects().as_slice(),
        [MutationsEffect::LoadMutationSource(_)]
    ));
}

#[test]
fn selected_line_controls_unstage_and_confirm_discard() {
    let mut ui = harness();
    assert!(ui.try_find(By::name("Unstage selected lines")).is_none());
    ui.send_message(HostMessage::Action(MutationsAction::ToggleLineSelection(
        target(Side::New, 1..3),
    )));
    assert!(ui.painted_text().contains("2 lines selected"));
    ui.click_node(By::role_name(Role::Button, "Discard selected lines"));
    assert!(matches!(
        ui.app().intents().as_slice(),
        [Intent::RequestConfirmation(_)]
    ));
    ui.click_node(By::role_name(Role::Button, "Unstage selected lines"));
    assert!(matches!(
        ui.app().effects().as_slice(),
        [MutationsEffect::LoadMutationSource(key)] if key.change.bucket == ChangeBucket::Staged
    ));
    assert!(ui.try_find(By::name("Unstage selected lines")).is_none());
}

#[test]
fn commit_message_editor_takes_typing_and_generates() {
    let mut ui = harness();
    ui.click_node(By::role_name(Role::Button, "Commit"));
    ui.click_node(By::role_name(Role::MultilineTextInput, "Commit message"));
    ui.type_text("Fix parser");
    assert_eq!(ui.app().controller.commit_text(), "Fix parser");
    assert_eq!(
        ui.find(By::name("Commit message")).value.as_deref(),
        Some("Fix parser")
    );

    ui.send_message(HostMessage::Action(MutationsAction::SetAiKey {
        kind: AiKeyKind::OpenAi,
        value: crate::contracts::review::Secret::new("key"),
    }));
    ui.click_node(By::role_name(Role::Button, "Generate commit message"));
    let effects = ui.app().effects();
    let Some(MutationsEffect::GenerateCommitMessage(request)) = effects.last() else {
        panic!("expected a generation, got {effects:?}");
    };
    assert_eq!(request.subject_override.as_deref(), Some("Fix parser"));
    let generation = request.generation;
    ui.send_message(HostMessage::Event(chunk(generation, "Body text")));
    assert_eq!(
        ui.find(By::name("Commit message")).value.as_deref(),
        Some("Fix parser\nBody text")
    );
    ui.click_node(By::role_name(Role::Button, "Create commit"));
    assert!(matches!(
        ui.app().effects().last(),
        Some(MutationsEffect::CreateCommit { message, .. }) if message == "Fix parser\nBody text"
    ));
}

#[test]
fn publish_menu_lists_plan_actions() {
    let mut ui = harness();
    ui.click_node(By::role_name(Role::Button, "Publish"));
    let Some(MutationsEffect::LoadPublishPlan { job, .. }) = ui.app().effects().last().cloned()
    else {
        panic!("expected a plan load");
    };
    let plan = PublishPlan {
        primary: PublishAction {
            label: "Push main".to_owned(),
            description: "Push main to origin/main".to_owned(),
            kind: PublishActionKind::PushRef {
                remote: "origin".to_owned(),
                refspec: "refs/heads/main:refs/heads/main".to_owned(),
                force_with_lease: false,
            },
            disabled_reason: None,
            change_id_token: None,
        },
        alternatives: Vec::new(),
    };
    ui.send_message(HostMessage::Event(MutationsEvent::PublishPlanReady {
        job,
        path: PathBuf::from("/repo"),
        plan: Box::new(plan),
    }));
    ui.click_node(By::role_name(Role::Button, "Push main"));
    assert!(matches!(
        ui.app().effects().last(),
        Some(MutationsEffect::Publish {
            action: Some(_),
            ..
        })
    ));
    assert!(
        ui.try_find(By::role_name(Role::Button, "Push main"))
            .is_none()
    );
}

#[test]
fn presentation_marks_selected_lines_and_offers_hunk_controls() {
    use diffy_ui::diff_view::decorator::{DiffDecorator, FileSource, LineContext, LineMark};
    let cx = shared();
    let mut controller = MutationsController::default();
    controller.apply_intent(
        &Intent::RepositoryChanged(cx.repository.clone().unwrap()),
        &cx,
    );
    let before = controller.presentation();
    controller.reduce_action(
        MutationsAction::ToggleLineSelection(target(Side::New, 3..4)),
        &cx,
    );
    let presentation = controller.presentation();
    assert!(!Rc::ptr_eq(&before, &presentation));
    assert_ne!(before.revision(), presentation.revision());
    let line = |line| LineContext {
        file: 0,
        source: Some(FileSource {
            id: FILE,
            revision: diffy_diff::Revision(REVISION.0),
        }),
        side: Side::New,
        line,
        kind: diffy_diff::RowKind::Added,
    };
    assert_eq!(presentation.line_mark(&line(3)), Some(LineMark::Selected));
    assert_eq!(presentation.line_mark(&line(4)), None);
    let labels: Vec<_> = presentation
        .hunk_actions(&hunk())
        .into_iter()
        .map(|(label, _)| label)
        .collect();
    assert_eq!(labels, ["Stage hunk", "Unstage hunk", "Discard hunk"]);
    let same = controller.presentation();
    assert!(Rc::ptr_eq(&presentation, &same));
}

struct SettingsHost {
    controller: MutationsController,
    shared: SharedContext,
    reductions: Vec<Reduction>,
    now_ms: u64,
}

impl UiApp for SettingsHost {
    type Action = Action;
    type Message = ();

    fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
        let (width, height) = cx.frame.size();
        let settings = self.shared.settings.clone();
        let content = super::clankers_settings(&mut self.controller, &settings, width - 40.0, cx);
        quark::view! { <div w={width} h={height} p={20.0}>{content}</div> }
    }

    fn update(&mut self, action: Action, _cx: &mut UiContext) {
        if let Action::Mutations(action) = action {
            let reduction = self.controller.reduce_action(action, &self.shared);
            self.reductions.push(reduction);
        }
    }

    fn edit_text(&mut self, target: FocusId, command: TextEditCommand) -> TextEditOutcome {
        self.now_ms += 1;
        let cx = TextCx {
            shared: &self.shared,
            now_ms: self.now_ms,
        };
        self.controller
            .edit_text(target, command, &cx)
            .unwrap_or_default()
    }
}

#[test]
fn clankers_settings_save_keys_and_prompt() {
    let mut ui = UiTestHarness::new(
        SettingsHost {
            controller: MutationsController::default(),
            shared: shared(),
            reductions: Vec::new(),
            now_ms: 0,
        },
        (900.0, 900.0),
        2.0,
    );
    ui.click_node(By::role_name(Role::PasswordInput, "OpenAI API key"));
    ui.type_text("sk-test");
    ui.click_node(By::role_name(Role::Button, "Save OpenAI key"));
    let saved = ui
        .app()
        .reductions
        .iter()
        .flat_map(mutation_effects)
        .any(|effect| matches!(effect, MutationsEffect::SaveAiKey { kind: AiKeyKind::OpenAi, value } if value.expose() == "sk-test"));
    assert!(saved);
    assert!(
        ui.try_find(By::role_name(Role::Button, "Replace OpenAI key"))
            .is_some()
    );
    assert!(
        ui.try_find(By::role_name(Role::Button, "Remove OpenAI key"))
            .is_some()
    );

    ui.click_node(By::role_name(Role::Button, "Anthropic"));
    assert_eq!(
        ui.app().controller.ai().provider,
        Some(crate::ai::Provider::Anthropic)
    );

    ui.click_node(By::role_name(
        Role::MultilineTextInput,
        "Commit steering prompt",
    ));
    ui.key("mod+a");
    ui.type_text("Be brief");
    ui.click_node(By::role_name(Role::Button, "Save prompt"));
    assert_eq!(
        ui.app().controller.ai().steering.as_deref(),
        Some("Be brief")
    );
}

#[test]
fn mutation_controls_are_keyboard_reachable() {
    let mut ui = harness();
    ui.click_node(By::role_name(Role::Button, "Commit"));
    ui.click_node(By::role_name(Role::MultilineTextInput, "Commit message"));
    ui.type_text("Fix");
    ui.key("tab");
    let focused = ui.focused().expect("focus moved");
    assert_eq!(focused.role, Some(Role::Button));
    ui.key("escape");
    assert!(!ui.app().controller.composer_open());
}
