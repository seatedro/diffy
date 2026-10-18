use std::path::PathBuf;
use std::sync::Arc;

use diffy_diff::{DiffStats, FileStatus};

use super::PipelineController;
use super::state::{PipelineStatus, ViewportInstall};
use crate::apprt::services::viewport::pipeline::compare_result;
use crate::contracts::navigation::{SelectedFile, SelectionOrigin};
use crate::contracts::shared::{CompareGeneration, FileKey, FileRevision, SharedContext};
use crate::contracts::shell::{Controller, Effect, Intent, Reduction};
use crate::contracts::viewport::ViewportEffect;
use crate::contracts::viewport::pipeline::{
    CompareIntent, CompareWorkPriority, FileStat, PipelineAction, PipelineEffect, PipelineEvent,
    PipelineFailure, PreparedDiff,
};
use crate::core::compare::{CompareEntry, CompareOutput, LayoutMode, RendererKind};
use crate::core::vcs::model::{VcsCompareRequest, VcsCompareSpec};

const PATCH: &str = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-x\n+y\n";

fn intent(renderer: RendererKind) -> CompareIntent {
    CompareIntent {
        repo_path: PathBuf::from("/repo"),
        request: VcsCompareRequest {
            spec: VcsCompareSpec::WorkingCopy,
            layout: LayoutMode::Unified,
            renderer,
        },
    }
}

fn pipeline_effects(reduction: &Reduction) -> Vec<&PipelineEffect> {
    reduction
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Viewport(ViewportEffect::Pipeline(effect)) => Some(effect),
            _ => None,
        })
        .collect()
}

fn request(controller: &mut PipelineController) -> CompareGeneration {
    let reduction = controller.apply_intent(
        &Intent::RequestCompare(intent(RendererKind::Builtin)),
        &SharedContext::default(),
    );
    match pipeline_effects(&reduction).as_slice() {
        [PipelineEffect::RunCompare(job)] => job.generation,
        other => panic!("expected one compare job, got {other:?}"),
    }
}

fn finished(generation: CompareGeneration) -> PipelineEvent {
    let mut output = CompareOutput::from_patch(PATCH).unwrap();
    output.push_entry(CompareEntry::deferred(
        Some("b.rs"),
        Some("b.rs"),
        FileStatus::Modified,
    ));
    PipelineEvent::CompareFinished {
        generation,
        request: intent(RendererKind::Builtin).request,
        result: compare_result(generation, ("l".into(), "r".into()), output).unwrap(),
    }
}

fn published(reduction: &Reduction) -> Option<usize> {
    reduction.intents.iter().find_map(|intent| match intent {
        Intent::PublishManifest(manifest) => Some(manifest.files.len()),
        _ => None,
    })
}

fn select(key: u64) -> Intent {
    Intent::SelectFile(Some(SelectedFile {
        key: FileKey(key),
        origin: SelectionOrigin::Sidebar,
    }))
}

fn loaded(generation: CompareGeneration, key: u64) -> PreparedDiff {
    let output = CompareOutput::from_patch(PATCH).unwrap();
    crate::apprt::services::viewport::pipeline::prepare(
        FileKey(key),
        generation,
        FileRevision(generation.0),
        output.document(0).unwrap().clone(),
    )
    .unwrap()
}

#[test]
fn stale_compare_generations_are_rejected() {
    let cx = SharedContext::default();
    let mut controller = PipelineController::default();
    let first = request(&mut controller);
    let second = request(&mut controller);
    assert!(second > first);
    assert!(controller.take_installs().len() == 2);

    let late = controller.reduce_event(finished(first), &cx);
    assert!(!late.changed && late.intents.is_empty() && late.effects.is_empty());
    assert!(controller.take_installs().is_empty());
    assert_eq!(controller.status(), &PipelineStatus::Comparing(None));

    let failed_late = controller.reduce_event(
        PipelineEvent::CompareFailed {
            generation: first,
            failure: PipelineFailure::Failed("gone".into()),
        },
        &cx,
    );
    assert!(failed_late.intents.is_empty());

    let current = controller.reduce_event(finished(second), &cx);
    assert_eq!(published(&current), Some(2));
    assert_eq!(controller.status(), &PipelineStatus::Ready);
    let installs = controller.take_installs();
    assert!(matches!(
        installs.as_slice(),
        [ViewportInstall::Order { generation, files }, ViewportInstall::File { prepared, .. }]
            if *generation == second && files.len() == 2 && prepared.key() == FileKey(0)
    ));

    let old_file = controller.reduce_event(
        PipelineEvent::FileLoaded {
            generation: first,
            prepared: loaded(first, 1),
        },
        &cx,
    );
    assert!(!old_file.changed);
    let mismatched = controller.reduce_event(
        PipelineEvent::FileLoaded {
            generation: second,
            prepared: loaded(first, 1),
        },
        &cx,
    );
    assert!(!mismatched.changed);
    assert!(controller.take_installs().is_empty());
    assert!(controller.prepared(FileKey(1)).is_none());
}

