use crate::render::Rect;

/// A child in a stack layout: either a fixed size or a flex proportion.
#[derive(Debug, Clone, Copy)]
pub enum Child {
    /// Fixed size in pixels along the stack axis.
    Fixed(f32),
    /// Flex weight — shares remaining space proportionally.
    Flex(f32),
}

/// Shorthand for `Child::Fixed`.
pub use Child::Fixed as Fx;
/// Shorthand for `Child::Flex`.
pub use Child::Flex as Fl;

/// Lay out children vertically within `bounds`.
///
/// Returns one `Rect` per child, positioned absolutely.
/// Fixed children get their exact height; flex children share the remaining space.
pub fn vstack<const N: usize>(bounds: Rect, gap: f32, children: [Child; N]) -> [Rect; N] {
    let total_gap = gap * N.saturating_sub(1) as f32;
    let (fixed_sum, flex_sum) = sum_sizes(&children);
    let remaining = (bounds.height - fixed_sum - total_gap).max(0.0);

    let mut y = bounds.y;
    let mut rects = [Rect::default(); N];
    for (i, child) in children.iter().enumerate() {
        let h = resolve(*child, remaining, flex_sum);
        rects[i] = Rect {
            x: bounds.x,
            y,
            width: bounds.width,
            height: h,
        };
        y += h + gap;
    }
    rects
}

/// Lay out children horizontally within `bounds`.
///
/// Returns one `Rect` per child, positioned absolutely.
/// Fixed children get their exact width; flex children share the remaining space.
pub fn hstack<const N: usize>(bounds: Rect, gap: f32, children: [Child; N]) -> [Rect; N] {
    let total_gap = gap * N.saturating_sub(1) as f32;
    let (fixed_sum, flex_sum) = sum_sizes(&children);
    let remaining = (bounds.width - fixed_sum - total_gap).max(0.0);

    let mut x = bounds.x;
    let mut rects = [Rect::default(); N];
    for (i, child) in children.iter().enumerate() {
        let w = resolve(*child, remaining, flex_sum);
        rects[i] = Rect {
            x,
            y: bounds.y,
            width: w,
            height: bounds.height,
        };
        x += w + gap;
    }
    rects
}

/// Right-align a fixed-size child within a row rect.
pub fn right_align(row: Rect, child_width: f32, child_height: f32) -> Rect {
    Rect {
        x: row.right() - child_width,
        y: row.y + (row.height - child_height).max(0.0) * 0.5,
        width: child_width,
        height: child_height,
    }
}

fn sum_sizes(children: &[Child]) -> (f32, f32) {
    let mut fixed = 0.0_f32;
    let mut flex = 0.0_f32;
    for child in children {
        match child {
            Child::Fixed(v) => fixed += v,
            Child::Flex(v) => flex += v,
        }
    }
    (fixed, flex)
}

fn resolve(child: Child, remaining: f32, flex_sum: f32) -> f32 {
    match child {
        Child::Fixed(v) => v,
        Child::Flex(f) => {
            if flex_sum > 0.0 {
                remaining * f / flex_sum
            } else {
                0.0
            }
        }
    }
}
