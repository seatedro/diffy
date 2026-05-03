use crate::core::compare::CompareMode;
use crate::core::vcs::model::{
    ChangeBucket, ChangeIdToken, RefKind, RepoLocation, VCS_PROFILE_GIT, VCS_PROFILE_JJ, VcsChange,
    VcsRef,
};
use crate::ui::icons::lucide;

const GIT_HEAD_REF: &str = "HEAD";
const GIT_INDEX_REF: &str = "@index";
const GIT_WORKDIR_REF: &str = "@workdir";
const JJ_BASE_REF: &str = "@-";
const JJ_WORKING_COPY_REF: &str = "@";

#[derive(Debug, Clone, Copy)]
pub struct VcsUiProfile {
    descriptor: &'static VcsUiDescriptor,
}

#[derive(Debug)]
struct VcsUiDescriptor {
    profile: Option<&'static str>,
    ref_picker_placeholder: &'static str,
    compare_modes: &'static [CompareModeUi],
    default_compare: (&'static str, &'static str, CompareMode),
    working_copy_compare: (&'static str, &'static str, CompareMode),
    non_swappable_refs: &'static [&'static str],
    should_auto_select_trunk_mode: bool,
    shows_branch_preset: bool,
    uses_status_buckets: bool,
    shows_head_commit_preset: bool,
    publish_command_label: &'static str,
    publish_command_detail: &'static str,
    working_copy_ref_label: &'static str,
    working_copy_ref_icon: Option<&'static str>,
    status_compare_refs: fn(ChangeBucket) -> (String, String),
    status_view_label: fn(Option<ChangeBucket>) -> String,
    current_change_preset_label: fn(&VcsChange) -> Option<String>,
    repository_identity_from_changes: fn(&[VcsChange]) -> Option<RepositoryIdentityUi>,
    publish_status_ui: fn(&[VcsChange], &[VcsRef], bool) -> PublishStatusUi,
    working_copy_ref_suffix: fn(&[VcsChange]) -> Option<(String, String)>,
    change_ref_entry: fn(&VcsChange) -> ChangeRefUi,
}

#[derive(Debug, Clone, Copy)]
pub struct CompareModeUi {
    pub mode: CompareMode,
    pub label: &'static str,
    pub description: &'static str,
    pub tooltip: &'static str,
}

#[derive(Debug, Clone)]
pub struct ChangeRefUi {
    pub label: String,
    pub value: String,
    pub detail: String,
    pub search_text: String,
    pub default_highlights: Vec<(usize, usize)>,
    pub prefix_len: Option<usize>,
    pub working_copy: bool,
}

