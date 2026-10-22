use std::rc::Rc;

use quark::{Px, view};
use quark_app::ViewContext;
use quark_components::{BadgeVariant, avatar, badge, skeleton_lines};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};

use super::ReviewController;
use super::controls::{self, Tone};
use super::presentation::{OVERVIEW_DESCRIPTION, ReviewPresentation, relative_time_label};
use crate::app::icons;
use crate::app::shell::Slot;
use crate::app::text_targets::REVIEW_OVERVIEW;
use crate::app::tokens::{self, Ico, Layout, Rad, Sp, Sz, TextStyle, Tint};
use crate::contracts::review::{CardKey, ReviewAction};
use crate::core::forge::github::{
    PullRequestCheckContext, PullRequestInfo, PullRequestLabel, PullRequestOverview,
    PullRequestReviewer, PullRequestTimelineItem, PullRequestTimelineKind,
};

fn action(action: ReviewAction) -> quark_ui::Action {
    action.into()
}

struct Summary {
    number: i32,
    title: String,
    state: String,
    draft: bool,
    author: String,
    base: String,
    head: String,
    commits: Option<i32>,
}

struct Page<'a> {
    review: &'a ReviewController,
    presentation: Rc<ReviewPresentation>,
    theme: &'a Theme,
    scale: f32,
    width: f32,
    stale: bool,
}

#[derive(Clone, Copy)]
pub(super) enum CheckTone {
    Passed,
    Failed,
    Pending,
}

pub(super) fn check_tone(state: &str) -> CheckTone {
    match state {
        "SUCCESS" | "NEUTRAL" | "SKIPPED" => CheckTone::Passed,
        "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => {
            CheckTone::Failed
        }
        _ => CheckTone::Pending,
    }
}

pub(super) fn check_icon(tone: CheckTone, theme: &Theme) -> (&'static str, Color) {
    let tc = &theme.colors;
    match tone {
        CheckTone::Passed => (icons::CIRCLE_CHECK, tc.line_add_text),
        CheckTone::Failed => (icons::CIRCLE_X, tc.line_del_text),
        CheckTone::Pending => (icons::CLOCK, tc.status_warning),
    }
}

pub(super) fn reviewer_state(state: &str, theme: &Theme) -> (&'static str, &'static str, Color) {
    let tc = &theme.colors;
    match state {
        "APPROVED" => ("Approved", icons::CIRCLE_CHECK, tc.line_add_text),
        "CHANGES_REQUESTED" => ("Changes requested", lucide::ALERT_CIRCLE, tc.line_del_text),
        "DISMISSED" => ("Dismissed", lucide::MINUS, tc.text_muted),
        "COMMENTED" => ("Commented", icons::MESSAGE_SQUARE, tc.text_muted),
        _ => ("Awaiting review", icons::CLOCK, tc.status_warning),
    }
}

pub(super) fn state_badge(state: &str, draft: bool) -> (&'static str, &'static str, BadgeVariant) {
    match (state, draft) {
        ("MERGED", _) => ("Merged", icons::GIT_MERGE, BadgeVariant::Accent),
        ("CLOSED", _) => ("Closed", lucide::GIT_PULL_REQUEST, BadgeVariant::Error),
        (_, true) => ("Draft", lucide::GIT_PULL_REQUEST, BadgeVariant::Default),
        _ => ("Open", lucide::GIT_PULL_REQUEST, BadgeVariant::Success),
    }
}

pub(super) fn info_state(info: &PullRequestInfo) -> String {
    if info.merged {
        "MERGED".to_owned()
    } else {
        info.state.to_ascii_uppercase()
    }
}

fn review_verb(state: &str) -> &'static str {
    match state {
        "APPROVED" => "approved these changes",
        "CHANGES_REQUESTED" => "requested changes",
        "DISMISSED" => "left a dismissed review",
        _ => "reviewed",
    }
}

fn label_color(label: &PullRequestLabel, theme: &Theme) -> Color {
    let hex = label.color.trim_start_matches('#');
    match u32::from_str_radix(hex, 16) {
        Ok(rgb) if hex.len() == 6 => Color::rgba(
            ((rgb >> 16) & 0xff) as u8,
            ((rgb >> 8) & 0xff) as u8,
            (rgb & 0xff) as u8,
            255,
        ),
        _ => theme.colors.text_muted,
    }
}

