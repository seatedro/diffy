use std::collections::HashSet;

use quark::{Px, view};
use quark_app::ViewContext;
use quark_components::{avatar, badge, kbd, search_field, skeleton_lines};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};

use super::ReviewController;
use super::controls::{self, Tone};
use super::overview::{CheckTone, check_icon, check_tone, reviewer_state, state_badge};
use super::presentation::parse_rfc3339_utc_seconds;
use super::pulls::{PullGroup, PullStatus, PullsPage, pull_status, row_key};
use crate::app::icons;
use crate::app::shell::Slot;
use crate::app::text_targets::{REVIEW_PULLS, REVIEW_PULLS_FILTER};
use crate::app::tokens::{self, Bp, Ico, Layout, Rad, Sp, TextStyle};
use crate::contracts::review::{PullsAction, ReviewAction};
use crate::core::forge::github::{MyPullRequest, PullRequestCheckSummary, PullRequestReviewer};
use crate::core::ref_catalog::{now_secs, relative_age};

const ROW_FACES: usize = 3;

fn pulls(action: PullsAction) -> quark_ui::Action {
    ReviewAction::Pulls(action).into()
}

const KEYS: [(&str, PullsAction); 10] = [
    ("j", PullsAction::Move(1)),
    ("arrowdown", PullsAction::Move(1)),
    ("k", PullsAction::Move(-1)),
    ("arrowup", PullsAction::Move(-1)),
    ("enter", PullsAction::Review),
    ("o", PullsAction::OpenOnGitHub),
    ("y", PullsAction::CopyLink),
    ("r", PullsAction::Refresh),
    ("/", PullsAction::FocusFilter),
    ("escape", PullsAction::Close),
];

const HINTS: [(&str, &str); 6] = [
    ("j k", "move"),
    ("\u{21b5}", "review"),
    ("o", "GitHub"),
    ("y", "copy link"),
    ("r", "refresh"),
    ("esc", "close"),
];

struct Page<'a> {
    review: &'a ReviewController,
    slot: &'a Slot<'a>,
    theme: &'a Theme,
    scale: f32,
    focused: bool,
    now: i64,
}

fn status_color(status: PullStatus, theme: &Theme) -> Color {
    group_color(status.group(), theme)
}

fn group_color(group: PullGroup, theme: &Theme) -> Color {
    let tc = &theme.colors;
    match group {
        PullGroup::NeedsYou => tc.line_del_text,
        PullGroup::Waiting => tc.status_warning,
        PullGroup::Ready => tc.line_add_text,
        PullGroup::Drafts => tc.text_muted,
    }
}

fn merge_label(status: PullStatus) -> &'static str {
    match status {
        PullStatus::ChangesRequested => "Blocked by review",
        PullStatus::Conflicts => "Has merge conflicts",
        PullStatus::ChecksFailing => "Checks failing",
        PullStatus::ReviewRequired => "Review required",
        PullStatus::ChecksRunning => "Waiting for checks",
        PullStatus::CheckingMergeability => "Checking mergeability",
        PullStatus::Approved | PullStatus::Ready => "Ready to merge",
        PullStatus::Draft => "Draft, not ready for review",
    }
}

