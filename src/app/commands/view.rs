use quark_app::ViewContext;
use quark_ui::element::AnyElement;

use super::CommandsController;
use crate::app::shell::Slot;

pub fn overlays(
    _commands: &mut CommandsController,
    _slot: &Slot,
    _cx: &mut ViewContext,
) -> Option<AnyElement> {
    None
}

pub fn toasts(
    _commands: &mut CommandsController,
    _slot: &Slot,
    _cx: &mut ViewContext,
) -> Option<AnyElement> {
    None
}
