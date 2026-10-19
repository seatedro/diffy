use crate::core::theme_types::Rgba;
use crate::core::themes::SemanticPalette;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeColors {
    pub app_bg: Rgba,
    pub canvas: Rgba,
    pub panel: Rgba,
    pub panel_strong: Rgba,
    pub border_soft: Rgba,
    pub text_strong: Rgba,
    pub accent: Rgba,
    pub accent_strong: Rgba,
    pub selection_bg: Rgba,
    pub background: Rgba,
    pub surface: Rgba,
    pub editor_surface: Rgba,
    pub elevated_surface: Rgba,
    pub modal_surface: Rgba,
    pub overlay_scrim: Rgba,
    pub border: Rgba,
    pub border_variant: Rgba,
    pub focus_border: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub text_accent: Rgba,
    pub icon: Rgba,
    pub element_background: Rgba,
    pub element_hover: Rgba,
    pub element_active: Rgba,
    pub element_selected: Rgba,
    pub ghost_element_hover: Rgba,
    pub ghost_element_active: Rgba,
    pub ghost_element_selected: Rgba,
    pub title_bar_background: Rgba,
    pub status_bar_background: Rgba,
    pub sidebar_background: Rgba,
    pub sidebar_row_hover: Rgba,
    pub sidebar_row_selected: Rgba,
    pub empty_state_background: Rgba,
    pub empty_state_border: Rgba,
    pub scrollbar_thumb: Rgba,
    pub status_info: Rgba,
    pub status_warning: Rgba,
    pub status_error: Rgba,
    pub line_add: Rgba,
    pub line_del: Rgba,
    pub line_modified: Rgba,
    pub gutter_bg: Rgba,
    pub gutter_text: Rgba,
    pub file_header_bg: Rgba,
    pub hunk_header_bg: Rgba,
    pub line_add_text: Rgba,
    pub line_del_text: Rgba,
    pub line_add_word_bg: Rgba,
    pub line_del_word_bg: Rgba,
    pub hover_overlay: Rgba,
    pub search_match_bg: Rgba,
    pub search_match_active_bg: Rgba,
    pub syntax_keyword: Rgba,
    pub syntax_string: Rgba,
    pub syntax_comment: Rgba,
    pub syntax_function: Rgba,
    pub syntax_type: Rgba,
    pub syntax_number: Rgba,
    pub syntax_property: Rgba,
    pub syntax_operator: Rgba,
}

impl SemanticPalette {
    pub fn to_theme_colors(&self) -> ThemeColors {
        let s = self;
        let d = s.is_dark;

        let text = s.text_base.ensure_contrast(s.panel, 4.5, d);
        let text_strong = s.text_strong.ensure_contrast(s.panel_strong, 4.5, d);
        let text_muted = s.text_muted.ensure_contrast(s.panel, 3.0, d);
        let icon = s.text_base.ensure_contrast(s.panel, 4.5, d);

        // element_* are opaque tiers above panel (Radix step 3→4→5 pattern).
        // Ghost_* use alpha overlays so they work on any surface (sidebar rows,
        // toolbar slots) — the element renders as the parent bg + overlay.
        // Dark themes brighten with white, light themes darken with black.
        let overlay = if d {
            Rgba::new(255, 255, 255, 255)
        } else {
            Rgba::new(0, 0, 0, 255)
        };

        let element_hover = s.panel.lerp(overlay, if d { 0.12 } else { 0.08 });
        let element_active = s.panel.lerp(overlay, if d { 0.20 } else { 0.14 });
        let ghost_hover = overlay.with_alpha(if d { 26 } else { 22 });
        let ghost_active = overlay.with_alpha(if d { 32 } else { 40 });

        // Accent-tinted hover for "primary" slots (sidebar rows, selected
        // items). Alpha so it tints whatever surface it sits on.
        let accent_tint_hover = s.accent.with_alpha(if d { 38 } else { 48 });
        let accent_tint_selected = s.accent.with_alpha(if d { 66 } else { 76 });

        let c = |color: Rgba| color;
        ThemeColors {
            app_bg: c(s.app_bg),
            canvas: c(s.canvas),
            panel: c(s.panel),
            panel_strong: c(s.panel_strong),
            border_soft: c(s.border_soft),

            background: c(s.app_bg),
            surface: c(s.panel),
            editor_surface: c(s.canvas),
            elevated_surface: c(s.panel_strong),
            modal_surface: c(s.panel_strong),

            title_bar_background: c(s.panel_strong),
            status_bar_background: c(s.canvas),
            sidebar_background: c(s.canvas),
            empty_state_background: c(s.panel),
            gutter_bg: c(if d { s.app_bg } else { s.panel_strong }),
            file_header_bg: c(s.panel_strong),
            hunk_header_bg: c(s.panel_tint),

            element_background: c(s.panel),
            element_hover: c(element_hover),
            element_active: c(element_active),
            element_selected: c(accent_tint_selected),
            ghost_element_hover: c(ghost_hover),
            ghost_element_active: c(ghost_active),
            ghost_element_selected: c(accent_tint_selected),
            hover_overlay: c(accent_tint_hover),

            sidebar_row_hover: c(accent_tint_hover),
            sidebar_row_selected: c(accent_tint_selected),

            border: c(s.border_strong),
            border_variant: c(s.border_soft),
            focus_border: c(s.accent),
            empty_state_border: c(s.border_strong),

            text_strong: c(text_strong),
            text: c(text),
            text_muted: c(text_muted),
            text_accent: c(s.accent),
            icon: c(icon),
            gutter_text: c(s.text_faint),

            accent: c(s.accent),
            accent_strong: c(s.accent_strong),
            selection_bg: c(s.selection_bg),

            overlay_scrim: c(if d {
                Rgba::new(0, 0, 0, 180)
            } else {
                Rgba::new(0, 0, 0, 80)
            }),
            scrollbar_thumb: c(if d {
                Rgba::new(255, 255, 255, 100)
            } else {
                s.text_muted
            }),

            status_info: c(s.accent),
            status_warning: c(s.warning_text),
            status_error: c(s.danger_text),

            line_add: c(s.line_add),
            line_del: c(s.line_del),
            line_modified: c(s.app_bg.lerp(s.accent, 0.15)),
            line_add_text: c(s.success_text),
            line_del_text: c(s.danger_text),
            line_add_word_bg: c(s.line_add_accent),
            line_del_word_bg: c(s.line_del_accent),

            search_match_bg: c(s.warning_text.with_alpha(if d { 90 } else { 120 })),
            search_match_active_bg: c(s.warning_text.with_alpha(if d { 180 } else { 200 })),

            syntax_keyword: c(s.syn_keyword),
            syntax_string: c(s.syn_string),
            syntax_comment: c(s.syn_comment),
            syntax_function: c(s.syn_function),
            syntax_type: c(s.syn_type),
            syntax_number: c(s.syn_number),
            syntax_property: c(s.syn_property),
            syntax_operator: c(s.syn_operator),
        }
    }
}
