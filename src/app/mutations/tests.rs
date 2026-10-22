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

use super::controls::{self as mutation_controls, Chrome};
use super::{MutationsController, panel, view};
use crate::ai::{CliAuth, CliInfo, CliScan, CliTool, Provider};
use crate::app::shell::Slot;
use crate::app::text_targets::{TextCx, TextHost};
use crate::contracts::commands::CommandSource;
use crate::contracts::commands::{CommandsAction, ToastUpdate};
use crate::contracts::mutations::{
    AiBackend, ConfirmedMutation, MutationSelection, MutationSource, MutationSourceKey,
    MutationsAction, MutationsEffect, MutationsEvent, StagedDigest,
};
use crate::contracts::navigation::{
    RepositoryContext, RepositoryRevision, RepositorySnapshot, RepositorySyncReason,
};
use crate::contracts::shared::{FileKey, FileRevision, JobId, SharedContext, Side, SourceTarget};
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
                worktree: None,
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
        files: Arc::from(vec![ManifestFile {
            buckets: vec![ChangeBucket::Unstaged, ChangeBucket::Staged],
            ..ManifestFile::new(
                FILE,
                revision,
                CompareFilePaths::from_paths(Some("f.txt"), Some("f.txt")),
                diffy_diff::FileStatus::Modified,
            )
        }]),
        working_tree: true,
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
            undo: None,
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
fn discards_run_immediately_and_only_untracked_deletion_confirms() {
    let mut cx = shared();
    let mut controller = MutationsController::default();
    let reduction = controller.reduce_action(MutationsAction::DiscardHunkAt(hunk()), &cx);
    assert!(reduction.intents.is_empty());
    let effects = mutation_effects(&reduction);
    let [MutationsEffect::LoadMutationSource(key)] = effects.as_slice() else {
        panic!("expected a source load, got {:?}", reduction.intents);
    };
    assert_eq!(key.change.bucket, ChangeBucket::Unstaged);

    cx.repository = Some(repository(
        1,
        &[ChangeBucket::Untracked],
        RepoCapabilities::git(),
    ));
    cx.manifest = Some(Rc::new(CompareManifestSnapshot {
        files: Arc::from(vec![ManifestFile {
            buckets: vec![ChangeBucket::Untracked],
            ..ManifestFile::new(
                FILE,
                REVISION,
                CompareFilePaths::from_paths(None, Some("f.txt")),
                diffy_diff::FileStatus::Added,
            )
        }]),
        working_tree: true,
        ..CompareManifestSnapshot::default()
    }));
    let mut controller = MutationsController::default();
    let reduction = controller.reduce_action(MutationsAction::DiscardFile(FILE), &cx);
    let [Intent::RequestConfirmation(request)] = reduction.intents.as_slice() else {
        panic!("expected a confirmation, got {:?}", reduction.intents);
    };
    assert!(request.destructive);
    assert_eq!(
        *request.confirm,
        Action::Mutations(MutationsAction::ApplyConfirmed(ConfirmedMutation::Files(
            vec![FILE]
        )))
    );
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
fn picking_a_semantic_row_selects_every_line_it_stands_for() {
    use crate::core::compare::{LayoutMode, RendererKind};
    use crate::platform::persistence::{PersistedCompare, Settings};
    let mut cx = shared();
    let mut settings = Settings {
        last_compare: Some(PersistedCompare {
            renderer: RendererKind::Semantic,
            ..PersistedCompare::default()
        }),
        ..Settings::default()
    };
    settings.viewport.layout = LayoutMode::Unified;
    cx.settings = Rc::new(crate::contracts::commands::SettingsSnapshot {
        revision: 1,
        settings,
    });
    let mut file = manifest(REVISION).files[0].clone();
    file.line_groups = Arc::from([diffy_diff::LineGroup {
        new: 3..6,
        old: 2..3,
    }]);
    cx.manifest = Some(Rc::new(CompareManifestSnapshot {
        files: Arc::from([file]),
        ..(*manifest(REVISION)).clone()
    }));
    let mut controller = MutationsController::default();
    controller.reduce_action(MutationsAction::SelectLines(target(Side::New, 4..5)), &cx);
    let lines: Vec<_> = controller
        .selected_lines()
        .iter()
        .map(|key| (key.side, key.line))
        .collect();
    assert_eq!(
        lines,
        [
            (Side::Old, 2),
            (Side::New, 3),
            (Side::New, 4),
            (Side::New, 5)
        ]
    );
    controller.reduce_action(
        MutationsAction::ToggleLineSelection(target(Side::New, 3..4)),
        &cx,
    );
    assert!(controller.selected_lines().is_empty());
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
    let mut cx = shared();
    let mut controller = MutationsController::default();
    ai_ready(&mut controller, &cx);
    let mut settings = cx.settings.as_ref().clone();
    settings.settings.ai_provider = Some(crate::ai::Provider::OpenAi);
    cx.settings = Rc::new(settings.clone());
    let missing = controller.reduce_action(MutationsAction::GenerateCommitMessage, &cx);
    assert!(missing.effects.is_empty());
    settings.settings.ai_provider = None;
    settings.settings.ai_steering_prompt = "Use imperative mood".to_owned();
    cx.settings = Rc::new(settings);
    controller.apply_intent(&Intent::SettingsChanged(cx.settings.clone()), &cx);
    assert_eq!(controller.steering_editor().text(), "Use imperative mood");
    let reduction = controller.reduce_action(MutationsAction::GenerateCommitMessage, &cx);
    let effects = mutation_effects(&reduction);
    let [MutationsEffect::GenerateCommitMessage(request)] = effects.as_slice() else {
        panic!("expected a generation");
    };
    assert!(matches!(
        &request.backend,
        AiBackend::Api {
            provider: Provider::Anthropic,
            ..
        }
    ));
    assert_eq!(request.steering_prompt, "Use imperative mood");
}

fn cli_scan(tools: &[(CliTool, CliAuth)]) -> Arc<CliScan> {
    Arc::new(CliScan {
        tools: tools
            .iter()
            .map(|(tool, auth)| CliInfo {
                tool: *tool,
                program: PathBuf::from("/opt/tools").join(tool.binary()),
                version: Some("1.0.0".to_owned()),
                auth: *auth,
            })
            .collect(),
        ..CliScan::default()
    })
}

fn with_cli(cx: &mut SharedContext, provider: Option<Provider>) {
    let mut settings = cx.settings.as_ref().clone();
    settings.settings.ai_use_cli = true;
    settings.settings.ai_provider = provider;
    cx.settings = Rc::new(settings);
}

fn scanned(
    controller: &mut MutationsController,
    cx: &SharedContext,
    scan: Arc<CliScan>,
) -> Reduction {
    let reduction = controller.reduce_action(MutationsAction::ScanAiClis { force: true }, cx);
    let reduction_effects = mutation_effects(&reduction);
    let [MutationsEffect::ScanAiClis { generation }] = reduction_effects.as_slice() else {
        panic!("expected a scan");
    };
    controller.reduce_event(
        MutationsEvent::AiClisScanned {
            generation: *generation,
            scan,
        },
        cx,
    )
}

#[test]
fn generation_waits_for_the_cli_scan_then_uses_a_signed_in_cli() {
    let mut cx = shared();
    with_cli(&mut cx, None);
    let mut controller = MutationsController::default();
    let pending = controller.reduce_action(MutationsAction::GenerateCommitMessage, &cx);
    let pending_effects = mutation_effects(&pending);
    let [MutationsEffect::ScanAiClis { generation }] = pending_effects.as_slice() else {
        panic!("expected a scan before generating");
    };
    let stale = controller.reduce_event(
        MutationsEvent::AiClisScanned {
            generation: generation.wrapping_sub(1),
            scan: cli_scan(&[]),
        },
        &cx,
    );
    assert!(mutation_effects(&stale).is_empty());
    let ready = controller.reduce_event(
        MutationsEvent::AiClisScanned {
            generation: *generation,
            scan: cli_scan(&[
                (CliTool::ClaudeCode, CliAuth::SignedOut),
                (CliTool::Codex, CliAuth::SignedIn),
            ]),
        },
        &cx,
    );
    let ready_effects = mutation_effects(&ready);
    let [MutationsEffect::GenerateCommitMessage(request)] = ready_effects.as_slice() else {
        panic!("expected a generation after the scan");
    };
    let AiBackend::Cli(command) = &request.backend else {
        panic!("expected the CLI backend");
    };
    assert_eq!(command.tool, CliTool::Codex);
    assert_eq!(command.program, PathBuf::from("/opt/tools/codex"));
}

#[test]
fn signed_out_cli_reports_and_missing_cli_falls_back_to_keys() {
    let mut cx = shared();
    with_cli(&mut cx, Some(Provider::Anthropic));
    let mut controller = MutationsController::default();
    ai_ready(&mut controller, &cx);
    scanned(
        &mut controller,
        &cx,
        cli_scan(&[(CliTool::ClaudeCode, CliAuth::SignedOut)]),
    );
    let signed_out = controller.reduce_action(MutationsAction::GenerateCommitMessage, &cx);
    assert!(mutation_effects(&signed_out).is_empty());
    assert!(!signed_out.intents.is_empty());

    scanned(
        &mut controller,
        &cx,
        cli_scan(&[(CliTool::Codex, CliAuth::SignedIn)]),
    );
    let fallback = controller.reduce_action(MutationsAction::GenerateCommitMessage, &cx);
    let fallback_effects = mutation_effects(&fallback);
    let [MutationsEffect::GenerateCommitMessage(request)] = fallback_effects.as_slice() else {
        panic!("expected a generation");
    };
    assert!(matches!(
        &request.backend,
        AiBackend::Api {
            provider: Provider::Anthropic,
            ..
        }
    ));
}

#[test]
fn cancelling_a_generation_stops_the_worker() {
    let cx = shared();
    let mut controller = MutationsController::default();
    ai_ready(&mut controller, &cx);
    let started = controller.reduce_action(MutationsAction::GenerateCommitMessage, &cx);
    let started_effects = mutation_effects(&started);
    let [MutationsEffect::GenerateCommitMessage(request)] = started_effects.as_slice() else {
        panic!("expected a generation");
    };
    let cancelled = controller.reduce_action(MutationsAction::CancelGeneration, &cx);
    let cancelled_effects = mutation_effects(&cancelled);
    let [MutationsEffect::CancelCommitMessage { generation }] = cancelled_effects.as_slice() else {
        panic!("expected a cancel");
    };
    assert_ne!(*generation, request.generation);
    assert!(!controller.ai().active);
}

#[test]
fn file_ranking_uses_the_configured_key_or_reports_it_missing() {
    let cx = shared();
    let request = Rc::new(crate::contracts::navigation::RankFilesRequest {
        job: JobId(7),
        files: Vec::new(),
    });
    let mut controller = MutationsController::default();
    let missing = controller.apply_intent(&Intent::RankFiles(request.clone()), &cx);
    assert!(missing.effects.is_empty());
    assert!(matches!(
        missing.intents.as_slice(),
        [Intent::RankFilesUnavailable { job: JobId(7), .. }]
    ));

    ai_ready(&mut controller, &cx);
    let ready = controller.apply_intent(&Intent::RankFiles(request), &cx);
    let effects = mutation_effects(&ready);
    let [MutationsEffect::RankFiles(job)] = effects.as_slice() else {
        panic!("expected a ranking job");
    };
    assert_eq!(job.request.job, JobId(7));
    assert!(matches!(
        job.backend,
        AiBackend::Api {
            provider: Provider::Anthropic,
            ..
        }
    ));
}

#[test]
fn ranking_and_generation_wait_for_one_cli_scan_then_use_the_cli() {
    let mut cx = shared();
    with_cli(&mut cx, None);
    let mut controller = MutationsController::default();
    let request = Rc::new(crate::contracts::navigation::RankFilesRequest {
        job: JobId(7),
        files: Vec::new(),
    });
    let ranking = controller.apply_intent(&Intent::RankFiles(request), &cx);
    let ranking_effects = mutation_effects(&ranking);
    let [MutationsEffect::ScanAiClis { generation }] = ranking_effects.as_slice() else {
        panic!("expected a scan before ranking");
    };
    let generation = *generation;
    let pending = controller.reduce_action(MutationsAction::GenerateCommitMessage, &cx);
    assert!(mutation_effects(&pending).is_empty());

    let ready = controller.reduce_event(
        MutationsEvent::AiClisScanned {
            generation,
            scan: cli_scan(&[(CliTool::ClaudeCode, CliAuth::SignedIn)]),
        },
        &cx,
    );
    let ready_effects = mutation_effects(&ready);
    let [
        MutationsEffect::RankFiles(job),
        MutationsEffect::GenerateCommitMessage(commit),
    ] = ready_effects.as_slice()
    else {
        panic!("expected both waiting tasks to run");
    };
    assert!(matches!(&job.backend, AiBackend::Cli(command) if command.tool == CliTool::ClaudeCode));
    assert!(matches!(&commit.backend, AiBackend::Cli(_)));
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
            commit: commit_record("Add parser"),
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
        let chrome = Chrome::from_theme(cx.theme);
        let hunk = mutation_controls::row(presentation.actions_for_hunk(&hunk()), &chrome);
        let sidebar = Slot {
            shared: &self.shared,
            width: width * 0.4,
            height,
        };
        let commit =
            super::commit_box(&mut self.controller, &sidebar, cx).map(|(commit, _)| commit);
        let body = Slot {
            shared: &self.shared,
            width,
            height: height - status_h,
        };
        let bar = super::selection_bar(&mut self.controller, &body, cx);
        quark::view! {
            <div w={width} h={height} class="flex-col relative">
                <div class="flex-1 flex-row relative">
                    <div class="flex-col" w={width * 0.4}>
                        <div class="flex-1" />
                        {?commit}
                    </div>
                    <div class="flex-1">{hunk}</div>
                    {?bar}
                </div>
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
fn selection_bar_stages_and_discards_lines() {
    let mut ui = harness();
    assert!(ui.try_find(By::name("Selected lines")).is_none());
    ui.send_message(HostMessage::Action(MutationsAction::ToggleLineSelection(
        target(Side::New, 1..3),
    )));
    assert!(ui.painted_text().contains("2 lines"));
    assert!(ui.painted_text().contains("selected in f.txt"));
    ui.click_node(By::role_name(Role::Button, "Discard"));
    assert!(ui.app().intents().is_empty());
    assert!(matches!(
        ui.app().effects().as_slice(),
        [MutationsEffect::LoadMutationSource(key)] if key.change.bucket == ChangeBucket::Unstaged
    ));
    assert!(ui.try_find(By::name("Selected lines")).is_none());
}

#[test]
fn commit_box_generates_then_commits() {
    let mut ui = harness();
    assert!(ui.try_find(By::role_name(Role::Button, "Commit")).is_none());
    ui.send_message(HostMessage::Action(MutationsAction::SetAiKey {
        kind: AiKeyKind::OpenAi,
        value: crate::contracts::review::Secret::new("key"),
    }));
    ui.click_node(By::role_name(Role::Button, "Generate"));
    let effects = ui.app().effects();
    let Some(MutationsEffect::GenerateCommitMessage(request)) = effects.last() else {
        panic!("expected a generation, got {effects:?}");
    };
    let generation = request.generation;
    assert!(
        ui.try_find(By::role_name(Role::Button, "Generating\u{2026}"))
            .is_some()
    );
    ui.send_message(HostMessage::Event(chunk(generation, "Fix parser")));
    ui.send_message(HostMessage::Event(MutationsEvent::CommitMessageFinished {
        generation,
        repository: RepositoryRevision(1),
    }));
    assert_eq!(
        ui.find(By::name("Commit message")).value.as_deref(),
        Some("Fix parser")
    );
    ui.click_node(By::role_name(Role::MultilineTextInput, "Commit message"));
    ui.type_text(" now");
    ui.click_node(By::role_name(Role::Button, "Commit"));
    assert!(matches!(
        ui.app().effects().last(),
        Some(MutationsEffect::CreateCommit { message, amend: false, .. }) if message == "Fix parser now"
    ));
}

#[test]
fn commit_menu_amends_with_a_banner_and_restores_the_draft() {
    let mut ui = harness();
    ui.click_node(By::role_name(Role::MultilineTextInput, "Commit message"));
    ui.type_text("draft");
    ui.click_node(By::role_name(Role::Button, "More commit actions"));
    ui.click_node(By::role_name(Role::MenuItem, "Amend last commit"));
    assert!(matches!(
        ui.app().effects().last(),
        Some(MutationsEffect::LoadLastCommit { .. })
    ));
    ui.send_message(HostMessage::Event(MutationsEvent::LastCommitLoaded {
        path: PathBuf::from("/repo"),
        commit: Some(commit_record("Old subject")),
    }));
    assert!(ui.painted_text().contains("Amending"));
    assert_eq!(
        ui.find(By::name("Commit message")).value.as_deref(),
        Some("Old subject")
    );
    assert!(ui.try_find(By::role_name(Role::Button, "Amend")).is_some());
    ui.click_node(By::role_name(Role::Button, "Stop amending"));
    assert_eq!(
        ui.find(By::name("Commit message")).value.as_deref(),
        Some("draft")
    );
    assert!(ui.try_find(By::role_name(Role::Button, "Commit")).is_some());
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
fn publish_menu_without_a_remote_offers_no_push() {
    let mut ui = harness();
    ui.click_node(By::role_name(Role::Button, "Publish"));
    let Some(MutationsEffect::LoadPublishPlan { job, .. }) = ui.app().effects().last().cloned()
    else {
        panic!("expected a plan load");
    };
    ui.send_message(HostMessage::Event(MutationsEvent::PublishPlanReady {
        job,
        path: PathBuf::from("/repo"),
        plan: Box::new(PublishPlan::no_remote("main", "Add a remote.")),
    }));
    assert!(
        ui.try_find(By::test_id("mutations.publish.no-remote"))
            .is_some()
    );
    assert!(ui.try_find(By::test_id("mutations.publish.0")).is_none());
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
        .actions_for_hunk(&hunk())
        .into_iter()
        .map(|control| control.label)
        .collect();
    assert_eq!(labels, ["Stage hunk", "Discard hunk", "Unstage hunk"]);
    let same = controller.presentation();
    assert!(Rc::ptr_eq(&presentation, &same));
}

struct SettingsHost {
    controller: MutationsController,
    shared: SharedContext,
    reductions: Vec<Reduction>,
    commands: Vec<CommandsAction>,
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
        match action {
            Action::Mutations(action) => {
                let reduction = self.controller.reduce_action(action, &self.shared);
                self.reductions.push(reduction);
            }
            Action::Commands(action) => self.commands.push(action),
            _ => {}
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
            commands: Vec::new(),
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
        ui.app().commands.last(),
        Some(&CommandsAction::SetAiProvider(Some(
            crate::ai::Provider::Anthropic
        )))
    );

    ui.click_node(By::role_name(
        Role::MultilineTextInput,
        "Commit steering prompt",
    ));
    ui.key("mod+a");
    ui.type_text("Be brief");
    ui.click_node(By::role_name(Role::Button, "Save prompt"));
    assert_eq!(
        ui.app().commands.last(),
        Some(&CommandsAction::SetAiSteeringPrompt("Be brief".to_owned()))
    );
}

#[test]
fn clankers_settings_show_detected_clis() {
    let cx = shared();
    let mut controller = MutationsController::default();
    scanned(
        &mut controller,
        &cx,
        cli_scan(&[(CliTool::ClaudeCode, CliAuth::SignedIn)]),
    );
    let mut ui = UiTestHarness::new(
        SettingsHost {
            controller,
            shared: cx,
            reductions: Vec::new(),
            commands: Vec::new(),
            now_ms: 0,
        },
        (900.0, 1100.0),
        2.0,
    );
    assert_eq!(
        ui.find(By::test_id("settings.ai.cli.claude"))
            .name
            .as_deref(),
        Some("Claude Code 1.0.0, Signed in")
    );
    assert_eq!(
        ui.find(By::test_id("settings.ai.cli.codex"))
            .name
            .as_deref(),
        Some("Codex, Not found")
    );
    ui.click_node(By::role_name(Role::Switch, "Use local CLI"));
    assert_eq!(ui.app().commands, vec![CommandsAction::ToggleAiCli]);
    ui.click_node(By::role_name(Role::Button, "Check again"));
    assert!(matches!(
        ui.app().reductions.iter().flat_map(mutation_effects).last(),
        Some(MutationsEffect::ScanAiClis { .. })
    ));
}

#[test]
fn mutation_controls_are_keyboard_reachable() {
    let mut ui = harness();
    ui.click_node(By::role_name(Role::MultilineTextInput, "Commit message"));
    ui.type_text("Fix");
    ui.key("tab");
    let focused = ui.focused().expect("focus moved");
    assert_eq!(focused.role, Some(Role::Button));
    ui.key("enter");
    assert!(!ui.app().effects().is_empty() || ui.app().controller.commit_menu_open());
}

#[test]
fn nothing_mutates_outside_working_tree_compares() {
    let mut cx = shared();
    let mut manifest = (*manifest(REVISION)).clone();
    manifest.working_tree = false;
    cx.manifest = Some(Rc::new(manifest));
    let mut controller = MutationsController::default();
    controller.apply_intent(&Intent::PublishManifest(cx.manifest.clone().unwrap()), &cx);
    assert!(
        controller
            .presentation()
            .actions_for_hunk(&hunk())
            .is_empty()
    );
    let staged = controller.reduce_action(MutationsAction::StageHunkAt(hunk()), &cx);
    assert!(staged.effects.is_empty());
    let mut commands = Vec::new();
    controller.commands(&cx, &mut commands);
    assert!(commands.iter().all(|command| !matches!(
        command.id,
        "mutations.stage" | "mutations.discard" | "mutations.commit" | "mutations.publish"
    )));
    assert!(super::file_row_actions(&cx.manifest.as_ref().unwrap().files[0], &cx).is_empty());
}

#[test]
fn decorator_outputs_follow_the_displayed_compare() {
    use diffy_ui::diff_view::Revision;
    use diffy_ui::diff_view::decorator::{
        DiffDecorator, FileSource, GutterContext, HeaderContext, HeaderSlot,
    };
    let header = |revision: u64| HeaderContext {
        file: 0,
        source: Some(FileSource {
            id: FILE,
            revision: Revision(revision),
        }),
        title: "f.txt",
        status: diffy_diff::FileStatus::Modified,
        additions: 1,
        deletions: 1,
        binary: false,
        width: 400.0,
        height: 28.0,
    };
    let gutter = GutterContext {
        file: 0,
        source: header(REVISION.0).source,
        side: Side::New,
        line: 0,
        kind: diffy_diff::RowKind::Added,
        width: 16.0,
        height: 16.0,
    };
    let mut cx = shared();
    let mut controller = MutationsController::default();
    controller.apply_intent(&Intent::PublishManifest(cx.manifest.clone().unwrap()), &cx);
    let presentation = controller.presentation();
    assert_eq!(
        presentation.file_state(&header(REVISION.0)).as_deref(),
        Some("Modified \u{00b7} unstaged + staged")
    );
    assert!(
        presentation
            .header_slot(HeaderSlot::Actions, &header(REVISION.0))
            .is_some()
    );
    assert!(presentation.gutter_utility(&gutter).is_some());
    assert!(presentation.file_state(&header(REVISION.0 + 1)).is_none());
    assert!(
        presentation
            .header_slot(HeaderSlot::Actions, &header(REVISION.0 + 1))
            .is_none()
    );
    let mut stale = hunk();
    stale.source.revision = FileRevision(REVISION.0 + 1);
    assert!(presentation.actions_for_hunk(&stale).is_empty());

    let mut manifest = (*manifest(REVISION)).clone();
    manifest.working_tree = false;
    cx.manifest = Some(Rc::new(manifest));
    controller.apply_intent(&Intent::PublishManifest(cx.manifest.clone().unwrap()), &cx);
    let presentation = controller.presentation();
    assert!(presentation.file_state(&header(REVISION.0)).is_none());
    assert!(
        presentation
            .header_slot(HeaderSlot::Actions, &header(REVISION.0))
            .is_none()
    );
    assert!(presentation.gutter_utility(&gutter).is_none());
}

#[test]
fn commit_message_separates_subject_from_body() {
    use super::state::commit_message;
    assert_eq!(commit_message("subject\nbody"), "subject\n\nbody");
    assert_eq!(
        commit_message("subject\nbody\nmore"),
        "subject\n\nbody\nmore"
    );
    assert_eq!(commit_message("subject\n\nbody"), "subject\n\nbody");
    assert_eq!(commit_message("subject\n  \nbody"), "subject\n  \nbody");
    assert_eq!(commit_message(" subject \n"), "subject");
}

#[test]
fn submitted_commits_get_a_blank_line_after_the_subject() {
    let cx = shared();
    let mut controller = MutationsController::default();
    controller.set_text_value(
        crate::app::text_targets::COMMIT_MESSAGE,
        "subject\nbody".to_owned(),
        &TextCx {
            shared: &cx,
            now_ms: 0,
        },
    );
    let reduction = controller.reduce_action(MutationsAction::SubmitCommit, &cx);
    let effects = mutation_effects(&reduction);
    let [MutationsEffect::CreateCommit { message, .. }] = effects.as_slice() else {
        panic!("expected a commit");
    };
    assert_eq!(message, "subject\n\nbody");
}

#[test]
fn undo_toasts_restore_discards_and_commits() {
    let cx = shared();
    let mut controller = MutationsController::default();
    let reduction = controller.reduce_action(MutationsAction::DiscardFile(FILE), &cx);
    let effects = mutation_effects(&reduction);
    let [MutationsEffect::ApplyFileOperation(request)] = effects.as_slice() else {
        panic!("expected a file discard");
    };
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("f.txt"), b"before").unwrap();
    let pending =
        crate::core::vcs::worktree::PendingSnapshot::capture(dir.path(), ["f.txt"]).unwrap();
    std::fs::write(dir.path().join("f.txt"), b"after").unwrap();
    let snapshot = Arc::new(pending.finish().unwrap());
    let applied = controller.reduce_event(
        MutationsEvent::FileOperationApplied {
            job: request.job,
            path: PathBuf::from("/repo"),
            undo: Some(snapshot.clone()),
        },
        &cx,
    );
    assert!(matches!(applied.intents.as_slice(), [Intent::Toast(_)]));
    assert!(controller.can_undo(request.job));
    let undo = controller.reduce_action(MutationsAction::UndoDiscard(request.job), &cx);
    assert!(matches!(
        mutation_effects(&undo).as_slice(),
        [MutationsEffect::RestoreDiscard { snapshot: restored, .. }] if Arc::ptr_eq(restored, &snapshot)
    ));
    assert!(!controller.can_undo(request.job));

    controller.commit_editor_mut().set_text("Add parser");
    let commit = controller.reduce_action(MutationsAction::SubmitCommit, &cx);
    let effects = mutation_effects(&commit);
    let [MutationsEffect::CreateCommit { job, .. }] = effects.as_slice() else {
        panic!("expected a commit");
    };
    controller.reduce_event(
        MutationsEvent::CommitCreated {
            job: *job,
            path: PathBuf::from("/repo"),
            commit: commit_record("Add parser"),
        },
        &cx,
    );
    let mut commands = Vec::new();
    controller.commands(&cx, &mut commands);
    assert!(
        commands
            .iter()
            .any(|command| command.id == "mutations.undo_commit")
    );
    let undo = controller.reduce_action(MutationsAction::UndoCommit(*job), &cx);
    let effects = mutation_effects(&undo);
    let [
        MutationsEffect::UndoCommit {
            job: undo_job, sha, ..
        },
    ] = effects.as_slice()
    else {
        panic!("expected an undo");
    };
    assert_eq!(sha, "abc1234def");
    controller.reduce_event(
        MutationsEvent::CommitUndone {
            job: *undo_job,
            path: PathBuf::from("/repo"),
        },
        &cx,
    );
    assert_eq!(controller.commit_text(), "Add parser");
}

fn commit_record(message: &str) -> crate::core::vcs::model::CommitRecord {
    crate::core::vcs::model::CommitRecord {
        sha: "abc1234def".to_owned(),
        short: "abc1234".to_owned(),
        message: message.to_owned(),
    }
}

#[test]
fn line_operations_bind_to_displayed_buckets_and_objects() {
    let mut cx = shared();
    let mut controller = MutationsController::default();
    let entry = |buckets: Vec<ChangeBucket>, ids: Option<(&str, &str)>| {
        Rc::new(CompareManifestSnapshot {
            files: Arc::from(vec![ManifestFile {
                buckets,
                old_id: ids.and_then(|(old, _)| diffy_diff::ObjectId::parse(old)),
                new_id: ids.and_then(|(_, new)| diffy_diff::ObjectId::parse(new)),
                ..ManifestFile::new(
                    FILE,
                    REVISION,
                    CompareFilePaths::from_paths(Some("f.txt"), Some("f.txt")),
                    diffy_diff::FileStatus::Modified,
                )
            }]),
            working_tree: true,
            ..CompareManifestSnapshot::default()
        })
    };

    cx.manifest = Some(entry(Vec::new(), None));
    let refs = controller.reduce_action(MutationsAction::StageHunkAt(hunk()), &cx);
    assert!(refs.effects.is_empty());

    cx.manifest = Some(entry(vec![ChangeBucket::Staged], None));
    let nothing = controller.reduce_action(MutationsAction::StageHunkAt(hunk()), &cx);
    assert!(nothing.effects.is_empty());

    cx.manifest = Some(entry(
        vec![ChangeBucket::Unstaged],
        Some(("1111111111", "2222222222")),
    ));
    let load = controller.reduce_action(MutationsAction::StageHunkAt(hunk()), &cx);
    let effects = mutation_effects(&load);
    let [MutationsEffect::LoadMutationSource(key)] = effects.as_slice() else {
        panic!("expected a source load");
    };
    let mismatched = Arc::new(MutationSource::new(
        key.clone(),
        MutationMaterial {
            patch: Arc::from(&b"--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n+b\n"[..]),
            identity: SourceIdentity {
                old: diffy_diff::ObjectId::parse("3333333333"),
                new: diffy_diff::ObjectId::parse("2222222222"),
            },
        },
    ));
    let rejected = controller.reduce_event(
        MutationsEvent::MutationSourceReady { source: mismatched },
        &cx,
    );
    assert!(mutation_effects(&rejected).is_empty());
    assert!(!controller.busy());

    let load = controller.reduce_action(MutationsAction::StageHunkAt(hunk()), &cx);
    let effects = mutation_effects(&load);
    let [MutationsEffect::LoadMutationSource(key)] = effects.as_slice() else {
        panic!("expected a fresh source load");
    };
    let matching = Arc::new(MutationSource::new(
        key.clone(),
        MutationMaterial {
            patch: Arc::from(&b"--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n+b\n"[..]),
            identity: SourceIdentity {
                old: diffy_diff::ObjectId::parse("1111111"),
                new: diffy_diff::ObjectId::parse("2222222222"),
            },
        },
    ));
    let applied = controller.reduce_event(
        MutationsEvent::MutationSourceReady { source: matching },
        &cx,
    );
    assert!(matches!(
        mutation_effects(&applied).as_slice(),
        [MutationsEffect::ApplyPatch(_)]
    ));
}
