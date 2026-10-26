use quark::{Px, view};
use quark_app::ViewContext;
use quark_components::{Avatar, Modal, avatar, badge};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};

use super::ReviewController;
use super::controls::{self, GhostChip, Tone};
use super::overview::{CheckTone, check_icon, info_state, state_badge};
use super::state::{AuthStatus, ReviewOverlay};
use crate::app::icons;
use crate::app::shell::Slot;
use crate::app::text_targets::{REVIEW_ACCOUNT, REVIEW_DIALOG};
use crate::app::tokens::{self, Bp, Ico, Layout, Rad, Shadow, Sp, Sz, TextStyle};
use crate::contracts::review::{HeadChange, PullsAction, ReviewAction};
use crate::contracts::shell::ShellAction;
use crate::core::forge::github::PullRequestCheckSummary;
use crate::core::review::ReviewDecision;

const CARD_MAX_WIDTH: f32 = 600.0;
const ACCOUNT_MENU_WIDTH: f32 = 260.0;

fn action(action: ReviewAction) -> quark_ui::Action {
    action.into()
}

pub fn default_card_width(width: f32, theme: &Theme) -> f32 {
    let scale = tokens::ui_scale(theme);
    (width - tokens::scaled(Sp::LG, scale) * 2.0)
        .min(tokens::scaled(CARD_MAX_WIDTH, scale))
        .max(1.0)
}

pub fn prepare(review: &mut ReviewController, width: f32, cx: &mut ViewContext) {
    let default = default_card_width(width, cx.theme);
    let prepared = review.host.prepare(cx, default);
    let measured = review.presentation().measure(cx);
    if prepared || measured {
        review.bump_presentation();
    }
}

pub fn view(review: &mut ReviewController, slot: &Slot, cx: &mut ViewContext) -> AnyElement {
    prepare(review, slot.width, cx);
    review.observe_focus(cx.focus());
    let theme = cx.theme;
    let scale = tokens::ui_scale(theme);
    let submit = review
        .active_pull_request()
        .and(review.review_counts())
        .filter(|_| review.signed_in())
        .map(|(_, pending, failed)| {
            let tone = if pending + failed > 0 {
                Tone::Primary
            } else {
                Tone::Secondary
            };
            let button =
                controls::button("Submit review", action(ReviewAction::OpenSubmitMenu), tone);
            view! { <div class="shrink-0" test_id="review.submit">{button}</div> }
        });
    let review_pr = review.active_pull_request().is_none().then(|| {
        let button = controls::button(
            "Review PR",
            action(ReviewAction::Pulls(PullsAction::Open)),
            Tone::Ghost,
        )
        .icon(lucide::GIT_PULL_REQUEST)
        .active(review.pulls_open());
        view! { <div class="shrink-0" test_id="review.open_pull_request">{button}</div> }
    });
    let account = account_view(review, theme);
    view! { scale,
        <div h={Px(slot.height)} class="flex-row items-center justify-end overflow-hidden" min_w={0.0} gap={Sp::SM} test_id="title.review"
             on:key:escape={action(ReviewAction::CloseOverlay)}>
            {?submit}
            {?review_pr}
            <div class="flex-row items-center shrink-0">{account}</div>
        </div>
    }
}

pub fn pull_request_bar_height(review: &ReviewController, theme: &Theme) -> f32 {
    if review.active_pull_request().is_some() && !review.pulls_open() {
        tokens::scaled(Layout::COMPARE_BAR_H, tokens::ui_scale(theme))
    } else {
        0.0
    }
}

