use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use quark_ui::FocusId;
use quark_ui::element::{ScrollAlign, ScrollHandle};
use quark_ui::text_input::{TextEditCommand, TextEditOutcome, TextField};

use super::state::{Pending, ReviewController, ReviewOverlay, error, info};
use crate::app::text_targets::workspace_focus;
use crate::contracts::commands::SettingsSnapshot;
use crate::contracts::review::{GitHubRequest, PullsAction, ReviewEffect};
use crate::contracts::shared::{JobId, SharedContext};
use crate::contracts::shell::{Intent, LogicalFocus, Reduction, ShellEffect};
use crate::core::forge::github::MyPullRequest;
use crate::core::ref_catalog::now_secs;

const STALE_SECS: i64 = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullGroup {
    NeedsYou,
    Waiting,
    Ready,
    Drafts,
}

impl PullGroup {
    pub fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "Needs you",
            Self::Waiting => "Waiting on reviewers",
            Self::Ready => "Ready to merge",
            Self::Drafts => "Drafts",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::NeedsYou => "needs_you",
            Self::Waiting => "waiting",
            Self::Ready => "ready",
            Self::Drafts => "drafts",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullStatus {
    ChangesRequested,
    Conflicts,
    ChecksFailing,
    ReviewRequired,
    ChecksRunning,
    CheckingMergeability,
    Approved,
    Ready,
    Draft,
}

impl PullStatus {
    pub fn group(self) -> PullGroup {
        match self {
            Self::ChangesRequested | Self::Conflicts | Self::ChecksFailing => PullGroup::NeedsYou,
            Self::ReviewRequired | Self::ChecksRunning | Self::CheckingMergeability => {
                PullGroup::Waiting
            }
            Self::Approved | Self::Ready => PullGroup::Ready,
            Self::Draft => PullGroup::Drafts,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ChangesRequested => "Changes requested",
            Self::Conflicts => "Conflicts",
            Self::ChecksFailing => "Checks failing",
            Self::ReviewRequired => "Review required",
            Self::ChecksRunning => "Checks running",
            Self::CheckingMergeability => "Checking mergeability",
            Self::Approved => "Approved",
            Self::Ready => "Ready",
            Self::Draft => "Draft",
        }
    }
}

pub fn pull_status(pull: &MyPullRequest) -> PullStatus {
    let decision = pull.review_decision.as_deref();
    let checks = pull.checks.as_ref().and_then(|c| c.state.as_deref());
    let mergeable = pull.stats.as_ref().map(|s| s.mergeable.as_str());
    if pull.is_draft {
        PullStatus::Draft
    } else if decision == Some("CHANGES_REQUESTED") {
        PullStatus::ChangesRequested
    } else if mergeable == Some("CONFLICTING") {
        PullStatus::Conflicts
    } else if matches!(checks, Some("FAILURE" | "ERROR")) {
        PullStatus::ChecksFailing
    } else if decision == Some("REVIEW_REQUIRED")
        || (decision.is_none() && pull.requested_reviewers > 0)
    {
        PullStatus::ReviewRequired
    } else if matches!(checks, Some("PENDING" | "EXPECTED")) {
        PullStatus::ChecksRunning
    } else if mergeable != Some("MERGEABLE") {
        PullStatus::CheckingMergeability
    } else if decision == Some("APPROVED") {
        PullStatus::Approved
    } else {
        PullStatus::Ready
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullSection {
    pub group: PullGroup,
    pub rows: Vec<usize>,
}

const GROUP_ORDER: [PullGroup; 4] = [
    PullGroup::NeedsYou,
    PullGroup::Waiting,
    PullGroup::Ready,
    PullGroup::Drafts,
];

pub fn group_pulls(pulls: &[MyPullRequest], filter: &str) -> Vec<PullSection> {
    let terms: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    let mut sections: Vec<PullSection> = GROUP_ORDER
        .iter()
        .map(|group| PullSection {
            group: *group,
            rows: Vec::new(),
        })
        .collect();
    for (index, pull) in pulls.iter().enumerate() {
        if !terms.is_empty() {
            let haystack = format!(
                "{} {}#{}",
                pull.title.to_lowercase(),
                pull.repository.to_lowercase(),
                pull.number
            );
            if !terms.iter().all(|term| haystack.contains(term.as_str())) {
                continue;
            }
        }
        let group = pull_status(pull).group();
        if let Some(section) = sections.iter_mut().find(|s| s.group == group) {
            section.rows.push(index);
        }
    }
    for section in &mut sections {
        section
            .rows
            .sort_by(|a, b| pulls[*b].updated_at.cmp(&pulls[*a].updated_at));
    }
    sections.retain(|section| !section.rows.is_empty());
    sections
}

pub fn row_key(url: &str) -> String {
    format!("pulls.row.{url}")
}

#[derive(Debug, Default)]
pub struct PullsPage {
    pub open: bool,
    pub data: Option<Rc<[MyPullRequest]>>,
    pub sections: Rc<[PullSection]>,
    pub loading: bool,
    pub error: Option<String>,
    pub selected: Option<String>,
    pub filter: TextField,
    pub fetched_at: Option<i64>,
    pub list: ScrollHandle,
    pub preview: ScrollHandle,
    clones: HashMap<String, PathBuf>,
    request: Option<JobId>,
    clones_job: Option<JobId>,
    last_focus: Option<LogicalFocus>,
    return_focus: Option<LogicalFocus>,
    review_after_locate: Option<String>,
    slash_pending: bool,
}

impl PullsPage {
    pub fn visible(&self) -> impl Iterator<Item = usize> + '_ {
        self.sections
            .iter()
            .flat_map(|section| section.rows.iter().copied())
    }

    pub fn total(&self) -> usize {
        self.data.as_ref().map_or(0, |data| data.len())
    }

    pub fn selected_pull(&self) -> Option<&MyPullRequest> {
        let selected = self.selected.as_deref()?;
        self.data.as_ref()?.iter().find(|pull| pull.url == selected)
    }

    fn regroup(&mut self) {
        let Some(data) = self.data.clone() else {
            self.sections = Rc::from([]);
            return;
        };
        self.sections = group_pulls(&data, self.filter.text()).into();
        let visible = self
            .visible()
            .any(|index| Some(data[index].url.as_str()) == self.selected.as_deref());
        if !visible {
            let first = self.visible().next().map(|index| data[index].url.clone());
            self.selected = first;
        }
    }

    fn step(&mut self, delta: i32) -> bool {
        let Some(data) = self.data.clone() else {
            return false;
        };
        let order: Vec<usize> = self.visible().collect();
        if order.is_empty() {
            return false;
        }
        let current = order
            .iter()
            .position(|index| Some(data[*index].url.as_str()) == self.selected.as_deref());
        let next = match current {
            Some(at) => (at as i64 + i64::from(delta)).clamp(0, order.len() as i64 - 1) as usize,
            None => 0,
        };
        let url = data[order[next]].url.clone();
        if self.selected.as_deref() == Some(url.as_str()) {
            return false;
        }
        self.list
            .scroll_to_item(&row_key(&url), ScrollAlign::Nearest);
        self.preview.set_offset(0.0, 0.0);
        self.selected = Some(url);
        true
    }

    fn stale(&self) -> bool {
        self.fetched_at
            .is_none_or(|at| now_secs().saturating_sub(at) > STALE_SECS)
    }

    pub(super) fn reset(&mut self) {
        self.loading = false;
        self.request = None;
        self.data = None;
        self.sections = Rc::from([]);
        self.error = None;
        self.fetched_at = None;
    }
}

impl ReviewController {
    pub fn pulls(&self) -> &PullsPage {
        &self.pulls
    }

    pub fn pulls_open(&self) -> bool {
        self.pulls.open
    }

    pub fn observe_focus(&mut self, focus: Option<FocusId>) {
        if !self.pulls.open
            && let Some(focus) = focus.and_then(workspace_focus)
        {
            self.pulls.last_focus = Some(focus);
        }
    }

    pub fn clone_path<'a>(&'a self, cx: &'a SharedContext, repo: &str) -> Option<&'a Path> {
        let key = repo.to_ascii_lowercase();
        cx.settings
            .settings
            .github_clones
            .get(&key)
            .or_else(|| self.pulls.clones.get(&key))
            .map(PathBuf::as_path)
    }

    pub(super) fn reduce_pulls(&mut self, action: PullsAction, cx: &SharedContext) -> Reduction {
        match action {
            PullsAction::Open => self.open_pulls(),
            PullsAction::Close => self.close_pulls(),
            PullsAction::Refresh => {
                if !self.signed_in() {
                    self.open_overlay(ReviewOverlay::Auth);
                    return Reduction::changed();
                }
                if self.pulls.loading {
                    return Reduction::default();
                }
                self.fetch_pulls().effect(self.resolve_clones())
            }
            PullsAction::Move(delta) => {
                if self.pulls.step(delta) {
                    Reduction::changed()
                } else {
                    Reduction::default()
                }
            }
            PullsAction::Select(url) => {
                self.focus_request = Some(LogicalFocus::ReviewPulls);
                if self.pulls.selected.as_deref() != Some(url.as_str()) {
                    self.pulls.preview.set_offset(0.0, 0.0);
                    self.pulls.selected = Some(url);
                }
                Reduction::changed()
            }
            PullsAction::Review => self.review_selected(cx),
            PullsAction::OpenOnGitHub => match self.pulls.selected_pull() {
                Some(pull) => Reduction::default().effect(ShellEffect::OpenBrowser {
                    url: pull.url.clone(),
                }),
                None => Reduction::default(),
            },
            PullsAction::CopyLink => match self.pulls.selected_pull() {
                Some(pull) => Reduction::changed()
                    .effect(ShellEffect::SetClipboard(pull.url.clone()))
                    .intent(info("Link copied.")),
                None => Reduction::default(),
            },
            PullsAction::FocusFilter => {
                self.pulls.slash_pending = true;
                self.focus_request = Some(LogicalFocus::ReviewPullsFilter);
                Reduction::changed()
            }
            PullsAction::EscapeFilter => {
                if self.pulls.filter.text().is_empty() {
                    self.focus_request = Some(LogicalFocus::ReviewPulls);
                } else {
                    self.set_pulls_filter("");
                }
                Reduction::changed()
            }
            PullsAction::ClearFilter => {
                self.set_pulls_filter("");
                self.focus_request = Some(LogicalFocus::ReviewPullsFilter);
                Reduction::changed()
            }
            PullsAction::LocateClone => self.locate_clone(false),
            PullsAction::CloneChosen { repo, path } => {
                Reduction::default().effect(ReviewEffect::ValidateClone { repo, path })
            }
        }
    }

    pub(super) fn set_pulls_filter(&mut self, text: &str) {
        self.pulls.filter.set_text(text);
        self.pulls.regroup();
        self.pulls.list.set_offset(0.0, 0.0);
    }

    pub(super) fn edit_pulls_filter(
        &mut self,
        command: TextEditCommand,
        now_ms: u64,
    ) -> TextEditOutcome {
        // The "/" that focused the field arrives next as typed text.
        if std::mem::take(&mut self.pulls.slash_pending)
            && matches!(&command, TextEditCommand::InsertText(text) if text == "/")
        {
            return TextEditOutcome::default();
        }
        let outcome = self.pulls.filter.apply_at(command, now_ms);
        if outcome.text_changed {
            self.pulls.regroup();
            self.pulls.list.set_offset(0.0, 0.0);
        }
        outcome
    }

    fn open_pulls(&mut self) -> Reduction {
        if !self.pulls.open {
            self.pulls.open = true;
            self.pulls.return_focus = self.pulls.last_focus;
        }
        self.focus_request = Some(LogicalFocus::ReviewPulls);
        let mut reduction = Reduction::changed().effect(self.resolve_clones());
        if self.signed_in() && !self.pulls.loading && self.pulls.stale() {
            reduction.merge(self.fetch_pulls());
        }
        reduction
    }

    pub(super) fn close_pulls(&mut self) -> Reduction {
        if !self.pulls.open {
            return Reduction::default();
        }
        self.pulls.open = false;
        self.focus_request = Some(
            self.pulls
                .return_focus
                .take()
                .unwrap_or(LogicalFocus::Viewport),
        );
        Reduction::changed()
    }

    pub(super) fn fetch_pulls(&mut self) -> Reduction {
        let Some(token) = self.token() else {
            return Reduction::default();
        };
        let scope = self.job(None, Pending::Pulls);
        self.pulls.loading = true;
        self.pulls.request = Some(scope.request);
        Reduction::changed().effect(ReviewEffect::GitHub {
            scope,
            request: Box::new(GitHubRequest::FetchMyPullRequests { token }),
        })
    }

    fn resolve_clones(&mut self) -> ReviewEffect {
        let job = self.next_job_id();
        self.pulls.clones_job = Some(job);
        ReviewEffect::ResolveClones {
            job,
            current: self.repo_path.clone(),
        }
    }

    pub(super) fn pulls_loaded(&mut self, request: JobId, pulls: Vec<MyPullRequest>) -> Reduction {
        if self.pulls.request != Some(request) {
            return Reduction::default();
        }
        self.pulls.request = None;
        self.pulls.loading = false;
        self.pulls.error = None;
        self.pulls.fetched_at = Some(now_secs());
        self.pulls.data = Some(pulls.into());
        self.pulls.regroup();
        Reduction::changed()
    }

    pub(super) fn pulls_failed(&mut self, request: JobId, message: String) -> Reduction {
        if self.pulls.request != Some(request) {
            return Reduction::default();
        }
        tracing::warn!("failed to list your pull requests: {message}");
        self.pulls.request = None;
        self.pulls.loading = false;
        self.pulls.error = Some(message);
        Reduction::changed()
    }

    pub(super) fn clones_resolved(
        &mut self,
        job: JobId,
        clones: Vec<(String, PathBuf)>,
    ) -> Reduction {
        if self.pulls.clones_job != Some(job) {
            return Reduction::default();
        }
        self.pulls.clones_job = None;
        self.pulls.clones = clones.into_iter().collect();
        Reduction::changed()
    }

    pub(super) fn clone_validated(
        &mut self,
        repo: String,
        result: Result<PathBuf, String>,
        cx: &SharedContext,
    ) -> Reduction {
        let review = self.pulls.review_after_locate.take();
        let root = match result {
            Ok(root) => root,
            Err(message) => return Reduction::changed().intent(error(message)),
        };
        let mut settings = cx.settings.settings.clone();
        settings
            .github_clones
            .insert(repo.to_ascii_lowercase(), root.clone());
        let snapshot = Rc::new(SettingsSnapshot {
            revision: cx.settings.revision + 1,
            settings,
        });
        let mut reduction = Reduction::changed()
            .intent(Intent::SettingsChanged(snapshot))
            .intent(info(format!("Linked {repo} to {}", root.display())));
        self.pulls.clones.insert(repo.to_ascii_lowercase(), root);
        let resume = review.is_some_and(|url| {
            self.pulls.open
                && self
                    .pulls
                    .selected_pull()
                    .is_some_and(|pull| pull.url == url && pull.repository == repo)
        });
        if resume {
            reduction.merge(self.review_selected(cx));
        }
        reduction
    }

    fn locate_clone(&mut self, review: bool) -> Reduction {
        let Some((url, repo)) = self
            .pulls
            .selected_pull()
            .map(|pull| (pull.url.clone(), pull.repository.clone()))
        else {
            return Reduction::default();
        };
        self.pulls.review_after_locate = review.then_some(url);
        Reduction::changed().effect(ShellEffect::OpenCloneDialog { repo })
    }

    fn review_selected(&mut self, cx: &SharedContext) -> Reduction {
        let Some((url, repo)) = self
            .pulls
            .selected_pull()
            .map(|pull| (pull.url.clone(), pull.repository.clone()))
        else {
            return Reduction::default();
        };
        let Some(path) = self.clone_path(cx, &repo).map(Path::to_path_buf) else {
            return self.locate_clone(true);
        };
        let mut reduction = self.close_pulls();
        self.focus_request = Some(LogicalFocus::Viewport);
        if self.repo_path.as_deref() != Some(path.as_path()) {
            reduction = reduction.intent(Intent::ActivateWorkspace { path });
        }
        reduction.intent(Intent::OpenPullRequest { url })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::forge::github::{PullRequestCheckSummary, PullRequestStats};

    fn pull(number: i32, updated: &str) -> MyPullRequest {
        MyPullRequest {
            id: format!("PR_{number}"),
            number,
            title: format!("Tidy the widget {number}"),
            url: format!("https://github.com/owner/repo/pull/{number}"),
            repository: "owner/repo".to_owned(),
            updated_at: updated.to_owned(),
            stats: Some(PullRequestStats {
                mergeable: "MERGEABLE".to_owned(),
                ..PullRequestStats::default()
            }),
            checks: Some(PullRequestCheckSummary {
                state: Some("SUCCESS".to_owned()),
                ..PullRequestCheckSummary::default()
            }),
            ..MyPullRequest::default()
        }
    }

    fn with(mut pull: MyPullRequest, edit: impl FnOnce(&mut MyPullRequest)) -> MyPullRequest {
        edit(&mut pull);
        pull
    }

    fn checks(state: &str) -> Option<PullRequestCheckSummary> {
        Some(PullRequestCheckSummary {
            state: Some(state.to_owned()),
            ..PullRequestCheckSummary::default()
        })
    }

    #[test]
    fn status_follows_what_the_pull_request_needs() {
        let base = pull(1, "2026-01-01T00:00:00Z");
        let cases = [
            (with(base.clone(), |p| p.is_draft = true), PullStatus::Draft),
            (
                with(base.clone(), |p| {
                    p.review_decision = Some("CHANGES_REQUESTED".to_owned());
                    p.checks = checks("FAILURE");
                }),
                PullStatus::ChangesRequested,
            ),
            (
                with(base.clone(), |p| {
                    p.stats.as_mut().unwrap().mergeable = "CONFLICTING".to_owned()
                }),
                PullStatus::Conflicts,
            ),
            (
                with(base.clone(), |p| p.checks = checks("ERROR")),
                PullStatus::ChecksFailing,
            ),
            (
                with(base.clone(), |p| {
                    p.review_decision = Some("REVIEW_REQUIRED".to_owned())
                }),
                PullStatus::ReviewRequired,
            ),
            (
                with(base.clone(), |p| p.requested_reviewers = 1),
                PullStatus::ReviewRequired,
            ),
            (
                with(base.clone(), |p| p.checks = checks("PENDING")),
                PullStatus::ChecksRunning,
            ),
            (
                with(base.clone(), |p| p.stats = None),
                PullStatus::CheckingMergeability,
            ),
            (
                with(base.clone(), |p| {
                    p.review_decision = Some("APPROVED".to_owned());
                    p.requested_reviewers = 1;
                }),
                PullStatus::Approved,
            ),
            (base.clone(), PullStatus::Ready),
        ];
        for (pull, expected) in cases {
            assert_eq!(pull_status(&pull), expected, "{pull:?}");
        }
    }

    #[test]
    fn groups_keep_their_order_sort_by_update_and_skip_empty_ones() {
        let pulls = vec![
            pull(1, "2026-01-01T00:00:00Z"),
            with(pull(2, "2026-01-03T00:00:00Z"), |p| p.is_draft = true),
            with(pull(3, "2026-01-02T00:00:00Z"), |p| {
                p.checks = checks("FAILURE")
            }),
            pull(4, "2026-01-05T00:00:00Z"),
            with(pull(5, "2026-01-04T00:00:00Z"), |p| {
                p.review_decision = Some("CHANGES_REQUESTED".to_owned())
            }),
        ];

        let sections = group_pulls(&pulls, "");

        let shape: Vec<(PullGroup, Vec<usize>)> = sections
            .into_iter()
            .map(|section| (section.group, section.rows))
            .collect();
        assert_eq!(
            shape,
            [
                (PullGroup::NeedsYou, vec![4, 2]),
                (PullGroup::Ready, vec![3, 0]),
                (PullGroup::Drafts, vec![1]),
            ]
        );
    }

    #[test]
    fn filter_matches_title_and_repo_reference_case_insensitively() {
        let pulls = vec![
            pull(1, "2026-01-01T00:00:00Z"),
            with(pull(2, "2026-01-02T00:00:00Z"), |p| {
                p.repository = "owner/Gadget".to_owned();
                p.title = "Polish the gadget".to_owned();
            }),
        ];
        let rows = |filter: &str| -> Vec<usize> {
            group_pulls(&pulls, filter)
                .into_iter()
                .flat_map(|section| section.rows)
                .collect()
        };

        assert_eq!(rows("WIDGET"), [0]);
        assert_eq!(rows("gadget#2"), [1]);
        assert_eq!(rows("owner polish"), [1]);
        assert!(rows("missing").is_empty());
    }

    #[test]
    fn selection_moves_through_visible_rows_and_clamps() {
        let mut page = PullsPage {
            data: Some(
                vec![
                    pull(1, "2026-01-01T00:00:00Z"),
                    pull(2, "2026-01-02T00:00:00Z"),
                ]
                .into(),
            ),
            ..PullsPage::default()
        };
        page.regroup();
        assert_eq!(page.selected_pull().map(|p| p.number), Some(2));
        assert!(page.step(1));
        assert_eq!(page.selected_pull().map(|p| p.number), Some(1));
        assert!(!page.step(1));
        assert!(page.step(-5));
        assert_eq!(page.selected_pull().map(|p| p.number), Some(2));
    }
}
