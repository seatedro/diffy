use std::sync::Arc;
use std::time::Instant;

use winit::event::{
    ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{CursorIcon, Window};

use crate::render::Renderer;
use crate::ui::actions::Action;
use crate::ui::components::{TooltipSide, TooltipState};
use crate::ui::editor::element::EditorElement;
use crate::ui::effects::Effect;
use crate::ui::element::{ClickEvent, ClickResult, DragHandler};
use crate::ui::shell::UiFrame;
use crate::ui::state::{AppState, CompareField, FocusTarget, OverlaySurface, WorkspaceMode};

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
        self.modifiers.control_key() || self.modifiers.super_key()
    }

    pub fn shift(&self) -> bool {
        self.modifiers.shift_key()
    }

    pub fn logical_char(&self) -> Option<&str> {
        match &self.logical {
            KeyKind::Character(text) => Some(text.as_str()),
            _ => None,
        }
    }

    pub fn named(&self) -> Option<NamedKey> {
        match self.logical {
            KeyKind::Named(named) => Some(named),
            _ => None,
        }
    }
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
    pointer_capture: Option<Box<dyn DragHandler>>,
    file_list_scroll_remainder_px: f32,
    overlay_scroll_remainder_px: f32,
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
            pointer_capture: None,
            file_list_scroll_remainder_px: 0.0,
            overlay_scroll_remainder_px: 0.0,
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
                    key_text_from_key_event(&event, self.modifiers, self.ime_composing)
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
            InputEvent::KeyPress(chord) => self.route_key_press(state, chord),
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
            InputEvent::PointerButton { .. } => InputOutcome::default(),
            InputEvent::Wheel { delta, phase } => {
                self.handle_wheel(state, ui_frame, editor, delta, phase)
            }
            InputEvent::Focused(focused) => {
                if !focused {
                    self.pending_g = false;
                    self.mouse_drag_target = None;
                    self.pointer_capture = None;
                    self.ime_composing = false;
                }
                InputOutcome::default()
            }
            InputEvent::ImePreedit(_, _) => InputOutcome::default(),
        }
    }

    fn route_text_input(&mut self, state: &AppState, text: String) -> InputOutcome {
        let ctx = resolve_input_context(state, self.ime_composing);
        match ctx.owner {
            InputOwner::TextField(_) if !text.is_empty() => {
                InputOutcome::action(Action::InsertText(text))
            }
            _ => InputOutcome::default(),
        }
    }

    fn route_key_press(&mut self, state: &AppState, chord: KeyChord) -> InputOutcome {
        if chord.logical_char() != Some("g") {
            self.pending_g = false;
        }

        if let Some(action) = global_shortcut_action(&chord) {
            return InputOutcome::action(action);
        }

        let ctx = resolve_input_context(state, self.ime_composing);
        if matches!(ctx.owner, InputOwner::TextField(_))
            && let Some(action) = clipboard_shortcut_action(&chord)
        {
            return InputOutcome::action(action);
        }

        let actions = match ctx.owner {
            InputOwner::TextField(target) => {
                text_field_key_actions(&ctx, target, &chord).or_else(|| {
                    ctx.overlay
                        .and_then(|surface| overlay_key_actions(state, surface, &chord))
                })
            }
            InputOwner::Overlay(surface) => overlay_key_actions(state, surface, &chord),
            InputOwner::Editor => editor_key_actions(self, state, &chord),
            InputOwner::Workspace => workspace_key_actions(self, state, &chord),
        };

        InputOutcome::actions(actions.unwrap_or_default())
    }

    fn handle_left_click(
        &mut self,
        state: &AppState,
        ui_frame: &mut UiFrame,
        editor: &EditorElement,
        renderer: Option<&mut Renderer>,
        x: f32,
        y: f32,
    ) -> InputOutcome {
        if let Some(track) = ui_frame
            .scrollbar_tracks
            .iter()
            .rev()
            .find(|t| t.track_rect.contains(x, y))
        {
            let on_thumb = y >= track.thumb_top && y <= track.thumb_top + track.thumb_height;
            let mut handler = crate::ui::element::ScrollbarDragHandler::new(track, y);
            let mut outcome = InputOutcome::default();
            if !on_thumb {
                outcome.actions = handler.on_move(x, y);
                outcome.dirty = !outcome.actions.is_empty();
            }
            self.pointer_capture = Some(Box::new(handler));
            return outcome;
        }

        if let Some(hit_area) = ui_frame
            .text_input_hit_areas
            .iter()
            .rev()
            .find(|ha| ha.bounds.contains(x, y))
        {
            let byte_offset = hit_test_text_offset(
                renderer.map(Renderer::font_system),
                &hit_area.value,
                hit_area.font_size,
                x - hit_area.text_x,
            );
            self.mouse_drag_target = Some(hit_area.focus_target);
            return InputOutcome::actions(vec![
                Action::SetFocus(Some(hit_area.focus_target)),
                Action::SetTextCursor(byte_offset),
            ]);
        }

        if let Some(idx) = ui_frame
            .hits
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, hit)| hit.rect.contains(x, y).then_some(i))
        {
            let hit = &mut ui_frame.hits[idx];
            let mut actions = Vec::new();
            if let Some(handler) = hit.on_click.take() {
                match handler.invoke(ClickEvent { x, y }) {
                    ClickResult::Handled => {}
                    ClickResult::Actions(handler_actions) => actions.extend(handler_actions),
                    ClickResult::CaptureDrag(drag) => {
                        self.pointer_capture = Some(drag);
                    }
                }
            } else {
                let action = hit.action.clone();
                if matches!(action, Action::SelectFile(_)) {
                    actions.push(Action::SetFocus(Some(FocusTarget::FileList)));
                }
                actions.push(action);
            }
            return InputOutcome::actions(actions);
        }

        if ui_frame
            .viewport_rect
            .is_some_and(|rect| rect.contains(x, y))
        {
            let hovered = editor.hit_test_row(&state.editor, x, y);
            return InputOutcome::actions(vec![
                Action::FocusViewport,
                Action::HoverViewportRow(hovered),
            ]);
        }

        InputOutcome::default()
    }

    fn handle_pointer_moved(
        &mut self,
        state: &AppState,
        ui_frame: &UiFrame,
        editor: &EditorElement,
        renderer: Option<&mut Renderer>,
        window: Option<&Arc<Window>>,
        tooltip_state: &mut TooltipState,
        launch_at: Instant,
        x: f32,
        y: f32,
    ) -> InputOutcome {
        self.mouse_position = Some((x, y));

        let mut actions = Vec::new();
        if let Some(ref mut capture) = self.pointer_capture {
            actions.extend(capture.on_move(x, y));
        }

        if let Some(drag_target) = self.mouse_drag_target
            && let Some(hit_area) = ui_frame
                .text_input_hit_areas
                .iter()
                .find(|ha| ha.focus_target == drag_target)
        {
            let byte_offset = hit_test_text_offset(
                renderer.map(Renderer::font_system),
                &hit_area.value,
                hit_area.font_size,
                x - hit_area.text_x,
            );
            actions.push(Action::ExtendTextSelection(byte_offset));
        }

        let hovered_hit = ui_frame
            .hits
            .iter()
            .rev()
            .find(|hit| hit.rect.contains(x, y));
        let hovered_file = hovered_hit.and_then(|hit| match &hit.action {
            Action::SelectFile(i) => Some(*i),
            _ => None,
        });
        let hovered_toast = hovered_hit.and_then(|hit| match &hit.action {
            Action::DismissToast(i) => Some(*i),
            _ => None,
        });
        let cursor_hint = if let Some(ref capture) = self.pointer_capture {
            capture.cursor()
        } else {
            hovered_hit
                .map(|hit| hit.cursor)
                .unwrap_or(crate::ui::shell::CursorHint::Default)
        };

        if hovered_file != state.file_list.hovered_index {
            actions.push(Action::HoverFile(hovered_file));
        }
        let current_hovered_toast = state.toasts.iter().position(|toast| toast.hovered);
        if hovered_toast != current_hovered_toast {
            actions.push(Action::HoverToast(hovered_toast));
        }

        let hovered_row = if input_is_blocked_by_overlay(state, ui_frame, x, y) {
            None
        } else {
            editor.hit_test_row(&state.editor, x, y)
        };
        if hovered_row != state.editor.hovered_row {
            actions.push(Action::HoverViewportRow(hovered_row));
        }

        let now_ms = launch_at.elapsed().as_millis() as u64;
        let hovered_tooltip = ui_frame
            .tooltip_regions
            .iter()
            .rev()
            .find(|region| region.bounds.contains(x, y));
        if let Some(region) = hovered_tooltip {
            if tooltip_state.text != region.text {
                tooltip_state.show(
                    &region.text,
                    x,
                    region.bounds.y + region.bounds.height,
                    TooltipSide::Bottom,
                    500,
                    now_ms,
                );
            }
        } else {
            tooltip_state.hide();
        }
        tooltip_state.tick(now_ms);

        if let Some(window) = window {
            let icon = match cursor_hint {
                crate::ui::shell::CursorHint::Default => CursorIcon::Default,
                crate::ui::shell::CursorHint::Pointer => CursorIcon::Pointer,
                crate::ui::shell::CursorHint::Text => CursorIcon::Text,
                crate::ui::shell::CursorHint::ResizeCol => CursorIcon::EwResize,
            };
            window.set_cursor(icon);
        }

        InputOutcome::actions(actions)
    }

    fn handle_left_release(&mut self, state: &AppState) -> InputOutcome {
        let mut outcome = InputOutcome::default();
        if let Some(mut capture) = self.pointer_capture.take() {
            let result = capture.on_release(state);
            outcome.actions = result.actions;
            outcome.effects = result.effects;
            outcome.dirty = true;
        }
        self.mouse_drag_target = None;
        outcome
    }

    fn handle_wheel(
        &mut self,
        state: &AppState,
        ui_frame: &UiFrame,
        editor: &EditorElement,
        delta: MouseScrollDelta,
        phase: TouchPhase,
    ) -> InputOutcome {
        let Some((x, y)) = self.mouse_position else {
            return InputOutcome::default();
        };

        if matches!(phase, TouchPhase::Started | TouchPhase::Cancelled) {
            self.reset_scroll_remainders();
        }

        let Some(target) = self.scroll_target_at(state, ui_frame, x, y) else {
            return InputOutcome::default();
        };
        let line_step_px = self.scroll_target_line_step_px(state, &target, editor);
        let delta_px = scroll_delta_to_px(delta, line_step_px);
        let rounded_delta_px = match &target {
            ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::FileList) => {
                quantize_scroll_delta_px(&mut self.file_list_scroll_remainder_px, delta_px)
            }
            ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::Custom(_)) => {
                quantize_scroll_delta_px(&mut self.overlay_scroll_remainder_px, delta_px)
            }
            ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::ViewportLines)
            | ScrollTarget::ViewportFallback => {
                quantize_scroll_delta_px(&mut self.viewport_scroll_remainder_px, delta_px)
            }
        };

        let mut actions = Vec::new();
        if rounded_delta_px != 0 {
            match target {
                ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::FileList) => {
                    actions.push(Action::ScrollFileListPx(rounded_delta_px));
                }
                ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::Custom(build)) => {
                    actions.push(build(rounded_delta_px));
                }
                ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::ViewportLines)
                | ScrollTarget::ViewportFallback => {
                    actions.push(Action::ScrollViewportPx(rounded_delta_px));
                }
            }
        }

        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.reset_scroll_remainders();
        }

        InputOutcome::actions(actions)
    }

    fn reset_scroll_remainders(&mut self) {
        self.file_list_scroll_remainder_px = 0.0;
        self.overlay_scroll_remainder_px = 0.0;
        self.viewport_scroll_remainder_px = 0.0;
    }

    fn scroll_target_at(
        &self,
        state: &AppState,
        ui_frame: &UiFrame,
        x: f32,
        y: f32,
    ) -> Option<ScrollTarget> {
        for region in ui_frame.scroll_regions.iter().rev() {
            if region.bounds.contains(x, y) {
                return Some(ScrollTarget::Region(region.action_builder.clone()));
            }
        }

        if input_is_blocked_by_overlay(state, ui_frame, x, y) {
            return None;
        }

        ui_frame
            .viewport_rect
            .filter(|rect| rect.contains(x, y))
            .map(|_| ScrollTarget::ViewportFallback)
    }

    fn scroll_target_line_step_px(
        &self,
        state: &AppState,
        target: &ScrollTarget,
        editor: &EditorElement,
    ) -> f32 {
        match target {
            ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::FileList) => {
                state.file_list.row_stride().max(1.0)
            }
            ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::Custom(_)) => {
                active_overlay_row_height_px(state)
            }
            ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::ViewportLines)
            | ScrollTarget::ViewportFallback => editor.scroll_line_height_px(),
        }
    }
}