pub fn pull_request_bar(
    review: &ReviewController,
    width: f32,
    height: f32,
    theme: &Theme,
) -> Option<AnyElement> {
    let active = review
        .active_pull_request()
        .filter(|_| !review.pulls_open())?;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let info = active
        .comparison
        .as_ref()
        .map(|comparison| &comparison.info);
    let overview_data = review.overview().data.as_deref();
    let (number, title, color) = match (info, &active.error) {
        (_, Some(_)) => (
            format!("#{}", active.pr.number),
            "Couldn\u{2019}t load this pull request".to_owned(),
            tc.status_error,
        ),
        (Some(info), None) => (
            format!("#{}", info.number),
            info.title.clone(),
            tc.text_strong,
        ),
        (None, None) => (
            format!("#{}", active.pr.number),
            "Loading pull request\u{2026}".to_owned(),
            tc.text_muted,
        ),
    };
    let state = match (overview_data, info) {
        (Some(overview), _) => Some((overview.state.clone(), overview.is_draft)),
        (None, Some(info)) => Some((info_state(info), info.draft)),
        (None, None) => None,
    }
    .map(|(state, draft)| {
        let (label, icon, variant) = state_badge(&state, draft);
        view! {
            <div class="shrink-0" accessibility_role={AccessibilityRole::Label}
                 aria-label={format!("State: {label}")}
                 test_id="review.pull_request.state">
                {badge(label).icon(icon).variant(variant)}
            </div>
        }
    });
    let fits = width >= Bp::COMPACT * scale;
    let status = status_pill(
        overview_data
            .filter(|_| fits)
            .map(|overview| &overview.checks),
        review.review_counts(),
        theme,
    );
    let update = active.update.map(|change| {
        let label = if active.loading {
            "Updating\u{2026}".to_owned()
        } else {
            match change {
                HeadChange::Ahead(1) => "1 new commit".to_owned(),
                HeadChange::Ahead(count) => format!("{count} new commits"),
                HeadChange::Rewritten => "Force-pushed".to_owned(),
                HeadChange::Moved => "New changes".to_owned(),
                HeadChange::Same => "Base changed".to_owned(),
            }
        };
        let button = controls::button(
            label,
            action(ReviewAction::UpdatePullRequest),
            Tone::Secondary,
        )
        .icon(lucide::REFRESH)
        .tooltip("Load the latest push".to_owned());
        view! { <div class="shrink-0" test_id="review.pull_request.update">{button}</div> }
    });
    let open = controls::icon_button(
        lucide::EXTERNAL_LINK,
        "Open on GitHub",
        action(ReviewAction::OpenPullRequestInBrowser),
        theme,
    );
    let overview = review.overview_open();
    let tab = |label: &str, active: bool, on_click: ReviewAction| {
        controls::button(label, action(on_click), Tone::Ghost).active(active)
    };
    let overview_tab = tab("Overview", overview, ReviewAction::OpenOverview);
    let files_tab = tab("Files changed", !overview, ReviewAction::CloseOverview);
    Some(view! { scale,
        <div w={Px(width)} h={Px(height)} class="flex-row items-center shrink-0 overflow-hidden"
             px={Sp::MD} gap={Sp::SM}
             bg={tc.title_bar_background} border_b={tc.border_variant}
             accessibility_role={AccessibilityRole::Toolbar}
             aria-label="Pull request"
             test_id="review.pull_request">
            {?state}
            <div class="flex-row items-center overflow-hidden" min_w={0.0} gap={Sp::SM}
                 px={Sp::XS} py={Sp::XXS} rounded={Rad::MD}
                 hover_bg={tc.ghost_element_hover}
                 cursor={CursorHint::Pointer}
                 accessibility_role={AccessibilityRole::Button}
                 aria-label="Pull request overview"
                 tooltip={if overview { "Back to changed files" } else { "Open the overview" }}
                 test_id="review.pull_request.title"
                 on:click={action(ReviewAction::ToggleOverview)}>
                <text size={Px(small)} color={tc.text_muted}>{number}</text>
                <text size={Px(small)} class="truncate font-medium" color={color}>{title}</text>
            </div>
            <spacer />
            {?update}
            {?status}
            <div class="flex-row items-center shrink-0" gap={Sp::XXS}
                 accessibility_role={AccessibilityRole::Group}
                 aria-label="Pull request views">
                <div test_id="review.pull_request.overview">{overview_tab}</div>
                <div test_id="review.pull_request.files">{files_tab}</div>
            </div>
            <div class="shrink-0">{open}</div>
        </div>
    })
}

