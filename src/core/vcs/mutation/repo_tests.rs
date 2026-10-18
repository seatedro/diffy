use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

use super::{PatchSelection, Side, SideRange, apply_selection};
use crate::contracts::navigation::RepositorySyncReason;
use crate::core::error::Result;
use crate::core::vcs::discovery;
use crate::core::vcs::model::{ChangeBucket, FileChange, FileOperation};

fn run(dir: &Path, program: &str, args: &[&str]) -> Vec<u8> {
    let output = Command::new(program)
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn git(dir: &Path, args: &[&str]) -> Vec<u8> {
    run(dir, "git", args)
}

fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q"]);
    for (key, value) in [
        ("user.name", "Diffy"),
        ("user.email", "diffy@example.com"),
        ("core.autocrlf", "false"),
        ("core.filemode", "true"),
    ] {
        git(dir.path(), &["config", key, value]);
    }
    dir
}

fn write(dir: &Path, path: &str, bytes: &[u8]) {
    fs::write(dir.join(path), bytes).unwrap();
}

fn commit(dir: &Path, files: &[(&str, &[u8])]) {
    for (path, bytes) in files {
        write(dir, path, bytes);
    }
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "base"]);
}

fn index(dir: &Path, path: &str) -> Vec<u8> {
    git(dir, &["show", &format!(":{path}")])
}

fn worktree(dir: &Path, path: &str) -> Vec<u8> {
    fs::read(dir.join(path)).unwrap()
}

fn numbered(lines: &[&str]) -> Vec<u8> {
    lines
        .iter()
        .flat_map(|line| format!("{line}\n").into_bytes())
        .collect()
}

fn base_lines() -> Vec<String> {
    (0..20).map(|i| format!("l{i}")).collect()
}

fn text(lines: &[String]) -> Vec<u8> {
    numbered(&lines.iter().map(String::as_str).collect::<Vec<_>>())
}

fn change(dir: &Path, path: &str, bucket: ChangeBucket) -> FileChange {
    let mut repo = discovery::open_repository(dir).unwrap();
    let snapshot = repo.snapshot(RepositorySyncReason::Rescan, None).unwrap();
    snapshot
        .file_changes
        .into_iter()
        .find(|change| change.path == path && change.bucket == bucket)
        .unwrap_or_else(|| panic!("no {bucket:?} change for {path}"))
}

fn apply(
    dir: &Path,
    path: &str,
    bucket: ChangeBucket,
    selection: PatchSelection,
    operation: FileOperation,
) -> Result<()> {
    let change = change(dir, path, bucket);
    let mut repo = discovery::open_repository(dir)?;
    let material = repo.mutation_material(&change)?;
    apply_selection(repo.as_mut(), &material, &change, &selection, operation)
}

fn lines(ranges: &[(Side, std::ops::Range<u32>)]) -> PatchSelection {
    PatchSelection::Lines(
        ranges
            .iter()
            .map(|(side, lines)| SideRange {
                side: *side,
                lines: lines.clone(),
            })
            .collect(),
    )
}

fn hunk(side: Side, range: std::ops::Range<u32>) -> PatchSelection {
    PatchSelection::Hunks(vec![SideRange { side, lines: range }])
}

fn two_hunks(dir: &Path) -> (Vec<String>, Vec<String>) {
    let base = base_lines();
    commit(dir, &[("f.txt", &text(&base))]);
    let mut changed = base.clone();
    changed[2] = "X2".to_owned();
    changed[15] = "X15".to_owned();
    write(dir, "f.txt", &text(&changed));
    (base, changed)
}

#[test]
fn stages_one_hunk_forward() {
    let dir = repo();
    let (base, changed) = two_hunks(dir.path());
    apply(
        dir.path(),
        "f.txt",
        ChangeBucket::Unstaged,
        hunk(Side::New, 15..16),
        FileOperation::Stage,
    )
    .unwrap();
    let mut expected = base;
    expected[15] = "X15".to_owned();
    assert_eq!(index(dir.path(), "f.txt"), text(&expected));
    assert_eq!(worktree(dir.path(), "f.txt"), text(&changed));
}

#[test]
fn unstages_one_hunk_in_reverse() {
    let dir = repo();
    let (base, changed) = two_hunks(dir.path());
    git(dir.path(), &["add", "f.txt"]);
    apply(
        dir.path(),
        "f.txt",
        ChangeBucket::Staged,
        hunk(Side::Old, 2..3),
        FileOperation::Unstage,
    )
    .unwrap();
    let mut expected = base;
    expected[15] = "X15".to_owned();
    assert_eq!(index(dir.path(), "f.txt"), text(&expected));
    assert_eq!(worktree(dir.path(), "f.txt"), text(&changed));
}

fn replaced_block(dir: &Path) -> Vec<String> {
    let base = base_lines();
    commit(dir, &[("f.txt", &text(&base))]);
    let mut changed = base.clone();
    changed.splice(5..8, ["A", "B", "C"].map(str::to_owned));
    write(dir, "f.txt", &text(&changed));
    base
}