pub fn resolve_input_context(state: &AppState, ime_active: bool) -> InputContext {
    let owner = if let Some(target) = state.focus.current.filter(|_| state.is_text_focused()) {
        InputOwner::TextField(target)
    } else if let Some(overlay) = state.overlays.top() {
        InputOwner::Overlay(overlay)
    } else if state.focus.current == Some(FocusTarget::Editor) {
        InputOwner::Editor
    } else {
        InputOwner::Workspace
    };
    InputContext {
        owner,
        overlay: state.overlays.top(),
        focus: state.focus.current,
        workspace_mode: state.workspace_mode,
        ime_active,
    }
}

fn global_shortcut_action(chord: &KeyChord) -> Option<Action> {
    if !chord.ctrl_or_super() {
        return None;
    }
    match chord.logical_char()?.to_ascii_lowercase().as_str() {
        "f" => Some(Action::OpenSearch),
        "p" => Some(Action::OpenCommandPalette),
        "=" | "+" => Some(Action::IncreaseUiScale),
        "-" | "_" => Some(Action::DecreaseUiScale),
        "b" => Some(Action::ToggleSidebar),
        _ => None,
    }
}

fn clipboard_shortcut_action(chord: &KeyChord) -> Option<Action> {
    if !chord.ctrl_or_super() {
        return None;
    }
    match chord.logical_char()?.to_ascii_lowercase().as_str() {
        "a" => Some(Action::SelectAll),
        "c" => Some(Action::Copy),
        "x" => Some(Action::Cut),
        "v" => arboard::Clipboard::new()
            .ok()
            .and_then(|mut clipboard| clipboard.get_text().ok())
            .map(Action::Paste),
        _ => None,
    }
}