#[derive(Debug, Clone)]
pub struct RepositoryIdentityUi {
    pub icon: &'static str,
    pub label: String,
    pub label_style: RepositoryIdentityLabelStyle,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RepositoryIdentityLabelStyle {
    #[default]
    Plain,
    ChangeId {
        change_id_prefix_len: usize,
    },
}

#[derive(Debug, Clone, Default)]
pub struct PublishStatusUi {
    pub show_menu: bool,
    pub hint: Option<PublishHintUi>,
    pub ref_chips: Vec<PublishRefChipUi>,
}

#[derive(Debug, Clone)]
pub struct PublishHintUi {
    pub label: String,
    pub change_id_token: Option<ChangeIdToken>,
    pub tooltip: String,
}

#[derive(Debug, Clone)]
pub struct PublishRefChipUi {
    pub name: String,
    pub upstream: Option<String>,
    pub tracked: bool,
}

pub fn change_summary_label(change: &VcsChange) -> String {
    if change.summary.trim().is_empty() {
        "(no description set)".to_owned()
    } else {
        change.summary.clone()
    }
}

const GIT_COMPARE_MODES: &[CompareModeUi] = &[
    CompareModeUi {
        mode: CompareMode::ThreeDot,
        label: "merge",
        description: "Changes since fork point",
        tooltip: "Merge - changes since the right ref diverged from the left",
    },
    CompareModeUi {
        mode: CompareMode::TwoDot,
        label: "diff",
        description: "Compare two refs directly",
        tooltip: "Diff - compare two refs directly",
    },
    CompareModeUi {
        mode: CompareMode::SingleCommit,
        label: "commit",
        description: "Single commit vs parent",
        tooltip: "Single commit - diff a commit against its parent",
    },
];

const JJ_COMPARE_MODES: &[CompareModeUi] = &[
    CompareModeUi {
        mode: CompareMode::TwoDot,
        label: "range",
        description: "Compare two jj revisions",
        tooltip: "Range - compare two jj revisions",
    },
    CompareModeUi {
        mode: CompareMode::SingleCommit,
        label: "change",
        description: "Show one change against its parent",
        tooltip: "Change - show one jj revision against its parent",
    },
];

static GIT_PROFILE: VcsUiDescriptor = VcsUiDescriptor {
    profile: Some(VCS_PROFILE_GIT),
    ref_picker_placeholder: "Search branches, tags, commits",
    compare_modes: GIT_COMPARE_MODES,
    default_compare: (GIT_HEAD_REF, GIT_WORKDIR_REF, CompareMode::ThreeDot),
    working_copy_compare: (GIT_HEAD_REF, GIT_WORKDIR_REF, CompareMode::TwoDot),
    non_swappable_refs: &[GIT_WORKDIR_REF],
    should_auto_select_trunk_mode: true,
    shows_branch_preset: true,
    uses_status_buckets: true,
    shows_head_commit_preset: true,
    publish_command_label: "Push current branch",
    publish_command_detail: "Push the current Git branch to its upstream",
    working_copy_ref_label: "Working copy",
    working_copy_ref_icon: Some(lucide::FILE_DIFF),
    status_compare_refs: git_status_compare_refs,
    status_view_label: git_status_view_label,
    current_change_preset_label: git_current_change_preset_label,
    repository_identity_from_changes: git_repository_identity_from_changes,
    publish_status_ui: git_publish_status_ui,
    working_copy_ref_suffix: git_working_copy_ref_suffix,
    change_ref_entry: git_change_ref_entry,
};

static JJ_PROFILE: VcsUiDescriptor = VcsUiDescriptor {
    profile: Some(VCS_PROFILE_JJ),
    ref_picker_placeholder: "Search bookmarks, changes, revsets",
    compare_modes: JJ_COMPARE_MODES,
    default_compare: (JJ_BASE_REF, JJ_WORKING_COPY_REF, CompareMode::TwoDot),
    working_copy_compare: (JJ_BASE_REF, JJ_WORKING_COPY_REF, CompareMode::TwoDot),
    non_swappable_refs: &[],
    should_auto_select_trunk_mode: false,
    shows_branch_preset: false,
    uses_status_buckets: false,
    shows_head_commit_preset: false,
    publish_command_label: "Publish current change",
    publish_command_detail: "Publish the current jj change or its bookmark",
    working_copy_ref_label: "Working copy change",
    working_copy_ref_icon: Some(lucide::CIRCLE_DOT),
    status_compare_refs: jj_status_compare_refs,
    status_view_label: jj_status_view_label,
    current_change_preset_label: jj_current_change_preset_label,
    repository_identity_from_changes: jj_repository_identity_from_changes,
    publish_status_ui: jj_publish_status_ui,
    working_copy_ref_suffix: jj_working_copy_ref_suffix,
    change_ref_entry: jj_change_ref_entry,
};

static UI_PROFILES: [&VcsUiDescriptor; 2] = [&JJ_PROFILE, &GIT_PROFILE];

pub fn profile(location: Option<&RepoLocation>) -> VcsUiProfile {
    let profile_id = location.map(|location| location.profile);
    let descriptor = UI_PROFILES
        .iter()
        .copied()
        .find(|profile| profile.profile == profile_id)
        .unwrap_or(&GIT_PROFILE);
    VcsUiProfile { descriptor }
}

impl VcsUiProfile {
    pub fn ref_picker_placeholder(self) -> &'static str {
        self.descriptor.ref_picker_placeholder
    }