fn reference(pull: &MyPullRequest) -> String {
    let name = pull
        .repository
        .rsplit('/')
        .next()
        .unwrap_or(&pull.repository);
    format!("{name}#{}", pull.number)
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

impl Page<'_> {
    fn small(&self) -> f32 {
        TextStyle::BodySmall.font_size(self.theme)
    }

    fn muted(&self, label: impl Into<String>) -> AnyElement {
        let scale = self.scale;
        let label = label.into();
        view! { scale, <text size={Px(self.small())} class="truncate" color={self.theme.colors.text_muted}>{label}</text> }
    }

    fn dot(&self, color: Color) -> AnyElement {
        let size = tokens::scaled(Layout::STATUS_DOT, self.scale);
        view! { <div class="shrink-0" w={size} h={size} rounded={size} bg={color} /> }
    }

    fn chip(&self, label: String) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let size = TextStyle::Mono.font_size(self.theme);
        view! { scale,
            <div class="flex-row items-center overflow-hidden" min_w={0.0} px={Sp::XS} rounded={Rad::SM} bg={tc.element_background}>
                <text size={Px(size)} class="truncate font-mono" color={tc.text}>{label}</text>
            </div>
        }
    }

    fn face(&self, login: &str, size: f32) -> AnyElement {
        let face = self.review.host.face(login);
        view! { <div class="shrink-0">{avatar(login.to_owned()).size(size).image(face)}</div> }
    }

    fn header(&self, page: &PullsPage) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let count = page.data.as_ref().map(|_| format!("{} open", page.total()));
        let updated = match (page.loading, page.fetched_at) {
            (true, _) => Some("Updating\u{2026}".to_owned()),
            (false, Some(at)) => Some(match relative_age(self.now, at).as_str() {
                "now" => "Updated just now".to_owned(),
                age => format!("Updated {age} ago"),
            }),
            (false, None) => None,
        };
        let refresh = controls::icon_button(
            lucide::REFRESH,
            "Refresh",
            pulls(PullsAction::Refresh),
            self.theme,
        )
        .disabled(page.loading || !self.review.signed_in());
        let close = controls::button("Close", pulls(PullsAction::Close), Tone::Ghost)
            .icon(lucide::X)
            .tooltip("Esc");
        view! { scale,
            <div class="flex-row items-center w-full shrink-0" h={Px(tokens::scaled(Layout::COMPARE_BAR_H, scale))}
                 px={Sp::MD} gap={Sp::SM} border_b={tc.border_variant} test_id="review.pulls.header">
                <icon svg={lucide::GIT_PULL_REQUEST} size={Ico::SM} color={tc.text_muted} />
                <div accessibility_role={AccessibilityRole::Heading} aria-label="Your pull requests">
                    <text size={Px(TextStyle::Body.font_size(self.theme))} class="font-semibold" color={tc.text_strong}>"Your pull requests"</text>
                </div>
                if let Some(count) = count {
                    <text size={Px(self.small())} color={tc.text_muted}>{count}</text>
                }
                <spacer />
                if let Some(updated) = updated {
                    <div test_id="review.pulls.updated">{self.muted(updated)}</div>
                }
                <div test_id="review.pulls.refresh">{refresh}</div>
                <div test_id="review.pulls.close">{close}</div>
            </div>
        }
    }

    fn filter(&self, page: &PullsPage, focused: bool) -> AnyElement {
        let scale = self.scale;
        let m = &self.theme.metrics;
        let has_text = !page.filter.text().is_empty();
        let input = text_input("Filter pull requests", "")
            .field(&page.filter)
            .placeholder("Filter by title or repository")
            .focus_target(REVIEW_PULLS_FILTER)
            .focused(focused)
            .search(true)
            .bare()
            .w_full()
            .h((m.ui_small_font_size * 1.5).round());
        let field = search_field(
            input,
            has_text,
            Some(pulls(PullsAction::ClearFilter)),
            (!focused).then_some("/"),
            self.theme,
        );
        let mut wrap = div().flex_1().min_w(0.0).test_id("review.pulls.filter");
        for (binding, action) in [
            ("escape", PullsAction::EscapeFilter),
            ("enter", PullsAction::Review),
            ("arrowdown", PullsAction::Move(1)),
            ("arrowup", PullsAction::Move(-1)),
        ] {
            wrap = wrap.on_key(binding, pulls(action));
        }
        view! { scale,
            <div class="flex-row items-center w-full shrink-0" px={Sp::SM} py={Sp::XS}>
                {wrap.child(field)}
            </div>
        }
    }

    fn checks_summary(&self, checks: &PullRequestCheckSummary) -> Option<AnyElement> {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let (tone, count) = if checks.failure_count > 0 {
            (CheckTone::Failed, checks.failure_count)
        } else if checks.pending_count > 0 {
            (CheckTone::Pending, checks.pending_count)
        } else if checks.success_count > 0 {
            (CheckTone::Passed, checks.success_count)
        } else {
            return None;
        };
        let (icon, color) = check_icon(tone, self.theme);
        Some(view! { scale,
            <div class="flex-row items-center shrink-0" gap={Sp::XXS}>
                <icon svg={icon} size={Ico::XS} color={color} />
                <text size={Px(self.small())} color={tc.text_muted}>{count.to_string()}</text>
            </div>
        })
    }

    fn faces(&self, reviewers: &[PullRequestReviewer]) -> AnyElement {
        let tc = &self.theme.colors;
        let size = tokens::scaled(Ico::SM, self.scale);
        let step = size - (size * 0.25).round();
        let shown = reviewers.len().min(ROW_FACES);
        let faces: Vec<AnyElement> = reviewers[..shown]
            .iter()
            .enumerate()
            .map(|(index, reviewer)| {
                let ring = match reviewer.state.as_str() {
                    "APPROVED" => tc.line_add_text,
                    "CHANGES_REQUESTED" => tc.line_del_text,
                    _ => tc.editor_surface,
                };
                let face = view! {
                    <div class="shrink-0" border={ring} rounded={size}>
                        {self.face(&reviewer.actor.login, Ico::SM)}
                    </div>
                };
                controls::stacked(face, step, index + 1 == shown)
            })
            .collect();
        view! { <div class="flex-row items-center shrink-0">{...faces}</div> }
    }

    fn row(&self, pull: &MyPullRequest, selected: bool) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let paint = tokens::row_paint(self.theme, tc.editor_surface, selected, self.focused);
        let status = pull_status(pull);
        let color = if selected && self.focused {
            paint.text
        } else {
            status_color(status, self.theme)
        };
        let age = parse_rfc3339_utc_seconds(&pull.updated_at)
            .map(|then| relative_age(self.now, then as i64))
            .unwrap_or_default();
        let mut meta = vec![self.chip(reference(pull))];
        if let Some(checks) = pull.checks.as_ref().and_then(|c| self.checks_summary(c)) {
            meta.push(checks);
        }
        if pull.unresolved_threads > 0 {
            meta.push(self.muted(format!("{} unresolved", pull.unresolved_threads)));
        }
        if !pull.reviewers.is_empty() {
            meta.push(self.faces(&pull.reviewers));
        }
        let label = format!("{}, {}, {}", pull.title, reference(pull), status.label());
        view! { scale,
            <div class="flex-row items-center w-full shrink-0" px={Sp::MD} py={Sp::SM} gap={Sp::SM}
                 bg={paint.bg}
                 @when {!selected} { hover_bg={tc.ghost_element_hover} }
                 cursor={CursorHint::Pointer}
                 key={row_key(&pull.url)}
                 accessibility_role={AccessibilityRole::ListBoxOption}
                 aria-label={label}
                 aria-selected={selected}
                 test_id={format!("review.pulls.row.{}", pull.number)}
                 on:click={pulls(PullsAction::Select(pull.url.clone()))}>
                {self.dot(status_color(status, self.theme))}
                <div class="flex-1 flex-col" min_w={0.0} gap={Sp::XXS}>
                    <div class="flex-row w-full" min_w={0.0}>
                        <text size={Px(TextStyle::Body.font_size(self.theme))} class="truncate font-medium" color={paint.text}>{pull.title.clone()}</text>
                    </div>
                    <div class="flex-row items-center w-full overflow-hidden" gap={Sp::SM}>{...meta}</div>
                </div>
                <div class="flex-col items-end shrink-0" gap={Sp::XXS}>
                    <text size={Px(self.small())} color={color}>{status.label()}</text>
                    <text size={Px(self.small())} color={paint.muted}>{age}</text>
                </div>
            </div>
        }
    }

    fn list(&self, page: &PullsPage) -> Vec<AnyElement> {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let Some(data) = page.data.as_ref() else {
            return Vec::new();
        };
        page.sections
            .iter()
            .map(|section| {
                let rows: Vec<AnyElement> = section
                    .rows
                    .iter()
                    .map(|index| {
                        let pull = &data[*index];
                        self.row(pull, page.selected.as_deref() == Some(pull.url.as_str()))
                    })
                    .collect();
                let label = section.group.label();
                view! { scale,
                    <div class="flex-col w-full shrink-0"
                         accessibility_role={AccessibilityRole::Group}
                         aria-label={label}
                         test_id={format!("review.pulls.group.{}", section.group.id())}>
                        <div class="flex-row items-center w-full" px={Sp::MD} pt={Sp::MD} pb={Sp::XS} gap={Sp::XS}>
                            {self.dot(group_color(section.group, self.theme))}
                            <text size={Px(self.small())} class="font-medium" color={tc.text_muted}>{label}</text>
                            <div class="flex-row items-center" px={Sp::XS} rounded={Rad::PILL} bg={tc.element_background}>
                                <text size={Px(self.small())} color={tc.text_muted}>{section.rows.len().to_string()}</text>
                            </div>
                        </div>
                        {...rows}
                    </div>
                }
            })
            .collect()
    }

    fn notice(&self, title: &str, detail: &str, test_id: &str) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        view! { scale,
            <div class="flex-col items-center justify-center w-full flex-1" gap={Sp::XS} p={Sp::LG}
                 accessibility_role={AccessibilityRole::Status} aria-label={title.to_owned()} test_id={test_id.to_owned()}>
                <text size={Px(TextStyle::Body.font_size(self.theme))} class="font-medium" color={tc.text}>{title.to_owned()}</text>
                <text size={Px(self.small())} color={tc.text_muted}>{detail.to_owned()}</text>
            </div>
        }
    }

    fn banner(&self, message: &str) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let retry = controls::button("Retry", pulls(PullsAction::Refresh), Tone::Secondary);
        view! { scale,
            <div class="flex-row w-full shrink-0" px={Sp::SM} py={Sp::XS}>
                <div class="flex-1 flex-row items-center" min_w={0.0} gap={Sp::SM} px={Sp::MD} py={Sp::SM}
                     rounded={Rad::LG} bg={tc.elevated_surface} border={tc.border}
                     accessibility_role={AccessibilityRole::Alert}
                     aria-label="Couldn't load your pull requests"
                     test_id="review.pulls.error">
                    <icon svg={lucide::ALERT_CIRCLE} size={Ico::SM} color={tc.status_error} />
                    <div class="flex-1 flex-col" min_w={0.0} gap={Sp::XXS}>
                        <text size={Px(self.small())} class="font-medium" color={tc.text}>"Couldn\u{2019}t load your pull requests"</text>
                        <text size={Px(self.small())} class="truncate" color={tc.text_muted}>{message.to_owned()}</text>
                    </div>
                    <div class="shrink-0" test_id="review.pulls.retry">{retry}</div>
                </div>
            </div>
        }
    }

    fn footer(&self) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let hints: Vec<AnyElement> = HINTS
            .iter()
            .map(|(keys, label)| {
                let caps: Vec<AnyElement> = keys
                    .split(' ')
                    .map(|key| kbd(key.to_owned(), self.theme))
                    .collect();
                view! { scale,
                    <div class="flex-row items-center shrink-0" gap={Sp::XXS}>
                        {...caps}
                        <text size={Px(self.small())} color={tc.text_muted}>{(*label).to_owned()}</text>
                    </div>
                }
            })
            .collect();
        view! { scale,
            <div class="flex-row items-center w-full shrink-0 overflow-hidden" h={Px(tokens::scaled(Layout::COMPARE_BAR_H, scale))}
                 px={Sp::MD} gap={Sp::MD} border_t={tc.border_variant}
                 accessibility_role={AccessibilityRole::Group} aria-label="Keyboard shortcuts"
                 test_id="review.pulls.keys">
                {...hints}
            </div>
        }
    }

    fn list_pane(&self, page: &PullsPage, width: f32, filter_focused: bool) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let mut body: Vec<AnyElement> = Vec::new();
        if let (Some(error), Some(_)) = (page.error.as_deref(), page.data.as_ref()) {
            body.push(self.banner(error));
        }
        let content = match (page.data.as_ref(), page.error.as_deref()) {
            (None, Some(error)) => {
                body.push(self.banner(error));
                None
            }
            (None, None) => Some(view! { scale,
                <div class="flex-col w-full" p={Sp::MD} test_id="review.pulls.loading">
                    {skeleton_lines(8, self.theme)}
                </div>
            }),
            (Some(data), _) if data.is_empty() => Some(self.notice(
                "No open pull requests",
                "Pull requests you open on GitHub show up here.",
                "review.pulls.empty",
            )),
            (Some(_), _) if page.sections.is_empty() => Some(self.notice(
                "No matching pull requests",
                "Try a different title or repository.",
                "review.pulls.no_match",
            )),
            (Some(_), _) => {
                let groups = self.list(page);
                Some(view! { scale,
                    <div class="flex-col w-full flex-1 overflow-y-scroll scrollbar-auto-hide" min_h={0.0}
                         track_scroll={&page.list}
                         accessibility_role={AccessibilityRole::ListBox}
                         aria-label="Pull requests"
                         test_id="review.pulls.list">
                        {...groups}
                        <div class="w-full shrink-0" h={Sp::SM} />
                    </div>
                })
            }
        };
        view! { scale,
            <div class="flex-col h-full shrink-0" w={Px(width)} border_r={tc.border_variant}>
                {self.filter(page, filter_focused)}
                {...body}
                {?content}
                {self.footer()}
            </div>
        }
    }

    fn field(&self, label: &str, content: Vec<AnyElement>) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        view! { scale,
            <div class="flex-row items-start w-full" gap={Sp::MD}
                 accessibility_role={AccessibilityRole::Group} aria-label={label.to_owned()}
                 test_id={format!("review.pulls.preview.{}", label.to_ascii_lowercase())}>
                <div class="flex-row items-center shrink-0" w={Layout::OVERVIEW_LABEL_W} h={Px(controls::control_height(self.theme))}>
                    <text size={Px(self.small())} color={tc.text_muted}>{label.to_owned()}</text>
                </div>
                <div class="flex-1 flex-row flex-wrap items-center" min_w={0.0} min_h={Px(controls::control_height(self.theme))} gap={Sp::XS}>
                    {...content}
                </div>
            </div>
        }
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
                 tooltip={label}>
                {self.face(login, Ico::MD)}
                <text size={Px(self.small())} color={tc.text}>{login.clone()}</text>
                <icon svg={icon} size={Ico::XS} color={color} />
            </div>
        }
    }

    fn checks(&self, pull: &MyPullRequest) -> Vec<AnyElement> {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let Some(checks) = pull.checks.as_ref() else {
            return vec![self.muted(if pull.stats.is_some() {
                "No checks reported"
            } else {
                "Unavailable"
            })];
        };
        let mut sorted: Vec<_> = checks.contexts.iter().collect();
        sorted.sort_by_key(|check| match check_tone(&check.state) {
            CheckTone::Failed => 0,
            CheckTone::Pending => 1,
            CheckTone::Passed => 2,
        });
        let mut seen = HashSet::new();
        sorted.retain(|check| seen.insert(check.name.as_str()));
        let shown = sorted.len() as i32;
        let mut content = vec![self.muted(format!(
            "{} passed \u{b7} {} failed \u{b7} {} pending",
            checks.success_count, checks.failure_count, checks.pending_count
        ))];
        content.extend(sorted.into_iter().map(|check| {
            let (icon, color) = check_icon(check_tone(&check.state), self.theme);
            view! { scale,
                <div class="flex-row items-center shrink-0" gap={Sp::XXS}
                     accessibility_role={AccessibilityRole::Label}
                     aria-label={format!("{}: {}", check.name, check.state.to_ascii_lowercase())}>
                    <icon svg={icon} size={Ico::XS} color={color} />
                    <text size={Px(self.small())} color={tc.text}>{check.name.clone()}</text>
                </div>
            }
        }));
        if checks.total_count > shown {
            content.push(self.muted(format!("+{} more", checks.total_count - shown)));
        }
        content
    }

    fn location(&self, pull: &MyPullRequest) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let home = dirs::home_dir();
        match self.review.clone_path(self.slot.shared, &pull.repository) {
            Some(path) => {
                let shown = match home
                    .as_deref()
                    .and_then(|home| path.strip_prefix(home).ok())
                {
                    Some(rest) => format!("~/{}", rest.display()),
                    None => path.display().to_string(),
                };
                view! { scale,
                    <div class="flex-row items-center w-full" gap={Sp::XS} test_id="review.pulls.preview.path"
                         accessibility_role={AccessibilityRole::Label} aria-label={format!("Local clone {shown}")}>
                        <icon svg={lucide::FOLDER} size={Ico::SM} color={tc.text_muted} />
                        {self.muted(shown)}
                    </div>
                }
            }
            None => {
                let locate = controls::button(
                    "Locate clone\u{2026}",
                    pulls(PullsAction::LocateClone),
                    Tone::Ghost,
                )
                .icon(lucide::FOLDER_OPEN);
                view! { scale,
                    <div class="flex-row items-center w-full" gap={Sp::XS} test_id="review.pulls.preview.path">
                        <icon svg={lucide::FOLDER} size={Ico::SM} color={tc.text_muted} />
                        {self.muted("Not opened in Diffy")}
                        <div class="shrink-0" test_id="review.pulls.locate">{locate}</div>
                    </div>
                }
            }
        }
    }

    fn preview(&self, page: &PullsPage, width: f32) -> AnyElement {
        let scale = self.scale;
        let body = match page.selected_pull() {
            Some(pull) => self.preview_body(pull),
            None if page.data.is_none() && page.error.is_none() => skeleton_lines(6, self.theme),
            None => self.muted("Select a pull request to preview it."),
        };
        view! { scale,
            <div class="flex-col h-full shrink-0 overflow-y-scroll scrollbar-auto-hide" w={Px(width)}
                 p={Sp::LG} gap={Sp::MD}
                 track_scroll={&page.preview}
                 accessibility_role={AccessibilityRole::Region}
                 aria-label="Pull request preview"
                 test_id="review.pulls.preview">
                {body}
            </div>
        }
    }

    fn preview_body(&self, pull: &MyPullRequest) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let small = self.small();
        let mono = TextStyle::Mono.font_size(self.theme);
        let status = pull_status(pull);
        let (state, icon, variant) = state_badge("OPEN", pull.is_draft);
        let mut change = Vec::new();
        if let Some(stats) = pull.stats.as_ref() {
            change.push(view! { scale, <text size={Px(mono)} class="font-mono" color={tc.line_add_text}>{format!("+{}", stats.additions)}</text> });
            change.push(view! { scale, <text size={Px(mono)} class="font-mono" color={tc.line_del_text}>{format!("\u{2212}{}", stats.deletions)}</text> });
            change.push(self.muted(plural(stats.changed_files as usize, "file", "files")));
        }
        let mut reviewers: Vec<AnyElement> =
            pull.reviewers.iter().map(|r| self.reviewer(r)).collect();
        if reviewers.is_empty() {
            reviewers.push(self.muted("No reviewers"));
        }
        let threads = if pull.unresolved_threads == 0 {
            "None unresolved".to_owned()
        } else {
            format!("{} unresolved", pull.unresolved_threads)
        };
        let merge = view! { scale,
            <div class="flex-row items-center" gap={Sp::XS}>
                {self.dot(status_color(status, self.theme))}
                <text size={Px(small)} color={tc.text}>{merge_label(status)}</text>
            </div>
        };
        let description = if pull.excerpt.is_empty() {
            "No description provided.".to_owned()
        } else {
            pull.excerpt.clone()
        };
        let review = controls::button("Review", pulls(PullsAction::Review), Tone::Primary)
            .icon(lucide::GIT_PULL_REQUEST)
            .tooltip("Enter");
        let github = controls::button(
            "Open on GitHub",
            pulls(PullsAction::OpenOnGitHub),
            Tone::Secondary,
        )
        .icon(lucide::EXTERNAL_LINK)
        .tooltip("o");
        view! { scale,
            <div class="flex-col w-full" gap={Sp::MD}>
                <div class="flex-row items-center w-full" gap={Sp::SM}>
                    <div class="shrink-0" accessibility_role={AccessibilityRole::Label} aria-label={format!("State: {state}")}>
                        {badge(state).icon(icon).variant(variant)}
                    </div>
                    <text size={Px(mono)} class="truncate font-mono" color={tc.text_muted}>{format!("{}#{}", pull.repository, pull.number)}</text>
                </div>
                {self.location(pull)}
                <div class="flex-row w-full" accessibility_role={AccessibilityRole::Heading} aria-label={pull.title.clone()}>
                    <text size={Px(TextStyle::HeadingLg.font_size(self.theme))} class="font-semibold" color={tc.text_strong}>{pull.title.clone()}</text>
                </div>
                <div class="flex-row flex-wrap items-center w-full" gap={Sp::XS} test_id="review.pulls.preview.branches">
                    {self.chip(pull.head_branch.clone())}
                    <icon svg={icons::ARROW_RIGHT_TO_LINE} size={Ico::XS} color={tc.text_muted} />
                    {self.chip(pull.base_branch.clone())}
                    {...change}
                </div>
                {self.field("Reviewers", reviewers)}
                {self.field("Checks", self.checks(pull))}
                {self.field("Threads", vec![self.muted(threads)])}
                {self.field("Merge", vec![merge])}
                <div class="flex-col w-full" p={Sp::SM} rounded={Rad::LG} bg={tc.elevated_surface} border={tc.border_variant}
                     accessibility_role={AccessibilityRole::Group} aria-label="Description"
                     test_id="review.pulls.preview.description">
                    <text size={Px(small)} color={tc.text}>{description}</text>
                </div>
                <div class="flex-row flex-wrap items-center w-full" gap={Sp::SM}>
                    <div test_id="review.pulls.review">{review}</div>
                    <div test_id="review.pulls.open_in_browser">{github}</div>
                </div>
            </div>
        }
    }

    fn sign_in(&self) -> AnyElement {
        let scale = self.scale;
        let tc = &self.theme.colors;
        let button = controls::button("Sign in", ReviewAction::OpenAuthModal, Tone::Primary)
            .icon(lucide::GITHUB_MARK);
        view! { scale,
            <div class="flex-col items-center justify-center w-full flex-1" gap={Sp::SM} p={Sp::LG}
                 accessibility_role={AccessibilityRole::Group} aria-label="Sign in to see your pull requests"
                 test_id="review.pulls.sign_in">
                <icon svg={lucide::GITHUB_MARK} size={Ico::XL} color={tc.text_muted} />
                <text size={Px(TextStyle::Body.font_size(self.theme))} class="font-medium" color={tc.text}>"Sign in to see your pull requests"</text>
                <text size={Px(self.small())} color={tc.text_muted}>"Diffy lists the open pull requests you authored on GitHub."</text>
                {button}
            </div>
        }
    }
}

