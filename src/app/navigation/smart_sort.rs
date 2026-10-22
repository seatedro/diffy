use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use crate::ai::rank::{MAX_RANK_FILES, RankCandidate};
use crate::contracts::navigation::RankFilesRequest;
use crate::contracts::shared::{CompareGeneration, JobId};
use crate::contracts::viewport::pipeline::{CompareManifestSnapshot, ManifestFile};
use crate::core::file_rank::{self, Tier};
use diffy_diff::FileStatus;

const CACHE_LIMIT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortStatus {
    #[default]
    Heuristic,
    Ranking,
    Ranked,
    Unconfigured,
    Failed,
}

impl SortStatus {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Heuristic => "ranked by path",
            Self::Ranking => "ranking with AI\u{2026}",
            Self::Ranked => "ranked by AI",
            Self::Unconfigured => "ranked by path, add an AI key in Settings to use AI",
            Self::Failed => "AI ranking failed, ranked by path",
        }
    }
}

#[derive(Debug)]
struct Pending {
    job: JobId,
    key: u64,
    paths: Vec<String>,
}

pub enum Finish {
    Stale,
    Applied,
    Cached,
    Failed(String),
}

#[derive(Debug, Default)]
pub struct SmartSort {
    enabled: bool,
    status: SortStatus,
    generation: Option<CompareGeneration>,
    key: u64,
    heuristic: Vec<u32>,
    order: Option<Rc<[u32]>>,
    pending: Option<Pending>,
    cache: Vec<(u64, Rc<[String]>)>,
    next_job: u64,
    notified: bool,
}

impl SmartSort {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn status(&self) -> SortStatus {
        self.status
    }

    pub fn order(&self) -> Option<&Rc<[u32]>> {
        self.order.as_ref().filter(|_| self.enabled)
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.generation = None;
        self.order = None;
        if enabled {
            self.notified = false;
        }
    }

    pub fn reset(&mut self) {
        self.generation = None;
        self.order = None;
        self.pending = None;
        self.status = SortStatus::Heuristic;
    }

    pub fn first_notice(&mut self) -> bool {
        !std::mem::replace(&mut self.notified, true)
    }

    pub fn refresh(&mut self, manifest: &CompareManifestSnapshot) -> Option<RankFilesRequest> {
        if !self.enabled || self.generation == Some(manifest.generation) {
            return None;
        }
        self.generation = Some(manifest.generation);
        let paths = display_paths(&manifest.files);
        let tiers: Vec<(Tier, &str)> = manifest
            .files
            .iter()
            .zip(&paths)
            .map(|(file, path)| (file_rank::tier(path, file.binary), path.as_str()))
            .collect();
        self.heuristic = file_rank::heuristic_order(&tiers);
        self.key = rank_key(manifest, &paths);
        if let Some(ranked) = self.cached(self.key) {
            self.apply(&paths, &ranked);
            self.status = SortStatus::Ranked;
            return None;
        }
        self.order = Some(self.heuristic.as_slice().into());
        if self.pending.as_ref().is_some_and(|p| p.key == self.key) {
            self.status = SortStatus::Ranking;
            return None;
        }
        let candidates: Vec<usize> = self
            .heuristic
            .iter()
            .map(|&i| i as usize)
            .filter(|&i| tiers[i].0.worth_ranking())
            .take(MAX_RANK_FILES)
            .collect();
        if candidates.len() < 2 {
            self.pending = None;
            self.status = SortStatus::Heuristic;
            return None;
        }
        self.next_job += 1;
        let job = JobId(self.next_job);
        let files = candidates
            .iter()
            .map(|&i| candidate(&manifest.files[i], &paths[i]))
            .collect();
        self.pending = Some(Pending {
            job,
            key: self.key,
            paths: candidates.iter().map(|&i| paths[i].clone()).collect(),
        });
        self.status = SortStatus::Ranking;
        Some(RankFilesRequest { job, files })
    }

    pub fn finish(
        &mut self,
        job: JobId,
        ranking: Result<Vec<u32>, String>,
        manifest: Option<&CompareManifestSnapshot>,
    ) -> Finish {
        let Some(pending) = self.pending.take_if(|p| p.job == job) else {
            return Finish::Stale;
        };
        let current = pending.key == self.key;
        let ranked = match ranking {
            Ok(ranked) => ranked,
            Err(_) if !current || !self.enabled => return Finish::Stale,
            Err(message) => {
                self.status = SortStatus::Failed;
                return Finish::Failed(message);
            }
        };
        let ranked: Rc<[String]> = ranked
            .iter()
            .filter_map(|&i| pending.paths.get(i as usize).cloned())
            .collect();
        self.store(pending.key, ranked.clone());
        match manifest {
            Some(manifest) if current && self.generation.is_some() => {
                self.apply(&display_paths(&manifest.files), &ranked);
                self.status = SortStatus::Ranked;
                Finish::Applied
            }
            _ => Finish::Cached,
        }
    }

    pub fn unavailable(&mut self, job: JobId) -> bool {
        if self.pending.take_if(|p| p.job == job).is_none() {
            return false;
        }
        self.status = SortStatus::Unconfigured;
        true
    }

    fn apply(&mut self, paths: &[String], ranked: &[String]) {
        let index: HashMap<&str, u32> = paths
            .iter()
            .enumerate()
            .map(|(i, path)| (path.as_str(), i as u32))
            .collect();
        let ranked: Vec<u32> = ranked
            .iter()
            .filter_map(|path| index.get(path.as_str()).copied())
            .collect();
        self.order = Some(file_rank::merge_ranked(&self.heuristic, &ranked).into());
    }