#[test]
fn stages_selected_lines_exactly() {
    let dir = repo();
    let base = replaced_block(dir.path());
    apply(
        dir.path(),
        "f.txt",
        ChangeBucket::Unstaged,
        lines(&[(Side::Old, 6..7), (Side::New, 6..7)]),
        FileOperation::Stage,
    )
    .unwrap();
    let mut expected = base;
    expected.splice(6..7, []);
    expected.insert(7, "B".to_owned());
    assert_eq!(index(dir.path(), "f.txt"), text(&expected));
}

#[test]
fn unstages_selected_lines_exactly() {
    let dir = repo();
    let base = replaced_block(dir.path());
    git(dir.path(), &["add", "f.txt"]);
    apply(
        dir.path(),
        "f.txt",
        ChangeBucket::Staged,
        lines(&[(Side::New, 6..7)]),
        FileOperation::Unstage,
    )
    .unwrap();
    let mut expected = base;
    expected.splice(5..8, ["A", "C"].map(str::to_owned));
    assert_eq!(index(dir.path(), "f.txt"), text(&expected));
}

#[test]
fn discards_selected_lines_exactly() {
    let dir = repo();
    let base = replaced_block(dir.path());
    apply(
        dir.path(),
        "f.txt",
        ChangeBucket::Unstaged,
        lines(&[(Side::New, 6..7)]),
        FileOperation::Discard,
    )
    .unwrap();
    let mut expected = base.clone();
    expected.splice(5..8, ["A", "C"].map(str::to_owned));
    assert_eq!(worktree(dir.path(), "f.txt"), text(&expected));
    assert_eq!(index(dir.path(), "f.txt"), text(&base));
}

#[test]
fn crlf_lines_stay_byte_exact() {
    let dir = repo();
    commit(dir.path(), &[("f.txt", b"a\r\nb\r\nc\r\n")]);
    write(dir.path(), "f.txt", b"a\r\nB\r\nc\r\nd\r\n");
    apply(
        dir.path(),
        "f.txt",
        ChangeBucket::Unstaged,
        lines(&[(Side::Old, 1..2), (Side::New, 1..2)]),
        FileOperation::Stage,
    )
    .unwrap();
    assert_eq!(index(dir.path(), "f.txt"), b"a\r\nB\r\nc\r\n");
    apply(
        dir.path(),
        "f.txt",
        ChangeBucket::Staged,
        lines(&[(Side::New, 1..2)]),
        FileOperation::Unstage,
    )
    .unwrap();
    assert_eq!(index(dir.path(), "f.txt"), b"a\r\nc\r\n");
}

#[test]
fn missing_final_newline_is_explicit() {
    let dir = repo();
    commit(dir.path(), &[("f.txt", b"a\nb")]);
    write(dir.path(), "f.txt", b"a\nb\nc\n");
    let error = apply(
        dir.path(),
        "f.txt",
        ChangeBucket::Unstaged,
        lines(&[(Side::New, 2..3)]),
        FileOperation::Stage,
    )
    .unwrap_err();
    assert!(error.to_string().contains("final newline"), "{error}");
    assert_eq!(index(dir.path(), "f.txt"), b"a\nb");
    apply(
        dir.path(),
        "f.txt",
        ChangeBucket::Unstaged,
        hunk(Side::New, 1..3),
        FileOperation::Stage,
    )
    .unwrap();
    assert_eq!(index(dir.path(), "f.txt"), b"a\nb\nc\n");

    commit(dir.path(), &[("g.txt", b"a\nb\n")]);
    write(dir.path(), "g.txt", b"a\nb\nc\nd");
    apply(
        dir.path(),
        "g.txt",
        ChangeBucket::Unstaged,
        lines(&[(Side::New, 3..4)]),
        FileOperation::Stage,
    )
    .unwrap();
    assert_eq!(index(dir.path(), "g.txt"), b"a\nb\nd");
}

#[test]
fn hunk_operations_leave_mode_changes_alone() {
    let dir = repo();
    commit(dir.path(), &[("f.sh", b"a\n")]);
    write(dir.path(), "f.sh", b"b\n");
    run(dir.path(), "chmod", &["755", "f.sh"]);
    apply(
        dir.path(),
        "f.sh",
        ChangeBucket::Unstaged,
        hunk(Side::New, 0..1),
        FileOperation::Stage,
    )
    .unwrap();
    assert_eq!(index(dir.path(), "f.sh"), b"b\n");
    let staged = git(dir.path(), &["ls-files", "-s", "f.sh"]);
    assert!(
        staged.starts_with(b"100644 "),
        "{}",
        String::from_utf8_lossy(&staged)
    );
}

#[test]
fn unstaging_lines_of_a_staged_rename_keeps_the_rename() {
    let dir = repo();
    commit(dir.path(), &[("old.txt", b"a\nb\nc\nd\ne\n")]);
    git(dir.path(), &["mv", "old.txt", "new.txt"]);
    write(dir.path(), "new.txt", b"a\nB\nc\nd\ne\n");
    git(dir.path(), &["add", "-A"]);
    let renamed = change(dir.path(), "new.txt", ChangeBucket::Staged);
    assert_eq!(renamed.old_path.as_deref(), Some("old.txt"));
    apply(
        dir.path(),
        "new.txt",
        ChangeBucket::Staged,
        hunk(Side::New, 1..2),
        FileOperation::Unstage,
    )
    .unwrap();
    assert_eq!(index(dir.path(), "new.txt"), b"a\nb\nc\nd\ne\n");
    let files = git(dir.path(), &["ls-files"]);
    assert_eq!(files, b"new.txt\n");
}

