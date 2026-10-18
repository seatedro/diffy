use std::fmt;
use std::sync::Arc;

use quark_ui::text_input::{Editor, EditorMode, TextEditCommand, TextEditOutcome};

use crate::contracts::shared::CompareGeneration;
use crate::contracts::viewport::interaction::{
    TextCompareLanguage, TextCompareSide, TextCompareView,
};
use crate::contracts::viewport::pipeline::{PipelineEffect, TextCompareRequest};
use crate::core::compare::{LayoutMode, RendererKind};

pub struct TextCompare {
    pub active: bool,
    pub view: TextCompareView,
    pub left: Editor,
    pub right: Editor,
    pub language: TextCompareLanguage,
    pub language_menu: bool,
    pub generation: CompareGeneration,
    pub pending: bool,
}

impl Default for TextCompare {
    fn default() -> Self {
        Self {
            active: false,
            view: TextCompareView::Edit,
            left: Editor::new(EditorMode::CodeInput),
            right: Editor::new(EditorMode::CodeInput),
            language: TextCompareLanguage::Auto,
            language_menu: false,
            generation: CompareGeneration(0),
            pending: false,
        }
    }
}

impl fmt::Debug for TextCompare {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextCompare")
            .field("active", &self.active)
            .field("view", &self.view)
            .field("language", &self.language)
            .field("generation", &self.generation)
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

impl TextCompare {
    pub fn editor_mut(&mut self, side: TextCompareSide) -> &mut Editor {
        match side {
            TextCompareSide::Left => &mut self.left,
            TextCompareSide::Right => &mut self.right,
        }
    }

    pub fn edit(
        &mut self,
        side: TextCompareSide,
        command: TextEditCommand,
        now_ms: u64,
    ) -> TextEditOutcome {
        let outcome = self.editor_mut(side).apply_at(command, now_ms);
        if outcome.text_changed {
            self.invalidate();
        }
        outcome
    }

    pub fn swap(&mut self) {
        std::mem::swap(&mut self.left, &mut self.right);
        self.invalidate();
    }

    pub fn clear(&mut self, side: TextCompareSide) {
        self.editor_mut(side).set_text("");
        self.invalidate();
    }

    pub fn set_language(&mut self, language: TextCompareLanguage) -> bool {
        self.language_menu = false;
        if self.language == language {
            return false;
        }
        self.language = language;
        self.invalidate();
        true
    }

    pub fn invalidate(&mut self) {
        self.generation = CompareGeneration(self.generation.0 + 1);
        self.pending = false;
    }

    pub fn request(&mut self, floor: CompareGeneration, layout: LayoutMode) -> PipelineEffect {
        self.generation = CompareGeneration(self.generation.0.max(floor.0) + 1);
        self.pending = true;
        PipelineEffect::RunTextCompare(Box::new(TextCompareRequest {
            generation: self.generation,
            left_text: Arc::from(self.left.text()),
            right_text: Arc::from(self.right.text()),
            display_path: self.language.scratch_path().to_owned(),
            renderer: RendererKind::Builtin,
            layout,
        }))
    }
}