/// Checks, open threads and unsent comments in one pill that opens the
/// overview, where each has its detail.
fn status_pill(
    checks: Option<&PullRequestCheckSummary>,
    counts: Option<(usize, usize, usize)>,
    theme: &Theme,
) -> Option<AnyElement> {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let mut parts = Vec::new();
    let mut labels = Vec::new();
    if let Some(checks) = checks.filter(|checks| checks.total_count > 0) {
        let (tone, count) = if checks.failure_count > 0 {
            (CheckTone::Failed, checks.failure_count)
        } else if checks.pending_count > 0 {
            (CheckTone::Pending, checks.pending_count)
        } else {
            (CheckTone::Passed, checks.total_count)
        };
        let (icon, color) = check_icon(tone, theme);
        parts.push((icon, color, count.to_string(), tc.text));
        labels.push(format!(
            "Checks: {} passed, {} failed, {} pending",
            checks.success_count, checks.failure_count, checks.pending_count
        ));
    }
    if let Some((unresolved, pending, failed)) = counts {
        let threads = unresolved.to_string();
        parts.push((icons::MESSAGE_SQUARE, tc.text_muted, threads, tc.text));
        labels.push(format!("{unresolved} unresolved"));
        if pending > 0 {
            let label = format!("{pending} pending");
            parts.push((
                lucide::PENCIL,
                tc.status_warning,
                label.clone(),
                tc.status_warning,
            ));
            labels.push(label);
        }
        if failed > 0 {
            let label = format!("{failed} failed");
            parts.push((
                lucide::ALERT_CIRCLE,
                tc.status_error,
                label.clone(),
                tc.status_error,
            ));
            labels.push(label);
        }
    }
    if parts.is_empty() {
        return None;
    }
    let mut children = Vec::new();
    for (index, (icon, icon_color, count, count_color)) in parts.into_iter().enumerate() {
        if index > 0 {
            children.push(view! { scale,
                <div class="shrink-0" w={Px(1.0)} h={Ico::SM} bg={tc.border_variant} />
            });
        }
        children.push(view! { scale,
            <div class="flex-row items-center shrink-0" gap={Sp::XXS}>
                <icon svg={icon} size={Ico::SM} color={icon_color} />
                <text size={Px(small)} color={count_color}>{count}</text>
            </div>
        });
    }
    let label = labels.join("\n");
    Some(view! { scale,
        <div class="flex-row items-center shrink-0" gap={Sp::SM} px={Sp::SM} py={Sp::XXS}
             rounded={Rad::PILL} bg={tc.element_background}
             hover_bg={tc.element_hover}
             cursor={CursorHint::Pointer}
             accessibility_role={AccessibilityRole::Button}
             aria-label={label.replace('\n', ", ")}
             tooltip={label}
             test_id="review.pull_request.status"
             on:click={action(ReviewAction::OpenOverview)}>
            {...children}
        </div>
    })
}

fn account_view(review: &ReviewController, theme: &Theme) -> AnyElement {
    let scale = tokens::ui_scale(theme);
    if !review.signed_in() {
        return controls::ghost_chip(
            GhostChip {
                icon: lucide::GITHUB_MARK,
                label: "Sign in",
                accessible: "Sign in to GitHub",
                focus: REVIEW_ACCOUNT,
                test_id: "review.sign_in",
            },
            action(ReviewAction::OpenAuthModal),
            theme,
        );
    }
    let account = review.account();
    let login = account
        .user
        .as_ref()
        .map(|user| user.login.clone())
        .unwrap_or_else(|| "GitHub".to_owned());
    let picture = avatar(login).size(Ico::XL).image(account.avatar.clone());
    view! { scale,
        <div
            class="flex-row items-center justify-center"
            rounded={Rad::PILL}
            hover_bg={theme.colors.ghost_element_hover}
            p={Sp::XXS}
            accessibility_role={AccessibilityRole::Button}
            aria-label="GitHub account"
            focus_ring={REVIEW_ACCOUNT}
            test_id="review.account"
            on:click={action(ReviewAction::OpenAccountMenu)}
        >
            {picture}
        </div>
    }
}