fn text_field_key_actions(
    ctx: &InputContext,
    target: FocusTarget,
    chord: &KeyChord,
) -> Option<Vec<Action>> {
    match chord.named() {
        Some(NamedKey::Enter) if target == FocusTarget::SearchInput => {
            Some(vec![if chord.shift() {
                Action::SearchPrevious
            } else {
                Action::SearchNext
            }])
        }
        Some(NamedKey::ArrowLeft) => Some(vec![match (chord.ctrl_or_super(), chord.shift()) {
            (true, true) => Action::SelectWordLeft,
            (true, false) => Action::CursorWordLeft,
            (false, true) => Action::SelectLeft,
            (false, false) => Action::CursorLeft,
        }]),
        Some(NamedKey::ArrowRight) => Some(vec![match (chord.ctrl_or_super(), chord.shift()) {
            (true, true) => Action::SelectWordRight,
            (true, false) => Action::CursorWordRight,
            (false, true) => Action::SelectRight,
            (false, false) => Action::CursorRight,
        }]),
        Some(NamedKey::Home) => Some(vec![if chord.shift() {
            Action::SelectHome
        } else {
            Action::CursorHome
        }]),
        Some(NamedKey::End) => Some(vec![if chord.shift() {
            Action::SelectEnd
        } else {
            Action::CursorEnd
        }]),
        Some(NamedKey::Backspace) => Some(vec![Action::Backspace]),
        Some(NamedKey::Delete) => Some(vec![Action::DeleteForward]),
        Some(NamedKey::Escape) if ctx.overlay.is_some() => Some(vec![Action::CloseOverlay]),
        _ => None,
    }
}

