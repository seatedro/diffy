use crate::core::compare::CompareMode;
use crate::core::vcs::git::StatusScope;
use crate::core::vcs::git::service::WORKDIR_REF;
use crate::core::vcs::model::{RefKind, RepoLocation, VcsChange, VcsKind};
use crate::ui::icons::lucide;

#[derive(Debug, Clone, Copy)]
pub struct VcsUiProfile {
    family: VcsUiFamily,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VcsUiFamily {
    Git,
    Jj,
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

pub fn profile(location: Option<&RepoLocation>) -> VcsUiProfile {
    match location.map(|location| location.kind) {
        Some(VcsKind::Jj) => VcsUiProfile {
            family: VcsUiFamily::Jj,
        },
        Some(VcsKind::Git) | None => VcsUiProfile {
            family: VcsUiFamily::Git,
        },
    }
}

impl VcsUiProfile {
    pub fn ref_picker_placeholder(self) -> &'static str {
        match self.family {
            VcsUiFamily::Git => "Search branches, tags, commits",
            VcsUiFamily::Jj => "Search bookmarks, changes, revsets",
        }
    }

    pub fn compare_modes(self) -> &'static [CompareModeUi] {
        match self.family {
            VcsUiFamily::Git => GIT_COMPARE_MODES,
            VcsUiFamily::Jj => JJ_COMPARE_MODES,
        }
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
        match self.family {
            VcsUiFamily::Git => ("HEAD", WORKDIR_REF, CompareMode::ThreeDot),
            VcsUiFamily::Jj => ("@-", "@", CompareMode::TwoDot),
        }
    }

    pub fn working_copy_compare(self) -> (&'static str, &'static str, CompareMode) {
        match self.family {
            VcsUiFamily::Git => ("HEAD", WORKDIR_REF, CompareMode::TwoDot),
            VcsUiFamily::Jj => ("@-", "@", CompareMode::TwoDot),
        }
    }

    pub fn should_auto_select_trunk_mode(self) -> bool {
        self.family == VcsUiFamily::Git
    }

    pub fn shows_branch_preset(self) -> bool {
        self.family == VcsUiFamily::Git
    }

    pub fn uses_git_status_scopes(self) -> bool {
        self.family == VcsUiFamily::Git
    }

    pub fn current_change_preset_label(self, change: &VcsChange) -> Option<String> {
        match self.family {
            VcsUiFamily::Git => None,
            VcsUiFamily::Jj => {
                let revision = change
                    .short_change_id
                    .as_deref()
                    .or(change.change_id.as_deref())
                    .map(|id| id.to_owned())
                    .unwrap_or_else(|| change.short_revision.clone());
                Some(format!("@ ({revision})"))
            }
        }
    }

    pub fn shows_head_commit_preset(self) -> bool {
        self.family == VcsUiFamily::Git
    }

    pub fn repository_identity_from_changes(
        self,
        changes: &[VcsChange],
    ) -> Option<RepositoryIdentityUi> {
        match self.family {
            VcsUiFamily::Git => None,
            VcsUiFamily::Jj => {
                let label = changes
                    .iter()
                    .find(|change| change.flags.working_copy || change.flags.current)
                    .map(|change| {
                        let change_id = change
                            .short_change_id
                            .as_deref()
                            .or(change.change_id.as_deref())
                            .map(str::to_owned)
                            .unwrap_or_else(|| change.short_revision.clone());
                        format!("@ {change_id} {}", change.short_revision)
                    })
                    .unwrap_or_else(|| "@".to_owned());
                Some(RepositoryIdentityUi {
                    icon: lucide::CIRCLE_DOT,
                    label,
                })
            }
        }
    }

    pub fn working_copy_ref_suffix(self, changes: &[VcsChange]) -> Option<(String, String)> {
        match self.family {
            VcsUiFamily::Git => None,
            VcsUiFamily::Jj => changes
                .iter()
                .find(|change| change.flags.working_copy)
                .and_then(|change| {
                    let change_id = change.change_id.as_deref()?;
                    let short_change = change.short_change_id.as_deref().unwrap_or(change_id);
                    Some((
                        format!(" / {short_change} {}", change.short_revision),
                        format!(" {change_id} {}", change.short_revision),
                    ))
                }),
        }
    }

    pub fn change_ref_entry(self, change: &VcsChange) -> ChangeRefUi {
        match self.family {
            VcsUiFamily::Git => ChangeRefUi {
                label: change.short_revision.clone(),
                value: change.revision.id.clone(),
                detail: change.summary.clone(),
                search_text: format!(
                    "{} {} {}",
                    change.short_revision, change.summary, change.revision.id
                ),
                default_highlights: Vec::new(),
                prefix_len: None,
                working_copy: false,
            },
            VcsUiFamily::Jj => {
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
                ChangeRefUi {
                    label: label.clone(),
                    value: change_id.to_owned(),
                    detail: format!("{kind} / {} / {}", change.short_revision, change.summary),
                    search_text: format!(
                        "{label} {change_id} {} {} {}",
                        change.short_revision, change.summary, change.revision.id
                    ),
                    default_highlights: Vec::new(),
                    prefix_len: change.short_change_id_prefix_len,
                    working_copy: change.flags.working_copy,
                }
            }
        }
    }

    pub fn status_view_label(self, selected_scope: Option<StatusScope>) -> String {
        match self.family {
            VcsUiFamily::Git => selected_scope
                .map(|scope| format!("working tree / {}", scope.label()))
                .unwrap_or_else(|| "working tree".to_owned()),
            VcsUiFamily::Jj => "working copy change".to_owned(),
        }
    }

    pub fn ref_kind_label_and_icon(self, kind: RefKind) -> (&'static str, Option<&'static str>) {
        match (self.family, kind) {
            (_, RefKind::Branch) => ("Branch", Some(lucide::GIT_BRANCH)),
            (_, RefKind::RemoteBranch) => ("Remote branch", Some(lucide::GIT_BRANCH)),
            (_, RefKind::Bookmark) => ("Bookmark", Some(lucide::GIT_BRANCH)),
            (_, RefKind::RemoteBookmark) => ("Remote bookmark", Some(lucide::GIT_BRANCH)),
            (_, RefKind::Tag) => ("Tag", Some(lucide::HASH)),
            (_, RefKind::Head) => ("HEAD", Some(lucide::HASH)),
            (VcsUiFamily::Jj, RefKind::WorkingCopy) => {
                ("Working copy change", Some(lucide::CIRCLE_DOT))
            }
            (_, RefKind::WorkingCopy) => ("Working copy", Some(lucide::FILE_DIFF)),
            (_, RefKind::PullRequest) => ("Pull request", Some(lucide::GIT_PULL_REQUEST)),
        }
    }
}
