use crate::core::compare::LayoutMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffViewportState {
    pub layout: LayoutMode,
    pub wrap_enabled: bool,
    pub wrap_column: u32,
    pub hovered_row: Option<usize>,
    pub selected_rows: Option<(usize, usize)>,
}

impl Default for DiffViewportState {
    fn default() -> Self {
        Self {
            layout: LayoutMode::Unified,
            wrap_enabled: false,
            wrap_column: 0,
            hovered_row: None,
            selected_rows: None,
        }
    }
}