fn overlay_key_actions(
    state: &AppState,
    surface: OverlaySurface,
    chord: &KeyChord,
) -> Option<Vec<Action>> {
    match chord.named() {
        Some(NamedKey::Escape) => Some(vec![Action::CloseOverlay]),
        Some(NamedKey::Tab) => {
            if surface == OverlaySurface::RepoPicker {
                Some(vec![Action::TabCompletePickerDir])
            } else {
                Some(vec![Action::SetFocus(cycle_focus_target(state))])
            }
        }
        Some(NamedKey::Enter) => activate_current_focus_actions(state),
        Some(NamedKey::ArrowDown) => Some(vec![Action::MoveOverlaySelection(1)]),
        Some(NamedKey::ArrowUp) => Some(vec![Action::MoveOverlaySelection(-1)]),
        _ => None,
    }
}

fn editor_key_actions(
    input: &mut InputSystem,
    state: &AppState,
    chord: &KeyChord,
) -> Option<Vec<Action>> {
    workspace_key_actions_inner(Some(input), state, chord)
}

fn workspace_key_actions(
    input: &mut InputSystem,
    state: &AppState,
    chord: &KeyChord,
) -> Option<Vec<Action>> {
    workspace_key_actions_inner(Some(input), state, chord)
}

fn workspace_key_actions_inner(
    mut input: Option<&mut InputSystem>,
    state: &AppState,
    chord: &KeyChord,
) -> Option<Vec<Action>> {
    match chord.named() {
        Some(NamedKey::Escape) => {
            if state.overlays.top().is_some() {
                Some(vec![Action::CloseOverlay])
            } else if state.editor.search.open {
                Some(vec![Action::CloseSearch])
            } else if state.focus.current == Some(FocusTarget::SidebarSearch) {
                Some(vec![Action::ClearSidebarFilter, Action::SetFocus(None)])
            } else {
                None
            }
        }
        Some(NamedKey::Tab) => Some(vec![Action::SetFocus(cycle_focus_target(state))]),
        Some(NamedKey::Enter) => {
            if state.focus.current == Some(FocusTarget::SearchInput) {
                Some(vec![if chord.shift() {
                    Action::SearchPrevious
                } else {
                    Action::SearchNext
                }])
            } else {
                activate_current_focus_actions(state)
            }
        }
        Some(NamedKey::ArrowDown) => {
            if state.focus.current == Some(FocusTarget::Editor) {
                Some(vec![Action::ScrollViewportLines(1)])
            } else if state.workspace_mode == WorkspaceMode::Ready {
                Some(vec![Action::SelectNextFile])
            } else {
                None
            }
        }
        Some(NamedKey::ArrowUp) => {
            if state.focus.current == Some(FocusTarget::Editor) {
                Some(vec![Action::ScrollViewportLines(-1)])
            } else if state.workspace_mode == WorkspaceMode::Ready {
                Some(vec![Action::SelectPreviousFile])
            } else {
                None
            }
        }
        Some(NamedKey::PageDown) if state.workspace_mode == WorkspaceMode::Ready => {
            if state.focus.current == Some(FocusTarget::Editor) {
                Some(vec![Action::ScrollViewportPages(1)])
            } else {
                Some(vec![Action::ScrollFileList(10)])
            }
        }
        Some(NamedKey::PageUp) if state.workspace_mode == WorkspaceMode::Ready => {
            if state.focus.current == Some(FocusTarget::Editor) {
                Some(vec![Action::ScrollViewportPages(-1)])
            } else {
                Some(vec![Action::ScrollFileList(-10)])
            }
        }
        Some(NamedKey::Home) if state.workspace_mode == WorkspaceMode::Ready => {
            Some(vec![Action::ScrollViewportTo(0)])
        }
        Some(NamedKey::End) if state.workspace_mode == WorkspaceMode::Ready => {
            Some(vec![Action::ScrollViewportTo(
                state.editor.max_scroll_top_px(),
            )])
        }
        _ => {
            let ch = chord.logical_char()?;
            if ch == "?" {
                return Some(vec![Action::ShowKeyboardShortcuts]);
            }
            if state.overlays.top().is_some() || state.workspace_mode != WorkspaceMode::Ready {
                return None;
            }
            match ch {
                "/" => Some(vec![Action::SetFocus(Some(FocusTarget::SidebarSearch))]),
                "]" => Some(vec![Action::GoToNextHunk]),
                "[" => Some(vec![Action::GoToPreviousHunk]),
                "n" => Some(vec![Action::GoToNextFile]),
                "N" => Some(vec![Action::GoToPreviousFile]),
                "j" => Some(vec![Action::ScrollViewportLines(1)]),
                "k" => Some(vec![Action::ScrollViewportLines(-1)]),
                "d" => Some(vec![Action::ScrollViewportHalfPage(1)]),
                "u" => Some(vec![Action::ScrollViewportHalfPage(-1)]),
                "G" => Some(vec![Action::ScrollViewportTo(
                    state.editor.max_scroll_top_px(),
                )]),
                "g" => {
                    let input = input.as_mut()?;
                    if input.pending_g {
                        input.pending_g = false;
                        Some(vec![Action::ScrollViewportTo(0)])
                    } else {
                        input.pending_g = true;
                        Some(Vec::new())
                    }
                }
                "1" => Some(vec![Action::SetLayoutMode(
                    crate::core::compare::LayoutMode::Unified,
                )]),
                "2" => Some(vec![Action::SetLayoutMode(
                    crate::core::compare::LayoutMode::Split,
                )]),
                "w" => Some(vec![Action::ToggleWrap]),
                " " => Some(vec![if chord.shift() {
                    Action::ScrollViewportPages(-1)
                } else {
                    Action::ScrollViewportPages(1)
                }]),
                _ => None,
            }
        }
    }
}

