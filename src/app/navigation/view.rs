use quark::view;
use quark_app::ViewContext;
use quark_components::search_field;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::design::Sz;
use quark_ui::element::*;
use quark_ui::style::Styled;

use super::NavigationController;
use crate::app::shell::{Slot, empty_state};
use crate::app::text_targets::SIDEBAR_FILTER;
use crate::app::tokens::{self, Sp};

pub fn view(nav: &mut NavigationController, slot: &Slot, cx: &mut ViewContext) -> AnyElement {
    let focused = cx.is_focused(SIDEBAR_FILTER);
    let theme = cx.theme;
    let m = &theme.metrics;
    let scale = tokens::ui_scale(theme);
    let filter = nav.filter();
    let input = text_input("Filter files", "")
        .field(filter)
        .placeholder("Filter files")
        .focus_target(SIDEBAR_FILTER)
        .focused(focused)
        .search(true)
        .bare()
        .w_full()
        .h((m.ui_small_font_size * 1.5).round());
    let field = search_field(input, !filter.text().is_empty(), None, None, theme);
    let ring = focused.then(|| {
        let reach = tokens::scaled(Sz::FOCUS_RING_GAP, scale);
        let mut ring = div()
            .absolute()
            .left(-reach)
            .top(-reach)
            .right(-reach)
            .bottom(-reach)
            .rounded(m.control_radius + reach)
            .border(theme.colors.focus_border);
        ring.element_style_mut().border_widths = [tokens::scaled(Sz::FOCUS_RING_W, scale); 4];
        ring.into_any()
    });
    let body = if filter.text().is_empty() {
        empty_state("No files", "Open a repository to list its changes", theme)
    } else {
        empty_state("No matching files", "Try a different filter", theme)
    };
    view! {
        <div
            w={slot.width}
            h={slot.height}
            class="flex-col"
            bg={theme.colors.sidebar_background}
            accessibility_role={AccessibilityRole::Navigation}
            aria-label="Files"
            test_id="files"
        >
            <div class="w-full shrink-0" p={tokens::scaled(Sp::SM, scale)}>
                <div class="relative w-full">
                    {field}
                    {?ring}
                </div>
            </div>
            <div class="flex-1 w-full" min_h={0.0}>{body}</div>
        </div>
    }
}
