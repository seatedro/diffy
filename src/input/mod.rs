mod keyboard;
mod keymap;
mod pointer;
mod scroll;

use std::sync::Arc;
use std::time::Instant;

use winit::event::{
    ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::Window;

use crate::actions::Action;
use crate::effects::Effect;
use crate::render::Renderer;
use crate::ui::components::TooltipState;
use crate::ui::editor::element::EditorElement;
use crate::ui::element::DragHandler;
use crate::ui::shell::UiFrame;
use crate::ui::state::{AppState, FocusTarget, OverlaySurface, WorkspaceMode};

pub use keymap::{
    KeymapOverride, ShortcutCommand, ShortcutEntry, ShortcutGroup, active_bindings,
    binding_conflict, binding_matches, format_binding, override_for, reset_override, set_override,
    shortcut_entries, shortcut_entry, shortcut_groups,
};
pub use pointer::hit_test_text_offset;
pub use scroll::{quantize_scroll_delta_px, scroll_delta_to_px};

#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    TextInput(String),
    KeyPress(KeyChord),
    KeyRelease(KeyChord),
    PointerMoved {
        x: f32,
        y: f32,
    },
    PointerButton {
        button: MouseButton,
        state: ElementState,
    },
    Wheel {
        delta: MouseScrollDelta,
        phase: TouchPhase,
    },
    Focused(bool),
    ImePreedit(String, Option<(usize, usize)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyKind {
    Named(NamedKey),
    Character(String),
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyChord {
    pub logical: KeyKind,
    pub physical: Option<KeyCode>,
    pub modifiers: ModifiersState,
    pub repeat: bool,
}

impl KeyChord {
    fn from_key_event(event: &KeyEvent, modifiers: ModifiersState) -> Self {
        let logical = match &event.logical_key {
            Key::Named(named) => KeyKind::Named(*named),
            Key::Character(text) => KeyKind::Character(text.to_string()),
            _ => KeyKind::Other,
        };
        let physical = match event.physical_key {
            PhysicalKey::Code(code) => Some(code),
            PhysicalKey::Unidentified(_) => None,
        };
        Self {
            logical,
            physical,
            modifiers,
            repeat: event.repeat,
        }
    }

    pub fn ctrl_or_super(&self) -> bool {
        self.modifiers.super_key() || self.modifiers.control_key()
    }

    pub fn shift(&self) -> bool {
        self.modifiers.shift_key()
    }

    pub fn alt(&self) -> bool {
        self.modifiers.alt_key()
    }

    pub fn logical_char(&self) -> Option<&str> {
        match &self.logical {
            KeyKind::Character(text) => Some(text.as_str()),
            _ => None,
        }
    }

    pub fn named(&self) -> Option<NamedKey> {
        match &self.logical {
            KeyKind::Named(named) => Some(*named),
            _ => None,
        }
    }

    pub fn binding_string(&self) -> Option<String> {
        let (key, inferred_shift) = match &self.logical {
            KeyKind::Character(text) => character_binding_key(text)?,
            KeyKind::Named(named) => named_binding_key(*named)?,
            KeyKind::Other => return None,
        };

        let mut parts = Vec::new();
        if self.modifiers.super_key() {
            parts.push("cmd");
        }
        if self.modifiers.control_key() {
            parts.push("ctrl");
        }
        if self.modifiers.alt_key() {
            parts.push("alt");
        }
        if self.modifiers.shift_key() || inferred_shift {
            parts.push("shift");
        }
        parts.push(key);
        Some(parts.join("+"))
    }
}

fn character_binding_key(text: &str) -> Option<(&'static str, bool)> {
    if text.chars().count() != 1 {
        return None;
    }
    let ch = text.chars().next()?;
    Some(match ch {
        ' ' => ("space", false),
        'A'..='Z' => (lower_ascii_key(ch), true),
        'a'..='z' | '0'..='9' => (lower_ascii_key(ch), false),
        '}' => ("]", true),
        '{' => ("[", true),
        '+' => ("=", true),
        '_' => ("-", true),
        '?' => ("/", true),
        ')' => ("0", true),
        '!' => ("1", true),
        '@' => ("2", true),
        '#' => ("3", true),
        '$' => ("4", true),
        '%' => ("5", true),
        '^' => ("6", true),
        '&' => ("7", true),
        '*' => ("8", true),
        '(' => ("9", true),
        ',' => (",", false),
        '.' => (".", false),
        '/' => ("/", false),
        ';' => (";", false),
        '\'' => ("'", false),
        '[' => ("[", false),
        ']' => ("]", false),
        '\\' => ("\\", false),
        '-' => ("-", false),
        '=' => ("=", false),
        '`' => ("`", false),
        _ => return None,
    })
}

fn lower_ascii_key(ch: char) -> &'static str {
    match ch.to_ascii_lowercase() {
        'a' => "a",
        'b' => "b",
        'c' => "c",
        'd' => "d",
        'e' => "e",
        'f' => "f",
        'g' => "g",
        'h' => "h",
        'i' => "i",
        'j' => "j",
        'k' => "k",
        'l' => "l",
        'm' => "m",
        'n' => "n",
        'o' => "o",
        'p' => "p",
        'q' => "q",
        'r' => "r",
        's' => "s",
        't' => "t",
        'u' => "u",
        'v' => "v",
        'w' => "w",
        'x' => "x",
        'y' => "y",
        'z' => "z",
        '0' => "0",
        '1' => "1",
        '2' => "2",
        '3' => "3",
        '4' => "4",
        '5' => "5",
        '6' => "6",
        '7' => "7",
        '8' => "8",
        '9' => "9",
        _ => unreachable!("caller only passes ascii alphanumeric keys"),
    }
}

fn named_binding_key(named: NamedKey) -> Option<(&'static str, bool)> {
    Some(match named {
        NamedKey::Enter => ("enter", false),
        NamedKey::Tab => ("tab", false),
        NamedKey::Escape => ("escape", false),
        NamedKey::Space => ("space", false),
        NamedKey::ArrowUp => ("arrowup", false),
        NamedKey::ArrowDown => ("arrowdown", false),
        NamedKey::ArrowLeft => ("arrowleft", false),
        NamedKey::ArrowRight => ("arrowright", false),
        NamedKey::PageDown => ("pagedown", false),
        NamedKey::PageUp => ("pageup", false),
        NamedKey::Home => ("home", false),
        NamedKey::End => ("end", false),
        NamedKey::Backspace => ("backspace", false),
        NamedKey::Delete => ("delete", false),
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOwner {
    TextField(FocusTarget),
    Overlay(OverlaySurface),
    Editor,
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputContext {
    pub owner: InputOwner,
    pub overlay: Option<OverlaySurface>,
    pub focus: Option<FocusTarget>,
    pub workspace_mode: WorkspaceMode,
    pub ime_active: bool,
}

#[derive(Default)]
pub struct InputOutcome {
    pub actions: Vec<Action>,
    pub effects: Vec<Effect>,
    pub dirty: bool,
}

impl InputOutcome {
    fn action(action: Action) -> Self {
        Self {
            actions: vec![action],
            effects: Vec::new(),
            dirty: true,
        }
    }

    fn actions(actions: Vec<Action>) -> Self {
        Self {
            dirty: !actions.is_empty(),
            actions,
            effects: Vec::new(),
        }
    }

    fn merge(&mut self, mut other: Self) {
        self.actions.append(&mut other.actions);
        self.effects.append(&mut other.effects);
        self.dirty |= other.dirty;
    }
}

#[derive(Debug, Clone)]
enum ScrollTarget {
    Region(crate::ui::element::ScrollActionBuilder),
    ViewportFallback,
}

pub struct InputSystem {
    modifiers: ModifiersState,
    mouse_position: Option<(f32, f32)>,
    mouse_drag_target: Option<FocusTarget>,
    viewport_text_drag_active: bool,
    review_line_drag_anchor: Option<usize>,
    pointer_capture: Option<Box<dyn DragHandler>>,
    file_list_scroll_remainder_px: f32,
    overlay_scroll_remainder_px: f32,
    editor_scroll_remainder_px: f32,
    viewport_scroll_remainder_px: f32,
    pending_g: bool,
    ime_composing: bool,
}

impl Default for InputSystem {
    fn default() -> Self {
        Self {
            modifiers: ModifiersState::default(),
            mouse_position: None,
            mouse_drag_target: None,
            viewport_text_drag_active: false,
            review_line_drag_anchor: None,
            pointer_capture: None,
            file_list_scroll_remainder_px: 0.0,
            overlay_scroll_remainder_px: 0.0,
            editor_scroll_remainder_px: 0.0,
            viewport_scroll_remainder_px: 0.0,
            pending_g: false,
            ime_composing: false,
        }
    }
}

impl InputSystem {
    pub fn set_modifiers(&mut self, modifiers: ModifiersState) {
        self.modifiers = modifiers;
    }

    pub fn mouse_position(&self) -> Option<(f32, f32)> {
        self.mouse_position
    }

    pub fn handle_window_event(
        &mut self,
        state: &mut AppState,
        ui_frame: &mut UiFrame,
        editor: &EditorElement,
        renderer: Option<&mut Renderer>,
        window: Option<&Arc<Window>>,
        tooltip_state: &mut TooltipState,
        launch_at: Instant,
        event: WindowEvent,
    ) -> Option<InputOutcome> {
        let events = self.normalize_window_event(event);
        if events.is_empty() {
            return None;
        }

        let mut renderer = renderer;
        let mut outcome = InputOutcome::default();
        for event in events {
            let next = self.route_input_event(
                state,
                ui_frame,
                editor,
                renderer.as_deref_mut(),
                window,
                tooltip_state,
                launch_at,
                event,
            );
            outcome.merge(next);
        }
        Some(outcome)
    }

    #[cfg(test)]
    pub(crate) fn handle_input_event_for_test(
        &mut self,
        state: &mut AppState,
        ui_frame: &mut UiFrame,
        editor: &EditorElement,
        renderer: Option<&mut Renderer>,
        window: Option<&Arc<Window>>,
        tooltip_state: &mut TooltipState,
        launch_at: Instant,
        event: InputEvent,
    ) -> InputOutcome {
        self.route_input_event(
            state,
            ui_frame,
            editor,
            renderer,
            window,
            tooltip_state,
            launch_at,
            event,
        )
    }

    fn normalize_window_event(&mut self, event: WindowEvent) -> Vec<InputEvent> {
        match event {
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                Vec::new()
            }
            WindowEvent::Focused(focused) => vec![InputEvent::Focused(focused)],
            WindowEvent::CursorMoved { position, .. } => vec![InputEvent::PointerMoved {
                x: position.x as f32,
                y: position.y as f32,
            }],
            WindowEvent::MouseWheel { delta, phase, .. } => {
                vec![InputEvent::Wheel { delta, phase }]
            }
            WindowEvent::MouseInput { state, button, .. } => {
                vec![InputEvent::PointerButton { button, state }]
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if is_synthetic {
                    return Vec::new();
                }
                self.normalize_keyboard_event(event)
            }
            WindowEvent::Ime(ime) => self.normalize_ime_event(ime),
            _ => Vec::new(),
        }
    }

    fn normalize_keyboard_event(&mut self, event: KeyEvent) -> Vec<InputEvent> {
        let chord = KeyChord::from_key_event(&event, self.modifiers);
        let mut events = Vec::with_capacity(2);
        match event.state {
            ElementState::Pressed => {
                events.push(InputEvent::KeyPress(chord));
                if let Some(text) =
                    keyboard::key_text_from_key_event(&event, self.modifiers, self.ime_composing)
                {
                    events.push(InputEvent::TextInput(text));
                }
            }
            ElementState::Released => events.push(InputEvent::KeyRelease(chord)),
        }
        events
    }

    fn normalize_ime_event(&mut self, ime: Ime) -> Vec<InputEvent> {
        match ime {
            Ime::Enabled => Vec::new(),
            Ime::Preedit(text, cursor) => {
                self.ime_composing = !text.is_empty();
                vec![InputEvent::ImePreedit(text, cursor)]
            }
            Ime::Commit(text) => {
                self.ime_composing = false;
                vec![InputEvent::TextInput(text)]
            }
            Ime::Disabled => {
                self.ime_composing = false;
                Vec::new()
            }
        }
    }

    fn route_input_event(
        &mut self,
        state: &mut AppState,
        ui_frame: &mut UiFrame,
        editor: &EditorElement,
        renderer: Option<&mut Renderer>,
        window: Option<&Arc<Window>>,
        tooltip_state: &mut TooltipState,
        launch_at: Instant,
        event: InputEvent,
    ) -> InputOutcome {
        match event {
            InputEvent::TextInput(text) => self.route_text_input(state, text),
            InputEvent::KeyPress(chord) => self.route_key_press(state, ui_frame, editor, chord),
            InputEvent::KeyRelease(chord) => {
                if chord.logical_char() != Some("g") {
                    self.pending_g = false;
                }
                InputOutcome::default()
            }
            InputEvent::PointerMoved { x, y } => self.handle_pointer_moved(
                state,
                ui_frame,
                editor,
                renderer,
                window,
                tooltip_state,
                launch_at,
                x,
                y,
            ),
            InputEvent::PointerButton {
                button: MouseButton::Left,
                state: ElementState::Pressed,
            } => {
                let Some((x, y)) = self.mouse_position else {
                    return InputOutcome::default();
                };
                self.handle_left_click(state, ui_frame, editor, renderer, x, y)
            }
            InputEvent::PointerButton {
                button: MouseButton::Left,
                state: ElementState::Released,
            } => self.handle_left_release(state),
            InputEvent::PointerButton {
                button: MouseButton::Right,
                state: ElementState::Pressed,
            } => {
                let Some((x, y)) = self.mouse_position else {
                    return InputOutcome::default();
                };
                self.handle_right_click(state, ui_frame, editor, x, y)
            }
            InputEvent::PointerButton { .. } => InputOutcome::default(),
            InputEvent::Wheel { delta, phase } => {
                self.handle_wheel(state, ui_frame, editor, delta, phase)
            }
            InputEvent::Focused(focused) => {
                if !focused {
                    self.pending_g = false;
                    self.mouse_drag_target = None;
                    self.viewport_text_drag_active = false;
                    self.review_line_drag_anchor = None;
                    self.pointer_capture = None;
                    self.ime_composing = false;
                }
                InputOutcome::default()
            }
            InputEvent::ImePreedit(_, _) => InputOutcome::default(),
        }
    }
}

pub fn resolve_input_context(state: &AppState, ime_active: bool) -> InputContext {
    let owner = if let Some(target) = state
        .focus
        .get(&state.store)
        .filter(|_| state.is_text_focused())
    {
        InputOwner::TextField(target)
    } else if let Some(overlay) = state.overlays_top() {
        InputOwner::Overlay(overlay)
    } else if state.focus.get(&state.store) == Some(FocusTarget::Editor) {
        InputOwner::Editor
    } else {
        InputOwner::Workspace
    };
    InputContext {
        owner,
        overlay: state.overlays_top(),
        focus: state.focus.get(&state.store),
        workspace_mode: state.workspace_mode.get(&state.store),
        ime_active,
    }
}