pub fn pulls_page(review: &ReviewController, slot: &Slot, cx: &mut ViewContext) -> AnyElement {
    let theme = cx.theme;
    let scale = tokens::ui_scale(theme);
    let filter_focused = cx.is_focused(REVIEW_PULLS_FILTER);
    let page = Page {
        review,
        slot,
        theme,
        scale,
        focused: cx.is_focused(REVIEW_PULLS) || filter_focused,
        now: now_secs(),
    };
    let state = review.pulls();
    let header = page.header(state);
    let body = if review.signed_in() {
        let preview_w = tokens::scaled(Layout::PULLS_PREVIEW_W, scale);
        let wide = slot.width >= tokens::scaled(Bp::NARROW, scale);
        let list_w = if wide {
            (slot.width - preview_w).max(0.0)
        } else {
            slot.width
        };
        let list = page.list_pane(state, list_w, filter_focused);
        let preview = wide.then(|| page.preview(state, preview_w));
        view! { <div class="flex-row w-full flex-1" min_h={0.0}>{list}{?preview}</div> }
    } else {
        page.sign_in()
    };
    let mut root = div()
        .w(slot.width)
        .h(slot.height)
        .flex_col()
        .bg(theme.colors.editor_surface)
        .focus_target(REVIEW_PULLS)
        .accessibility_role(AccessibilityRole::Region)
        .accessibility_label("Your pull requests")
        .test_id("review.pulls");
    for (binding, action) in KEYS {
        root = root.on_key(binding, pulls(action));
    }
    root.child(header).child(body).into_any()
}
