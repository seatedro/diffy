use diffy_ui::DiffKey;

use crate::contracts::viewport::interaction::InteractionAction;
use crate::core::compare::LayoutMode;

pub const DIFF: &[(&str, DiffKey)] = &[
    ("]", DiffKey::NextHunk),
    ("[", DiffKey::PrevHunk),
    ("alt+arrowdown", DiffKey::NextHunk),
    ("alt+arrowup", DiffKey::PrevHunk),
    ("n", DiffKey::NextFile),
    ("shift+n", DiffKey::PrevFile),
    ("j", DiffKey::LineDown),
    ("k", DiffKey::LineUp),
    ("arrowdown", DiffKey::LineDown),
    ("arrowup", DiffKey::LineUp),
    ("space", DiffKey::PageDown),
    ("shift+space", DiffKey::PageUp),
    ("pagedown", DiffKey::PageDown),
    ("pageup", DiffKey::PageUp),
    ("home", DiffKey::Home),
    ("end", DiffKey::End),
    ("shift+g", DiffKey::End),
    ("mod+c", DiffKey::Copy),
    ("mod+a", DiffKey::SelectAll),
];

pub fn viewport() -> [(&'static str, InteractionAction); 6] {
    [
        ("mod+f", InteractionAction::OpenSearch),
        ("w", InteractionAction::ToggleWrap),
        ("d", InteractionAction::ScrollHalfPage(1)),
        ("u", InteractionAction::ScrollHalfPage(-1)),
        ("1", InteractionAction::SetLayout(LayoutMode::Unified)),
        ("2", InteractionAction::SetLayout(LayoutMode::Split)),
    ]
}

pub fn search() -> [(&'static str, InteractionAction); 4] {
    [
        ("enter", InteractionAction::SearchNext),
        ("shift+enter", InteractionAction::SearchPrevious),
        ("mod+g", InteractionAction::SearchNext),
        ("escape", InteractionAction::CloseSearch),
    ]
}