    pub fn compare_modes(self) -> &'static [CompareModeUi] {
        self.descriptor.compare_modes
    }

    pub fn compare_mode_ui(self, mode: CompareMode) -> CompareModeUi {
        self.compare_modes()
            .iter()
            .copied()
            .find(|item| item.mode == mode)
            .unwrap_or_else(|| self.compare_modes()[0])
    }

    pub fn accepts_compare_mode(self, mode: CompareMode) -> bool {
        self.compare_modes().iter().any(|item| item.mode == mode)
    }

    pub fn next_compare_mode(self, current: CompareMode) -> CompareMode {
        let modes = self.compare_modes();
        let index = modes
            .iter()
            .position(|item| item.mode == current)
            .unwrap_or(0);
        modes[(index + 1) % modes.len()].mode
    }

    pub fn default_compare(self) -> (&'static str, &'static str, CompareMode) {
        self.descriptor.default_compare
    }

    pub fn working_copy_compare(self) -> (&'static str, &'static str, CompareMode) {
        self.descriptor.working_copy_compare
    }

    pub fn should_auto_select_trunk_mode(self) -> bool {
        self.descriptor.should_auto_select_trunk_mode
    }

    pub fn shows_branch_preset(self) -> bool {
        self.descriptor.shows_branch_preset
    }

    pub fn uses_status_buckets(self) -> bool {
        self.descriptor.uses_status_buckets
    }

    pub fn status_compare_refs(self, bucket: ChangeBucket) -> (String, String) {
        (self.descriptor.status_compare_refs)(bucket)
    }

    pub fn is_working_copy_ref(self, reference: &str) -> bool {
        reference == self.descriptor.working_copy_compare.1
    }

    pub fn can_swap_ref(self, reference: &str) -> bool {
        !self
            .descriptor
            .non_swappable_refs
            .iter()
            .any(|blocked| reference == *blocked)
    }

    pub fn compare_ref_display_label(self, value: &str) -> String {
        if value.is_empty() {
            "\u{2014}".to_owned()
        } else if self.is_working_copy_ref(value) {
            self.descriptor.working_copy_ref_label.to_ascii_lowercase()
        } else {
            value.to_owned()
        }
    }

    pub fn history_right_ref(self, resolved_right: &str) -> String {
        if self.is_working_copy_ref(resolved_right) {
            self.descriptor.working_copy_compare.0.to_owned()
        } else {
            resolved_right.to_owned()
        }
    }

    pub fn current_change_preset_label(self, change: &VcsChange) -> Option<String> {
        (self.descriptor.current_change_preset_label)(change)
    }

    pub fn shows_head_commit_preset(self) -> bool {
        self.descriptor.shows_head_commit_preset
    }

    pub fn repository_identity_from_changes(
        self,
        changes: &[VcsChange],
    ) -> Option<RepositoryIdentityUi> {
        (self.descriptor.repository_identity_from_changes)(changes)
    }

    pub fn publish_status_ui(
        self,
        changes: &[VcsChange],
        refs: &[VcsRef],
        has_remotes: bool,
    ) -> PublishStatusUi {
        (self.descriptor.publish_status_ui)(changes, refs, has_remotes)
    }

    pub fn working_copy_ref_suffix(self, changes: &[VcsChange]) -> Option<(String, String)> {
        (self.descriptor.working_copy_ref_suffix)(changes)
    }

    pub fn change_ref_entry(self, change: &VcsChange) -> ChangeRefUi {
        (self.descriptor.change_ref_entry)(change)
    }

    pub fn status_view_label(self, selected_bucket: Option<ChangeBucket>) -> String {
        (self.descriptor.status_view_label)(selected_bucket)
    }

    pub fn publish_command_label(self) -> &'static str {
        self.descriptor.publish_command_label
    }

    pub fn publish_command_detail(self) -> &'static str {
        self.descriptor.publish_command_detail
    }

    pub fn ref_kind_label_and_icon(self, kind: RefKind) -> (&'static str, Option<&'static str>) {
        match kind {
            RefKind::Branch => ("Branch", Some(lucide::GIT_BRANCH)),
            RefKind::RemoteBranch => ("Remote branch", Some(lucide::GIT_BRANCH)),
            RefKind::Bookmark => ("Bookmark", Some(lucide::GIT_BRANCH)),
            RefKind::RemoteBookmark => ("Remote bookmark", Some(lucide::GIT_BRANCH)),
            RefKind::Tag => ("Tag", Some(lucide::HASH)),
            RefKind::Head => ("HEAD", Some(lucide::HASH)),
            RefKind::WorkingCopy => (
                self.descriptor.working_copy_ref_label,
                self.descriptor.working_copy_ref_icon,
            ),
            RefKind::PullRequest => ("Pull request", Some(lucide::GIT_PULL_REQUEST)),
        }
    }
}

fn git_status_compare_refs(bucket: ChangeBucket) -> (String, String) {
    match bucket {
        ChangeBucket::Staged => (GIT_HEAD_REF.to_owned(), GIT_INDEX_REF.to_owned()),
        ChangeBucket::Untracked => (String::new(), GIT_WORKDIR_REF.to_owned()),
        ChangeBucket::WorkingCopy | ChangeBucket::Unstaged | ChangeBucket::Conflicted => {
            (GIT_INDEX_REF.to_owned(), GIT_WORKDIR_REF.to_owned())
        }
    }
}

