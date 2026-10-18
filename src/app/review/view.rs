use quark::view;
use quark_app::ViewContext;
use quark_components::{Button, ButtonSize, ButtonStyle, Modal, avatar};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};

use super::ReviewController;
use super::state::{AuthStatus, ReviewOverlay};
use crate::app::shell::Slot;
use crate::app::text_targets::{REVIEW_ACCOUNT, REVIEW_DIALOG};
use crate::app::tokens::{self, Ico, Rad, Shadow, Sp, Sz, TextStyle};
use crate::contracts::review::ReviewAction;
use crate::contracts::shell::ShellAction;
use crate::core::review::ReviewDecision;

const CARD_MAX_WIDTH: f32 = 760.0;
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
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let pull_request = review.active_pull_request().map(|active| {
        let (label, color) = match (&active.comparison, &active.error) {
            (_, Some(_)) => (format!("Couldn't load #{}", active.pr.number), tc.status_error),
            (Some(comparison), None) => (
                format!("#{} {}", comparison.info.number, comparison.info.title),
                tc.text,
            ),
            (None, None) => ("Loading pull request\u{2026}".to_owned(), tc.text_muted),
        };
        let counts = review.review_counts();
        let detail = counts.map(|(unresolved, pending, failed)| {
            let mut parts = vec![format!("{unresolved} unresolved")];
            if pending > 0 {
                parts.push(format!("{pending} pending"));
            }
            if failed > 0 {
                parts.push(format!("{failed} failed"));
            }
            parts.join(" \u{b7} ")
        });
        let submit = counts.filter(|_| review.signed_in()).map(|(_, pending, failed)| {
            let style = if pending + failed > 0 {
                ButtonStyle::Filled
            } else {
                ButtonStyle::Subtle
            };
            let button = Button::new(action(ReviewAction::OpenSubmitMenu))
                .label("Submit review")
                .style(style)
                .size(ButtonSize::Compact);
            view! { <div test_id="review.submit">{button}</div> }
        });
        let open = Button::new(action(ReviewAction::OpenPullRequestInBrowser))
            .icon(lucide::EXTERNAL_LINK)
            .tooltip("Open on GitHub")
            .style(ButtonStyle::Ghost)
            .size(ButtonSize::Compact);
        view! { scale,
            <div class="flex-row items-center" gap={Sp::SM} min_w={0.0} test_id="review.pull_request">
                <icon svg={lucide::GIT_PULL_REQUEST} size={small} color={tc.text_muted} />
                <text size={small} class="truncate font-medium" color={color}>{label}</text>
                if let Some(detail) = detail {
                    <text size={small} class="truncate" color={tc.text_muted}>{detail}</text>
                }
                {open}
                {?submit}
            </div>
        }
    });
    let account = account_view(review, theme);
    view! { scale,
        <div h={slot.height} class="flex-row items-center shrink-0" gap={Sp::SM} test_id="title.review"
             on:key:escape={action(ReviewAction::CloseOverlay)}>
            {?pull_request}
            {account}
        </div>
    }
}

