use std::sync::Arc;

use quark_ui::text_input::TextField;

use crate::contracts::commands::RefPickerRequest;
use crate::contracts::navigation::CompareField;
use crate::contracts::viewport::pipeline::DiffSize;
use crate::core::ref_catalog::{RefCatalog, RefEntry, RefEntryKind, RefResults, RefRow};

pub const REF_PICKER_VISIBLE_ROWS: usize = 11;

#[derive(Debug)]
pub struct RefPicker {
    pub request: RefPickerRequest,
    pub query: TextField,
    pub catalog: Option<Arc<RefCatalog>>,
    pub results: Arc<RefResults>,
    pub catalog_generation: u64,
    pub filter_generation: u64,
    pub selected: Option<usize>,
    pub scroll_px: f32,
    pub pending_reveal: bool,
    pub size_token: u64,
    pub size: Option<DiffSize>,
    pub now: i64,
    pub query_dirty: bool,
}

impl RefPicker {
    pub fn new(request: RefPickerRequest, now: i64) -> Self {
        Self {
            request,
            query: TextField::new(""),
            catalog: None,
            results: Arc::default(),
            catalog_generation: 0,
            filter_generation: 0,
            selected: None,
            scroll_px: 0.0,
            pending_reveal: true,
            size_token: 0,
            size: None,
            now,
            query_dirty: false,
        }
    }

    pub fn side(&self) -> CompareField {
        self.request.side
    }

    pub fn current_name(&self) -> &str {
        match self.request.side {
            CompareField::Left => &self.request.left,
            CompareField::Right => &self.request.right,
        }
    }

    pub fn other_name(&self) -> &str {
        match self.request.side {
            CompareField::Left => &self.request.right,
            CompareField::Right => &self.request.left,
        }
    }

    pub fn entry(&self, row: usize) -> Option<&RefEntry> {
        let catalog = self.catalog.as_ref()?;
        match self.results.rows.get(row)? {
            RefRow::Entry { index, .. } => catalog.entries.get(*index as usize),
            RefRow::Section { .. } => None,
        }
    }

    pub fn selected_entry(&self) -> Option<&RefEntry> {
        self.entry(self.selected?)
    }

    pub fn set_results(&mut self, results: Arc<RefResults>) {
        self.results = results;
        let current = self.current_name().to_owned();
        let query_empty = self.query.text().trim().is_empty();
        let first = (0..self.results.rows.len()).find(|&row| self.entry(row).is_some());
        let current_row = query_empty
            .then(|| {
                (0..self.results.rows.len())
                    .find(|&row| self.entry(row).is_some_and(|e| e.name == current))
            })
            .flatten();
        self.selected = current_row.or(first);
        self.scroll_px = 0.0;
        self.pending_reveal = true;
        self.size = None;
    }

    pub fn move_selection(&mut self, delta: i32, row_stride: f32) {
        let rows = self.results.rows.len();
        if rows == 0 {
            return;
        }
        let mut row = self.selected.unwrap_or(0) as i32;
        let step = delta.signum();
        let mut remaining = delta.abs();
        let mut last_entry = self.selected;
        while remaining > 0 {
            row += step;
            if row < 0 || row >= rows as i32 {
                break;
            }
            if self.entry(row as usize).is_some() {
                last_entry = Some(row as usize);
                remaining -= 1;
            }
        }
        if last_entry != self.selected {
            self.selected = last_entry;
            self.size = None;
            self.reveal(row_stride);
        }
    }

    pub fn reveal_pending(&mut self, row_stride: f32) {
        if self.pending_reveal && row_stride > 0.0 {
            self.pending_reveal = false;
            self.reveal(row_stride);
        }
    }

    fn reveal(&mut self, row_stride: f32) {
        let Some(selected) = self.selected else {
            return;
        };
        let top = selected.saturating_sub(1) as f32 * row_stride;
        let bottom = (selected + 1) as f32 * row_stride;
        let viewport = REF_PICKER_VISIBLE_ROWS as f32 * row_stride;
        if top < self.scroll_px {
            self.scroll_px = top;
        } else if bottom > self.scroll_px + viewport {
            self.scroll_px = bottom - viewport;
        }
    }

    pub fn scroll(&mut self, lines: i32, row_stride: f32) {
        let max = self
            .results
            .rows
            .len()
            .saturating_sub(REF_PICKER_VISIBLE_ROWS) as f32
            * row_stride;
        self.scroll_px = (self.scroll_px + lines as f32 * row_stride).clamp(0.0, max.max(0.0));
    }

    pub fn switch_side(&mut self, side: CompareField) {
        if self.request.side == side {
            return;
        }
        self.request.side = side;
        let results = self.results.clone();
        self.set_results(results);
    }
}

pub fn is_working_tree(entry: &RefEntry) -> bool {
    entry.kind == RefEntryKind::WorkingTree
}
