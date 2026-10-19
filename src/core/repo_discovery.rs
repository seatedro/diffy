use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ignore::{DirEntry, WalkBuilder, WalkState};
use serde::{Deserialize, Serialize};

use crate::core::error::Result;
use crate::platform::persistence::RepoDiscovery;

const CACHE_FILE_NAME: &str = "repositories.json";
const REPO_MARKERS: &[&str] = &[".git", ".jj"];
const SKIPPED_DIRS: &[&str] = &[
    "Library",
    "Applications",
    "Movies",
    "Music",
    "Pictures",
    "node_modules",
    "target",
];
const SHORT_SHA: usize = 7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredRepo {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub active: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoScan {
    pub scanned_at: i64,
    pub millis: u64,
    pub repos: Vec<DiscoveredRepo>,
    #[serde(default)]
    pub known: Vec<DiscoveredRepo>,
}

impl RepoScan {
    pub fn find(&self, path: &Path) -> Option<&DiscoveredRepo> {
        self.repos
            .iter()
            .chain(&self.known)
            .find(|repo| repo.path == path)
    }
}

pub fn scan(options: &RepoDiscovery, known: &[PathBuf], home: Option<&Path>) -> RepoScan {
    let started = Instant::now();
    let roots: Vec<PathBuf> = options
        .roots
        .iter()
        .map(|root| expand_home(root, home))
        .filter(|root| root.is_dir())
        .collect();
    let found = Arc::new(Mutex::new(BTreeSet::new()));
    if let Some((first, rest)) = roots.split_first() {
        let mut builder = WalkBuilder::new(first);
        for root in rest {
            builder.add(root);
        }
        let hidden = options.hidden;
        builder
            .standard_filters(false)
            .follow_links(false)
            .max_depth(Some(usize::from(options.depth) + 1))
            .filter_entry(move |entry| keep(entry, hidden));
        builder.build_parallel().run(|| {
            let found = found.clone();
            Box::new(move |entry| {
                let Ok(entry) = entry else {
                    return WalkState::Continue;
                };
                if !is_marker(&entry) {
                    return WalkState::Continue;
                }
                if let (Some(repo), Ok(mut found)) = (entry.path().parent(), found.lock()) {
                    found.insert(repo.to_path_buf());
                }
                WalkState::Skip
            })
        });
    }
    let paths = Arc::try_unwrap(found)
        .ok()
        .and_then(|found| found.into_inner().ok())
        .unwrap_or_default();
    let known = known
        .iter()
        .filter(|path| !paths.contains(*path))
        .map(|path| describe(path.clone()))
        .collect();
    let repos = paths.into_iter().map(describe).collect();
    RepoScan {
        scanned_at: unix_secs(SystemTime::now()).unwrap_or_default(),
        millis: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        repos,
        known,
    }
}

fn describe(path: PathBuf) -> DiscoveredRepo {
    let git = git_dir(&path);
    DiscoveredRepo {
        branch: git.as_deref().and_then(branch),
        active: git.as_deref().and_then(last_active),
        path,
    }
}

pub fn load_cache(dir: &Path) -> Option<RepoScan> {
    let bytes = fs::read(dir.join(CACHE_FILE_NAME)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn save_cache(dir: &Path, scan: &RepoScan) -> Result<()> {
    fs::create_dir_all(dir)?;
    let path = dir.join(CACHE_FILE_NAME);
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(scan)?;
    fs::write(&temp, bytes)?;
    fs::rename(&temp, &path)?;
    Ok(())
}

pub fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix('~'), home) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches(['/', '\\'])),
        _ => PathBuf::from(path),
    }
}

fn keep(entry: &DirEntry, hidden: bool) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if !entry.file_type().is_some_and(|kind| kind.is_dir()) {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    if REPO_MARKERS.contains(&name.as_ref()) {
        return true;
    }
    if !hidden && name.starts_with('.') {
        return false;
    }
    !SKIPPED_DIRS.contains(&name.as_ref())
}

fn is_marker(entry: &DirEntry) -> bool {
    entry.depth() > 0
        && REPO_MARKERS
            .iter()
            .any(|marker| entry.file_name() == std::ffi::OsStr::new(marker))
}

fn git_dir(repo: &Path) -> Option<PathBuf> {
    let dot_git = repo.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let pointer = fs::read_to_string(&dot_git).ok()?;
    let target = pointer.strip_prefix("gitdir:")?.trim();
    let target = Path::new(target);
    Some(if target.is_absolute() {
        target.to_path_buf()
    } else {
        repo.join(target)
    })
}

fn branch(git_dir: &Path) -> Option<String> {
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref:") {
        Some(reference) => {
            let reference = reference.trim();
            Some(
                reference
                    .strip_prefix("refs/heads/")
                    .unwrap_or(reference)
                    .to_owned(),
            )
        }
        None => head.get(..SHORT_SHA).map(str::to_owned),
    }
}

fn last_active(git_dir: &Path) -> Option<i64> {
    ["index", "logs/HEAD", "HEAD"]
        .iter()
        .filter_map(|name| fs::metadata(git_dir.join(name)).ok()?.modified().ok())
        .filter_map(unix_secs)
        .max()
}

fn unix_secs(time: SystemTime) -> Option<i64> {
    let secs = time.duration_since(UNIX_EPOCH).ok()?.as_secs();
    i64::try_from(secs).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn repo(root: &Path, path: &str, head: &str) {
        let git = root.join(path).join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), head).unwrap();
    }

    #[test]
    fn finds_repos_skips_hidden_and_heavy_folders_and_reads_branches() {
        let home = TempDir::new().unwrap();
        let root = home.path();
        repo(root, "code/alpha", "ref: refs/heads/main\n");
        repo(root, "code/alpha/nested", "ref: refs/heads/dev\n");
        repo(root, "work/beta", "0123456789abcdef\n");
        repo(root, ".config/hidden", "ref: refs/heads/main\n");
        repo(root, "node_modules/pkg", "ref: refs/heads/main\n");
        repo(root, "a/b/c/d/e/f/deep", "ref: refs/heads/main\n");

        let options = RepoDiscovery::default();
        let result = scan(&options, &[], Some(root));
        let found: Vec<(String, Option<String>)> = result
            .repos
            .iter()
            .map(|repo| {
                (
                    repo.path.strip_prefix(root).unwrap().display().to_string(),
                    repo.branch.clone(),
                )
            })
            .collect();
        assert_eq!(
            found,
            vec![
                ("code/alpha".to_owned(), Some("main".to_owned())),
                ("code/alpha/nested".to_owned(), Some("dev".to_owned())),
                ("work/beta".to_owned(), Some("0123456".to_owned())),
            ]
        );

        let hidden = RepoDiscovery {
            hidden: true,
            ..RepoDiscovery::default()
        };
        assert!(
            scan(&hidden, &[], Some(root))
                .repos
                .iter()
                .any(|repo| repo.path.ends_with(".config/hidden"))
        );
    }

    #[test]
    fn cache_round_trips() {
        let dir = TempDir::new().unwrap();
        let scan = RepoScan {
            scanned_at: 42,
            millis: 7,
            repos: vec![DiscoveredRepo {
                path: PathBuf::from("/repo"),
                branch: Some("main".to_owned()),
                active: Some(40),
            }],
            known: Vec::new(),
        };
        save_cache(dir.path(), &scan).unwrap();
        assert_eq!(load_cache(dir.path()), Some(scan));
    }
}