fn account_view(review: &ReviewController, theme: &Theme) -> AnyElement {
    let scale = tokens::ui_scale(theme);
    if !review.signed_in() {
        let tc = &theme.colors;
        let small = TextStyle::BodySmall.font_size(theme);
        return view! { scale,
            <div
                class="flex-row items-center shrink-0"
                gap={Sp::XS}
                px={Sp::SM}
                py={Sp::XXS}
                rounded={Rad::MD}
                bg={tc.element_background}
                hover_bg={tc.element_hover}
                cursor={CursorHint::Pointer}
                focus_ring={REVIEW_ACCOUNT}
                accessibility_role={AccessibilityRole::Button}
                aria-label="Sign in to GitHub"
                test_id="review.sign_in"
                on:click={action(ReviewAction::OpenAuthModal)}
            >
                <icon svg={lucide::KEY} size={small} color={tc.text_muted} />
                <text size={small} class="font-medium" color={tc.text}>"Sign in to GitHub"</text>
            </div>
        };
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
    let auth = &review.account().auth;
    let body = match auth {
        _ if review.signed_in() => view! { scale,
            <div class="flex-col items-center w-full" gap={Sp::MD}>
                <icon svg={lucide::CHECK} size={small * 2.0} color={tc.accent} />
                <text size={small} color={tc.text}>"Signed in to GitHub."</text>
            </div>
        },
        AuthStatus::Idle | AuthStatus::Done => {
            let start = Button::new(action(ReviewAction::StartDeviceFlow))
                .icon(lucide::KEY)
                .label("Continue with GitHub")
                .style(ButtonStyle::Filled);
            view! { scale,
                <div class="flex-col items-center w-full" gap={Sp::MD}>
                    {start}
                    <text size={small} color={tc.text_muted}>
                        "GitHub generates a one-time device code for this app."
                    </text>
                </div>
            }
        }
        AuthStatus::Starting => view! { scale,
            <div class="flex-col items-center w-full" gap={Sp::MD} test_id="review.auth.starting">
                <icon svg={lucide::LOADER} size={small * 2.0} color={tc.text_accent} />
                <text size={small} color={tc.text_muted}>"Contacting GitHub\u{2026}"</text>
            </div>
        },
        AuthStatus::Awaiting(flow) => {
            let copy = Button::new(ShellAction::CopyText(flow.user_code.clone()))
                .icon(lucide::COPY)
                .tooltip("Copy code")
                .style(ButtonStyle::Subtle)
                .size(ButtonSize::Compact);
            let browser = Button::new(action(ReviewAction::OpenDeviceFlowBrowser))
                .icon(lucide::EXTERNAL_LINK)
                .label("Open GitHub in browser")
                .style(ButtonStyle::Filled);
            view! { scale,
                <div class="flex-col items-center w-full" gap={Sp::MD}>
                    <text size={small} color={tc.text_muted}>"Enter this code at"</text>
                    <text size={small} color={tc.text_accent}>{flow.verification_uri.clone()}</text>
                    <div class="flex-row items-center justify-center" gap={Sp::MD} p={Sp::LG}
                         rounded={Rad::XL} bg={tc.surface} test_id="review.auth.code">
                        <text size={small * 2.0} class="font-semibold" color={tc.text_strong}>
                            {flow.user_code.clone()}
                        </text>
                        {copy}
                    </div>
                    {browser}
                    <text size={small} color={tc.text_muted}>"Waiting for you to authorize\u{2026}"</text>
                </div>
            }
        }
        AuthStatus::Failed(message) => {
            let retry = Button::new(action(ReviewAction::StartDeviceFlow))
                .icon(lucide::REFRESH)
                .label("Try again")
                .style(ButtonStyle::Filled);
            view! { scale,
                <div class="flex-col items-center w-full" gap={Sp::MD}>
                    <div class="flex-row items-center w-full" gap={Sp::SM} p={Sp::MD}
                         rounded={Rad::LG} bg={tc.surface} test_id="review.auth.error">
                        <icon svg={lucide::ALERT_CIRCLE} size={small} color={tc.status_error} />
                        <text size={small} color={tc.status_error}>{message.clone()}</text>
                    </div>
                    {retry}
                </div>
            }
        }
    };
    let cancel = Button::new(action(ReviewAction::CloseOverlay))
        .label(if review.signed_in() {
            "Close"
        } else {
            "Cancel"
        })
        .style(ButtonStyle::Ghost);
    let modal = Modal::new(
        "Sign in to GitHub",
        "Connect your account to load pull requests and review diffs.",
        lucide::GITHUB_MARK,
        tokens::scaled(Sz::MODAL_LG, scale),
        slot.width,
        slot.height,
        action(ReviewAction::CloseOverlay),
    )
    .body_child(body)
    .footer_child(cancel);
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
    let sign_out = Button::new(action(ReviewAction::SignOut))
        .icon(lucide::KEY)
        .label("Sign out")
        .style(ButtonStyle::Ghost);
    view! { scale,
        <div class="absolute" left={0.0} top={0.0} w={slot.width} h={slot.height}
             bg={Color::TRANSPARENT}
             on:click={action(ReviewAction::CloseOverlay)}
             on:key:escape={action(ReviewAction::CloseOverlay)}>
            <div class="absolute flex-col" left={left} top={top} w={width} p={Sp::XS} gap={Sp::XS}
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
                        <text size={small} class="font-medium truncate" color={tc.text_strong}>{title}</text>
                        if !login.is_empty() {
                            <text size={small} class="truncate" color={tc.text_muted}>{login}</text>
                        }
                    </div>
                </div>
                <div class="w-full" h={Sz::SEPARATOR_W} bg={tc.border_variant} />
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
    let summary = match (pending, failed) {
        (0, 0) => "No pending comments".to_owned(),
        (pending, 0) => format!("{pending} pending comments will be submitted"),
        (pending, failed) => format!("{pending} pending, {failed} to retry"),
    };
    let presentation = review.presentation();
    let summary_card = presentation.general_composer().and_then(|card| {
        presentation.card_element(&card.key, width - tokens::scaled(Sp::MD, scale))
    });
    let add_summary = summary_card.is_none().then(|| {
        Button::new(action(ReviewAction::OpenComposer))
            .label("Add summary")
            .style(ButtonStyle::Ghost)
            .size(ButtonSize::Compact)
    });
    let decision = |label: &str, decision: ReviewDecision, style: ButtonStyle| {
        Button::new(action(ReviewAction::SubmitReview { decision }))
            .label(label)
            .style(style)
            .size(ButtonSize::Compact)
    };
    view! { scale,
        <div class="absolute" left={0.0} top={0.0} w={slot.width} h={slot.height}
             bg={Color::TRANSPARENT}
             on:click={action(ReviewAction::CloseOverlay)}
             on:key:escape={action(ReviewAction::CloseOverlay)}>
            <div class="absolute flex-col" left={left} top={top} w={width} p={Sp::SM} gap={Sp::SM}
                 bg={tc.elevated_surface} border={tc.border} rounded={Rad::XL}
                 shadow_preset={Shadow::DROPDOWN}
                 accessibility_role={AccessibilityRole::Dialog}
                 aria-label="Submit review"
                 focus_ring={REVIEW_DIALOG}
                 test_id="review.submit_menu"
                 on:click={quark_ui::element::NoopAction}>
                <text size={small} class="font-medium" color={tc.text_strong}>"Submit review"</text>
                <text size={small} color={tc.text_muted}>{summary}</text>
                {?summary_card}
                {?add_summary}
                <div class="flex-row items-center" gap={Sp::XS}>
                    <spacer />
                    {decision("Comment", ReviewDecision::Comment, ButtonStyle::Subtle)}
                    {decision("Request changes", ReviewDecision::RequestChanges, ButtonStyle::Danger)}
                    {decision("Approve", ReviewDecision::Approve, ButtonStyle::Filled)}
                </div>
            </div>
        </div>
    }
}