pub fn overlay(review: &ReviewController, slot: &Slot, cx: &mut ViewContext) -> Option<AnyElement> {
    let theme = cx.theme;
    match review.overlay_open()? {
        ReviewOverlay::Auth => Some(auth_modal(review, slot, theme)),
        ReviewOverlay::AccountMenu => Some(account_menu(review, slot, theme)),
        ReviewOverlay::SubmitReview => Some(submit_menu(review, slot, theme)),
    }
}

fn auth_modal(review: &ReviewController, slot: &Slot, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let note = |label: &str| {
        let label = label.to_owned();
        view! { <text size={small} color={tc.text_muted}>{label}</text> }
    };
    let cancel = || controls::button("Cancel", action(ReviewAction::CloseOverlay), Tone::Ghost);
    let (content, actions) = match &review.account().auth {
        _ if review.signed_in() => {
            let login = review
                .account()
                .user
                .as_ref()
                .map_or_else(|| "GitHub".to_owned(), |user| format!("@{}", user.login));
            (
                note(&format!("Signed in as {login}.")),
                vec![controls::button(
                    "Done",
                    action(ReviewAction::CloseOverlay),
                    Tone::Primary,
                )],
            )
        }
        AuthStatus::Idle | AuthStatus::Done => (
            note("GitHub shows a one-time code to approve this app in your browser."),
            vec![
                cancel(),
                controls::button(
                    "Continue with GitHub",
                    action(ReviewAction::StartDeviceFlow),
                    Tone::Primary,
                ),
            ],
        ),
        AuthStatus::Starting => (
            view! {
                <div test_id="review.auth.starting">{note("Contacting GitHub\u{2026}")}</div>
            },
            vec![cancel()],
        ),
        AuthStatus::Awaiting(flow) => {
            let copied = slot.shared.copied.as_deref() == Some(flow.user_code.as_str());
            let copy = controls::icon_button(
                if copied { lucide::CHECK } else { lucide::COPY },
                if copied { "Copied" } else { "Copy code" },
                ShellAction::CopyText(flow.user_code.clone()),
                theme,
            );
            let content = view! { scale,
                <div class="flex-col w-full" gap={Sp::SM}>
                    {note(&format!("Enter this code at {}", flow.verification_uri))}
                    <div class="flex-row items-center w-full" gap={Sp::SM} px={Sp::MD} py={Sp::SM}
                         rounded={Rad::LG} bg={tc.element_background} test_id="review.auth.code">
                        <text size={Px(TextStyle::HeadingLg.font_size(theme))} class="font-mono font-semibold"
                              color={tc.text_strong}>
                            {flow.user_code.clone()}
                        </text>
                        <spacer />
                        {copy}
                    </div>
                    {note("Waiting for you to approve\u{2026}")}
                </div>
            };
            (
                content,
                vec![
                    cancel(),
                    controls::button(
                        "Open GitHub",
                        action(ReviewAction::OpenDeviceFlowBrowser),
                        Tone::Primary,
                    )
                    .icon(lucide::EXTERNAL_LINK),
                ],
            )
        }
        AuthStatus::Failed(message) => (
            view! { scale,
                <div class="flex-row items-center w-full" gap={Sp::SM} test_id="review.auth.error">
                    <icon svg={lucide::ALERT_CIRCLE} size={Px(small)} color={tc.status_error} />
                    <text size={Px(small)} color={tc.status_error}>{message.clone()}</text>
                </div>
            },
            vec![
                cancel(),
                controls::button(
                    "Try again",
                    action(ReviewAction::StartDeviceFlow),
                    Tone::Primary,
                ),
            ],
        ),
    };
    let body = view! { scale,
        <div class="flex-col w-full" gap={Sp::LG}>
            {content}
            <div class="flex-row items-center w-full" gap={Sp::SM}>
                <spacer />
                {...actions}
            </div>
        </div>
    };
    let modal = Modal::new(
        "Sign in to GitHub",
        "Load pull requests and post reviews from Diffy.",
        lucide::GITHUB_MARK,
        tokens::scaled(Sz::MODAL_SM, scale),
        slot.width,
        slot.height,
        action(ReviewAction::CloseOverlay),
    )
    .padding(Sp::XL)
    .gap(Sp::MD)
    .body_child(body);
    view! {
        <div class="absolute" left={0.0} top={0.0} w={slot.width} h={slot.height}
             focus_ring={REVIEW_DIALOG}
             on:key:escape={action(ReviewAction::CloseOverlay)}>
            {modal}
        </div>
    }
}

