use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ignore::{DirEntry, WalkBuilder, WalkState};
use sea_query::{Expr, ExprTrait, OnConflict, Order, Query, SqliteQueryBuilder};
use sea_query_rusqlite::RusqliteBinder;

use crate::core::error::Result;
use crate::platform::database::{Database, Meta, Repositories};
use crate::platform::persistence::RepoDiscovery;

const SCANNED_AT: &str = "repositories.scanned_at";
const SCAN_MILLIS: &str = "repositories.millis";
const REPO_MARKERS: &[&str] = &[".git", ".jj"];
const SKIPPED_DIRS: &[&str] = &[
    "Library",
    "Applications",
    "Movies",
    "Music",
    "Pictures",
    ".Trash",
    "node_modules",
    "bower_components",
    "jspm_packages",
    ".pnpm-store",
    ".yarn",
    "Pods",
    "Carthage",
    ".venv",
    "venv",
    "__pycache__",
    ".tox",
    ".nox",
    ".mypy_cache",
    ".pytest_cache",
    "site-packages",
    ".gradle",
    ".m2",
    ".ivy2",
    ".terraform",
    ".dart_tool",
    ".pub-cache",
    "elm-stuff",
    ".stack-work",
    "_build",
    "target",
    "build",
    "dist",
    "out",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
    ".parcel-cache",
    "DerivedData",
    ".build",
    "zig-out",
    ".zig-cache",
    "zig-cache",
    ".cache",
    ".npm",
];
const SHORT_SHA: usize = 7;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredRepo {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub active: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoScan {
    pub scanned_at: i64,
    pub millis: u64,
    pub repos: Vec<DiscoveredRepo>,
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
            .max_depth(Some(usize::from(options.depth)))
            .filter_entry(move |entry| keep(entry, hidden));
        builder.build_parallel().run(|| {
            let found = found.clone();
            Box::new(move |entry| {
                let Ok(entry) = entry else {
                    return WalkState::Continue;
                };
                if !is_repo(&entry) {
                    return WalkState::Continue;
                }
                if let Ok(mut found) = found.lock() {
                    found.insert(entry.path().to_path_buf());
                }
                if entry.depth() == 0 {
                    WalkState::Continue
                } else {
                    WalkState::Skip
                }
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
    let db = Database::in_dir(dir).ok()?;
    let meta = |key: &str| -> Option<i64> {
        let select = Query::select()
            .column(Meta::Value)
            .from(Meta::Table)
            .and_where(Expr::col(Meta::Key).eq(key))
            .to_owned();
        db.query(&select, |row| row.get::<_, String>(0))
            .ok()?
            .first()?
            .parse()
            .ok()
    };
    let scanned_at = meta(SCANNED_AT)?;
    let millis = meta(SCAN_MILLIS).and_then(|ms| u64::try_from(ms).ok())?;
    let select = Query::select()
        .columns([
            Repositories::Path,
            Repositories::Branch,
            Repositories::Active,
            Repositories::Discovered,
        ])
        .from(Repositories::Table)
        .order_by(Repositories::Path, Order::Asc)
        .to_owned();
    let rows = db
        .query(&select, |row| {
            Ok((
                DiscoveredRepo {
                    path: PathBuf::from(row.get::<_, String>(0)?),
                    branch: row.get(1)?,
                    active: row.get(2)?,
                },
                row.get::<_, bool>(3)?,
            ))
        })
        .ok()?;
    let (repos, known): (Vec<_>, Vec<_>) = rows.into_iter().partition(|(_, found)| *found);
    Some(RepoScan {
        scanned_at,
        millis,
        repos: repos.into_iter().map(|(repo, _)| repo).collect(),
        known: known.into_iter().map(|(repo, _)| repo).collect(),
    })
}

pub fn save_cache(dir: &Path, scan: &RepoScan) -> Result<()> {
    let db = Database::in_dir(dir)?;
    let mut statements = vec![
        Query::delete()
            .from_table(Repositories::Table)
            .to_owned()
            .build_rusqlite(SqliteQueryBuilder),
    ];
    let rows = scan
        .repos
        .iter()
        .map(|repo| (repo, true))
        .chain(scan.known.iter().map(|repo| (repo, false)));
    for (repo, discovered) in rows {
        statements.push(
            Query::insert()
                .into_table(Repositories::Table)
                .columns([
                    Repositories::Path,
                    Repositories::Branch,
                    Repositories::Active,
                    Repositories::Discovered,
                ])
                .values_panic([
                    repo.path.to_string_lossy().into_owned().into(),
                    repo.branch.clone().into(),
                    repo.active.into(),
                    discovered.into(),
                ])
                .to_owned()
                .build_rusqlite(SqliteQueryBuilder),
        );
    }
    for (key, value) in [
        (SCANNED_AT, scan.scanned_at.to_string()),
        (SCAN_MILLIS, scan.millis.to_string()),
    ] {
        statements.push(
            Query::insert()
                .into_table(Meta::Table)
                .columns([Meta::Key, Meta::Value])
                .values_panic([key.into(), value.into()])
                .on_conflict(
                    OnConflict::column(Meta::Key)
                        .update_column(Meta::Value)
                        .to_owned(),
                )
                .to_owned()
                .build_rusqlite(SqliteQueryBuilder),
        );
    }
    db.transaction(statements)
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
    if REPO_MARKERS.contains(&name.as_ref()) || (!hidden && name.starts_with('.')) {
        return false;
    }
    !SKIPPED_DIRS.contains(&name.as_ref())
}

fn is_repo(entry: &DirEntry) -> bool {
    entry.file_type().is_some_and(|kind| kind.is_dir())
        && REPO_MARKERS
            .iter()
            .any(|marker| fs::symlink_metadata(entry.path().join(marker)).is_ok())
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
    fn finds_top_level_repos_skips_hidden_and_heavy_folders_and_reads_branches() {
        let home = TempDir::new().unwrap();
        let root = home.path();
        repo(root, "code/alpha", "ref: refs/heads/main\n");
        repo(
            root,
            "code/alpha/build/checkouts/dep",
            "ref: refs/heads/dev\n",
        );
        repo(root, "work/beta", "0123456789abcdef\n");
        repo(root, ".config/hidden", "ref: refs/heads/main\n");
        repo(root, "node_modules/pkg", "ref: refs/heads/main\n");
        repo(root, "code/.venv/tool", "ref: refs/heads/main\n");
        repo(root, "code/DerivedData/pkg", "ref: refs/heads/main\n");
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
        assert!(
            !scan(&hidden, &[], Some(root))
                .repos
                .iter()
                .any(|repo| repo.path.ends_with(".venv/tool"))
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