    fn cached(&mut self, key: u64) -> Option<Rc<[String]>> {
        let at = self.cache.iter().position(|(k, _)| *k == key)?;
        let entry = self.cache.remove(at);
        let ranked = entry.1.clone();
        self.cache.push(entry);
        Some(ranked)
    }

    fn store(&mut self, key: u64, ranked: Rc<[String]>) {
        self.cache.retain(|(k, _)| *k != key);
        if self.cache.len() == CACHE_LIMIT {
            self.cache.remove(0);
        }
        self.cache.push((key, ranked));
    }
}

fn display_paths(files: &[ManifestFile]) -> Vec<String> {
    files
        .iter()
        .map(|file| file.paths.display_path().into_owned())
        .collect()
}

fn candidate(file: &ManifestFile, path: &str) -> RankCandidate {
    let old_path = matches!(file.status, FileStatus::Renamed | FileStatus::Copied)
        .then(|| file.paths.old_path().map(|old| old.into_owned()))
        .flatten()
        .filter(|old| old != path);
    RankCandidate {
        path: path.to_owned(),
        old_path,
        status: file_rank::status_code(file.status),
        stats: file.stats.known(),
    }
}

fn rank_key(manifest: &CompareManifestSnapshot, paths: &[String]) -> u64 {
    let mut hasher = DefaultHasher::new();
    manifest.repo_path.hash(&mut hasher);
    manifest.resolved_left.hash(&mut hasher);
    manifest.resolved_right.hash(&mut hasher);
    manifest.working_tree.hash(&mut hasher);
    for (file, path) in manifest.files.iter().zip(paths) {
        path.hash(&mut hasher);
        file.status.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::contracts::shared::{FileKey, FileRevision};
    use crate::core::compare::stats::CompareFilePaths;

    fn manifest(generation: u64, paths: &[&str]) -> CompareManifestSnapshot {
        let files: Arc<[ManifestFile]> = paths
            .iter()
            .enumerate()
            .map(|(i, path)| {
                ManifestFile::new(
                    FileKey(i as u64 + 1),
                    FileRevision(1),
                    CompareFilePaths::from_paths(Some(path), Some(path)),
                    FileStatus::Modified,
                )
            })
            .collect();
        CompareManifestSnapshot {
            generation: CompareGeneration(generation),
            resolved_left: "base".to_owned(),
            resolved_right: "head".to_owned(),
            files,
            ..CompareManifestSnapshot::default()
        }
    }

    const PATHS: [&str; 5] = [
        "README.md",
        "src/view/panel.rs",
        "Cargo.lock",
        "src/main.rs",
        "src/model/store.rs",
    ];

    fn enabled() -> SmartSort {
        let mut sort = SmartSort::default();
        sort.set_enabled(true);
        sort
    }

    #[test]
    fn sends_only_rankable_files_and_orders_by_heuristics_meanwhile() {
        let mut sort = enabled();
        let manifest = manifest(1, &PATHS);
        let request = sort.refresh(&manifest).expect("request");
        let sent: Vec<&str> = request.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            sent,
            ["src/main.rs", "src/model/store.rs", "src/view/panel.rs"]
        );
        assert_eq!(sort.order().map(|o| o.to_vec()), Some(vec![3, 4, 1, 0, 2]));
        assert_eq!(sort.status(), SortStatus::Ranking);
        assert!(sort.refresh(&manifest).is_none());
    }

    #[test]
    fn applies_ranking_and_reuses_the_cache_for_the_same_file_set() {
        let mut sort = enabled();
        let first = manifest(1, &PATHS);
        let request = sort.refresh(&first).expect("request");
        assert!(matches!(
            sort.finish(request.job, Ok(vec![2, 9, 0]), Some(&first)),
            Finish::Applied
        ));
        assert_eq!(sort.order().map(|o| o.to_vec()), Some(vec![1, 3, 4, 0, 2]));
        assert_eq!(sort.status(), SortStatus::Ranked);

        sort.set_enabled(false);
        assert!(sort.order().is_none());
        sort.set_enabled(true);
        let again = manifest(2, &PATHS);
        assert!(sort.refresh(&again).is_none(), "cache hit skips the model");
        assert_eq!(sort.order().map(|o| o.to_vec()), Some(vec![1, 3, 4, 0, 2]));
    }

    #[test]
    fn stale_jobs_and_failures_keep_the_heuristic_order() {
        let mut sort = enabled();
        let first = manifest(1, &PATHS);
        let old = sort.refresh(&first).expect("request");
        let second = manifest(2, &PATHS[..4]);
        let new = sort.refresh(&second).expect("request");
        assert!(matches!(
            sort.finish(old.job, Ok(vec![0]), Some(&second)),
            Finish::Stale
        ));
        assert!(matches!(
            sort.finish(new.job, Err("boom".to_owned()), Some(&second)),
            Finish::Failed(_)
        ));
        assert_eq!(sort.status(), SortStatus::Failed);
        assert_eq!(sort.order().map(|o| o.to_vec()), Some(vec![3, 1, 0, 2]));
    }

    #[test]
    fn unconfigured_ai_falls_back_without_a_pending_job() {
        let mut sort = enabled();
        let request = sort.refresh(&manifest(1, &PATHS)).expect("request");
        assert!(sort.unavailable(request.job));
        assert!(!sort.unavailable(request.job));
        assert_eq!(sort.status(), SortStatus::Unconfigured);
        assert!(sort.first_notice());
        assert!(!sort.first_notice());
    }

    #[test]
    fn small_or_low_value_compares_skip_the_model() {
        let mut sort = enabled();
        assert!(
            sort.refresh(&manifest(1, &["src/main.rs", "README.md", "Cargo.lock"]))
                .is_none()
        );
        assert_eq!(sort.status(), SortStatus::Heuristic);
    }
}
