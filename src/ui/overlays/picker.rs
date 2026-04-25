use halogen::view;

use crate::actions::Action;
use crate::ui::components::picker::picker_list;
use crate::ui::design::{Rad, Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::state::{AppState, FocusTarget, PickerItem};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

#[derive(Default, Clone, Copy)]
pub struct PickerLayout {
    /// Vertical offset of the panel from the top of the window (design units).
    /// Also where the scrim begins — keeps the area above interactive.
    pub top_offset: Option<f32>,
    /// Corner radius for the panel in design units. Default: Rad::XXL.
    pub panel_radius: Option<f32>,
}

#[allow(clippy::too_many_arguments)]
pub fn picker<T: PickerItem>(
    query: &str,
    placeholder: &str,
    entries: &[T],
    selected_index: usize,
    scroll_top_px: f32,
    panel_width_class: f32,
    focus_target: FocusTarget,
    state: &AppState,
    theme: &Theme,
    width: f32,
    height: f32,
) -> AnyElement {
    picker_with_header(
        query,
        placeholder,
        entries,
        selected_index,
        scroll_top_px,
        panel_width_class,
        focus_target,
        state,
        theme,
        width,
        height,
        None,
        PickerLayout::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn picker_with_header<T: PickerItem>(
    query: &str,
    placeholder: &str,
    entries: &[T],
    selected_index: usize,
    scroll_top_px: f32,
    panel_width_class: f32,
    focus_target: FocusTarget,
    state: &AppState,
    theme: &Theme,
    width: f32,
    height: f32,
    header: Option<AnyElement>,
    layout: PickerLayout,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let panel_width = (panel_width_class * scale).min(width - (Sz::MODAL_MARGIN * scale).round());
    let has_header = header.is_some();
    let header_el = header.unwrap_or_else(|| view! { scale, <div /> });
    let pt = layout.top_offset.unwrap_or(Sz::MODAL_TOP_OFFSET);
    let radius = layout.panel_radius.unwrap_or(Rad::XXL);
    // When a caller opts into flush-with-title-bar positioning (top_offset
    // set), they also want the panel to visually flow out of the title bar —
    // so the scrim starts below the title bar and the panel drops its top
    // border.
    let flush = layout.top_offset.is_some();
    let scrim_top_design = layout.top_offset.unwrap_or(0.0);
    let scrim_top_px = (scrim_top_design * scale).round();
    let scrim_h_px = (height - scrim_top_px).max(0.0);
    let pt_in_scrim = if flush { 0.0 } else { pt };

    view! { scale,
        <div class="absolute flex-col items-center" top={scrim_top_px} left={0.0}
             w={width} h={scrim_h_px} z_index={100}
             bg={tc.overlay_scrim}
             on_click={crate::actions::OverlayAction::CloseOverlay.into()}
             hit_identity={HitIdentity::OverlayBackdrop}
             pt={pt_in_scrim}>
            <div class="flex-col overflow-hidden"
                 w={panel_width}
                 bg={tc.elevated_surface}
                 rounded={radius}
                 border_l={tc.border}
                 border_r={tc.border}
                 border_b={tc.border}
                 @when { !flush } { border_t={tc.border} }
                 shadow_preset={Shadow::MODAL}
                 on_click={Action::Noop}>
                if has_header {
                    <div class="w-full" px={Sp::MD} py={Sp::SM}>
                        {header_el}
                    </div>
                    <div class="w-full" h={Sz::SEPARATOR_W} bg={tc.border_variant} />
                }
                <div class="w-full" px={Sp::MD}>
                    {text_input("", query)
                        .placeholder(placeholder)
                        .focused(state.focus.get(&state.store) == Some(focus_target))
                        .on_click(
                            crate::actions::AppAction::SetFocus(Some(focus_target)).into(),
                        )
                        .cursor(state.text_edit.cursor.get(&state.store))
                        .anchor(state.text_edit.anchor.get(&state.store))
                        .cursor_moved_at(state.text_edit.cursor_moved_at_ms.get(&state.store))
                        .focus_target(focus_target)
                        .bare()
                        .w_full()
                        .h(theme.metrics.ui_row_height.round())}
                </div>
                <div class="w-full" h={Sz::SEPARATOR_W} bg={tc.border_variant} />
                <div p={Sp::XS}>
                    {picker_list(entries, selected_index, scroll_top_px, Sz::PICKER_MAX_ROWS, theme)}
                </div>
            </div>
        </div>
    }
}
