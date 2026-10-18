use quark_ui::text_input::{Editor, EditorMode, TextEditCommand, TextEditOutcome};

use crate::contracts::review::{CardKey, ComposerFormat};
use crate::contracts::shared::SourceTarget;
use crate::core::review::{ReviewDraftId, ReviewThreadId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerTarget {
    Inline {
        path: String,
        target: SourceTarget,
    },
    General,
    Draft(ReviewDraftId),
    Reply {
        thread: ReviewThreadId,
        draft: ReviewDraftId,
    },
    Edit {
        thread: ReviewThreadId,
        comment_node_id: String,
    },
}

impl ComposerTarget {
    pub fn draft(&self) -> Option<ReviewDraftId> {
        match self {
            Self::Draft(id) | Self::Reply { draft: id, .. } => Some(*id),
            Self::Inline { .. } | Self::General | Self::Edit { .. } => None,
        }
    }

    pub fn card(&self) -> CardKey {
        match self {
            Self::Inline { .. } | Self::General => CardKey::NewComment,
            Self::Draft(id) => CardKey::Draft(*id),
            Self::Reply { thread, .. } | Self::Edit { thread, .. } => {
                CardKey::Thread(thread.clone())
            }
        }
    }

    pub fn submit_label(&self) -> &'static str {
        match self {
            Self::Inline { .. } | Self::General => "Add review comment",
            Self::Draft(_) => "Update comment",
            Self::Reply { .. } => "Reply",
            Self::Edit { .. } => "Save",
        }
    }

    pub fn test_id(&self) -> String {
        match self {
            Self::Inline { .. } | Self::General => "review.composer.new".to_owned(),
            Self::Draft(id) | Self::Reply { draft: id, .. } => {
                format!("review.composer.draft.{}", id.0)
            }
            Self::Edit {
                comment_node_id, ..
            } => format!("review.composer.edit.{comment_node_id}"),
        }
    }
}

pub fn format_insertion(format: ComposerFormat, selected: &str) -> String {
    let pick = |fallback: &'static str| {
        if selected.is_empty() {
            fallback
        } else {
            selected
        }
    };
    match format {
        ComposerFormat::Bold => format!("**{}**", pick("bold")),
        ComposerFormat::Italic => format!("*{}*", pick("italic")),
        ComposerFormat::Code => format!("`{}`", pick("code")),
        ComposerFormat::Link => format!("[{}](url)", pick("text")),
        ComposerFormat::BulletList => format!("\n- {selected}"),
    }
}

pub struct ComposerEditor {
    pub editor: Editor,
    pub height: f32,
    pub focused: bool,
}

impl Default for ComposerEditor {
    fn default() -> Self {
        Self {
            editor: Editor::new(EditorMode::ProseInput),
            height: 0.0,
            focused: false,
        }
    }
}

impl ComposerEditor {
    pub fn with_text(text: &str) -> Self {
        let mut composer = Self::default();
        composer.editor.set_text(text);
        composer
            .editor
            .apply(TextEditCommand::SetTextCursor(text.len()));
        composer
    }

    pub fn format(&mut self, format: ComposerFormat) -> TextEditOutcome {
        let insertion = format_insertion(format, self.editor.selected_text().unwrap_or_default());
        self.editor.apply(TextEditCommand::InsertText(insertion))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_wraps_the_selection_or_inserts_a_placeholder() {
        assert_eq!(format_insertion(ComposerFormat::Bold, "x"), "**x**");
        assert_eq!(format_insertion(ComposerFormat::Italic, ""), "*italic*");
        assert_eq!(format_insertion(ComposerFormat::Code, "a.b"), "`a.b`");
        assert_eq!(format_insertion(ComposerFormat::Link, ""), "[text](url)");
        assert_eq!(
            format_insertion(ComposerFormat::BulletList, "item"),
            "\n- item"
        );
    }

    #[test]
    fn formatting_replaces_the_editor_selection() {
        let mut composer = ComposerEditor::with_text("make it fast");
        composer.editor.apply(TextEditCommand::SetTextCursor(8));
        composer
            .editor
            .apply(TextEditCommand::ExtendTextSelection(12));

        let outcome = composer.format(ComposerFormat::Bold);

        assert!(outcome.text_changed);
        assert_eq!(composer.editor.text(), "make it **fast**");
    }
}
