use crate::core::vcs::model::{
    ChangeBucket, ChangeFlags, FileChange, FileChangeStatus, RefKind, RevisionId, VcsChange,
    VcsKind, VcsRef,
};

pub fn parse_change_log(output: &str) -> Vec<VcsChange> {
    let mut changes: Vec<VcsChange> = output.lines().filter_map(parse_change_log_line).collect();
    if let Some(current) = changes.first_mut() {
        current.flags.current = true;
        current.flags.working_copy = true;
    }
    changes
}

pub fn parse_change_log_line(line: &str) -> Option<VcsChange> {
    let mut fields = line.splitn(7, '\t');
    let change_id = fields.next()?.to_owned();
    let short_change_prefix = fields.next()?.to_owned();
    let short_change_rest = fields.next()?.to_owned();
    let commit_id = fields.next()?.to_owned();
    let summary = fields.next().unwrap_or_default().to_owned();
    let author_name = fields.next().unwrap_or_default().to_owned();
    let short_change_id = format!("{short_change_prefix}{short_change_rest}");
    let short_change_id_prefix_len = short_change_prefix.len();
    let short_revision = commit_id.chars().take(12).collect::<String>();
    Some(VcsChange {
        revision: RevisionId {
            backend: VcsKind::JJ,
            id: commit_id,
        },
        change_id: Some(change_id),
        short_change_id: (!short_change_id.is_empty()).then_some(short_change_id),
        short_change_id_prefix_len: (!short_change_prefix.is_empty())
            .then_some(short_change_id_prefix_len),
        short_revision,
        summary,
        author_name,
        timestamp: 0,
        flags: ChangeFlags {
            current: false,
            working_copy: false,
            divergent: false,
            immutable: false,
            conflicted: false,
        },
    })
}

pub fn parse_bookmark_list(output: &str) -> Vec<VcsRef> {
    output.lines().filter_map(parse_bookmark_line).collect()
}

pub fn parse_bookmark_line(line: &str) -> Option<VcsRef> {
    let mut fields = line.splitn(2, '\t');
    let name = fields.next()?.trim();
    let target = fields.next()?.trim();
    if name.is_empty() || target.is_empty() {
        return None;
    }
    Some(VcsRef {
        name: name.to_owned(),
        kind: RefKind::Bookmark,
        target: RevisionId {
            backend: VcsKind::JJ,
            id: target.to_owned(),
        },
        active: false,
        upstream: None,
        ahead_behind: None,
    })
}

pub fn parse_diff_summary(output: &str) -> Vec<FileChange> {
    output.lines().filter_map(parse_diff_summary_line).collect()
}

pub fn parse_diff_summary_line(line: &str) -> Option<FileChange> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut chars = trimmed.chars();
    let code = chars.next()?;
    if chars.next()? != ' ' {
        return None;
    }
    let rest = chars.as_str().trim();
    if rest.is_empty() {
        return None;
    }
    let (path, old_path) = if code == 'R' {
        parse_rename_path(rest)
            .map(|(old_path, path)| (path, Some(old_path)))
            .unwrap_or_else(|| (rest.to_owned(), None))
    } else {
        (rest.to_owned(), None)
    };
    Some(FileChange {
        path,
        old_path,
        status: summary_status(code),
        bucket: ChangeBucket::WorkingCopy,
    })
}

pub fn parse_conflict_list(output: &str) -> Vec<FileChange> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|path| FileChange {
            path: path.to_owned(),
            old_path: None,
            status: FileChangeStatus::Conflicted,
            bucket: ChangeBucket::Conflicted,
        })
        .collect()
}

fn summary_status(code: char) -> FileChangeStatus {
    match code {
        'A' => FileChangeStatus::Added,
        'D' => FileChangeStatus::Deleted,
        'R' => FileChangeStatus::Renamed,
        'C' => FileChangeStatus::Copied,
        _ => FileChangeStatus::Modified,
    }
}

fn parse_rename_path(path: &str) -> Option<(String, String)> {
    let start = path.find('{')?;
    let end = path[start..].find('}')? + start;
    let inner = &path[start + 1..end];
    let (old, new) = inner.split_once(" => ")?;
    let prefix = &path[..start];
    let suffix = &path[end + 1..];
    Some((
        format!("{prefix}{old}{suffix}"),
        format!("{prefix}{new}{suffix}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::parse_bookmark_line;
    use super::{
        parse_change_log, parse_change_log_line, parse_conflict_list, parse_diff_summary_line,
    };
    use crate::core::vcs::model::{ChangeBucket, FileChangeStatus, VcsKind};

    #[test]
    fn parses_change_log_rows() {
        let change =
            parse_change_log_line("change123\tch\tange\tabcdef1234567890\tmy change\tro\tignored")
                .unwrap();
        assert_eq!(change.revision.backend, VcsKind::JJ);
        assert_eq!(change.revision.id, "abcdef1234567890");
        assert_eq!(change.change_id.as_deref(), Some("change123"));
        assert_eq!(change.short_change_id.as_deref(), Some("change"));
        assert_eq!(change.short_change_id_prefix_len, Some(2));
        assert_eq!(change.short_revision, "abcdef123456");
        assert_eq!(change.summary, "my change");
        assert!(!change.flags.working_copy);
    }

    #[test]
    fn preserves_empty_change_descriptions() {
        let change =
            parse_change_log_line("change123\tch\tange\tabcdef1234567890\t\tro\tignored").unwrap();
        assert_eq!(change.summary, "");
    }

    #[test]
    fn marks_first_change_log_row_as_working_copy() {
        let changes = parse_change_log(
            "change1\tch\t1\tabcdef1234567890\tcurrent\tro\tignored\nchange2\tch\t2\t123456abcdef\tparent\tro\tignored\n",
        );
        assert!(changes[0].flags.current);
        assert!(changes[0].flags.working_copy);
        assert!(!changes[1].flags.current);
        assert!(!changes[1].flags.working_copy);
    }

    #[test]
    fn parses_bookmark_rows() {
        let bookmark = parse_bookmark_line("main\tabcdef1234567890").unwrap();
        assert_eq!(bookmark.kind, crate::core::vcs::model::RefKind::Bookmark);
        assert_eq!(bookmark.name, "main");
        assert_eq!(bookmark.target.id, "abcdef1234567890");
    }

    #[test]
    fn parses_basic_summary_rows() {
        let added = parse_diff_summary_line("A README.md").unwrap();
        assert_eq!(added.status, FileChangeStatus::Added);
        assert_eq!(added.path, "README.md");
        assert_eq!(added.bucket, ChangeBucket::WorkingCopy);

        let modified = parse_diff_summary_line("M src/main.rs").unwrap();
        assert_eq!(modified.status, FileChangeStatus::Modified);
        assert_eq!(modified.path, "src/main.rs");
    }

    #[test]
    fn parses_braced_renames() {
        let renamed = parse_diff_summary_line("R src/{old => new}.rs").unwrap();
        assert_eq!(renamed.status, FileChangeStatus::Renamed);
        assert_eq!(renamed.old_path.as_deref(), Some("src/old.rs"));
        assert_eq!(renamed.path, "src/new.rs");
    }

    #[test]
    fn parses_conflict_rows_as_conflicted_file_changes() {
        let conflicts = parse_conflict_list("src/lib.rs\n\nCargo.toml\n");
        assert_eq!(conflicts.len(), 2);
        assert_eq!(conflicts[0].status, FileChangeStatus::Conflicted);
        assert_eq!(conflicts[0].bucket, ChangeBucket::Conflicted);
    }
}
