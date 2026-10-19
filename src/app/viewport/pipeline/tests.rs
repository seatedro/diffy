use std::path::PathBuf;
use std::sync::Arc;

use diffy_diff::{DiffStats, FileStatus};

use super::PipelineController;
use super::state::{
    INSTALL_LINES_PER_FRAME, PipelineInstall, PipelineStatus, ViewportInstall, WORKING_SET_LINES,
};
use crate::apprt::services::viewport::pipeline::compare_result;
use crate::contracts::navigation::{SelectedFile, SelectionOrigin};
use crate::contracts::shared::{CompareGeneration, FileKey, FileRevision, SharedContext};
use crate::contracts::shell::{Controller, Effect, Intent, Reduction};
use crate::contracts::viewport::ViewportEffect;
use crate::contracts::viewport::pipeline::SlotUpdate;
use crate::contracts::viewport::pipeline::{
    CompareIntent, CompareWorkPriority, FileStat, PipelineAction, PipelineEffect, PipelineEvent,
    PipelineFailure, PreparedDiff,
};
use crate::core::compare::{CompareEntry, CompareOutput, LayoutMode, RendererKind};
use crate::core::vcs::model::{VcsCompareRequest, VcsCompareSpec};
use diffy_diff::{SlotState, SourcePurpose};

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
        result: compare_result(generation, ("l".into(), "r".into()), output, None).unwrap(),
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
    assert!(matches!(
        controller.take_installs().as_slice(),
        [ViewportInstall::Begin { generation }] if *generation == second
    ));

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
    let installs = controller.take_batch();
    assert!(matches!(
        installs.as_slice(),
        [
            PipelineInstall::View(ViewportInstall::File { prepared, .. }),
            PipelineInstall::Slot(SlotUpdate::Placeholder { key: FileKey(1), placeholder }),
            PipelineInstall::View(ViewportInstall::Order { generation, files }),
        ] if *generation == second
            && files.len() == 2
            && prepared.key() == FileKey(0)
            && placeholder.state == SlotState::Unloaded
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
    let loads: Vec<_> = pipeline_effects(&done)
        .into_iter()
        .filter(|effect| matches!(effect, PipelineEffect::LoadFile(_)))
        .collect();
    match loads.as_slice() {
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
fn renderer_changes_rerun_the_current_compare_and_persist() {
    let cx = SharedContext::default();
    let mut controller = PipelineController::default();
    let persisted = |reduction: &Reduction| {
        reduction.intents.iter().find_map(|intent| match intent {
            Intent::SettingsChanged(snapshot) => snapshot
                .settings
                .last_compare
                .as_ref()
                .map(|compare| compare.renderer),
            _ => None,
        })
    };
    let idle = controller.reduce_action(PipelineAction::SetRenderer(RendererKind::Semantic), &cx);
    assert!(pipeline_effects(&idle).is_empty());
    let first = request(&mut controller);
    let rerun = controller.reduce_action(PipelineAction::SetRenderer(RendererKind::Semantic), &cx);
    match pipeline_effects(&rerun).as_slice() {
        [PipelineEffect::RunCompare(job)] => {
            assert!(job.generation > first);
            assert_eq!(job.intent.request.renderer, RendererKind::Semantic);
        }
        other => panic!("expected a new compare, got {other:?}"),
    }
    let saved = std::rc::Rc::new(crate::contracts::commands::SettingsSnapshot {
        revision: 1,
        settings: crate::platform::persistence::Settings {
            last_compare: Some(crate::platform::persistence::PersistedCompare {
                renderer: RendererKind::Semantic,
                ..Default::default()
            }),
            ..Default::default()
        },
    });
    let cx = SharedContext {
        settings: saved,
        ..SharedContext::default()
    };
    let same = controller.reduce_action(PipelineAction::SetRenderer(RendererKind::Semantic), &cx);
    assert!(pipeline_effects(&same).is_empty());
    assert_eq!(persisted(&same), None);
    let back = controller.reduce_action(PipelineAction::SetRenderer(RendererKind::Builtin), &cx);
    assert_eq!(persisted(&back), Some(RendererKind::Builtin));
    assert_eq!(persisted(&idle), Some(RendererKind::Semantic));
    assert_eq!(persisted(&rerun), Some(RendererKind::Semantic));
}

#[test]
fn an_empty_compare_publishes_an_empty_manifest_and_finishes() {
    let cx = SharedContext::default();
    let mut controller = PipelineController::default();
    let generation = request(&mut controller);
    controller.take_batch();
    let done = controller.reduce_event(
        PipelineEvent::CompareFinished {
            generation,
            request: intent(RendererKind::Builtin).request,
            result: compare_result(
                generation,
                ("l".into(), "r".into()),
                CompareOutput::default(),
                None,
            )
            .unwrap(),
        },
        &cx,
    );
    assert_eq!(published(&done), Some(0));
    assert_eq!(controller.status(), &PipelineStatus::Ready);
    assert!(controller.manifest().unwrap().working_tree);
    assert!(matches!(
        controller.take_batch().as_slice(),
        [PipelineInstall::View(ViewportInstall::Order { generation: g, files })]
            if *g == generation && files.is_empty()
    ));
    assert!(pipeline_effects(&done).is_empty());
}

fn big(generation: CompareGeneration, key: u64, lines: usize) -> PreparedDiff {
    let old: String = (0..lines).map(|i| format!("{i}\n")).collect();
    let doc = diffy_diff::diff_texts(Some("f"), Some("f"), Some(""), Some(&old), 3);
    crate::apprt::services::viewport::pipeline::prepare(
        FileKey(key),
        generation,
        FileRevision(generation.0),
        Arc::new(doc),
    )
    .unwrap()
}

fn deferred_compare(controller: &mut PipelineController, files: usize) -> CompareGeneration {
    let generation = request(controller);
    let mut output = CompareOutput::default();
    for i in 0..files {
        let path = format!("f{i}.rs");
        output.push_entry(CompareEntry::deferred(
            Some(&path),
            Some(&path),
            FileStatus::Modified,
        ));
    }
    controller.reduce_event(
        PipelineEvent::CompareFinished {
            generation,
            request: intent(RendererKind::Builtin).request,
            result: compare_result(generation, ("l".into(), "r".into()), output, None).unwrap(),
        },
        &SharedContext::default(),
    );
    generation
}

#[test]
fn large_installs_spread_across_frames() {
    let cx = SharedContext::default();
    let mut controller = PipelineController::default();
    let generation = deferred_compare(&mut controller, 3);
    controller.take_batch();
    let lines = INSTALL_LINES_PER_FRAME as usize / 2 + 1;
    let mut paced = 0;
    for key in 0..3 {
        let reduction = controller.reduce_event(
            PipelineEvent::FileLoaded {
                generation,
                prepared: big(generation, key, lines),
            },
            &cx,
        );
        paced += pipeline_effects(&reduction)
            .iter()
            .filter(|effect| matches!(effect, PipelineEffect::ResumeInstalls(_)))
            .count();
    }
    assert_eq!(paced, 1);
    let mut frames = 0;
    while controller.has_pending_installs() {
        let batch = controller.take_batch();
        assert!(!batch.is_empty());
        frames += 1;
    }
    assert!(frames >= 2, "{frames} frames");
    let resumed = controller.reduce_event(PipelineEvent::InstallsResumed { generation }, &cx);
    assert!(resumed.changed);
}

#[test]
fn the_working_set_evicts_least_recently_used_files_but_keeps_the_selection() {
    let cx = SharedContext::default();
    let mut controller = PipelineController::default();
    let generation = deferred_compare(&mut controller, 3);
    let lines = WORKING_SET_LINES as usize / 2;
    controller.apply_intent(&select(0), &cx);
    for key in 0..3 {
        controller.reduce_event(
            PipelineEvent::FileLoaded {
                generation,
                prepared: big(generation, key, lines),
            },
            &cx,
        );
    }
    let evicted: Vec<_> = std::iter::from_fn(|| {
        controller
            .has_pending_installs()
            .then(|| controller.take_batch())
    })
    .flatten()
    .filter_map(|install| match install {
        PipelineInstall::Slot(SlotUpdate::Evict { key, .. }) => Some(key),
        _ => None,
    })
    .collect();
    assert_eq!(evicted, [FileKey(1)]);
    assert!(controller.prepared(FileKey(0)).is_some());
    assert!(controller.prepared(FileKey(1)).is_none());
    assert_eq!(controller.slot_state(FileKey(1)), Some(SlotState::Evicted));
    let reload = controller.apply_intent(&select(1), &cx);
    assert!(matches!(
        pipeline_effects(&reload).as_slice(),
        [PipelineEffect::LoadFile(load)] if load.file.key == FileKey(1)
    ));
    assert_eq!(controller.slot_state(FileKey(1)), Some(SlotState::Loading));
}

#[test]
fn totals_load_after_a_deferred_compare_and_republish() {
    let cx = SharedContext::default();
    let mut controller = PipelineController::default();
    let generation = request(&mut controller);
    let mut output = CompareOutput::default();
    output.push_entry(CompareEntry::deferred(
        Some("a"),
        Some("a"),
        FileStatus::Modified,
    ));
    let done = controller.reduce_event(
        PipelineEvent::CompareFinished {
            generation,
            request: intent(RendererKind::Builtin).request,
            result: compare_result(generation, ("l".into(), "r".into()), output, None).unwrap(),
        },
        &cx,
    );
    assert!(
        pipeline_effects(&done)
            .iter()
            .any(|effect| matches!(effect, PipelineEffect::LoadTotalStats(_)))
    );
    let again = controller.reduce_action(PipelineAction::RequestTotals, &cx);
    assert!(pipeline_effects(&again).is_empty());
    let ready = controller.reduce_event(
        PipelineEvent::TotalStatsReady {
            generation,
            additions: 10,
            deletions: 4,
        },
        &cx,
    );
    assert_eq!(published(&ready), Some(1));
    assert_eq!(controller.manifest().unwrap().totals.known(), Some((10, 4)));
}

#[test]
fn source_requests_hydrate_patch_only_files_once() {
    let cx = SharedContext::default();
    let mut controller = PipelineController::default();
    let generation = request(&mut controller);
    controller.reduce_event(finished(generation), &cx);
    let ask = || PipelineAction::RequestSources {
        file: FileKey(0),
        purpose: SourcePurpose::ExpandContext,
    };
    let first = controller.reduce_action(ask(), &cx);
    match pipeline_effects(&first).as_slice() {
        [PipelineEffect::LoadSources(load)] => {
            assert_eq!(load.old_reference, "l");
            assert_eq!(load.new_reference, "r");
            assert_eq!(load.prepared.key(), FileKey(0));
        }
        other => panic!("expected a source load, got {other:?}"),
    }
    assert!(pipeline_effects(&controller.reduce_action(ask(), &cx)).is_empty());
    let missing = controller.reduce_action(
        PipelineAction::RequestSources {
            file: FileKey(1),
            purpose: SourcePurpose::CompleteSyntax,
        },
        &cx,
    );
    assert!(pipeline_effects(&missing).is_empty());
}

#[test]
#[ignore = "release measurement: cargo test --release --lib ui_turns -- --ignored --nocapture"]
fn ui_turns_stay_within_budget_for_50k_deferred_files() {
    use std::time::{Duration, Instant};

    let cx = SharedContext::default();
    let manifest = |files: usize| {
        let mut output = CompareOutput::default();
        for i in 0..files {
            let path = format!("dir{}/sub{}/file{i}.rs", i % 64, i % 7);
            output.push_entry(CompareEntry::deferred(
                Some(&path),
                Some(&path),
                FileStatus::Modified,
            ));
        }
        output.compact_paths();
        output
    };
    let mut worst = Duration::ZERO;
    fn turn(label: &str, work: &mut dyn FnMut()) -> Duration {
        let start = Instant::now();
        work();
        let took = start.elapsed();
        println!("{label}: {:.3} ms", took.as_secs_f64() * 1e3);
        took
    }
    let files = 50_000;
    let mut controller = PipelineController::default();
    let generation = request(&mut controller);
    controller.take_batch();
    let start = Instant::now();
    let result =
        compare_result(generation, ("l".into(), "r".into()), manifest(files), None).unwrap();
    println!(
        "worker manifest, placeholders and install list: {:.1} ms",
        start.elapsed().as_secs_f64() * 1e3
    );
    let mut result = Some(result);
    worst = worst.max(turn("finish compare + first batch", &mut || {
        let result = result.take().unwrap();
        controller.reduce_event(
            PipelineEvent::CompareFinished {
                generation,
                request: intent(RendererKind::Builtin).request,
                result,
            },
            &cx,
        );
        controller.take_batch();
    }));
    let mut resumes = 0;
    while controller.has_pending_installs() {
        resumes += 1;
        let start = Instant::now();
        controller.reduce_event(PipelineEvent::InstallsResumed { generation }, &cx);
        controller.take_batch();
        worst = worst.max(start.elapsed());
    }
    println!("{resumes} resume turns");
    let visible: Arc<[FileStat]> = controller.manifest().unwrap().files[..100]
        .iter()
        .map(|file| FileStat {
            key: file.key,
            additions: 1,
            deletions: 1,
        })
        .collect();
    worst = worst.max(turn("visible sidebar stats", &mut || {
        controller.reduce_event(
            PipelineEvent::FileStatsReady {
                generation,
                stats: visible.clone(),
                complete: false,
            },
            &cx,
        );
    }));
    worst = worst.max(turn("recompare (retires 50k state)", &mut || {
        controller.apply_intent(&Intent::RequestCompare(intent(RendererKind::Builtin)), &cx);
        controller.take_batch();
    }));
    println!("worst UI turn: {:.3} ms", worst.as_secs_f64() * 1e3);
    if !cfg!(debug_assertions) {
        assert!(worst < Duration::from_millis(4), "worst UI turn {worst:?}");
    }
}