fn plural(count: i64, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

fn time_label(iso: &str) -> Option<String> {
    relative_time_label(Some(iso).filter(|iso| !iso.is_empty()))
}

impl Page<'_> {
    fn small(&self) -> f32 {
        TextStyle::BodySmall.font_size(self.theme)
    }

    fn face(&self, login: &str, size: f32) -> AnyElement {
        let face = self.review.host.face(login);
        let test_id = format!("review.overview.avatar.{login}");
        view! { <div class="shrink-0" test_id={test_id}>{avatar(login.to_owned()).size(size).image(face)}</div> }
    }

    fn muted(&self, label: impl Into<String>) -> AnyElement {
        let scale = self.scale;
        let label = label.into();
        view! { scale, <text size={Px(self.small())} color={self.theme.colors.text_muted}>{label}</text> }
    }

    fn doc(&mut self, key: CardKey, width: f32, label: &str) -> Option<AnyElement> {
        let (element, stale) = self.presentation.document_element(&key, width, label)?;
        self.stale |= stale;
        let copy = action(ReviewAction::CopyCardSelection(key));
        Some(view! { <div class="flex-col" on:key:mod+c={copy}>{element}</div> })
    }

    fn card(&self, label: String, test_id: String, children: Vec<AnyElement>) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        view! { scale,
            <div
                class="flex-col shrink-0"
                w={Px(self.width)}
                p={Sp::MD}
                gap={Sp::SM}
                bg={tc.elevated_surface}
                border={tc.border}
                rounded={Rad::LG}
                accessibility_role={AccessibilityRole::Group}
                aria-label={label}
                test_id={test_id}
            >
                {...children}
            </div>
        }
    }

    fn inner_width(&self) -> f32 {
        (self.width - tokens::scaled(Sp::MD, self.scale) * 2.0).max(1.0)
    }

    fn header(&self, summary: &Summary) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let small = self.small();
        let (state, icon, variant) = state_badge(&summary.state, summary.draft);
        let refresh_label = if self.review.overview.loading {
            "Refreshing\u{2026}"
        } else {
            "Refresh"
        };
        let refresh = controls::icon_button(
            lucide::REFRESH,
            refresh_label,
            action(ReviewAction::RefreshOverview),
            self.theme,
        )
        .disabled(self.review.overview.loading);
        let browser = controls::icon_button(
            lucide::EXTERNAL_LINK,
            "Open on GitHub",
            action(ReviewAction::OpenPullRequestInBrowser),
            self.theme,
        );
        let back = controls::button(
            "Files changed",
            action(ReviewAction::CloseOverview),
            Tone::Secondary,
        )
        .icon(lucide::FILE_DIFF)
        .tooltip("Esc");
        let verb = match (summary.state.as_str(), summary.commits) {
            ("MERGED", Some(n)) => {
                format!("merged {} into", plural(i64::from(n), "commit", "commits"))
            }
            ("MERGED", None) => "merged into".to_owned(),
            (_, Some(n)) => format!(
                "wants to merge {} into",
                plural(i64::from(n), "commit", "commits")
            ),
            (_, None) => "wants to merge into".to_owned(),
        };
        let branch = |name: &str| {
            let name = name.to_owned();
            view! { scale,
                <div class="flex-row items-center shrink-0" px={Sp::XS} py={Sp::XXS} rounded={Rad::SM} bg={tc.element_background}>
                    <text size={Px(TextStyle::Mono.font_size(self.theme))} class="font-mono" color={tc.text}>{name}</text>
                </div>
            }
        };
        let author = (!summary.author.is_empty()).then(|| self.face(&summary.author, Ico::LG));
        view! { scale,
            <div class="flex-col shrink-0" w={Px(self.width)} gap={Sp::SM} test_id="review.overview.header">
                <div class="flex-row items-center w-full" gap={Sp::SM}>
                    <div accessibility_role={AccessibilityRole::Label} aria-label={format!("State: {state}")} test_id="review.overview.state">
                        {badge(state).icon(icon).variant(variant)}
                    </div>
                    <text size={Px(small)} color={tc.text_muted}>{format!("#{}", summary.number)}</text>
                    <spacer />
                    <div test_id="review.overview.refresh">{refresh}</div>
                    <div test_id="review.overview.open_in_browser">{browser}</div>
                    <div test_id="review.overview.back">{back}</div>
                </div>
                <div class="flex-row w-full" accessibility_role={AccessibilityRole::Heading} aria-label={summary.title.clone()}>
                    <text size={Px(TextStyle::HeadingLg.font_size(self.theme))} class="font-semibold" color={tc.text_strong}>
                        {summary.title.clone()}
                    </text>
                </div>
                <div class="flex-row flex-wrap items-center w-full" gap={Sp::XS}>
                    {?author}
                    if !summary.author.is_empty() {
                        <text size={Px(small)} class="font-medium" color={tc.text}>{format!("@{}", summary.author)}</text>
                    }
                    <text size={Px(small)} color={tc.text_muted}>{verb}</text>
                    {branch(&summary.base)}
                    <text size={Px(small)} color={tc.text_muted}>"from"</text>
                    {branch(&summary.head)}
                </div>
            </div>
        }
    }

    fn row(&self, icon: &'static str, label: &str, content: Vec<AnyElement>) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let small = self.small();
        let id = format!("review.overview.row.{}", label.to_ascii_lowercase());
        view! { scale,
            <div class="flex-row items-start w-full" gap={Sp::MD} accessibility_role={AccessibilityRole::Group} aria-label={label.to_owned()} test_id={id}>
                <div class="flex-row items-center shrink-0" w={Layout::OVERVIEW_LABEL_W} h={Px(controls::control_height(self.theme))} gap={Sp::XS}>
                    <icon svg={icon} size={Ico::SM} color={tc.text_muted} />
                    <text size={Px(small)} color={tc.text_muted}>{label.to_owned()}</text>
                </div>
                <div class="flex-1 flex-row flex-wrap items-center" min_h={Px(controls::control_height(self.theme))} min_w={0.0} gap={Sp::XS}>
                    {...content}
                </div>
            </div>
        }
    }

    fn sign_in_hint(&self, label: &str) -> Vec<AnyElement> {
        let button = controls::button("Sign in", action(ReviewAction::OpenAuthModal), Tone::Ghost)
            .icon(lucide::GITHUB_MARK);
        vec![
            self.muted(label),
            view! { <div class="shrink-0">{button}</div> },
        ]
    }

    fn reviewer(&self, reviewer: &PullRequestReviewer) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let (label, icon, color) = reviewer_state(&reviewer.state, self.theme);
        let login = &reviewer.actor.login;
        view! { scale,
            <div class="flex-row items-center shrink-0" gap={Sp::XS} pl={Sp::XXS} pr={Sp::SM} py={Sp::XXS}
                 rounded={Rad::PILL} bg={tc.element_background}
                 accessibility_role={AccessibilityRole::Label}
                 aria-label={format!("@{login}: {label}")}
                 tooltip={label}
                 test_id={format!("review.overview.reviewer.{login}")}>
                {self.face(login, Ico::MD)}
                <text size={Px(self.small())} color={tc.text}>{login.clone()}</text>
                <icon svg={icon} size={Ico::XS} color={color} />
            </div>
        }
    }

    fn reviewers(&self, overview: &PullRequestOverview) -> AnyElement {
        let mut content: Vec<AnyElement> = overview
            .reviewers
            .iter()
            .map(|reviewer| self.reviewer(reviewer))
            .collect();
        if overview.team_reviewers > 0 {
            content.push(self.muted(format!(
                "+{}",
                plural(overview.team_reviewers as i64, "team", "teams")
            )));
        }
        if content.is_empty() {
            content = if overview.detailed {
                vec![self.muted("No reviewers")]
            } else {
                self.sign_in_hint("Sign in to see reviews")
            };
        }
        self.row(icons::USER, "Reviewers", content)
    }

    fn labels(&self, overview: &PullRequestOverview) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let content: Vec<AnyElement> = if overview.labels.is_empty() {
            vec![self.muted("None")]
        } else {
            overview
                .labels
                .iter()
                .map(|label| {
                    let color = label_color(label, self.theme);
                    let dot = tokens::scaled(Layout::STATUS_DOT, self.scale);
                    view! { scale,
                        <div class="flex-row items-center shrink-0" gap={Sp::XS} px={Sp::SM} py={Sp::XXS}
                             rounded={Rad::PILL} bg={tc.element_background.lerp(color, Tint::BADGE)}
                             accessibility_role={AccessibilityRole::Label}
                             aria-label={format!("Label {}", label.name)}>
                            <div w={Px(dot)} h={Px(dot)} rounded={Px(dot)} bg={color} />
                            <text size={Px(self.small())} color={tc.text}>{label.name.clone()}</text>
                        </div>
                    }
                })
                .collect()
        };
        self.row(icons::TAG, "Labels", content)
    }

    fn checks(&self, overview: &PullRequestOverview) -> Vec<AnyElement> {
        let scale = self.scale;
        let checks = &overview.checks;
        if !overview.detailed {
            return vec![self.row(
                icons::CIRCLE_CHECK,
                "Checks",
                self.sign_in_hint("Sign in to see checks"),
            )];
        }
        if checks.contexts.is_empty() {
            return vec![self.row(
                icons::CIRCLE_CHECK,
                "Checks",
                vec![self.muted("No checks reported")],
            )];
        }
        let tc = &self.theme.colors;
        let count = |tone: CheckTone, count: i32, word: &str| {
            (count > 0).then(|| {
                let (icon, color) = check_icon(tone, self.theme);
                view! { scale,
                    <div class="flex-row items-center shrink-0" gap={Sp::XXS}>
                        <icon svg={icon} size={Ico::SM} color={color} />
                        <text size={Px(self.small())} color={tc.text}>{format!("{count} {word}")}</text>
                    </div>
                }
            })
        };
        let expanded = self.review.overview.checks_expanded;
        let toggle = controls::button(
            if expanded {
                "Hide checks"
            } else {
                "Show checks"
            },
            action(ReviewAction::ToggleOverviewChecks),
            Tone::Ghost,
        )
        .icon(if expanded {
            lucide::CHEVRON_UP
        } else {
            lucide::CHEVRON_DOWN
        })
        .active(expanded);
        let mut content: Vec<AnyElement> = [
            count(CheckTone::Failed, checks.failure_count, "failed"),
            count(CheckTone::Pending, checks.pending_count, "pending"),
            count(CheckTone::Passed, checks.success_count, "passed"),
        ]
        .into_iter()
        .flatten()
        .collect();
        content.push(view! { scale, <spacer /> });
        content.push(view! { scale, <div test_id="review.overview.checks.toggle">{toggle}</div> });
        let mut rows = vec![self.row(icons::CIRCLE_CHECK, "Checks", content)];
        if expanded {
            let mut sorted: Vec<&PullRequestCheckContext> = checks.contexts.iter().collect();
            sorted.sort_by_key(|check| match check_tone(&check.state) {
                CheckTone::Failed => 0,
                CheckTone::Pending => 1,
                CheckTone::Passed => 2,
            });
            let list: Vec<AnyElement> = sorted.into_iter().map(|check| self.check(check)).collect();
            rows.push(view! { scale,
                <div class="flex-col w-full" pl={Px(tokens::scaled(Layout::OVERVIEW_LABEL_W + Sp::MD, self.scale))}
                     accessibility_role={AccessibilityRole::List} aria-label="Checks"
                     test_id="review.overview.checks">
                    {...list}
                </div>
            });
        }
        rows
    }

    fn check(&self, check: &PullRequestCheckContext) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let (icon, color) = check_icon(check_tone(&check.state), self.theme);
        let state = check.state.to_ascii_lowercase().replace('_', " ");
        let link = !check.details_url.is_empty();
        let label = format!("{}: {state}", check.name);
        view! { scale,
            <div class="flex-row items-center w-full" gap={Sp::SM} px={Sp::XS}
                 h={Px(controls::control_height(self.theme))}
                 rounded={Rad::MD}
                 accessibility_role={if link { AccessibilityRole::Link } else { AccessibilityRole::ListItem }}
                 aria-label={label}
                 @when {link} {
                     hover_bg={tc.ghost_element_hover}
                     cursor={CursorHint::Pointer}
                     on:click={action(ReviewAction::OpenLink(check.details_url.clone()))}
                 }>
                <icon svg={icon} size={Ico::SM} color={color} />
                <div class="flex-1 flex-row" min_w={0.0}>
                    <text size={Px(self.small())} class="truncate" color={tc.text}>{check.name.clone()}</text>
                </div>
                <text size={Px(self.small())} color={tc.text_muted}>{state}</text>
                if link {
                    <icon svg={lucide::EXTERNAL_LINK} size={Ico::XS} color={tc.text_muted} />
                }
            </div>
        }
    }

    fn merge_status(&self, overview: &PullRequestOverview) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let decision = overview.review_decision.as_deref();
        let (icon, color, label) = match (
            overview.state.as_str(),
            overview.mergeable.as_str(),
            overview.merge_state_status.as_str(),
        ) {
            ("MERGED", _, _) => (icons::GIT_MERGE, tc.accent, "Merged"),
            ("CLOSED", _, _) => (icons::CIRCLE_X, tc.text_muted, "Closed without merging"),
            _ if overview.is_draft => {
                (lucide::PENCIL, tc.text_muted, "Draft, not ready for review")
            }
            (_, "CONFLICTING", _) | (_, _, "DIRTY") => {
                (icons::CIRCLE_X, tc.line_del_text, "Has merge conflicts")
            }
            (_, _, "BEHIND") => (
                lucide::ALERT_CIRCLE,
                tc.status_warning,
                "Behind the base branch",
            ),
            (_, _, "BLOCKED") => match decision {
                Some("CHANGES_REQUESTED") => {
                    (lucide::ALERT_CIRCLE, tc.line_del_text, "Changes requested")
                }
                Some("REVIEW_REQUIRED") => (icons::CLOCK, tc.status_warning, "Review required"),
                _ => (
                    lucide::ALERT_CIRCLE,
                    tc.status_warning,
                    "Merging is blocked",
                ),
            },
            (_, _, "UNSTABLE") => (
                lucide::ALERT_CIRCLE,
                tc.status_warning,
                "Mergeable with failing checks",
            ),
            (_, "MERGEABLE", _) => (icons::CIRCLE_CHECK, tc.line_add_text, "Ready to merge"),
            _ => (icons::CLOCK, tc.text_muted, "Checking mergeability"),
        };
        let content = vec![view! { scale,
            <div class="flex-row items-center" gap={Sp::XS} test_id="review.overview.merge_state">
                <icon svg={icon} size={Ico::SM} color={color} />
                <text size={Px(self.small())} color={tc.text}>{label}</text>
            </div>
        }];
        self.row(lucide::GIT_PULL_REQUEST, "Merge", content)
    }

    fn changes(&self, overview: &PullRequestOverview) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let mono = TextStyle::Mono.font_size(self.theme);
        let content = vec![
            self.muted(format!(
                "{} \u{b7} {} \u{b7}",
                plural(i64::from(overview.commit_count), "commit", "commits"),
                plural(i64::from(overview.changed_files), "file", "files")
            )),
            view! { scale, <text size={Px(mono)} class="font-mono" color={tc.line_add_text}>{format!("+{}", overview.additions)}</text> },
            view! { scale, <text size={Px(mono)} class="font-mono" color={tc.line_del_text}>{format!("\u{2212}{}", overview.deletions)}</text> },
        ];
        self.row(lucide::FILE_DIFF, "Changes", content)
    }

    fn details(&self, overview: &PullRequestOverview) -> AnyElement {
        let mut children = vec![self.reviewers(overview), self.labels(overview)];
        children.extend(self.checks(overview));
        children.push(self.merge_status(overview));
        children.push(self.changes(overview));
        self.card(
            "Details".to_owned(),
            "review.overview.details".to_owned(),
            children,
        )
    }

    fn author_line(
        &self,
        login: &str,
        verb: Option<&str>,
        time: &str,
        trailing: Option<AnyElement>,
    ) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let small = self.small();
        let time = time_label(time);
        view! { scale,
            <div class="flex-row items-center w-full" gap={Sp::XS}>
                {self.face(login, Ico::LG)}
                <text size={Px(small)} class="font-medium truncate" color={tc.text_strong}>{format!("@{login}")}</text>
                if let Some(verb) = verb {
                    <text size={Px(small)} class="truncate" color={tc.text_muted}>{verb.to_owned()}</text>
                }
                if let Some(time) = time {
                    <text size={Px(small)} color={tc.text_muted}>{format!("\u{b7} {time}")}</text>
                }
                <spacer />
                {?trailing}
            </div>
        }
    }

    fn description(&mut self, overview: &PullRequestOverview) -> AnyElement {
        let header = self.author_line(
            &overview.author.login,
            Some("opened this pull request"),
            &overview.created_at,
            None,
        );
        let width = self.inner_width();
        let body = if overview.body.trim().is_empty() {
            Some(self.muted("No description provided."))
        } else {
            self.doc(
                CardKey::Overview(OVERVIEW_DESCRIPTION.to_owned()),
                width,
                "Pull request description",
            )
        };
        let body = body.unwrap_or_else(|| skeleton_lines(3, self.theme));
        self.card(
            "Description".to_owned(),
            "review.overview.description".to_owned(),
            vec![header, body],
        )
    }

    fn link_button(&self, url: &str) -> Option<AnyElement> {
        (!url.is_empty()).then(|| {
            let button = controls::icon_button(
                lucide::EXTERNAL_LINK,
                "Open on GitHub",
                action(ReviewAction::OpenLink(url.to_owned())),
                self.theme,
            );
            view! { <div class="shrink-0">{button}</div> }
        })
    }

    fn timeline_item(&mut self, item: &PullRequestTimelineItem) -> AnyElement {
        let scale = self.scale;
        match &item.kind {
            PullRequestTimelineKind::Commit { short_oid } => self.commit(item, short_oid),
            PullRequestTimelineKind::Comment => {
                let header = self.author_line(
                    &item.author.login,
                    Some("commented"),
                    &item.created_at,
                    self.link_button(&item.url),
                );
                let width = self.inner_width();
                let body = self.doc(CardKey::Overview(item.id.clone()), width, "Comment");
                let label = format!("Comment by @{}", item.author.login);
                self.card(
                    label,
                    format!("review.overview.item.{}", item.id),
                    [Some(header), body].into_iter().flatten().collect(),
                )
            }
            PullRequestTimelineKind::Review { state, comments } => {
                if item.body.trim().is_empty() {
                    return self.review_event(item, state, *comments);
                }
                let (_, icon, color) = reviewer_state(state, self.theme);
                let header = self.author_line(
                    &item.author.login,
                    Some(review_verb(state)),
                    &item.created_at,
                    self.link_button(&item.url),
                );
                let width = self.inner_width();
                let body = self.doc(CardKey::Overview(item.id.clone()), width, "Review");
                let label = format!("Review by @{}, {}", item.author.login, review_verb(state));
                let tc = &self.theme.colors;
                let note = (*comments > 0).then(|| {
                    view! { scale,
                        <div class="flex-row items-center" gap={Sp::XS}>
                            <icon svg={icon} size={Ico::XS} color={color} />
                            <text size={Px(self.small())} color={tc.text_muted}>
                                {plural(i64::from(*comments), "review comment", "review comments")}
                            </text>
                        </div>
                    }
                });
                self.card(
                    label,
                    format!("review.overview.item.{}", item.id),
                    [Some(header), body, note].into_iter().flatten().collect(),
                )
            }
        }
    }

    fn event_row(&self, label: String, test_id: String, children: Vec<AnyElement>) -> AnyElement {
        let scale = self.scale;
        view! { scale,
            <div class="flex-row items-center shrink-0" w={Px(self.width)} px={Sp::MD} gap={Sp::SM}
                 h={Px(controls::control_height(self.theme))}
                 accessibility_role={AccessibilityRole::Group}
                 aria-label={label}
                 test_id={test_id}>
                {...children}
            </div>
        }
    }

    fn review_event(
        &self,
        item: &PullRequestTimelineItem,
        state: &str,
        comments: i32,
    ) -> AnyElement {
        let scale = self.scale;
        let (_, icon, color) = reviewer_state(state, self.theme);
        let tc = &self.theme.colors;
        let small = self.small();
        let login = &item.author.login;
        let mut detail = review_verb(state).to_owned();
        if comments > 0 {
            detail.push_str(&format!(
                " \u{b7} {}",
                plural(i64::from(comments), "comment", "comments")
            ));
        }
        if let Some(time) = time_label(&item.created_at) {
            detail.push_str(&format!(" \u{b7} {time}"));
        }
        let label = format!("@{login} {}", review_verb(state));
        let children = vec![
            view! { scale, <icon svg={icon} size={Ico::SM} color={color} /> },
            self.face(login, Ico::MD),
            view! { scale, <text size={Px(small)} class="font-medium" color={tc.text}>{format!("@{login}")}</text> },
            view! { scale, <div class="flex-1 flex-row" min_w={0.0}><text size={Px(small)} class="truncate" color={tc.text_muted}>{detail}</text></div> },
        ];
        self.event_row(label, format!("review.overview.item.{}", item.id), children)
    }

    fn commit(&self, item: &PullRequestTimelineItem, short_oid: &str) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let small = self.small();
        let mono = TextStyle::Mono.font_size(self.theme);
        let time = time_label(&item.created_at).unwrap_or_default();
        let label = format!("Commit {short_oid}: {}", item.body);
        let link = !item.url.is_empty();
        view! { scale,
            <div class="flex-row items-center shrink-0" w={Px(self.width)} px={Sp::MD} gap={Sp::SM}
                 h={Px(controls::control_height(self.theme))}
                 rounded={Rad::MD}
                 accessibility_role={if link { AccessibilityRole::Link } else { AccessibilityRole::Group }}
                 aria-label={label}
                 test_id={format!("review.overview.item.{}", item.id)}
                 @when {link} {
                     hover_bg={tc.ghost_element_hover}
                     cursor={CursorHint::Pointer}
                     on:click={action(ReviewAction::OpenLink(item.url.clone()))}
                 }>
                <icon svg={icons::COMMIT} size={Ico::SM} color={tc.text_muted} />
                <div class="flex-1 flex-row" min_w={0.0}>
                    <text size={Px(small)} class="truncate" color={tc.text}>{item.body.clone()}</text>
                </div>
                <text size={Px(mono)} class="font-mono" color={tc.text_muted}>{short_oid.to_owned()}</text>
                <text size={Px(small)} color={tc.text_muted}>{time}</text>
            </div>
        }
    }

    fn conversation(&mut self, overview: &PullRequestOverview) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let comments = overview
            .timeline
            .iter()
            .filter(|item| matches!(item.kind, PullRequestTimelineKind::Comment))
            .count();
        let mut items: Vec<AnyElement> = Vec::new();
        let hidden = overview
            .timeline_total
            .saturating_sub(overview.timeline.len());
        if hidden > 0 {
            let open = controls::button(
                "View on GitHub",
                action(ReviewAction::OpenPullRequestInBrowser),
                Tone::Ghost,
            )
            .icon(lucide::EXTERNAL_LINK);
            items.push(self.event_row(
                format!("{hidden} earlier events"),
                "review.overview.earlier".to_owned(),
                vec![
                    self.muted(plural(hidden as i64, "earlier event", "earlier events")),
                    view! { scale, <spacer /> },
                    view! { scale, <div class="shrink-0">{open}</div> },
                ],
            ));
        }
        for item in &overview.timeline {
            let element = self.timeline_item(item);
            items.push(element);
        }
        if overview.timeline.is_empty() {
            items.push(self.event_row(
                "No activity".to_owned(),
                "review.overview.empty".to_owned(),
                vec![self.muted("No comments yet.")],
            ));
        }
        if !overview.detailed {
            items.push(self.event_row(
                "Sign in for reviews and commits".to_owned(),
                "review.overview.sign_in".to_owned(),
                self.sign_in_hint("Sign in to see reviews and commits"),
            ));
        }
        let composing = self
            .review
            .composer
            .as_ref()
            .is_some_and(|c| c.target == super::ComposerTarget::Conversation);
        let composer = if composing {
            self.presentation
                .card_element(&CardKey::Conversation, self.width)
        } else {
            let button = controls::button(
                "Comment",
                action(ReviewAction::OpenConversationComposer),
                Tone::Secondary,
            )
            .icon(icons::MESSAGE_SQUARE);
            Some(view! { scale,
                <div class="flex-row w-full" test_id="review.overview.comment">
                    <spacer />
                    {button}
                </div>
            })
        };
        let count = if comments == 0 {
            String::new()
        } else {
            comments.to_string()
        };
        view! { scale,
            <div class="flex-col shrink-0" w={Px(self.width)} gap={Sp::SM}
                 accessibility_role={AccessibilityRole::Group}
                 aria-label="Conversation"
                 test_id="review.overview.conversation">
                <div class="flex-row items-center w-full" gap={Sp::XS} pt={Sp::SM}>
                    <text size={Px(TextStyle::Body.font_size(self.theme))} class="font-semibold" color={tc.text_strong}>"Conversation"</text>
                    <text size={Px(self.small())} color={tc.text_muted}>{count}</text>
                </div>
                <div class="flex-col w-full" gap={Sp::SM}
                     accessibility_role={AccessibilityRole::List}
                     aria-label="Timeline"
                     test_id="review.overview.timeline">
                    {...items}
                </div>
                {?composer}
            </div>
        }
    }

    fn banner(&self, message: &str) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let retry = controls::button(
            "Retry",
            action(ReviewAction::RefreshOverview),
            Tone::Secondary,
        );
        view! { scale,
            <div class="flex-row items-center shrink-0" w={Px(self.width)} gap={Sp::SM} px={Sp::MD} py={Sp::SM}
                 rounded={Rad::LG} bg={tc.elevated_surface} border={tc.border}
                 accessibility_role={AccessibilityRole::Alert}
                 aria-label="Couldn't load the pull request overview"
                 test_id="review.overview.error">
                <icon svg={lucide::ALERT_CIRCLE} size={Ico::SM} color={tc.status_error} />
                <div class="flex-1 flex-col" min_w={0.0} gap={Sp::XXS}>
                    <text size={Px(self.small())} class="font-medium" color={tc.text}>"Couldn\u{2019}t load the overview"</text>
                    <text size={Px(self.small())} class="truncate" color={tc.text_muted}>{message.to_owned()}</text>
                </div>
                <div class="shrink-0" test_id="review.overview.retry">{retry}</div>
            </div>
        }
    }

    fn loading(&self) -> AnyElement {
        let lines = skeleton_lines(6, self.theme);
        self.card(
            "Loading".to_owned(),
            "review.overview.loading".to_owned(),
            vec![lines],
        )
    }
}