#[test]
fn invalid_utf8_lines_stay_byte_exact() {
    let dir = repo();
    commit(dir.path(), &[("f.txt", b"a\n\xff\nz\n")]);
    write(dir.path(), "f.txt", b"a\n\xfe\nz\n\xfd\n");
    apply(
        dir.path(),
        "f.txt",
        ChangeBucket::Unstaged,
        lines(&[(Side::Old, 1..2), (Side::New, 1..2)]),
        FileOperation::Stage,
    )
    .unwrap();
    assert_eq!(index(dir.path(), "f.txt"), b"a\n\xfe\nz\n");
}

#[test]
fn stages_part_of_an_untracked_file() {
    let dir = repo();
    commit(dir.path(), &[("f.txt", b"a\n")]);
    write(dir.path(), "n.txt", b"x\ny\nz\n");
    apply(
        dir.path(),
        "n.txt",
        ChangeBucket::Untracked,
        lines(&[(Side::New, 0..1)]),
        FileOperation::Stage,
    )
    .unwrap();
    assert_eq!(index(dir.path(), "n.txt"), b"x\n");
}

#[test]
fn rejects_targets_after_the_worktree_changes() {
    let dir = repo();
    let (base, _) = two_hunks(dir.path());
    let change = change(dir.path(), "f.txt", ChangeBucket::Unstaged);
    let mut repo = discovery::open_repository(dir.path()).unwrap();
    let material = repo.mutation_material(&change).unwrap();
    let mut edited = base.clone();
    edited[2] = "Y2".to_owned();
    write(dir.path(), "f.txt", &text(&edited));
    let error = apply_selection(
        repo.as_mut(),
        &material,
        &change,
        &hunk(Side::New, 2..3),
        FileOperation::Stage,
    )
    .unwrap_err();
    assert!(error.to_string().contains("no longer matches"), "{error}");
    assert_eq!(index(dir.path(), "f.txt"), text(&base));

    let error = apply(
        dir.path(),
        "f.txt",
        ChangeBucket::Unstaged,
        lines(&[(Side::New, 15..16)]),
        FileOperation::Stage,
    )
    .unwrap_err();
    assert!(error.to_string().contains("no longer matches"), "{error}");
}

#[test]
fn jj_discards_selected_working_copy_lines() {
    if Command::new("jj").arg("--version").output().is_err() {
        eprintln!("skipping: jj is not installed");
        return;
    }
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    run(
        root,
        "jj",
        &[
            "--quiet",
            "--config",
            "git.colocate=false",
            "git",
            "init",
            ".",
        ],
    );
    for (key, value) in [("user.name", "Diffy"), ("user.email", "diffy@example.com")] {
        run(
            root,
            "jj",
            &["--quiet", "config", "set", "--repo", key, value],
        );
    }
    write(root, "f.txt", b"a\r\nb\nc\n");
    run(root, "jj", &["--quiet", "commit", "-m", "base"]);
    write(root, "f.txt", b"a\r\nB\nc\nd\n");
    apply(
        root,
        "f.txt",
        ChangeBucket::WorkingCopy,
        lines(&[(Side::New, 3..4)]),
        FileOperation::Discard,
    )
    .unwrap();
    assert_eq!(worktree(root, "f.txt"), b"a\r\nB\nc\n");
    let error = apply(
        root,
        "f.txt",
        ChangeBucket::WorkingCopy,
        lines(&[(Side::New, 1..2)]),
        FileOperation::Stage,
    )
    .unwrap_err();
    assert!(error.to_string().contains("staging area"), "{error}");
}

#[test]
fn commits_amend_and_soft_undo() {
    let dir = repo();
    commit(dir.path(), &[("f.txt", b"a\n")]);
    write(dir.path(), "f.txt", b"b\n");
    git(dir.path(), &["add", "f.txt"]);
    let mut repo = discovery::open_repository(dir.path()).unwrap();
    let created = repo.create_commit("Change f\n\nBody", false).unwrap();
    assert!(created.sha.starts_with(&created.short));
    let last = repo.last_commit().unwrap().unwrap();
    assert_eq!(last.sha, created.sha);
    assert_eq!(last.message, "Change f\n\nBody");

    let amended = repo.create_commit("Change f again", true).unwrap();
    assert_ne!(amended.sha, created.sha);
    assert_eq!(
        repo.last_commit().unwrap().unwrap().message,
        "Change f again"
    );

    assert!(repo.undo_commit(&created.sha).is_err());
    repo.undo_commit(&amended.sha).unwrap();
    assert_eq!(index(dir.path(), "f.txt"), b"b\n");
    let head = git(dir.path(), &["log", "-1", "--format=%s"]);
    assert_eq!(head, b"base\n");
}
