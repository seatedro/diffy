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
    use super::{virtual_list_total_extent, virtual_list_window};

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
}
