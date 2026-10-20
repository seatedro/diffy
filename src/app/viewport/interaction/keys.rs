use diffy_ui::DiffKey;

use crate::contracts::viewport::interaction::InteractionAction;

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

pub fn search() -> [(&'static str, InteractionAction); 4] {
    [
        ("enter", InteractionAction::SearchNext),
        ("shift+enter", InteractionAction::SearchPrevious),
        ("mod+g", InteractionAction::SearchNext),
        ("escape", InteractionAction::CloseSearch),
    ]
}
