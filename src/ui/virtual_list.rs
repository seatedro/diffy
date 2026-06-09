use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VirtualListWindow {
    pub range: Range<usize>,
    pub top_spacer: f32,
    pub bottom_spacer: f32,
    pub total_extent: f32,
}

pub(crate) fn virtual_list_total_extent(item_count: usize, item_extent: f32, item_gap: f32) -> f32 {
    if item_count == 0 {
        return 0.0;
    }

    let item_extent = item_extent.max(0.0);
    let item_gap = item_gap.max(0.0);
    item_count as f32 * (item_extent + item_gap) - item_gap
}

/// Extent reserved by the wrapper around one windowed row: every row keeps
/// its full stride (row + gap) except the last, which drops the trailing gap
/// so the column height matches `virtual_list_total_extent`.
pub(crate) fn virtual_row_wrapper_extent(
    global_index: usize,
    total_rows: usize,
    row_extent: f32,
    stride: f32,
) -> f32 {
    if global_index + 1 == total_rows {
        row_extent
    } else {
        stride
    }
}

/// Build a flat row list from filtered item indices, inserting a section
/// header row whenever the section key changes between consecutive items.
/// Indices whose item fails to resolve are skipped without affecting the
/// current section.
pub(crate) fn build_sectioned_rows<R, S: PartialEq>(
    filtered_indices: &[usize],
    mut section_of: impl FnMut(usize) -> Option<S>,
    mut section_row: impl FnMut(&S) -> R,
    mut item_row: impl FnMut(usize) -> Option<R>,
) -> Vec<R> {
    let mut rows = Vec::with_capacity(filtered_indices.len());
    let mut last_section: Option<S> = None;

    for &index in filtered_indices {
        let Some(row) = item_row(index) else {
            continue;
        };
        let section = section_of(index);
        if section != last_section {
            if let Some(section) = &section {
                rows.push(section_row(section));
            }
            last_section = section;
        }
        rows.push(row);
    }

    rows
}

/// Step a list selection by `delta` rows, clamping to bounds and skipping
/// section-header rows in the direction of travel. Returns `None` when the
/// list is empty.
pub(crate) fn step_selection(
    current: usize,
    delta: i32,
    len: usize,
    mut is_header: impl FnMut(usize) -> bool,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let max = len.saturating_sub(1) as i32;
    let mut idx = (current as i32 + delta).clamp(0, max) as usize;
    while idx < len && is_header(idx) {
        if delta > 0 {
            let next = (idx + 1).min(len.saturating_sub(1));
            if next == idx {
                break;
            }
            idx = next;
        } else {
            if idx == 0 {
                break;
            }
            idx -= 1;
        }
    }
    Some(idx)
}

pub(crate) fn virtual_list_window(
    item_count: usize,
    scroll_offset: f32,
    viewport_extent: f32,
    item_extent: f32,
    item_gap: f32,
    overscan_items: usize,
) -> VirtualListWindow {
    let total_extent = virtual_list_total_extent(item_count, item_extent, item_gap);
    let item_extent = item_extent.max(0.0);
    let item_gap = item_gap.max(0.0);
    let stride = item_extent + item_gap;
    if item_count == 0 || stride <= 0.0 {
        return VirtualListWindow {
            range: 0..0,
            top_spacer: 0.0,
            bottom_spacer: 0.0,
            total_extent,
        };
    }

    let first = (scroll_offset.max(0.0) / stride).floor().max(0.0) as usize;
    let first = first.min(item_count);
    let visible = (viewport_extent.max(0.0) / stride).ceil().max(1.0) as usize;
    let start = first.saturating_sub(overscan_items).min(item_count);
    let end = first
        .saturating_add(visible)
        .saturating_add(overscan_items)
        .min(item_count)
        .max(start);
    let range = start..end;
    let top_spacer = range.start as f32 * stride;
    let remaining = item_count.saturating_sub(range.end);
    let bottom_spacer = if remaining == 0 {
        0.0
    } else {
        remaining as f32 * stride - item_gap
    };

    VirtualListWindow {
        range,
        top_spacer,
        bottom_spacer,
        total_extent,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_sectioned_rows, step_selection, virtual_list_total_extent, virtual_list_window,
        virtual_row_wrapper_extent,
    };

    #[test]
    fn virtual_list_window_overscans_and_clamps() {
        let window = virtual_list_window(100, 120.0, 80.0, 40.0, 0.0, 8);
        assert_eq!(window.range, 0..13);

        let near_end = virtual_list_window(100, 3_760.0, 80.0, 40.0, 0.0, 8);
        assert_eq!(near_end.range, 86..100);
    }

    #[test]
    fn virtual_list_spacers_preserve_total_extent() {
        let window = virtual_list_window(30, 400.0, 200.0, 36.0, 4.0, 0);

        assert_eq!(window.range, 10..15);
        assert_eq!(window.top_spacer, 400.0);
        assert_eq!(window.bottom_spacer, 596.0);
        assert_eq!(window.total_extent, 1_196.0);
    }

    #[test]
    fn virtual_list_total_extent_has_no_trailing_gap() {
        assert_eq!(virtual_list_total_extent(0, 36.0, 4.0), 0.0);
        assert_eq!(virtual_list_total_extent(1, 36.0, 4.0), 36.0);
        assert_eq!(virtual_list_total_extent(3, 36.0, 4.0), 116.0);
    }

    #[test]
    fn last_row_wrapper_drops_trailing_gap() {
        assert_eq!(virtual_row_wrapper_extent(0, 3, 36.0, 40.0), 40.0);
        assert_eq!(virtual_row_wrapper_extent(2, 3, 36.0, 40.0), 36.0);
    }

    #[test]
    fn sectioned_rows_insert_headers_and_skip_missing_items() {
        let sections = [Some(1_u8), Some(1), None, Some(2)];
        let rows = build_sectioned_rows(
            &[0, 1, 2, 3],
            |index| sections[index],
            |section| format!("section {section}"),
            |index| (index != 2).then(|| format!("item {index}")),
        );

        assert_eq!(
            rows,
            ["section 1", "item 0", "item 1", "section 2", "item 3"]
        );
    }

    #[test]
    fn step_selection_clamps_and_skips_headers() {
        let headers = [true, false, false, true, false];
        let is_header = |i: usize| headers[i];

        assert_eq!(step_selection(0, 1, 0, is_header), None);
        assert_eq!(step_selection(2, 1, headers.len(), is_header), Some(4));
        assert_eq!(step_selection(4, -1, headers.len(), is_header), Some(2));
        assert_eq!(step_selection(1, -1, headers.len(), is_header), Some(0));
        assert_eq!(step_selection(4, 10, headers.len(), is_header), Some(4));
    }
}
