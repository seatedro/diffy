use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use quark_ui::document::DocumentEvent;

use crate::contracts::shared::{AccountEpoch, JobId, SourceTarget};
use crate::core::forge::github::{
    CreatePullRequestReview, CreatePullRequestReviewComment, CreatePullRequestReviewReply,
    DeviceFlowState, GitHubPullRequestReviewData, GitHubPullRequestReviewThreadComment,
    GitHubReviewThreadResolution, GitHubUser, MyPullRequest, PullRequestInfo, PullRequestOverview,
    PullRequestReview, PullRequestReviewComment, SubmitPullRequestReview,
    UpdatePullRequestReviewComment,
};
use crate::core::review::{
    ReviewDecision, ReviewDraftId, ReviewSession, ReviewSessionKey, ReviewTarget, ReviewThreadId,
};

#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerFormat {
    Bold,
    Italic,
    Code,
    Link,
    BulletList,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CardKey {
    Thread(ReviewThreadId),
    Draft(ReviewDraftId),
    NewComment,
    Conversation,
    Overview(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReviewAction {
    OpenAuthModal,
    CloseOverlay,
    StartDeviceFlow,
    OpenDeviceFlowBrowser,
    OpenAccountMenu,
    SignOut,
    LoadPullRequest { url: String },
    PreviewPullRequest,
    OpenPullRequestInBrowser,
    OpenComposer,
    OpenComposerAt(SourceTarget),
    SubmitComment,
    CancelComment,
    FormatComment(ComposerFormat),
    SetComposerPreview(bool),
    ReplyToThread(ReviewThreadId),
    EditComment { comment_node_id: String },
    ToggleCommentMenu { comment_node_id: String },
    DeleteComment { comment_node_id: String },
    SubmitReview { decision: ReviewDecision },
    DiscardDrafts,
    ToggleThread(ReviewThreadId),
    SetThreadResolved { id: ReviewThreadId, resolved: bool },
    EditDraft(ReviewDraftId),
    DeleteDraft(ReviewDraftId),
    CardDocument { card: CardKey, event: DocumentEvent },
    CopyCardSelection(CardKey),
    ScrollComposer(i32),
    OpenSubmitMenu,
    RequestAuthorAvatars(Vec<String>),
    PeekPullRequest { url: String },
    ToggleFileComments(String),
    OpenOverview,
    CloseOverview,
    ToggleOverview,
    RefreshOverview,
    ToggleOverviewChecks,
    OpenConversationComposer,
    OpenLink(String),
    Pulls(PullsAction),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PullsAction {
    Open,
    Close,
    Refresh,
    Move(i32),
    Select(String),
    Review,
    OpenOnGitHub,
    CopyLink,
    FocusFilter,
    EscapeFilter,
    ClearFilter,
    LocateClone,
    CloneChosen { repo: String, path: PathBuf },
}

impl From<PullsAction> for ReviewAction {
    fn from(action: PullsAction) -> Self {
        Self::Pulls(action)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ReviewScope {
    pub account: AccountEpoch,
    pub request: JobId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PullRequestRef {
    pub owner: String,
    pub repo: String,
    pub number: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubRequest {
    LoadPullRequest {
        url: String,
        repo_path: PathBuf,
        token: Option<Secret>,
    },
    FetchUser {
        token: Secret,
    },
    PeekPullRequest {
        pr: PullRequestRef,
        token: Option<Secret>,
    },
    FetchReviewComments {
        pr: PullRequestRef,
        token: Option<Secret>,
    },
    FetchReviewData {
        pr: PullRequestRef,
        token: Option<Secret>,
    },
    CreateReviewComment {
        pr: PullRequestRef,
        token: Option<Secret>,
        comment: CreatePullRequestReviewComment,
    },
    CreateReviewReply {
        pr: PullRequestRef,
        comment_id: i64,
        token: Option<Secret>,
        reply: CreatePullRequestReviewReply,
    },
    UpdateReviewComment {
        pr: PullRequestRef,
        comment_id: i64,
        token: Option<Secret>,
        update: UpdatePullRequestReviewComment,
    },
    DeleteReviewComment {
        pr: PullRequestRef,
        comment_id: i64,
        token: Option<Secret>,
    },
    CreateReview {
        pr: PullRequestRef,
        token: Option<Secret>,
        review: CreatePullRequestReview,
    },
    SubmitSessionDrafts {
        session: Box<ReviewSession>,
        decision: ReviewDecision,
        body: Option<String>,
        token: Option<Secret>,
    },
    AddThreadReply {
        pr: PullRequestRef,
        thread_node_id: String,
        review_node_id: Option<String>,
        token: Option<Secret>,
        body: String,
    },
    UpdateCommentGraphql {
        pr: PullRequestRef,
        comment_node_id: String,
        token: Option<Secret>,
        body: String,
    },
    DeleteCommentGraphql {
        pr: PullRequestRef,
        comment_node_id: String,
        token: Option<Secret>,
    },
    SetThreadResolution {
        pr: PullRequestRef,
        thread_node_id: String,
        token: Option<Secret>,
        resolved: bool,
    },
    SubmitReview {
        pr: PullRequestRef,
        review_id: i64,
        token: Option<Secret>,
        submit: SubmitPullRequestReview,
    },
    FetchPullRequestOverview {
        pr: PullRequestRef,
        token: Option<Secret>,
    },
    AddPullRequestComment {
        pr: PullRequestRef,
        token: Option<Secret>,
        body: String,
    },
    FetchMyPullRequests {
        token: Secret,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestComparison {
    pub info: PullRequestInfo,
    pub left_ref: String,
    pub right_ref: String,
}

#[derive(Debug, Clone)]
pub enum GitHubResponse {
    PullRequestCached {
        url: String,
        comparison: PullRequestComparison,
    },
    PullRequestLoaded {
        url: String,
        comparison: PullRequestComparison,
    },
    PullRequestLoadFailed {
        url: String,
        message: String,
    },
    UserFetched(GitHubUser),
    UserFetchFailed {
        message: String,
    },
    PullRequestPeeked {
        pr: PullRequestRef,
        info: PullRequestInfo,
    },
    PullRequestPeekFailed {
        pr: PullRequestRef,
        message: String,
    },
    ReviewCommentsLoaded {
        pr: PullRequestRef,
        comments: Vec<PullRequestReviewComment>,
    },
    ReviewCommentsLoadFailed {
        pr: PullRequestRef,
        message: String,
    },
    ReviewDataLoaded {
        pr: PullRequestRef,
        data: Box<GitHubPullRequestReviewData>,
    },
    ReviewDataLoadFailed {
        pr: PullRequestRef,
        message: String,
    },
    ReviewCommentCreated {
        pr: PullRequestRef,
        comment: PullRequestReviewComment,
    },
    ReviewCommentCreateFailed {
        pr: PullRequestRef,
        message: String,
    },
    ReviewCommentReplied {
        pr: PullRequestRef,
        comment: PullRequestReviewComment,
    },
    ReviewCommentReplyFailed {
        pr: PullRequestRef,
        message: String,
    },
    ReviewCommentUpdated {
        pr: PullRequestRef,
        comment: PullRequestReviewComment,
    },
    ReviewCommentUpdateFailed {
        pr: PullRequestRef,
        comment_id: i64,
        message: String,
    },
    ReviewCommentDeleted {
        pr: PullRequestRef,
        comment_id: i64,
    },
    ReviewCommentDeleteFailed {
        pr: PullRequestRef,
        comment_id: i64,
        message: String,
    },
    ReviewCreated {
        pr: PullRequestRef,
        review: PullRequestReview,
    },
    ReviewCreateFailed {
        pr: PullRequestRef,
        message: String,
    },
    DraftsSubmitted {
        pr: PullRequestRef,
        review: PullRequestReview,
        draft_ids: Vec<ReviewDraftId>,
    },
    DraftsSubmitFailed {
        pr: PullRequestRef,
        draft_ids: Vec<ReviewDraftId>,
        message: String,
    },
    ReviewSubmitted {
        pr: PullRequestRef,
        review: PullRequestReview,
    },
    ReviewSubmitFailed {
        pr: PullRequestRef,
        review_id: i64,
        message: String,
    },
    ThreadReplyAdded {
        pr: PullRequestRef,
        thread_node_id: String,
        comment: GitHubPullRequestReviewThreadComment,
    },
    ThreadReplyAddFailed {
        pr: PullRequestRef,
        thread_node_id: String,
        message: String,
    },
    CommentGraphqlUpdated {
        pr: PullRequestRef,
        comment: GitHubPullRequestReviewThreadComment,
    },
    CommentGraphqlUpdateFailed {
        pr: PullRequestRef,
        comment_node_id: String,
        message: String,
    },
    CommentGraphqlDeleted {
        pr: PullRequestRef,
        comment_node_id: String,
        comment: Option<GitHubPullRequestReviewThreadComment>,
    },
    CommentGraphqlDeleteFailed {
        pr: PullRequestRef,
        comment_node_id: String,
        message: String,
    },
    ThreadResolutionChanged {
        pr: PullRequestRef,
        resolution: GitHubReviewThreadResolution,
    },
    ThreadResolutionChangeFailed {
        pr: PullRequestRef,
        thread_node_id: String,
        message: String,
    },
    PullRequestOverviewLoaded {
        pr: PullRequestRef,
        overview: Box<PullRequestOverview>,
    },
    PullRequestOverviewLoadFailed {
        pr: PullRequestRef,
        message: String,
    },
    PullRequestCommentAdded {
        pr: PullRequestRef,
    },
    PullRequestCommentAddFailed {
        pr: PullRequestRef,
        body: String,
        message: String,
    },
    MyPullRequestsLoaded(Vec<MyPullRequest>),
    MyPullRequestsLoadFailed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewEffect {
    GitHub {
        scope: ReviewScope,
        request: Box<GitHubRequest>,
    },
    StartDeviceFlow {
        flow: JobId,
        client_id: String,
    },
    PollDeviceFlow {
        flow: JobId,
        client_id: String,
        device_code: String,
        interval_seconds: u32,
    },
    LoadToken,
    SaveToken(Secret),
    ClearToken,
    LoadSession {
        scope: ReviewScope,
        target: ReviewTarget,
        pull_request: Box<PullRequestInfo>,
    },
    SaveSession {
        scope: ReviewScope,
        session: Box<ReviewSession>,
    },
    SaveSessionDraft {
        scope: ReviewScope,
        session: Box<ReviewSession>,
    },
    FetchAvatar {
        url: String,
    },
    ResolveClones {
        job: JobId,
        current: Option<PathBuf>,
    },
    ValidateClone {
        repo: String,
        path: PathBuf,
    },
}

#[derive(Debug)]
pub struct AvatarImage {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub enum ReviewEvent {
    GitHub {
        scope: ReviewScope,
        response: Box<GitHubResponse>,
    },
    DeviceFlowStarted {
        flow: JobId,
        state: DeviceFlowState,
    },
    DeviceFlowStartFailed {
        flow: JobId,
        message: String,
    },
    DeviceFlowCompleted {
        flow: JobId,
        token: Secret,
    },
    DeviceFlowFailed {
        flow: JobId,
        message: String,
    },
    TokenLoaded {
        token: Option<Secret>,
    },
    TokenLoadFailed {
        message: String,
    },
    TokenSaveFailed {
        message: String,
    },
    SessionLoaded {
        scope: ReviewScope,
        target: ReviewTarget,
        session: Box<ReviewSession>,
    },
    SessionLoadFailed {
        scope: ReviewScope,
        target: ReviewTarget,
        message: String,
    },
    SessionSaved {
        scope: ReviewScope,
        key: ReviewSessionKey,
    },
    SessionSaveFailed {
        scope: ReviewScope,
        key: ReviewSessionKey,
        message: String,
    },
    AvatarFetched {
        url: String,
        image: Arc<AvatarImage>,
    },
    AvatarFetchFailed {
        url: String,
        message: String,
    },
    ClonesResolved {
        job: JobId,
        clones: Vec<(String, PathBuf)>,
    },
    CloneValidated {
        repo: String,
        result: Result<PathBuf, String>,
    },
}

impl ReviewEvent {
    pub fn account_scoped(&self) -> bool {
        match self {
            Self::GitHub { response, .. } => matches!(
                **response,
                GitHubResponse::UserFetched(_)
                    | GitHubResponse::UserFetchFailed { .. }
                    | GitHubResponse::MyPullRequestsLoaded(_)
                    | GitHubResponse::MyPullRequestsLoadFailed { .. }
            ),
            Self::DeviceFlowStarted { .. }
            | Self::DeviceFlowStartFailed { .. }
            | Self::DeviceFlowCompleted { .. }
            | Self::DeviceFlowFailed { .. }
            | Self::TokenLoaded { .. }
            | Self::TokenLoadFailed { .. }
            | Self::TokenSaveFailed { .. }
            | Self::AvatarFetched { .. }
            | Self::AvatarFetchFailed { .. }
            | Self::ClonesResolved { .. }
            | Self::CloneValidated { .. } => true,
            Self::SessionLoaded { .. }
            | Self::SessionLoadFailed { .. }
            | Self::SessionSaved { .. }
            | Self::SessionSaveFailed { .. } => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReviewContext {
    pub account: AccountEpoch,
    pub target: ReviewTarget,
    pub pull_request: Arc<PullRequestInfo>,
    pub left_ref: String,
    pub right_ref: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::shell::{Action, AppEvent, Effect};

    #[test]
    fn routes_through_root_contracts() {
        assert_eq!(
            Action::from(ReviewAction::SignOut),
            Action::Review(ReviewAction::SignOut)
        );
        assert_eq!(
            Effect::from(ReviewEffect::LoadToken),
            Effect::Review(ReviewEffect::LoadToken)
        );
        assert!(matches!(
            AppEvent::from(ReviewEvent::TokenLoaded { token: None }),
            AppEvent::Review(ReviewEvent::TokenLoaded { token: None })
        ));
    }
}