fn summary(review: &ReviewController) -> Option<Summary> {
    let active = review.active_pull_request()?;
    if let Some(overview) = review.overview.data.as_deref() {
        return Some(Summary {
            number: active.pr.number,
            title: overview.title.clone(),
            state: overview.state.clone(),
            draft: overview.is_draft,
            author: overview.author.login.clone(),
            base: overview.base_branch.clone(),
            head: overview.head_branch.clone(),
            commits: Some(overview.commit_count),
        });
    }
    let info = active.comparison.as_ref().map(|c| &c.info);
    Some(Summary {
        number: active.pr.number,
        title: info.map_or_else(
            || "Loading pull request\u{2026}".to_owned(),
            |info| info.title.clone(),
        ),
        state: info.map_or_else(String::new, info_state),
        draft: info.is_some_and(|info| info.draft),
        author: info.map_or_else(String::new, |info| info.author_login.clone()),
        base: info.map_or_else(String::new, |info| info.base_branch.clone()),
        head: info.map_or_else(String::new, |info| info.head_branch.clone()),
        commits: None,
    })
}

pub fn overview(review: &ReviewController, slot: &Slot, cx: &mut ViewContext) -> AnyElement {
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let pad = tokens::scaled(Sp::LG, scale);
    let width = (slot.width - pad * 2.0)
        .min(tokens::scaled(Sz::CARD_LG, scale))
        .max(1.0);
    let mut page = Page {
        review,
        presentation: review.presentation(),
        theme,
        scale,
        width,
        stale: false,
    };
    let mut sections: Vec<AnyElement> = Vec::new();
    if let Some(summary) = summary(review) {
        sections.push(page.header(&summary));
    }
    let state = &review.overview;
    match (state.data.as_deref(), state.error.as_deref()) {
        (Some(overview), error) => {
            if let Some(error) = error {
                sections.push(page.banner(error));
            }
            sections.push(page.details(overview));
            sections.push(page.description(overview));
            sections.push(page.conversation(overview));
        }
        (None, Some(error)) => sections.push(page.banner(error)),
        (None, None) => sections.push(page.loading()),
    }
    if page.stale {
        cx.frame.request_frame();
    }
    view! { scale,
        <div
            w={Px(slot.width)}
            h={Px(slot.height)}
            class="flex-col overflow-y-scroll scrollbar-auto-hide"
            bg={tc.editor_surface}
            track_scroll={&state.scroll}
            focus_ring={REVIEW_OVERVIEW}
            accessibility_role={AccessibilityRole::Region}
            aria-label="Pull request overview"
            test_id="review.overview"
            on:key:escape={action(ReviewAction::CloseOverview)}
        >
            <div class="flex-col items-center w-full shrink-0" py={Sp::LG} gap={Sp::MD}>
                {...sections}
            </div>
        </div>
    }
}