fn cycle_focus_target(state: &AppState) -> Option<FocusTarget> {
    match state.overlays.top() {
        Some(OverlaySurface::CompareSheet) => match state.focus.current {
            Some(FocusTarget::CompareRepoButton) => Some(FocusTarget::CompareLeftRef),
            Some(FocusTarget::CompareLeftRef) => Some(FocusTarget::CompareRightRef),
            Some(FocusTarget::CompareRightRef) => Some(FocusTarget::CompareStartButton),
            _ => Some(FocusTarget::CompareRepoButton),
        },
        Some(OverlaySurface::RepoPicker | OverlaySurface::RefPicker(_)) => {
            match state.focus.current {
                Some(FocusTarget::PickerInput) => Some(FocusTarget::PickerList),
                _ => Some(FocusTarget::PickerInput),
            }
        }
        Some(OverlaySurface::CommandPalette) => match state.focus.current {
            Some(FocusTarget::CommandPaletteInput) => Some(FocusTarget::CommandPaletteList),
            _ => Some(FocusTarget::CommandPaletteInput),
        },
        Some(OverlaySurface::PullRequestModal) => match state.focus.current {
            Some(FocusTarget::PullRequestInput) => Some(FocusTarget::PullRequestConfirm),
            _ => Some(FocusTarget::PullRequestInput),
        },
        Some(OverlaySurface::ThemePicker) => match state.focus.current {
            Some(FocusTarget::PickerInput) => Some(FocusTarget::PickerList),
            _ => Some(FocusTarget::PickerInput),
        },
        Some(OverlaySurface::GitHubAuthModal) => Some(FocusTarget::AuthPrimaryAction),
        Some(OverlaySurface::KeyboardShortcuts) => None,
        None => match state.focus.current {
            Some(FocusTarget::FileList) => Some(FocusTarget::Editor),
            Some(FocusTarget::Editor) => Some(FocusTarget::FileList),
            Some(FocusTarget::WorkspacePrimaryButton) => Some(FocusTarget::TitleBar),
            _ => Some(if state.workspace_mode == WorkspaceMode::Ready {
                FocusTarget::FileList
            } else {
                FocusTarget::WorkspacePrimaryButton
            }),
        },
    }
}