fn account_menu(review: &ReviewController, slot: &Slot, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let width = tokens::scaled(ACCOUNT_MENU_WIDTH, scale);
    let left = (slot.width - width - tokens::scaled(Sp::MD, scale)).max(0.0);
    let top = theme.metrics.title_bar_height + tokens::scaled(Sp::XS, scale);
    let account = review.account();
    let (title, login) = match account.user.as_ref() {
        Some(user) if !user.name.is_empty() && user.name != user.login => {
            (user.name.clone(), format!("@{}", user.login))
        }
        Some(user) => (format!("@{}", user.login), String::new()),
        None => ("GitHub".to_owned(), String::new()),
    };
    let picture = avatar(title.clone())
        .size(Ico::HERO)
        .image(account.avatar.clone());
    let sign_out = controls::button("Sign out", action(ReviewAction::SignOut), Tone::Ghost);
    view! { scale,
        <div class="absolute" left={0.0} top={0.0} w={Px(slot.width)} h={Px(slot.height)}
             bg={Color::TRANSPARENT}
             on:click={action(ReviewAction::CloseOverlay)}
             on:key:escape={action(ReviewAction::CloseOverlay)}>
            <div class="absolute flex-col" left={Px(left)} top={Px(top)} w={Px(width)} p={Sp::XS} gap={Sp::XS}
                 bg={tc.elevated_surface} border={tc.border} rounded={Rad::XL}
                 shadow_preset={Shadow::DROPDOWN}
                 accessibility_role={AccessibilityRole::Menu}
                 aria-label="GitHub account"
                 focus_ring={REVIEW_DIALOG}
                 test_id="review.account_menu"
                 on:click={quark_ui::element::NoopAction}>
                <div class="flex-row items-center" gap={Sp::SM} p={Sp::SM}>
                    {picture}
                    <div class="flex-col flex-1" min_w={0.0}>
                        <text size={Px(small)} class="font-medium truncate" color={tc.text_strong}>{title}</text>
                        if !login.is_empty() {
                            <text size={Px(small)} class="truncate" color={tc.text_muted}>{login}</text>
                        }
                    </div>
                </div>
                <div class="w-full" h={Px(Sz::SEPARATOR_W)} bg={tc.border_variant} />
                {sign_out}
            </div>
        </div>
    }
}

