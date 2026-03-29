use crate::core::compare::LayoutMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffViewportState {
    pub layout: LayoutMode,
    pub wrap_enabled: bool,
    pub wrap_column: u32,
    pub scroll_top_px: u32,
    pub content_height_px: u32,
    pub viewport_width_px: u32,
    pub viewport_height_px: u32,
    pub hovered_row: Option<usize>,
    pub visible_row_start: Option<usize>,
    pub visible_row_end: Option<usize>,
    pub focused: bool,
}

impl Default for DiffViewportState {
    fn default() -> Self {
        Self {
            layout: LayoutMode::Unified,
            wrap_enabled: false,
            wrap_column: 0,
            scroll_top_px: 0,
            content_height_px: 0,
            viewport_width_px: 0,
            viewport_height_px: 0,
            hovered_row: None,
            visible_row_start: None,
            visible_row_end: None,
            focused: false,
        }
    }
}

impl DiffViewportState {
    pub fn clear_document(&mut self) {
        self.scroll_top_px = 0;
        self.content_height_px = 0;
        self.hovered_row = None;
        self.visible_row_start = None;
        self.visible_row_end = None;
    }

    pub fn max_scroll_top_px(&self) -> u32 {
        self.content_height_px
            .saturating_sub(self.viewport_height_px.max(1))
    }

    pub fn clamp_scroll(&mut self) {
        self.scroll_top_px = self.scroll_top_px.min(self.max_scroll_top_px());
    }
}
