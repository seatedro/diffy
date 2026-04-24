//! Shimmer-backed file-list skeleton, shown in the sidebar while a
//! compare is running and the real file list is still empty. Gives the
//! user a preview of the destination UI so the wait feels structured
//! rather than blank.
//!
//! Widths are deterministic (fixed table) to avoid layout thrash between
//! frames. The animation is GPU-side via halogen's shimmer effect — no
//! per-frame CPU work.

use halogen::view;

use crate::ui::design::{Rad, Sp};
use crate::ui::element::*;
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme};

/// Widths for 14 skeleton rows. Pseudo-random lengths so the placeholder
/// reads as "files", not a ladder. Units are design pixels; the panel
/// clamps to the sidebar width, so short values yield short shimmer bars.
const ROW_WIDTHS: [f32; 14] = [
    148.0, 92.0, 176.0, 120.0, 64.0, 210.0, 136.0, 104.0, 180.0, 88.0, 156.0, 128.0, 72.0, 200.0,
];

pub fn sidebar_skeleton(theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let base = tc.element_background;
    let highlight = tc.ghost_element_hover;

    view! { scale,
        <div class="flex-col w-full" p={Sp::MD} gap={Sp::MD}>
            for w in ROW_WIDTHS.iter().copied() {
                <div w={w} h={12.0}
                     rounded={Rad::SM}
                     bg={Color::TRANSPARENT}
                     bg_effect={crate::ui::element::shimmer(base, highlight, 1.0)} />
            }
        </div>
    }
}
