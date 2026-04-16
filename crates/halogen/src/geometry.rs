//! Pure geometry primitives used by halogen hit-testing and scene emission.

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x <= self.right() && y <= self.bottom()
    }

    pub fn right(self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    pub fn inset(self, amount: f32) -> Self {
        Self {
            x: self.x + amount,
            y: self.y + amount,
            width: (self.width - amount * 2.0).max(0.0),
            height: (self.height - amount * 2.0).max(0.0),
        }
    }

    pub fn pad(self, left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            x: self.x + left,
            y: self.y + top,
            width: (self.width - left - right).max(0.0),
            height: (self.height - top - bottom).max(0.0),
        }
    }

    pub fn center(self, child_w: f32, child_h: f32) -> Self {
        Self {
            x: self.x + ((self.width - child_w).max(0.0) * 0.5),
            y: self.y + ((self.height - child_h).max(0.0) * 0.5),
            width: child_w.min(self.width - 24.0),
            height: child_h.min(self.height - 24.0),
        }
    }

    pub fn offset(self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..self
        }
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        let width = right - left;
        let height = bottom - top;
        if width <= 0.0 || height <= 0.0 {
            None
        } else {
            Some(Self {
                x: left,
                y: top,
                width,
                height,
            })
        }
    }
}