#[test]
fn selecting_a_file_loads_it_once_at_interactive_priority() {
    let cx = SharedContext::default();
    let mut controller = PipelineController::default();
    let generation = request(&mut controller);
    let early = controller.apply_intent(&select(1), &cx);
    assert!(pipeline_effects(&early).is_empty());

    let done = controller.reduce_event(finished(generation), &cx);
    match pipeline_effects(&done).as_slice() {
        [PipelineEffect::LoadFile(load)] => {
            assert_eq!(load.priority, CompareWorkPriority::InteractiveSelectedFile);
            assert_eq!(load.file.key, FileKey(1));
            assert_eq!(load.file.entry.path(), "b.rs");
            assert_eq!(load.generation, generation);
        }
        other => panic!("expected a file load, got {other:?}"),
    }
    assert!(pipeline_effects(&controller.apply_intent(&select(1), &cx)).is_empty());
    assert!(pipeline_effects(&controller.apply_intent(&select(0), &cx)).is_empty());
    controller.take_installs();

    let result = controller.reduce_event(
        PipelineEvent::FileLoaded {
            generation,
            prepared: loaded(generation, 1),
        },
        &cx,
    );
    assert!(result.changed);
    assert_eq!(published(&result), Some(2));
    let manifest = controller.manifest().unwrap();
    assert_eq!(
        manifest.file(FileKey(1)).unwrap().stats,
        DiffStats::Known {
            additions: 1,
            deletions: 1
        }
    );
    assert!(matches!(
        controller.take_installs().as_slice(),
        [ViewportInstall::File { prepared, .. }] if prepared.key() == FileKey(1)
    ));
}

#[test]
fn pending_stats_are_requested_once_and_republished() {
    let cx = SharedContext::default();
    let mut controller = PipelineController::default();
    let generation = request(&mut controller);
    controller.reduce_event(finished(generation), &cx);
    let ask = || PipelineAction::RequestStats {
        files: vec![FileKey(0), FileKey(1)],
        priority: CompareWorkPriority::VisibleSidebarStats,
    };
    let first = controller.reduce_action(ask(), &cx);
    match pipeline_effects(&first).as_slice() {
        [PipelineEffect::LoadFileStats(stats)] => {
            assert_eq!(stats.files.len(), 1);
            assert_eq!(stats.files[0].key, FileKey(1));
        }
        other => panic!("expected one stats load, got {other:?}"),
    }
    assert!(pipeline_effects(&controller.reduce_action(ask(), &cx)).is_empty());
    let ready = controller.reduce_event(
        PipelineEvent::FileStatsReady {
            generation,
            stats: Arc::from([FileStat {
                key: FileKey(1),
                additions: 4,
                deletions: 2,
            }]),
            complete: true,
        },
        &cx,
    );
    assert_eq!(published(&ready), Some(2));
    assert_eq!(
        controller
            .manifest()
            .unwrap()
            .file(FileKey(1))
            .unwrap()
            .stats,
        DiffStats::Known {
            additions: 4,
            deletions: 2
        }
    );
    let stale = controller.reduce_event(
        PipelineEvent::FileStatsReady {
            generation: CompareGeneration(generation.0 - 1),
            stats: Arc::from([]),
            complete: true,
        },
        &cx,
    );
    assert!(!stale.changed);
}

#[test]
fn renderer_changes_rerun_the_current_compare() {
    let cx = SharedContext::default();
    let mut controller = PipelineController::default();
    let first = request(&mut controller);
    let rerun =
        controller.reduce_action(PipelineAction::SetRenderer(RendererKind::Difftastic), &cx);
    match pipeline_effects(&rerun).as_slice() {
        [PipelineEffect::RunCompare(job)] => {
            assert!(job.generation > first);
            assert_eq!(job.intent.request.renderer, RendererKind::Difftastic);
        }
        other => panic!("expected a new compare, got {other:?}"),
    }
    let same = controller.reduce_action(PipelineAction::SetRenderer(RendererKind::Difftastic), &cx);
    assert!(pipeline_effects(&same).is_empty());
}
