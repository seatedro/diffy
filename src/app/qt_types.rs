use qmetaobject::*;

use crate::core::diff::{DiffLine, FileDiff, Hunk, LineKind};
use crate::core::vcs::git::{BranchInfo, CommitInfo, TagInfo};
use crate::core::vcs::github::PullRequestInfo;

fn qv_string(value: &str) -> QVariant {
    QVariant::from(QString::from(value))
}

pub fn line_kind_to_qstring(kind: LineKind) -> QString {
    QString::from(match kind {
        LineKind::Context => "context",
        LineKind::Added => "added",
        LineKind::Removed => "removed",
    })
}

pub fn line_to_qvariant_map(line: &DiffLine, text: &str) -> QVariantMap {
    [
        (
            QString::from("oldLine"),
            QVariant::from(line.old_line_number.unwrap_or(-1)),
        ),
        (
            QString::from("newLine"),
            QVariant::from(line.new_line_number.unwrap_or(-1)),
        ),
        (
            QString::from("kind"),
            QVariant::from(line_kind_to_qstring(line.kind)),
        ),
        (QString::from("text"), qv_string(text)),
    ]
    .into_iter()
    .collect()
}

pub fn hunk_to_qvariant_map(hunk: &Hunk, lines: QVariantList) -> QVariantMap {
    [
        (QString::from("header"), qv_string(&hunk.header)),
        (QString::from("lines"), QVariant::from(lines)),
    ]
    .into_iter()
    .collect()
}

pub fn file_diff_to_qvariant_map(file: &FileDiff) -> QVariantMap {
    [
        (QString::from("path"), qv_string(&file.path)),
        (QString::from("status"), qv_string(&file.status)),
        (QString::from("isBinary"), QVariant::from(file.is_binary)),
        (QString::from("additions"), QVariant::from(file.additions)),
        (QString::from("deletions"), QVariant::from(file.deletions)),
    ]
    .into_iter()
    .collect()
}

pub fn file_diffs_to_qvariant_list(files: &[FileDiff]) -> QVariantList {
    files.iter().map(file_diff_to_qvariant_map).collect()
}

pub fn branch_infos_to_qvariant_list(branches: &[BranchInfo]) -> QVariantList {
    branches
        .iter()
        .map(|branch| {
            [
                (QString::from("name"), qv_string(&branch.name)),
                (QString::from("isRemote"), QVariant::from(branch.is_remote)),
                (QString::from("isHead"), QVariant::from(branch.is_head)),
            ]
            .into_iter()
            .collect::<QVariantMap>()
        })
        .collect()
}

pub fn tag_infos_to_qvariant_list(tags: &[TagInfo]) -> QVariantList {
    tags.iter()
        .map(|tag| {
            [
                (QString::from("name"), qv_string(&tag.name)),
                (QString::from("targetOid"), qv_string(&tag.target_oid)),
            ]
            .into_iter()
            .collect::<QVariantMap>()
        })
        .collect()
}

pub fn commit_infos_to_qvariant_list(commits: &[CommitInfo]) -> QVariantList {
    commits
        .iter()
        .map(|commit| {
            [
                (QString::from("oid"), qv_string(&commit.oid)),
                (QString::from("shortOid"), qv_string(&commit.short_oid)),
                (QString::from("summary"), qv_string(&commit.summary)),
                (QString::from("author"), qv_string(&commit.author_name)),
                (QString::from("timestamp"), QVariant::from(commit.timestamp)),
            ]
            .into_iter()
            .collect::<QVariantMap>()
        })
        .collect()
}

pub fn pull_request_info_to_qvariant_map(info: &PullRequestInfo) -> QVariantMap {
    [
        (QString::from("title"), qv_string(&info.title)),
        (QString::from("baseBranch"), qv_string(&info.base_branch)),
        (QString::from("headBranch"), qv_string(&info.head_branch)),
        (QString::from("baseSha"), qv_string(&info.base_sha)),
        (QString::from("headSha"), qv_string(&info.head_sha)),
        (QString::from("state"), qv_string(&info.state)),
        (QString::from("author"), qv_string(&info.author_login)),
        (QString::from("number"), QVariant::from(info.number)),
        (QString::from("additions"), QVariant::from(info.additions)),
        (QString::from("deletions"), QVariant::from(info.deletions)),
        (
            QString::from("changedFiles"),
            QVariant::from(info.changed_files),
        ),
    ]
    .into_iter()
    .collect()
}