fn jj_status_compare_refs(_bucket: ChangeBucket) -> (String, String) {
    (JJ_BASE_REF.to_owned(), JJ_WORKING_COPY_REF.to_owned())
}

fn git_current_change_preset_label(_change: &VcsChange) -> Option<String> {
    None
}

fn git_status_view_label(selected_bucket: Option<ChangeBucket>) -> String {
    selected_bucket
        .map(|bucket| format!("working tree / {}", bucket_label(bucket)))
        .unwrap_or_else(|| "working tree".to_owned())
}

fn jj_status_view_label(_selected_bucket: Option<ChangeBucket>) -> String {
    "working copy change".to_owned()
}

fn bucket_label(bucket: ChangeBucket) -> &'static str {
    match bucket {
        ChangeBucket::Staged => "Staged",
        ChangeBucket::Unstaged => "Unstaged",
        ChangeBucket::Untracked => "Untracked",
        ChangeBucket::WorkingCopy => "Changed files",
        ChangeBucket::Conflicted => "Conflicts",
    }
}

fn jj_current_change_preset_label(change: &VcsChange) -> Option<String> {
    let revision = change
        .short_change_id
        .as_deref()
        .or(change.change_id.as_deref())
        .map(|id| id.to_owned())
        .unwrap_or_else(|| change.short_revision.clone());
    Some(format!("@ ({revision})"))
}

fn git_repository_identity_from_changes(_changes: &[VcsChange]) -> Option<RepositoryIdentityUi> {
    None
}

fn git_publish_status_ui(
    _changes: &[VcsChange],
    _refs: &[VcsRef],
    _has_remotes: bool,
) -> PublishStatusUi {
    PublishStatusUi::default()
}

fn jj_repository_identity_from_changes(changes: &[VcsChange]) -> Option<RepositoryIdentityUi> {
    let identity = changes
        .iter()
        .find(|change| change.flags.working_copy || change.flags.current)
        .map(|change| {
            let change_id = change
                .short_change_id
                .as_deref()
                .or(change.change_id.as_deref())
                .map(str::to_owned)
                .unwrap_or_else(|| change.short_revision.clone());
            let prefix_len = change
                .short_change_id_prefix_len
                .unwrap_or(change_id.len())
                .min(change_id.len());
            (
                format!("@ {change_id} {}", change.short_revision),
                RepositoryIdentityLabelStyle::ChangeId {
                    change_id_prefix_len: prefix_len,
                },
            )
        });
    let (label, label_style) = identity.unwrap_or_else(|| {
        (
            "@".to_owned(),
            RepositoryIdentityLabelStyle::ChangeId {
                change_id_prefix_len: 1,
            },
        )
    });
    Some(RepositoryIdentityUi {
        icon: lucide::CIRCLE_DOT,
        label,
        label_style,
    })
}

fn jj_publish_status_ui(
    changes: &[VcsChange],
    refs: &[VcsRef],
    has_remotes: bool,
) -> PublishStatusUi {
    let hint = has_remotes
        .then(|| jj_publish_target_hint(changes, refs))
        .flatten();
    PublishStatusUi {
        show_menu: hint.is_some(),
        hint,
        ref_chips: publish_ref_chips(changes, refs),
    }
}

fn jj_publish_target_hint(changes: &[VcsChange], refs: &[VcsRef]) -> Option<PublishHintUi> {
    let wc_idx = changes
        .iter()
        .position(|change| change.flags.working_copy || change.flags.current)?;
    // Mirror backend publish targeting: undescribed working-copy changes are
    // not pushed directly, so `@-` becomes the target when possible.
    let head_described = changes
        .get(wc_idx)
        .is_some_and(|change| !change.summary.trim().is_empty());
    let (target, target_revision) = if head_described {
        (changes.get(wc_idx)?, "@")
    } else if let Some(parent) = changes.get(wc_idx + 1) {
        (parent, "@-")
    } else {
        return None;
    };
    if target.summary.trim().is_empty() {
        return None;
    }
    let remote = default_remote_from_refs(refs).unwrap_or_else(|| "origin".to_owned());

    let bookmark_name = refs
        .iter()
        .filter(|reference| matches!(reference.kind, RefKind::Bookmark))
        .find(|reference| reference.target.id == target.revision.id)
        .map(|reference| reference.name.clone());

    if let Some(name) = bookmark_name {
        return Some(PublishHintUi {
            label: name.clone(),
            change_id_token: None,
            tooltip: format!("Push bookmark {name} at {target_revision} to {remote}"),
        });
    }

    let short_id = target
        .short_change_id
        .clone()
        .unwrap_or_else(|| target.short_revision.clone());
    if short_id.is_empty() {
        return None;
    }
    let prefix_len = target.short_change_id_prefix_len.unwrap_or(1).max(1);
    Some(PublishHintUi {
        label: short_id.clone(),
        change_id_token: Some(ChangeIdToken {
            text: short_id.clone(),
            prefix_len,
        }),
        tooltip: format!("Push change {short_id} at {target_revision} to {remote}"),
    })
}

