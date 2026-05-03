use halogen::view;

use crate::actions::{Action, OverlayAction, RepositoryAction};
use crate::core::vcs::model::{ChangeIdToken, PublishAction, PublishActionKind, PublishPlan};
use crate::ui::design::{Ico, Rad, Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::AppState;
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme};
use crate::ui::vcs::PublishRefChipUi;

pub fn publish_menu(state: &AppState, theme: &Theme, width: f32, height: f32) -> AnyElement {
    let tc = &theme.colors;
    let m = &theme.metrics;
    let scale = m.ui_scale();

    let menu_w = (Sz::CARD_SM * scale).round();
    let margin = (Sp::LG * scale).round();
    let menu_x = margin;
    let menu_bottom_gap = m.status_bar_height + (Sp::XS * scale).round();

    let plan = state
        .repository
        .publish_plan
        .with(&state.store, |p| p.clone());
    let bookmarks = publish_ref_chips(state);

    let body = match plan {
        Some(plan) => plan_body(plan, theme),
        None => loading_body(theme),
    };

    let bookmark_section = (!bookmarks.is_empty()).then(|| bookmark_block(&bookmarks, theme));

    view! { scale,
        <div class="absolute" left={0.0} top={0.0} w={width} h={height}
             z_index={200}
             bg={Color::TRANSPARENT}
             on_click={OverlayAction::CloseOverlay.into()}
             hit_identity={HitIdentity::OverlayBackdrop}>
            <div class="absolute flex-col overflow-hidden"
                 left={menu_x}
                 bottom={menu_bottom_gap}
                 w={menu_w}
                 py={Sp::XS}
                 bg={tc.elevated_surface}
                 border={tc.border}
                 rounded={Rad::XL}
                 shadow_preset={Shadow::DROPDOWN}
                 on_click={Action::Noop}>
                {header(theme)}
                {?bookmark_section}
                {body}
            </div>
        </div>
    }
}

fn header(theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    view! { scale,
        <div class="flex-row items-center"
             gap={Sp::SM}
             px={Sp::MD}
             py={Sp::XS + Sp::XXS}>
            <icon svg={lucide::ARROW_UP} size={Ico::SM} color={tc.text_strong} />
            <text class="text-sm font-semibold" color={tc.text_strong}>{"Publish"}</text>
        </div>
    }
    .into_any()
}

fn separator(theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    view! { scale,
        <div class="w-full" py={Sp::XS} px={Sp::SM}>
            <div class="w-full" h={Sz::SEPARATOR_W} bg={tc.border_variant} />
        </div>
    }
    .into_any()
}

fn loading_body(theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    view! { scale,
        <div class="flex-col">
            {separator(theme)}
            <div class="flex-row items-center"
                 gap={Sp::SM}
                 px={Sp::MD}
                 py={Sp::SM}>
                <icon svg={lucide::LOADER} size={Ico::SM} color={tc.text_muted} />
                <text class="text-sm" color={tc.text_muted}>{"Loading publish options\u{2026}"}</text>
            </div>
        </div>
    }
    .into_any()
}

fn plan_body(plan: PublishPlan, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let primary = plan.primary;
    let alternatives = plan.alternatives;

    let mut alt_rows: Vec<AnyElement> = Vec::with_capacity(alternatives.len());
    for action in alternatives {
        alt_rows.push(action_row(action, false, theme));
    }

    let alt_section = (!alt_rows.is_empty()).then(|| {
        view! { scale,
            <div class="flex-col">
                <div class="flex-row items-center"
                     px={Sp::MD}
                     pt={Sp::SM}
                     pb={Sp::XXS}>
                    <text class="text-xs" color={tc.text_muted}>{"More options"}</text>
                </div>
                {...alt_rows}
            </div>
        }
        .into_any()
    });

    view! { scale,
        <div class="flex-col">
            {separator(theme)}
            {action_row(primary, true, theme)}
            {?alt_section}
        </div>
    }
    .into_any()
}

fn action_row(action: PublishAction, primary: bool, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let (icon, accent) = action_visuals(&action.kind, tc);
    let label_color = if primary { tc.text_strong } else { tc.text };
    let click: Action = RepositoryAction::Publish(action.clone()).into();
    let token = action.change_id_token.as_ref();
    let title = highlighted_runs(
        &action.label,
        token,
        SpanStyle {
            base_color: label_color,
            small: false,
            bold: primary,
        },
        tc,
    );
    let description = highlighted_runs(
        &action.description,
        token,
        SpanStyle {
            base_color: tc.text_muted,
            small: true,
            bold: false,
        },
        tc,
    );

    view! { scale,
        <div class="flex-row items-start"
             gap={Sp::SM}
             px={Sp::MD}
             py={Sp::XS + Sp::XXS}
             rounded={Rad::MD}
             hover_bg={tc.sidebar_row_hover}
             cursor={CursorHint::Pointer}
             on_click={click}>
            <div class="shrink-0" pt={2.0}>
                <icon svg={icon} size={Ico::SM} color={accent} />
            </div>
            <div class="flex-col flex-1" gap={1.0} min_w={0.0}>
                {title}
                {description}
            </div>
        </div>
    }
    .into_any()
}

