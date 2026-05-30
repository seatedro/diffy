use crate::render::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorOverlayKind {
    ReviewAddButton { line_index: usize, emphasised: bool },
    ReviewComposerBlock { block_index: usize },
    ReviewThreadBlock { block_index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedEditorOverlay {
    pub kind: EditorOverlayKind,
    pub rect: Rect,
    pub clip: Rect,
}

impl ResolvedEditorOverlay {
    pub fn new(kind: EditorOverlayKind, rect: Rect, clip: Rect) -> Option<Self> {
        Some(Self {
            kind,
            rect,
            clip: rect.intersection(clip)?,
        })
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.rect.contains(x, y) && self.clip.contains(x, y)
    }
}