fn default_remote_from_refs(refs: &[VcsRef]) -> Option<String> {
    let mut remotes: Vec<String> = refs
        .iter()
        .filter_map(|reference| {
            reference
                .upstream
                .as_deref()
                .and_then(|u| u.split_once('/').map(|(remote, _)| remote.to_owned()))
                .or_else(|| {
                    matches!(
                        reference.kind,
                        RefKind::RemoteBranch | RefKind::RemoteBookmark
                    )
                    .then(|| {
                        reference
                            .name
                            .split_once('/')
                            .map(|(remote, _)| remote.to_owned())
                    })
                    .flatten()
                })
        })
        .collect();
    remotes.sort();
    remotes.dedup();
    remotes
        .iter()
        .find(|remote| remote.as_str() == "origin")
        .cloned()
        .or_else(|| remotes.into_iter().next())
}

fn publish_ref_chips(changes: &[VcsChange], refs: &[VcsRef]) -> Vec<PublishRefChipUi> {
    let publish_targets: Vec<&str> = changes
        .iter()
        .take(2)
        .map(|change| change.revision.id.as_str())
        .collect();
    if publish_targets.is_empty() {
        return Vec::new();
    }
    refs.iter()
        .filter(|reference| matches!(reference.kind, RefKind::Bookmark))
        .filter(|reference| {
            publish_targets
                .iter()
                .any(|id| *id == reference.target.id.as_str())
        })
        .map(|reference| PublishRefChipUi {
            name: reference.name.clone(),
            upstream: reference.upstream.clone(),
            tracked: reference.upstream.is_some(),
        })
        .collect()
}

fn git_working_copy_ref_suffix(_changes: &[VcsChange]) -> Option<(String, String)> {
    None
}

fn jj_working_copy_ref_suffix(changes: &[VcsChange]) -> Option<(String, String)> {
    changes
        .iter()
        .find(|change| change.flags.working_copy)
        .and_then(|change| {
            let change_id = change.change_id.as_deref()?;
            let short_change = change.short_change_id.as_deref().unwrap_or(change_id);
            Some((
                format!(" / {short_change} {}", change.short_revision),
                format!(" {change_id} {}", change.short_revision),
            ))
        })
}

fn git_change_ref_entry(change: &VcsChange) -> ChangeRefUi {
    let summary = change_summary_label(change);
    ChangeRefUi {
        label: change.short_revision.clone(),
        value: change.revision.id.clone(),
        detail: summary.clone(),
        search_text: format!(
            "{} {} {}",
            change.short_revision, summary, change.revision.id
        ),
        default_highlights: Vec::new(),
        prefix_len: None,
        working_copy: false,
    }
}

fn jj_change_ref_entry(change: &VcsChange) -> ChangeRefUi {
    let change_id = change.change_id.as_deref().unwrap_or(&change.revision.id);
    let label = change
        .short_change_id
        .as_deref()
        .unwrap_or(change_id)
        .to_owned();
    let kind = if change.flags.working_copy {
        "Working copy change"
    } else {
        "Change"
    };
    let summary = change_summary_label(change);
    ChangeRefUi {
        label: label.clone(),
        value: change_id.to_owned(),
        detail: format!("{kind} / {} / {}", change.short_revision, summary),
        search_text: format!(
            "{label} {change_id} {} {} {}",
            change.short_revision, summary, change.revision.id
        ),
        default_highlights: Vec::new(),
        prefix_len: change.short_change_id_prefix_len,
        working_copy: change.flags.working_copy,
    }
}