fn submit_menu(review: &ReviewController, slot: &Slot, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let width = tokens::scaled(Sz::CARD_SM, scale).min(slot.width);
    let left = (slot.width - width - tokens::scaled(Sp::MD, scale)).max(0.0);
    let top = theme.metrics.title_bar_height + tokens::scaled(Sp::XS, scale);
    let (pending, failed) = review
        .review_counts()
        .map_or((0, 0), |(_, pending, failed)| (pending, failed));
    let staged = review.summary_staged();
    let comments = pending.saturating_sub(usize::from(staged));
    let noun = if comments == 1 { "comment" } else { "comments" };
    let summary = match (comments, staged, failed) {
        (_, _, 1..) => format!("{pending} pending, {failed} to retry"),
        (0, false, _) => "No pending comments".to_owned(),
        (0, true, _) => "The summary will be submitted".to_owned(),
        (n, false, _) => format!("{n} {noun} will be submitted"),
        (n, true, _) => format!("{n} {noun} and the summary will be submitted"),
    };
    let own = review.viewer_is_author();
    let presentation = review.presentation();
    let summary_card = presentation.general_composer().and_then(|card| {
        presentation.card_element(&card.key, width - tokens::scaled(Sp::MD, scale))
    });
    let add_summary = summary_card.is_none().then(|| {
        controls::button(
            "Add summary",
            action(ReviewAction::OpenComposer),
            Tone::Ghost,
        )
    });
    let decision = |label: &str, decision: ReviewDecision, tone: Tone| {
        let button = controls::button(label, action(ReviewAction::SubmitReview { decision }), tone);
        if own && decision != ReviewDecision::Comment {
            button
                .disabled(true)
                .tooltip("You can't approve or request changes on your own pull request")
        } else {
            button
        }
    };
    view! { scale,
        <div class="absolute" left={0.0} top={0.0} w={Px(slot.width)} h={Px(slot.height)}
             bg={Color::TRANSPARENT}
             on:click={action(ReviewAction::CloseOverlay)}
             on:key:escape={action(ReviewAction::CloseOverlay)}>
            <div class="absolute flex-col" left={Px(left)} top={Px(top)} w={Px(width)} p={Sp::SM} gap={Sp::SM}
                 bg={tc.elevated_surface} border={tc.border} rounded={Rad::XL}
                 shadow_preset={Shadow::DROPDOWN}
                 accessibility_role={AccessibilityRole::Dialog}
                 aria-label="Submit review"
                 focus_ring={REVIEW_DIALOG}
                 test_id="review.submit_menu"
                 on:click={quark_ui::element::NoopAction}>
                <text size={Px(small)} class="font-medium" color={tc.text_strong}>"Submit review"</text>
                <text size={Px(small)} color={tc.text_muted}>{summary}</text>
                {?summary_card}
                {?add_summary}
                <div class="flex-row items-center" gap={Sp::XS}>
                    <spacer />
                    {decision("Comment", ReviewDecision::Comment, Tone::Secondary)}
                    {decision("Request changes", ReviewDecision::RequestChanges, Tone::Destructive)}
                    {decision("Approve", ReviewDecision::Approve, Tone::Primary)}
                </div>
            </div>
        </div>
    }
}

pub fn author_avatar(review: &ReviewController, name: &str, email: &str) -> Avatar {
    avatar(name.to_owned()).image(review.author_face(email))
}

pub fn accounts_settings(review: &ReviewController, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let body = TextStyle::Body.font_size(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let account = review.account();
    let user = account.user.as_ref().filter(|_| review.signed_in());
    let (title, detail) = match (user, &account.auth) {
        (Some(user), _) if !user.name.is_empty() && user.name != user.login => {
            (user.name.clone(), format!("Signed in as @{}", user.login))
        }
        (Some(user), _) => (format!("@{}", user.login), "Signed in".to_owned()),
        (None, AuthStatus::Starting | AuthStatus::Awaiting(_)) => (
            "GitHub".to_owned(),
            "Waiting for you to approve in the browser\u{2026}".to_owned(),
        ),
        (None, _) => (
            "GitHub".to_owned(),
            "Sign in to load pull requests and post reviews.".to_owned(),
        ),
    };
    let picture = avatar(title.clone())
        .size(Ico::XXL)
        .image(user.and(account.avatar.clone()));
    let control = if review.signed_in() {
        controls::button("Sign out", action(ReviewAction::SignOut), Tone::Secondary)
    } else {
        controls::button(
            "Sign in",
            action(ReviewAction::OpenAuthModal),
            Tone::Primary,
        )
        .icon(lucide::GITHUB_MARK)
    };
    view! { scale,
        <div class="flex-col w-full" gap={Sp::LG} test_id="settings.accounts">
            <div class="flex-col" gap={Sp::XXS}>
                <text size={Px(body)} class="font-semibold" color={tc.text_strong}>"Accounts"</text>
                <text size={Px(small)} color={tc.text_muted}>"Services Diffy signs in to."</text>
            </div>
            <div class="flex-row items-center w-full" gap={Sp::MD} p={Sp::LG}
                 bg={tc.elevated_surface} border={tc.border_variant} rounded={Rad::XL}
                 accessibility_role={AccessibilityRole::Group}
                 aria-label="GitHub account"
                 test_id="settings.accounts.github">
                {picture}
                <div class="flex-col flex-1" min_w={0.0} gap={Sp::XXS}>
                    <text size={Px(body)} class="font-medium truncate" color={tc.text}>{title}</text>
                    <text size={Px(small)} class="truncate" color={tc.text_muted}>{detail}</text>
                </div>
                {control}
            </div>
        </div>
    }
}