fn activate_current_focus_actions(state: &AppState) -> Option<Vec<Action>> {
    match state.overlays.top() {
        Some(OverlaySurface::CompareSheet) => Some(match state.focus.current {
            Some(FocusTarget::CompareRepoButton) => vec![Action::OpenRepoPicker],
            Some(FocusTarget::CompareLeftRef) => vec![Action::OpenRefPicker(CompareField::Left)],
            Some(FocusTarget::CompareRightRef) => vec![Action::OpenRefPicker(CompareField::Right)],
            _ => vec![Action::StartCompare],
        }),
        Some(
            OverlaySurface::RepoPicker
            | OverlaySurface::RefPicker(_)
            | OverlaySurface::CommandPalette
            | OverlaySurface::ThemePicker,
        ) => Some(vec![Action::ConfirmOverlaySelection]),
        Some(OverlaySurface::PullRequestModal) => Some(vec![Action::SubmitPullRequest]),
        Some(OverlaySurface::GitHubAuthModal) => {
            Some(vec![if state.github.auth.device_flow.is_some() {
                Action::OpenDeviceFlowBrowser
            } else {
                Action::StartGitHubDeviceFlow
            }])
        }
        Some(OverlaySurface::KeyboardShortcuts) => Some(Vec::new()),
        None => match state.focus.current {
            Some(FocusTarget::WorkspacePrimaryButton) => Some(vec![Action::OpenCompareSheet]),
            Some(FocusTarget::ThemeToggle) => Some(vec![Action::ToggleThemeMode]),
            _ => None,
        },
    }
}