#[derive(Clone, Copy)]
struct SpanStyle {
    base_color: crate::ui::theme::Color,
    small: bool,
    bold: bool,
}

fn styled_text(s: String, color: crate::ui::theme::Color, style: SpanStyle) -> AnyElement {
    let mut t = text(s).color(color);
    t = if style.small {
        t.text_xs()
    } else {
        t.text_sm()
    };
    if style.bold {
        t = t.semibold();
    }
    t.into_any()
}

fn highlighted_runs(
    s: &str,
    token: Option<&ChangeIdToken>,
    style: SpanStyle,
    tc: &crate::ui::theme::ThemeColors,
) -> AnyElement {
    let Some(token) = token else {
        return styled_text(s.to_owned(), style.base_color, style);
    };
    let Some(idx) = s.find(token.text.as_str()) else {
        return styled_text(s.to_owned(), style.base_color, style);
    };
    let pre = &s[..idx];
    let id_text = &s[idx..idx + token.text.len()];
    let post = &s[idx + token.text.len()..];
    let split = token.prefix_len.min(id_text.len());
    let split = if id_text.is_char_boundary(split) {
        split
    } else {
        0
    };
    let (prefix, rest) = id_text.split_at(split);
    let prefix_color = tc.syntax_keyword.lerp(tc.text_strong, 0.28);

    let pre_span = (!pre.is_empty()).then(|| styled_text(pre.to_owned(), style.base_color, style));
    let prefix_span = (!prefix.is_empty()).then(|| {
        let mut id_style = style;
        id_style.bold = true;
        styled_text(prefix.to_owned(), prefix_color, id_style)
    });
    let rest_span = (!rest.is_empty()).then(|| styled_text(rest.to_owned(), tc.text_muted, style));
    let post_span =
        (!post.is_empty()).then(|| styled_text(post.to_owned(), style.base_color, style));

    view! {
        <div class="flex-row flex-wrap items-baseline">
            {?pre_span}
            {?prefix_span}
            {?rest_span}
            {?post_span}
        </div>
    }
    .into_any()
}

fn action_visuals(
    kind: &PublishActionKind,
    tc: &crate::ui::theme::ThemeColors,
) -> (&'static str, Color) {
    match kind {
        PublishActionKind::PushRef { .. }
        | PublishActionKind::PushBookmark { .. }
        | PublishActionKind::PushTracked { .. } => (lucide::ARROW_UP, tc.text_strong),
        PublishActionKind::PushChange { .. } => (lucide::CIRCLE_DOT, tc.syntax_keyword),
        PublishActionKind::CreateBookmarkAndPush { .. } => (lucide::PLUS, tc.line_add_text),
        PublishActionKind::MoveBookmarkAndPush { .. } => (lucide::ARROW_LEFT_RIGHT, tc.text_strong),
    }
}

fn bookmark_block(bookmarks: &[PublishRefChipUi], theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let mut rows: Vec<AnyElement> = Vec::with_capacity(bookmarks.len());
    for bookmark in bookmarks {
        let (status_label, status_color) = if bookmark.tracked {
            ("tracked", tc.text_muted)
        } else {
            ("untracked", tc.status_warning)
        };
        let row = view! { scale,
            <div class="flex-row items-center"
                 gap={Sp::SM}
                 px={Sp::MD}
                 py={Sp::XXS}>
                <icon svg={lucide::GIT_BRANCH} size={Ico::XS} color={tc.text_muted} />
                <text class="text-sm truncate" color={tc.text}>{bookmark.name.clone()}</text>
                if let Some(upstream) = bookmark.upstream.clone() {
                    <text class="text-xs" color={tc.text_muted}>{"\u{2192}"}</text>
                    <text class="text-xs truncate" color={tc.text_muted}>{upstream}</text>
                }
                <spacer />
                <text class="text-xs" color={status_color}>{status_label}</text>
            </div>
        }
        .into_any();
        rows.push(row);
    }

    view! { scale,
        <div class="flex-col">
            {separator(theme)}
            <div class="flex-row items-center"
                 px={Sp::MD}
                 pt={Sp::SM}
                 pb={Sp::XXS}>
                <text class="text-xs" color={tc.text_muted}>{"Bookmarks here"}</text>
            </div>
            {...rows}
        </div>
    }
    .into_any()
}

fn publish_ref_chips(state: &AppState) -> Vec<PublishRefChipUi> {
    let profile = state.repository.location.with(&state.store, |location| {
        crate::ui::vcs::profile(location.as_ref())
    });
    let changes = state
        .repository
        .changes
        .with(&state.store, |changes| changes.clone());
    let refs = state
        .repository
        .refs
        .with(&state.store, |refs| refs.clone());
    let has_remotes = state
        .repository
        .capabilities
        .with(&state.store, |capabilities| {
            capabilities.is_some_and(|capabilities| capabilities.remotes)
        });
    profile
        .publish_status_ui(&changes, &refs, has_remotes)
        .ref_chips
}
