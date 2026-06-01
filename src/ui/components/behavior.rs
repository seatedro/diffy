use crate::actions::Action;
use crate::ui::element::Div;
use crate::ui::state::FocusTarget;
use crate::ui::style::Styled;

use super::Button;

pub trait Clickable: Sized {
    fn on_click_action(self, action: Action) -> Self;
}

pub trait Disableable: Sized {
    fn disabled(self, disabled: bool) -> Self;
}

pub trait Focusable: Sized {
    fn track_focus(self, target: FocusTarget) -> Self;
}

pub trait Tooltipable: Sized {
    fn tooltip(self, text: impl Into<String>) -> Self;
}

pub trait Selectable: Sized {
    fn selected(self, selected: bool) -> Self;
}

pub trait Layered: Sized {
    fn layer(self, z_index: i32) -> Self;
}

pub trait ButtonLike: Clickable + Disableable + Focusable + Tooltipable {}
pub trait InputLike: Disableable + Focusable + Tooltipable {}
pub trait MenuLike: Focusable + Layered {}
pub trait ListLike: Focusable + Selectable {}
pub trait TreeLike: Focusable + Selectable {}
pub trait PopoverLike: Focusable + Layered {}

impl Clickable for Div {
    fn on_click_action(self, action: Action) -> Self {
        self.on_click(action)
    }
}

impl Focusable for Div {
    fn track_focus(self, target: FocusTarget) -> Self {
        Div::track_focus(self, target)
    }
}

impl Tooltipable for Div {
    fn tooltip(self, text: impl Into<String>) -> Self {
        Div::tooltip(self, text)
    }
}

impl Disableable for Div {
    fn disabled(self, disabled: bool) -> Self {
        self.accessibility_disabled(disabled)
    }
}

impl Selectable for Div {
    fn selected(self, selected: bool) -> Self {
        self.accessibility_selected(selected)
    }
}

impl Layered for Div {
    fn layer(self, z_index: i32) -> Self {
        self.z_index(z_index)
    }
}

impl Disableable for Button {
    fn disabled(self, disabled: bool) -> Self {
        self.disabled(disabled)
    }
}

impl Tooltipable for Button {
    fn tooltip(self, text: impl Into<String>) -> Self {
        self.tooltip(text)
    }
}

impl Selectable for Button {
    fn selected(self, selected: bool) -> Self {
        self.active(selected)
    }
}

impl ButtonLike for Div {}
impl InputLike for Div {}
impl MenuLike for Div {}
impl ListLike for Div {}
impl TreeLike for Div {}
impl PopoverLike for Div {}
