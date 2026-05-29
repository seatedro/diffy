use std::collections::BTreeMap;

use carbon::{Anchor, DiffSide, FileDiff, LineRange};
use serde::{Deserialize, Serialize};

use crate::core::error::{DiffyError, Result};
use crate::core::forge::github::{
    CreatePullRequestReview, CreatePullRequestReviewDraftComment, GitHubPullRequestReviewData,
    GitHubPullRequestReviewEvent, GitHubPullRequestReviewThread, GitHubReviewSide, PullRequestInfo,
    PullRequestReviewComment, PullRequestReviewMetadata, PullRequestReviewSummary,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
pub enum ReviewForge {
    GitHub,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
pub struct ReviewTarget {
    pub forge: ReviewForge,
    pub owner: String,
    pub repo: String,
    pub number: i32,
}

impl ReviewTarget {
    pub fn github(owner: impl Into<String>, repo: impl Into<String>, number: i32) -> Self {
        Self {
            forge: ReviewForge::GitHub,
            owner: owner.into(),
            repo: repo.into(),
            number,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
pub struct ReviewSessionKey {
    pub target: ReviewTarget,
    pub head_sha: String,
}

impl ReviewSessionKey {
    pub fn new(target: ReviewTarget, head_sha: impl Into<String>) -> Self {
        Self {
            target,
            head_sha: head_sha.into(),
        }
    }

    pub fn storage_key(&self) -> String {
        match self.target.forge {
            ReviewForge::GitHub => format!(
                "github/{}/{}/{}/{}",
                sanitize_key_part(&self.target.owner),
                sanitize_key_part(&self.target.repo),
                self.target.number,
                sanitize_key_part(&self.head_sha)
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
pub enum ReviewSide {
    Old,
    New,
}

impl From<GitHubReviewSide> for ReviewSide {
    fn from(side: GitHubReviewSide) -> Self {
        match side {
            GitHubReviewSide::Left => Self::Old,
            GitHubReviewSide::Right => Self::New,
        }
    }
}

impl From<ReviewSide> for GitHubReviewSide {
    fn from(side: ReviewSide) -> Self {
        match side {
            ReviewSide::Old => Self::Left,
            ReviewSide::New => Self::Right,
        }
    }
}

impl From<ReviewSide> for DiffSide {
    fn from(side: ReviewSide) -> Self {
        match side {
            ReviewSide::Old => Self::Old,
            ReviewSide::New => Self::New,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
pub struct ReviewLineRange {
    /// One-based source line number.
    pub start: u32,
    pub len: u32,
}

impl ReviewLineRange {
    pub const fn new(start: u32, len: u32) -> Self {
        Self { start, len }
    }

    pub fn from_inclusive(start: u32, end: u32) -> Self {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        Self {
            start,
            len: end.saturating_sub(start).saturating_add(1),
        }
    }

    pub const fn end_exclusive(self) -> u32 {
        self.start.saturating_add(self.len)
    }

    pub const fn end_inclusive(self) -> u32 {
        self.end_exclusive().saturating_sub(1)
    }

    pub const fn github_line(self) -> u32 {
        self.end_inclusive()
    }

    pub const fn github_start_line(self) -> Option<u32> {
        if self.len > 1 { Some(self.start) } else { None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
pub enum ReviewSubject {
    Line,
    File,
    Unknown,
}

impl ReviewSubject {
    fn from_github_subject(value: Option<&str>) -> Self {
        match value.unwrap_or_default().to_ascii_lowercase().as_str() {
            "line" => Self::Line,
            "file" => Self::File,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
pub struct ReviewAnchor {
    pub path: String,
    pub side: Option<ReviewSide>,
    pub line_range: Option<ReviewLineRange>,
    pub original_line_range: Option<ReviewLineRange>,
    pub subject: ReviewSubject,
    pub commit_id: Option<String>,
    pub original_commit_id: Option<String>,
}

impl ReviewAnchor {
    pub fn inline(path: impl Into<String>, side: ReviewSide, line_range: ReviewLineRange) -> Self {
        Self {
            path: path.into(),
            side: Some(side),
            line_range: Some(line_range),
            original_line_range: None,
            subject: ReviewSubject::Line,
            commit_id: None,
            original_commit_id: None,
        }
    }

    pub fn from_github_comment(comment: &PullRequestReviewComment) -> Option<Self> {
        if comment.path.is_empty() {
            return None;
        }

        let side = comment.side.or(comment.start_side).map(ReviewSide::from);
        let line_range = review_line_range(comment.start_line, comment.line);
        let original_line_range =
            review_line_range(comment.original_start_line, comment.original_line);
        let subject = ReviewSubject::from_github_subject(comment.subject_type.as_deref());
        if side.is_none() && line_range.is_none() && original_line_range.is_none() {
            return Some(Self {
                path: comment.path.clone(),
                side: None,
                line_range: None,
                original_line_range: None,
                subject,
                commit_id: non_empty(&comment.commit_id),
                original_commit_id: non_empty(&comment.original_commit_id),
            });
        }

        Some(Self {
            path: comment.path.clone(),
            side,
            line_range,
            original_line_range,
            subject,
            commit_id: non_empty(&comment.commit_id),
            original_commit_id: non_empty(&comment.original_commit_id),
        })
    }

    pub fn from_github_thread(thread: &GitHubPullRequestReviewThread) -> Self {
        let side = thread
            .diff_side
            .or(thread.start_diff_side)
            .map(ReviewSide::from);
        let line_range = review_line_range(thread.start_line, thread.line);
        let original_line_range =
            review_line_range(thread.original_start_line, thread.original_line);
        Self {
            path: thread.path.clone(),
            side,
            line_range,
            original_line_range,
            subject: ReviewSubject::from_github_subject(Some(&thread.subject_type)),
            commit_id: None,
            original_commit_id: None,
        }
    }

    pub fn is_outdated(&self) -> bool {
        self.line_range.is_none() && self.original_line_range.is_some()
    }

    pub fn active_line_range(&self) -> Option<ReviewLineRange> {
        self.line_range.or(self.original_line_range)
    }

    pub fn to_carbon_anchor(&self, file: &FileDiff) -> Option<Anchor> {
        let matches_old = file.old_path.as_deref() == Some(self.path.as_str());
        let matches_new = file.new_path.as_deref() == Some(self.path.as_str());
        if !matches_old && !matches_new {
            return None;
        }

        let side = match self.side {
            Some(ReviewSide::Old) => Some(DiffSide::Old),
            Some(ReviewSide::New) => Some(DiffSide::New),
            None if matches_new && !matches_old => Some(DiffSide::New),
            None if matches_old && !matches_new => Some(DiffSide::Old),
            None => None,
        };
        let line_range = self
            .active_line_range()
            .map(|range| LineRange::new(range.start, range.len))
            .unwrap_or_default();

        Some(Anchor {
            file_id: file.id,
            side,
            line_range,
            byte_range: None,
            old_oid: file.old_oid.clone(),
            new_oid: file.new_oid.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
pub struct ReviewCommentId(pub String);

impl ReviewCommentId {
    pub fn github(id: i64) -> Self {
        Self(format!("github:{id}"))
    }

    pub fn github_node(id: impl Into<String>) -> Self {
        Self(format!("github-node:{}", id.into()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
pub struct ReviewThreadId(pub String);

impl ReviewThreadId {
    pub fn github_root(id: i64) -> Self {
        Self(format!("github-thread:{id}"))
    }

    pub fn github_node(id: impl Into<String>) -> Self {
        Self(format!("github-thread-node:{}", id.into()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReviewReactionGroup {
    pub content: String,
    pub count: u32,
    pub viewer_has_reacted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReviewComment {
    pub id: ReviewCommentId,
    pub backend_id: Option<i64>,
    pub backend_node_id: Option<String>,
    pub thread_id: ReviewThreadId,
    pub in_reply_to: Option<ReviewCommentId>,
    pub in_reply_to_node_id: Option<String>,
    pub author_login: Option<String>,
    #[serde(default)]
    pub author_avatar_url: Option<String>,
    pub body: String,
    pub anchor: Option<ReviewAnchor>,
    pub html_url: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub outdated: bool,
    pub state: Option<String>,
    pub viewer_can_update: bool,
    pub viewer_can_delete: bool,
    #[serde(default)]
    pub reactions: Vec<ReviewReactionGroup>,
}

impl ReviewComment {
    pub fn from_github_comment(comment: &PullRequestReviewComment) -> Self {
        let root_id = comment.in_reply_to_id.unwrap_or(comment.id);
        let anchor = ReviewAnchor::from_github_comment(comment);
        let outdated = anchor.as_ref().is_some_and(ReviewAnchor::is_outdated);
        Self {
            id: ReviewCommentId::github(comment.id),
            backend_id: Some(comment.id),
            backend_node_id: None,
            thread_id: ReviewThreadId::github_root(root_id),
            in_reply_to: comment.in_reply_to_id.map(ReviewCommentId::github),
            in_reply_to_node_id: None,
            author_login: comment
                .user
                .as_ref()
                .and_then(|user| non_empty(&user.login)),
            author_avatar_url: comment
                .user
                .as_ref()
                .and_then(|user| non_empty(&user.avatar_url)),
            body: comment.body.clone(),
            anchor,
            html_url: non_empty(&comment.html_url),
            created_at: non_empty(&comment.created_at),
            updated_at: non_empty(&comment.updated_at),
            outdated,
            state: None,
            viewer_can_update: false,
            viewer_can_delete: false,
            reactions: Vec::new(),
        }
    }

    pub fn from_github_thread_comment(
        thread: &GitHubPullRequestReviewThread,
        comment: &crate::core::forge::github::GitHubPullRequestReviewThreadComment,
    ) -> Self {
        let thread_id = ReviewThreadId::github_node(thread.node_id.clone());
        let id = comment
            .database_id
            .map(ReviewCommentId::github)
            .unwrap_or_else(|| ReviewCommentId::github_node(comment.node_id.clone()));
        let in_reply_to = comment
            .reply_to_database_id
            .map(ReviewCommentId::github)
            .or_else(|| {
                comment
                    .reply_to_node_id
                    .clone()
                    .map(ReviewCommentId::github_node)
            });
        let anchor = Some(ReviewAnchor::from_github_thread(thread));
        Self {
            id,
            backend_id: comment.database_id,
            backend_node_id: Some(comment.node_id.clone()),
            thread_id,
            in_reply_to,
            in_reply_to_node_id: comment.reply_to_node_id.clone(),
            author_login: non_empty(&comment.author_login),
            author_avatar_url: non_empty(&comment.author_avatar_url),
            body: comment.body.clone(),
            anchor,
            html_url: non_empty(&comment.url),
            created_at: non_empty(&comment.created_at),
            updated_at: non_empty(&comment.updated_at),
            outdated: comment.outdated,
            state: non_empty(&comment.state),
            viewer_can_update: comment.viewer_can_update,
            viewer_can_delete: comment.viewer_can_delete,
            reactions: comment
                .reactions
                .iter()
                .map(|reaction| ReviewReactionGroup {
                    content: reaction.content.clone(),
                    count: reaction.count,
                    viewer_has_reacted: reaction.viewer_has_reacted,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum ReviewResolution {
    Unknown,
    Unresolved,
    Resolved,
}

impl Default for ReviewResolution {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReviewThreadStatus {
    pub resolution: ReviewResolution,
    pub outdated: bool,
    pub collapsed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReviewThreadPermissions {
    pub can_reply: bool,
    pub can_resolve: bool,
    pub can_unresolve: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReviewThread {
    pub id: ReviewThreadId,
    pub backend_node_id: Option<String>,
    pub anchor: Option<ReviewAnchor>,
    pub comments: Vec<ReviewComment>,
    pub status: ReviewThreadStatus,
    pub permissions: ReviewThreadPermissions,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum ReviewSessionStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ReviewDraftId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum ReviewDraftKind {
    InlineComment,
    Reply { thread_id: ReviewThreadId },
    General,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum ReviewDraftState {
    Pending,
    Submitting,
    Submitted {
        external_comment_id: Option<ReviewCommentId>,
    },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReviewDraft {
    pub id: ReviewDraftId,
    pub kind: ReviewDraftKind,
    pub anchor: Option<ReviewAnchor>,
    pub body: String,
    pub state: ReviewDraftState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum ReviewDecision {
    Comment,
    Approve,
    RequestChanges,
}

impl From<ReviewDecision> for GitHubPullRequestReviewEvent {
    fn from(decision: ReviewDecision) -> Self {
        match decision {
            ReviewDecision::Comment => Self::Comment,
            ReviewDecision::Approve => Self::Approve,
            ReviewDecision::RequestChanges => Self::RequestChanges,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReviewSessionMetrics {
    pub unresolved_threads: usize,
    pub resolved_threads: usize,
    pub outdated_threads: usize,
    pub pending_drafts: usize,
    pub failed_drafts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReviewSession {
    pub target: ReviewTarget,
    pub pull_request: PullRequestInfo,
    pub status: ReviewSessionStatus,
    pub status_message: Option<String>,
    pub metadata: Option<PullRequestReviewMetadata>,
    pub reviews: Vec<PullRequestReviewSummary>,
    pub threads: Vec<ReviewThread>,
    pub drafts: BTreeMap<ReviewDraftId, ReviewDraft>,
    #[serde(default)]
    next_draft_id: u64,
}

impl ReviewSession {
    pub fn new(target: ReviewTarget, pull_request: PullRequestInfo) -> Self {
        Self {
            target,
            pull_request,
            status: ReviewSessionStatus::Idle,
            status_message: None,
            metadata: None,
            reviews: Vec::new(),
            threads: Vec::new(),
            drafts: BTreeMap::new(),
            next_draft_id: 1,
        }
    }

    pub fn key(&self) -> ReviewSessionKey {
        ReviewSessionKey::new(self.target.clone(), self.pull_request.head_sha.clone())
    }

    pub fn from_github_review_comments(
        target: ReviewTarget,
        pull_request: PullRequestInfo,
        comments: Vec<PullRequestReviewComment>,
    ) -> Self {
        let mut session = Self::new(target, pull_request);
        session.status = ReviewSessionStatus::Ready;
        session.replace_github_comments(comments);
        session
    }

    pub fn replace_github_comments(&mut self, comments: Vec<PullRequestReviewComment>) {
        self.threads = build_threads(comments);
        self.status = ReviewSessionStatus::Ready;
        self.status_message = None;
    }

    pub fn apply_github_review_data(&mut self, data: GitHubPullRequestReviewData) {
        self.metadata = Some(data.metadata);
        self.reviews = data.reviews;
        self.threads = data
            .threads
            .iter()
            .map(ReviewThread::from_github_thread)
            .collect();
        self.status = ReviewSessionStatus::Ready;
        self.status_message = None;
    }

    pub fn merge_persisted_state(&mut self, persisted: ReviewSession) {
        if self.threads.is_empty() {
            self.metadata = persisted.metadata;
            self.reviews = persisted.reviews;
            self.threads = persisted.threads;
        }
        self.drafts = persisted.drafts;
        self.next_draft_id = self.next_draft_id.max(persisted.next_draft_id);
    }

    pub fn apply_github_comment(&mut self, comment: PullRequestReviewComment) {
        let normalized = ReviewComment::from_github_comment(&comment);
        if let Some(thread) = self
            .threads
            .iter_mut()
            .find(|thread| thread.id == normalized.thread_id)
        {
            if thread.anchor.is_none() {
                thread.anchor = normalized.anchor.clone();
            }
            thread.status.outdated |= normalized.outdated;
            thread.comments.push(normalized);
            thread.comments.sort_by(|a, b| {
                a.created_at
                    .cmp(&b.created_at)
                    .then_with(|| a.updated_at.cmp(&b.updated_at))
                    .then_with(|| a.id.cmp(&b.id))
            });
            return;
        }

        let anchor = normalized.anchor.clone();
        let outdated =
            normalized.outdated || anchor.as_ref().is_some_and(ReviewAnchor::is_outdated);
        self.threads.push(ReviewThread {
            id: normalized.thread_id.clone(),
            backend_node_id: None,
            anchor,
            comments: vec![normalized],
            status: ReviewThreadStatus {
                resolution: ReviewResolution::Unknown,
                outdated,
                collapsed: false,
            },
            permissions: ReviewThreadPermissions::default(),
        });
        self.threads.sort_by(|a, b| a.id.cmp(&b.id));
    }

    pub fn mark_thread_resolution(
        &mut self,
        thread_id: &ReviewThreadId,
        resolution: ReviewResolution,
    ) -> bool {
        let Some(thread) = self
            .threads
            .iter_mut()
            .find(|thread| &thread.id == thread_id)
        else {
            return false;
        };
        thread.status.resolution = resolution;
        true
    }

    pub fn thread_node_id(&self, thread_id: &ReviewThreadId) -> Option<String> {
        self.threads
            .iter()
            .find(|thread| &thread.id == thread_id)
            .and_then(|thread| thread.backend_node_id.clone())
    }

    pub fn comment_node_id(&self, comment_id: &ReviewCommentId) -> Option<String> {
        self.threads
            .iter()
            .flat_map(|thread| thread.comments.iter())
            .find(|comment| &comment.id == comment_id)
            .and_then(|comment| comment.backend_node_id.clone())
    }

    pub fn create_inline_draft(
        &mut self,
        anchor: ReviewAnchor,
        body: impl Into<String>,
    ) -> ReviewDraftId {
        self.insert_draft(ReviewDraftKind::InlineComment, Some(anchor), body)
    }

    pub fn create_reply_draft(
        &mut self,
        thread_id: ReviewThreadId,
        body: impl Into<String>,
    ) -> ReviewDraftId {
        self.insert_draft(ReviewDraftKind::Reply { thread_id }, None, body)
    }

    pub fn create_general_draft(&mut self, body: impl Into<String>) -> ReviewDraftId {
        self.insert_draft(ReviewDraftKind::General, None, body)
    }

    pub fn remove_draft(&mut self, id: ReviewDraftId) -> Option<ReviewDraft> {
        self.drafts.remove(&id)
    }

    pub fn update_draft_body(&mut self, id: ReviewDraftId, body: impl Into<String>) -> bool {
        let Some(draft) = self.drafts.get_mut(&id) else {
            return false;
        };
        draft.body = body.into();
        draft.state = ReviewDraftState::Pending;
        true
    }

    pub fn mark_draft_submitting(&mut self, id: ReviewDraftId) -> bool {
        let Some(draft) = self.drafts.get_mut(&id) else {
            return false;
        };
        draft.state = ReviewDraftState::Submitting;
        true
    }

    pub fn mark_draft_failed(&mut self, id: ReviewDraftId, message: impl Into<String>) -> bool {
        let Some(draft) = self.drafts.get_mut(&id) else {
            return false;
        };
        draft.state = ReviewDraftState::Failed(message.into());
        true
    }

    pub fn mark_draft_submitted(
        &mut self,
        id: ReviewDraftId,
        external_comment_id: Option<ReviewCommentId>,
    ) -> bool {
        let Some(draft) = self.drafts.get_mut(&id) else {
            return false;
        };
        draft.state = ReviewDraftState::Submitted {
            external_comment_id,
        };
        true
    }

    pub fn pending_drafts(&self) -> impl Iterator<Item = &ReviewDraft> {
        self.drafts
            .values()
            .filter(|draft| matches!(draft.state, ReviewDraftState::Pending))
    }

    pub fn metrics(&self) -> ReviewSessionMetrics {
        let mut metrics = ReviewSessionMetrics::default();
        for thread in &self.threads {
            match thread.status.resolution {
                ReviewResolution::Resolved => metrics.resolved_threads += 1,
                ReviewResolution::Unknown | ReviewResolution::Unresolved => {
                    metrics.unresolved_threads += 1
                }
            }
            if thread.status.outdated {
                metrics.outdated_threads += 1;
            }
        }
        for draft in self.drafts.values() {
            match draft.state {
                ReviewDraftState::Pending => metrics.pending_drafts += 1,
                ReviewDraftState::Failed(_) => metrics.failed_drafts += 1,
                ReviewDraftState::Submitting | ReviewDraftState::Submitted { .. } => {}
            }
        }
        metrics
    }

    pub fn build_github_review_request(
        &self,
        decision: ReviewDecision,
        body: Option<String>,
    ) -> Result<CreatePullRequestReview> {
        let mut comments = Vec::new();
        for draft in self.pending_drafts() {
            match &draft.kind {
                ReviewDraftKind::InlineComment => {
                    let anchor = draft.anchor.as_ref().ok_or_else(|| {
                        DiffyError::General("inline review draft is missing an anchor".to_owned())
                    })?;
                    let line_range = anchor.line_range.ok_or_else(|| {
                        DiffyError::General(
                            "inline review draft is missing a line range".to_owned(),
                        )
                    })?;
                    let side = anchor.side.ok_or_else(|| {
                        DiffyError::General("inline review draft is missing a side".to_owned())
                    })?;
                    comments.push(CreatePullRequestReviewDraftComment {
                        path: anchor.path.clone(),
                        body: draft.body.clone(),
                        line: line_range.github_line(),
                        side: side.into(),
                        start_line: line_range.github_start_line(),
                        start_side: line_range.github_start_line().map(|_| side.into()),
                    });
                }
                ReviewDraftKind::Reply { .. } => {
                    return Err(DiffyError::General(
                        "reply drafts must be submitted through the review thread reply operation"
                            .to_owned(),
                    ));
                }
                ReviewDraftKind::General => {}
            }
        }

        let body = body.filter(|body| !body.trim().is_empty()).or_else(|| {
            self.pending_drafts()
                .find(|draft| matches!(draft.kind, ReviewDraftKind::General))
                .map(|draft| draft.body.clone())
        });

        Ok(CreatePullRequestReview {
            commit_id: non_empty(&self.pull_request.head_sha),
            body,
            event: Some(decision.into()),
            comments,
        })
    }

    pub fn draft_count(&self) -> usize {
        self.drafts.len()
    }

    pub fn unresolved_thread_count(&self) -> usize {
        self.threads
            .iter()
            .filter(|thread| thread.status.resolution != ReviewResolution::Resolved)
            .count()
    }

    pub fn threads_for_path<'a>(&'a self, path: &'a str) -> impl Iterator<Item = &'a ReviewThread> {
        self.threads
            .iter()
            .filter(move |thread| thread.path() == Some(path))
    }

    fn insert_draft(
        &mut self,
        kind: ReviewDraftKind,
        anchor: Option<ReviewAnchor>,
        body: impl Into<String>,
    ) -> ReviewDraftId {
        let id = ReviewDraftId(self.next_draft_id);
        self.next_draft_id = self.next_draft_id.saturating_add(1);
        self.drafts.insert(
            id,
            ReviewDraft {
                id,
                kind,
                anchor,
                body: body.into(),
                state: ReviewDraftState::Pending,
            },
        );
        id
    }
}

fn build_threads(comments: Vec<PullRequestReviewComment>) -> Vec<ReviewThread> {
    let mut grouped: BTreeMap<i64, Vec<PullRequestReviewComment>> = BTreeMap::new();
    for comment in comments {
        grouped
            .entry(comment.in_reply_to_id.unwrap_or(comment.id))
            .or_default()
            .push(comment);
    }

    grouped
        .into_iter()
        .map(|(root_id, mut comments)| {
            comments.sort_by(|a, b| {
                a.created_at
                    .cmp(&b.created_at)
                    .then_with(|| a.updated_at.cmp(&b.updated_at))
                    .then_with(|| a.id.cmp(&b.id))
            });
            let thread_id = ReviewThreadId::github_root(root_id);
            let normalized = comments
                .iter()
                .map(ReviewComment::from_github_comment)
                .collect::<Vec<_>>();
            let anchor = normalized.iter().find_map(|comment| comment.anchor.clone());
            let outdated = normalized.iter().any(|comment| comment.outdated)
                || anchor.as_ref().is_some_and(ReviewAnchor::is_outdated);
            ReviewThread {
                id: thread_id,
                backend_node_id: None,
                anchor,
                comments: normalized,
                status: ReviewThreadStatus {
                    resolution: ReviewResolution::Unknown,
                    outdated,
                    collapsed: false,
                },
                permissions: ReviewThreadPermissions::default(),
            }
        })
        .collect()
}

impl ReviewThread {
    pub fn from_github_thread(thread: &GitHubPullRequestReviewThread) -> Self {
        let id = ReviewThreadId::github_node(thread.node_id.clone());
        let anchor = Some(ReviewAnchor::from_github_thread(thread));
        let resolution = if thread.is_resolved {
            ReviewResolution::Resolved
        } else {
            ReviewResolution::Unresolved
        };
        let comments = thread
            .comments
            .iter()
            .map(|comment| ReviewComment::from_github_thread_comment(thread, comment))
            .collect();
        Self {
            id,
            backend_node_id: Some(thread.node_id.clone()),
            anchor,
            comments,
            status: ReviewThreadStatus {
                resolution,
                outdated: thread.is_outdated,
                collapsed: thread.is_collapsed,
            },
            permissions: ReviewThreadPermissions {
                can_reply: thread.viewer_can_reply,
                can_resolve: thread.viewer_can_resolve,
                can_unresolve: thread.viewer_can_unresolve,
            },
        }
    }

    pub fn path(&self) -> Option<&str> {
        self.anchor.as_ref().map(|anchor| anchor.path.as_str())
    }
}

fn review_line_range(start: Option<u32>, end: Option<u32>) -> Option<ReviewLineRange> {
    let end = end?;
    Some(
        start
            .map(|start| ReviewLineRange::from_inclusive(start, end))
            .unwrap_or_else(|| ReviewLineRange::new(end, 1)),
    )
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn sanitize_key_part(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '_',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::forge::github::GitHubReviewCommentUser;

    fn github_comment(
        id: i64,
        in_reply_to_id: Option<i64>,
        body: &str,
    ) -> PullRequestReviewComment {
        PullRequestReviewComment {
            id,
            in_reply_to_id,
            path: "src/lib.rs".to_owned(),
            body: body.to_owned(),
            commit_id: "head".to_owned(),
            original_commit_id: "base".to_owned(),
            line: Some(12),
            original_line: Some(12),
            side: Some(GitHubReviewSide::Right),
            start_line: Some(10),
            original_start_line: Some(10),
            start_side: Some(GitHubReviewSide::Right),
            subject_type: Some("line".to_owned()),
            html_url: format!("https://github.test/comment/{id}"),
            created_at: format!("2026-01-01T00:00:0{id}Z"),
            updated_at: String::new(),
            user: Some(GitHubReviewCommentUser {
                login: "reviewer".to_owned(),
                avatar_url: String::new(),
            }),
        }
    }

    fn pull_request_info() -> PullRequestInfo {
        PullRequestInfo {
            title: "Review me".to_owned(),
            state: "open".to_owned(),
            author_login: "author".to_owned(),
            number: 42,
            additions: 10,
            deletions: 2,
            changed_files: 1,
            base_branch: "main".to_owned(),
            head_branch: "feature".to_owned(),
            base_sha: "base".to_owned(),
            head_sha: "head".to_owned(),
            base_repo_url: String::new(),
            head_repo_url: String::new(),
        }
    }

    #[test]
    fn github_review_comments_are_grouped_into_threads() {
        let session = ReviewSession::from_github_review_comments(
            ReviewTarget::github("owner", "repo", 42),
            pull_request_info(),
            vec![
                github_comment(2, Some(1), "reply"),
                github_comment(1, None, "root"),
            ],
        );

        assert_eq!(session.threads.len(), 1);
        let thread = &session.threads[0];
        assert_eq!(thread.id, ReviewThreadId::github_root(1));
        assert_eq!(thread.comments.len(), 2);
        assert_eq!(thread.comments[0].body, "root");
        assert_eq!(thread.comments[1].body, "reply");
        assert_eq!(thread.anchor.as_ref().unwrap().path, "src/lib.rs");
    }

    #[test]
    fn github_anchor_maps_to_carbon_anchor() {
        let anchor = ReviewAnchor::from_github_comment(&github_comment(1, None, "root")).unwrap();
        let carbon_file = FileDiff {
            id: carbon::FileId(7),
            new_path: Some("src/lib.rs".to_owned()),
            old_path: Some("src/lib.rs".to_owned()),
            old_oid: Some(carbon::ObjectId("base".to_owned())),
            new_oid: Some(carbon::ObjectId("head".to_owned())),
            ..FileDiff::default()
        };

        let carbon_anchor = anchor.to_carbon_anchor(&carbon_file).unwrap();

        assert_eq!(carbon_anchor.file_id, carbon::FileId(7));
        assert_eq!(carbon_anchor.side, Some(DiffSide::New));
        assert_eq!(carbon_anchor.line_range, LineRange::new(10, 3));
    }

    #[test]
    fn review_session_tracks_local_drafts() {
        let mut session = ReviewSession::new(
            ReviewTarget::github("owner", "repo", 42),
            pull_request_info(),
        );
        let first = session.create_general_draft("overall note");
        let second = session.create_inline_draft(
            ReviewAnchor::inline("src/lib.rs", ReviewSide::New, ReviewLineRange::new(12, 1)),
            "inline note",
        );

        assert_eq!(first, ReviewDraftId(1));
        assert_eq!(second, ReviewDraftId(2));
        assert_eq!(session.draft_count(), 2);
        assert_eq!(
            session.remove_draft(first).unwrap().kind,
            ReviewDraftKind::General
        );
    }

    #[test]
    fn review_session_builds_github_review_from_inline_drafts() {
        let mut session = ReviewSession::new(
            ReviewTarget::github("owner", "repo", 42),
            pull_request_info(),
        );
        session.create_inline_draft(
            ReviewAnchor::inline("src/lib.rs", ReviewSide::New, ReviewLineRange::new(10, 3)),
            "Please simplify this.",
        );
        session.create_general_draft("Overall note");

        let request = session
            .build_github_review_request(ReviewDecision::RequestChanges, None)
            .unwrap();

        assert_eq!(request.commit_id.as_deref(), Some("head"));
        assert_eq!(request.body.as_deref(), Some("Overall note"));
        assert_eq!(
            request.event,
            Some(GitHubPullRequestReviewEvent::RequestChanges)
        );
        assert_eq!(request.comments.len(), 1);
        assert_eq!(request.comments[0].path, "src/lib.rs");
        assert_eq!(request.comments[0].line, 12);
        assert_eq!(request.comments[0].start_line, Some(10));
    }

    #[test]
    fn review_session_rejects_reply_drafts_in_review_request() {
        let mut session = ReviewSession::new(
            ReviewTarget::github("owner", "repo", 42),
            pull_request_info(),
        );
        session.create_reply_draft(ReviewThreadId::github_node("thread"), "Reply");

        assert!(
            session
                .build_github_review_request(ReviewDecision::Comment, None)
                .is_err()
        );
    }
}