fn active_overlay_row_height_px(state: &AppState) -> f32 {
    match state.overlays.top() {
        Some(
            OverlaySurface::RepoPicker | OverlaySurface::RefPicker(_) | OverlaySurface::ThemePicker,
        ) => state.overlays.picker.list.stride_px().max(1) as f32,
        Some(OverlaySurface::CommandPalette) => {
            state.overlays.command_palette.list.stride_px().max(1) as f32
        }
        _ => 36.0,
    }
}

fn input_is_blocked_by_overlay(state: &AppState, ui_frame: &UiFrame, x: f32, y: f32) -> bool {
    state.overlays.top().is_some()
        && ui_frame
            .hits
            .iter()
            .rev()
            .any(|hit| hit.rect.contains(x, y))
}

fn key_text_from_key_event(
    event: &KeyEvent,
    modifiers: ModifiersState,
    ime_composing: bool,
) -> Option<String> {
    if ime_composing || modifiers.control_key() || modifiers.super_key() {
        return None;
    }
    let text = event.text.as_ref()?;
    if text.is_empty() || text.chars().all(char::is_control) {
        return None;
    }
    Some(text.to_string())
}

pub fn scroll_delta_to_px(delta: MouseScrollDelta, line_step_px: f32) -> f32 {
    match delta {
        MouseScrollDelta::LineDelta(_, y) => -y * line_step_px,
        MouseScrollDelta::PixelDelta(position) => -(position.y as f32),
    }
}

pub fn quantize_scroll_delta_px(remainder_px: &mut f32, delta_px: f32) -> i32 {
    *remainder_px += delta_px;
    let whole_px = remainder_px.trunc() as i32;
    *remainder_px -= whole_px as f32;
    whole_px
}

pub fn hit_test_text_offset(
    font_system: Option<&mut glyphon::FontSystem>,
    text: &str,
    font_size: f32,
    click_x: f32,
) -> usize {
    if text.is_empty() || click_x <= 0.0 {
        return 0;
    }
    let Some(font_system) = font_system else {
        return text.len();
    };

    let metrics = glyphon::Metrics::new(font_size, font_size * 1.2);
    let mut buffer = glyphon::Buffer::new(font_system, metrics);
    let attrs = glyphon::Attrs::new().family(glyphon::Family::SansSerif);
    buffer.set_size(font_system, None, None);
    buffer.set_text(font_system, text, &attrs, glyphon::Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);

    let mut best_offset = text.len();
    let mut best_dist = f32::MAX;

    for run in buffer.layout_runs() {
        let dist = click_x.abs();
        if dist < best_dist {
            best_dist = dist;
            best_offset = 0;
        }
        for glyph in run.glyphs.iter() {
            let left_dist = (click_x - glyph.x).abs();
            if left_dist < best_dist {
                best_dist = left_dist;
                best_offset = glyph.start;
            }
            let right_dist = (click_x - (glyph.x + glyph.w)).abs();
            if right_dist < best_dist {
                best_dist = right_dist;
                best_offset = glyph.end;
            }
        }
        let dist = (click_x - run.line_w).abs();
        if dist < best_dist {
            best_dist = dist;
            best_offset = text.len();
        }
    }

    best_offset
}
