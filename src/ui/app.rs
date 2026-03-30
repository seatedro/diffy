use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::app_runtime::{AppRuntime, AppServices};
use crate::platform::automation::{ErrorDump, FilesDump, StateDump, write_json};
use crate::platform::persistence::SettingsStore;
use crate::platform::startup::StartupOptions;
use crate::render::Renderer;
use crate::ui::actions::Action;
use crate::ui::diff_viewport::runtime::DiffViewportRuntime;
use crate::ui::shell::{CursorHint, UiFrame, build_ui_frame};
use crate::ui::state::{AppState, FocusTarget, OverlaySurface, WorkspaceMode};
use crate::ui::theme::Theme;

pub fn run() -> Result<(), Box<dyn Error>> {
    let startup = StartupOptions::load();
    init_logging(startup.log_debug);

    let settings_store = SettingsStore::new_default();
    let settings = settings_store.load()?;
    let (state, initial_effects) = AppState::bootstrap(startup, settings);
    let runtime = AppRuntime::new(AppServices::new(settings_store));
    runtime.dispatch_all(initial_effects);

    let event_loop = EventLoop::new()?;
    let should_poll = state.startup.exit_after.is_some();
    event_loop.set_control_flow(if should_poll {
        ControlFlow::Poll
    } else {
        ControlFlow::Wait
    });

    let mut app = NativeApp::new(state, runtime);
    event_loop.run_app(&mut app)?;
    Ok(())
}

struct NativeApp {
    state: AppState,
    theme: Theme,
    runtime: AppRuntime,
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    ui_frame: UiFrame,
    viewport_runtime: DiffViewportRuntime,
    mouse_position: Option<(f32, f32)>,
    signal_store: crate::ui::signals::SignalStore,
    launch_at: Instant,
    dumps_dirty: bool,
    modifiers: ModifiersState,
    capture_pending: Option<std::path::PathBuf>,
    /// When dragging in a text input, tracks which field is being drag-selected.
    mouse_drag_target: Option<FocusTarget>,
    /// When dragging a scrollbar thumb, tracks the drag state.
    scrollbar_drag: Option<ScrollbarDrag>,
    /// When dragging the sidebar splitter, tracks the drag baseline.
    sidebar_resize_drag: Option<SidebarResizeDrag>,
    file_list_scroll_remainder_px: f32,
    overlay_scroll_remainder_px: f32,
    viewport_scroll_remainder_px: f32,
}

/// State for an active scrollbar thumb drag.
struct ScrollbarDrag {
    /// The scroll action builder for the scrollbar being dragged.
    action_builder: crate::ui::element::ScrollActionBuilder,
    /// Y offset of the mouse within the track at drag start.
    track_top: f32,
    track_height: f32,
    thumb_height: f32,
    content_height: f32,
    viewport_height: f32,
    /// Mouse Y offset from the top of the thumb at drag start.
    grab_offset: f32,
}

struct SidebarResizeDrag {
    origin_x: f32,
    starting_width: f32,
}

impl NativeApp {
    fn new(state: AppState, runtime: AppRuntime) -> Self {
        let theme =
            Theme::for_mode(state.settings.theme_mode).with_ui_scale(state.ui_scale_factor());
        let capture_pending = std::env::var("DIFFY_CAPTURE_PATH")
            .ok()
            .map(std::path::PathBuf::from);
        Self {
            state,
            theme,
            runtime,
            renderer: None,
            window: None,
            ui_frame: UiFrame::default(),
            signal_store: crate::ui::signals::SignalStore::new(),
            viewport_runtime: DiffViewportRuntime::default(),
            mouse_position: None,
            launch_at: Instant::now(),
            dumps_dirty: true,
            modifiers: ModifiersState::default(),
            capture_pending,
            mouse_drag_target: None,
            scrollbar_drag: None,
            sidebar_resize_drag: None,
            file_list_scroll_remainder_px: 0.0,
            overlay_scroll_remainder_px: 0.0,
            viewport_scroll_remainder_px: 0.0,
        }
    }

    fn sync_theme(&mut self) {
        self.theme = Theme::for_mode(self.state.settings.theme_mode)
            .with_ui_scale(self.state.ui_scale_factor());
    }

    fn window_attributes(&self) -> WindowAttributes {
        Window::default_attributes()
            .with_title(self.state.window_title())
            .with_visible(!self.state.startup.hidden_window)
            .with_inner_size(LogicalSize::new(1320.0, 840.0))
            .with_min_inner_size(LogicalSize::new(980.0, 680.0))
    }

    fn window_id(&self) -> Option<WindowId> {
        self.window.as_ref().map(|window| window.id())
    }

    fn refresh_window_title(&self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(&self.state.window_title());
        }
    }

    fn process_runtime_events(&mut self) {
        let events = self.runtime.drain_events();
        if events.is_empty() {
            return;
        }

        for event in events {
            let effects = self.state.apply_event(event);
            self.runtime.dispatch_all(effects);
        }
        self.sync_theme();
        self.refresh_window_title();
        self.dumps_dirty = true;
    }

    fn write_dumps_if_needed(&mut self) {
        if !self.dumps_dirty {
            return;
        }

        if self.state.startup.hidden_window {
            let frame = self.build_frame();
            self.state.debug.last_scene_primitive_count = frame.scene.len();
            self.ui_frame = frame;
        }

        if let Some(path) = self.state.startup.dump_state_json.as_deref()
            && let Err(error) = write_json(path, &StateDump::from(&self.state))
        {
            eprintln!("failed to write state dump: {error}");
        }
        if let Some(path) = self.state.startup.dump_files_json.as_deref()
            && let Err(error) = write_json(path, &FilesDump::from(&self.state))
        {
            eprintln!("failed to write files dump: {error}");
        }
        if let Some(path) = self.state.startup.dump_errors_json.as_deref()
            && let Err(error) = write_json(path, &ErrorDump::from(&self.state))
        {
            eprintln!("failed to write errors dump: {error}");
        }

        self.dumps_dirty = false;
    }

    fn build_frame(&mut self) -> UiFrame {
        let size = self
            .window
            .as_ref()
            .map(|window| window.inner_size())
            .unwrap_or_else(|| winit::dpi::PhysicalSize::new(1320, 840));
        let text_metrics = self
            .renderer
            .as_ref()
            .map(Renderer::text_metrics)
            .unwrap_or_default();
        let scale_factor = self
            .renderer
            .as_ref()
            .map(|r| r.scale_factor() as f32)
            .unwrap_or(1.0);

        // Create a temporary font system for element layout if renderer isn't ready.
        let mut fallback_font_system;
        let font_system = if let Some(renderer) = self.renderer.as_mut() {
            renderer.font_system_mut()
        } else {
            fallback_font_system = crate::fonts::new_font_system();
            &mut fallback_font_system
        };

        let width = size.width.max(1) as f32;
        let height = size.height.max(1) as f32;
        let ui_scale = self.state.ui_scale_factor();

        let mut cx = crate::ui::element::ElementContext::new(
            &self.theme,
            scale_factor,
            font_system,
            self.mouse_position,
            &mut self.signal_store,
        )
        .with_focus(self.state.focus.current)
        .with_clock(self.state.clock_ms);
        cx.debug_wireframe = std::env::var("DIFFY_DEBUG_WIREFRAME").is_ok();

        build_ui_frame(
            &mut self.state,
            &self.theme,
            &mut self.viewport_runtime,
            scale_text_metrics(text_metrics, ui_scale),
            width,
            height,
            &mut cx,
        )
    }

    fn dispatch_action(&mut self, action: Action) {
        let effects = self.state.apply_action(action);
        self.runtime.dispatch_all(effects);
        self.sync_theme();
        self.refresh_window_title();
        self.dumps_dirty = true;
    }

    fn handle_left_click(&mut self, x: f32, y: f32) {
        if self
            .ui_frame
            .sidebar_resize_handle_rect
            .is_some_and(|rect| rect.contains(x, y))
        {
            let starting_width = self
                .ui_frame
                .file_list_rect
                .map(|rect| rect.width)
                .unwrap_or(self.theme.metrics.sidebar_width);
            self.sidebar_resize_drag = Some(SidebarResizeDrag {
                origin_x: x,
                starting_width,
            });
            return;
        }

        // Check scrollbar tracks for click/drag
        if let Some(track) = self
            .ui_frame
            .scrollbar_tracks
            .iter()
            .rev()
            .find(|t| t.track_rect.contains(x, y))
        {
            let on_thumb = y >= track.thumb_top && y <= track.thumb_top + track.thumb_height;
            let grab_offset = if on_thumb {
                y - track.thumb_top
            } else {
                track.thumb_height / 2.0
            };
            self.scrollbar_drag = Some(ScrollbarDrag {
                action_builder: track.action_builder.clone(),
                track_top: track.track_rect.y,
                track_height: track.track_rect.height,
                thumb_height: track.thumb_height,
                content_height: track.content_height,
                viewport_height: track.viewport_height,
                grab_offset,
            });
            // If clicked outside the thumb, jump to that position
            if !on_thumb {
                self.handle_scrollbar_drag_move(y);
            }
            return;
        }

        // Check text input hit areas for click-to-position
        if let Some(hit_area) = self
            .ui_frame
            .text_input_hit_areas
            .iter()
            .rev()
            .find(|ha| ha.bounds.contains(x, y))
        {
            let target = hit_area.focus_target;
            let byte_offset = hit_test_text_offset(
                self.renderer.as_mut().map(|r| r.font_system()),
                &hit_area.value,
                hit_area.font_size,
                x - hit_area.text_x,
            );
            // Focus the field and set cursor position
            self.dispatch_action(Action::SetFocus(Some(target)));
            self.dispatch_action(Action::SetTextCursor(byte_offset));
            self.mouse_drag_target = Some(target);
            return;
        }

        if let Some(hit) = self
            .ui_frame
            .hits
            .iter()
            .rev()
            .find(|hit| hit.rect.contains(x, y))
            .cloned()
        {
            if matches!(hit.action, Action::SelectFile(_)) {
                self.dispatch_action(Action::SetFocus(Some(FocusTarget::FileList)));
            }
            self.dispatch_action(hit.action);
            return;
        }

        if self
            .ui_frame
            .viewport_rect
            .is_some_and(|rect| rect.contains(x, y))
        {
            self.dispatch_action(Action::FocusViewport);
            let hovered = self
                .viewport_runtime
                .hit_test_row(&self.state.viewport, x, y);
            if hovered != self.state.viewport.hovered_row {
                self.dispatch_action(Action::HoverViewportRow(hovered));
            }
        }
    }

    fn handle_cursor_moved(&mut self, x: f32, y: f32) {
        self.mouse_position = Some((x, y));

        if self.sidebar_resize_drag.is_some() {
            self.handle_sidebar_resize_drag_move(x);
        }

        // Scrollbar thumb drag
        if self.scrollbar_drag.is_some() {
            self.handle_scrollbar_drag_move(y);
        }

        // Drag-to-select in text inputs
        if let Some(drag_target) = self.mouse_drag_target {
            if let Some(hit_area) = self
                .ui_frame
                .text_input_hit_areas
                .iter()
                .find(|ha| ha.focus_target == drag_target)
            {
                let byte_offset = hit_test_text_offset(
                    self.renderer.as_mut().map(|r| r.font_system()),
                    &hit_area.value,
                    hit_area.font_size,
                    x - hit_area.text_x,
                );
                self.dispatch_action(Action::ExtendTextSelection(byte_offset));
            }
        }

        let hovered_hit = self
            .ui_frame
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
        let cursor_hint = hovered_hit
            .map(|hit| hit.cursor)
            .unwrap_or(CursorHint::Default);
        let resize_hovered = self
            .ui_frame
            .sidebar_resize_handle_rect
            .is_some_and(|rect| rect.contains(x, y));
        let cursor_hint = if self.sidebar_resize_drag.is_some() || resize_hovered {
            CursorHint::ResizeCol
        } else {
            cursor_hint
        };

        if hovered_file != self.state.file_list.hovered_index {
            self.dispatch_action(Action::HoverFile(hovered_file));
        }
        let current_hovered_toast = self.state.toasts.iter().position(|toast| toast.hovered);
        if hovered_toast != current_hovered_toast {
            self.dispatch_action(Action::HoverToast(hovered_toast));
        }

        let hovered_row = if self.input_is_blocked_by_overlay(x, y) {
            None
        } else {
            self.viewport_runtime
                .hit_test_row(&self.state.viewport, x, y)
        };
        if hovered_row != self.state.viewport.hovered_row {
            self.dispatch_action(Action::HoverViewportRow(hovered_row));
        }

        if let Some(window) = self.window.as_ref() {
            let icon = match cursor_hint {
                CursorHint::Default => winit::window::CursorIcon::Default,
                CursorHint::Pointer => winit::window::CursorIcon::Pointer,
                CursorHint::Text => winit::window::CursorIcon::Text,
                CursorHint::ResizeCol => winit::window::CursorIcon::EwResize,
            };
            window.set_cursor(icon);
        }
    }

    fn handle_scroll(&mut self, delta: MouseScrollDelta, phase: TouchPhase) {
        let Some((x, y)) = self.mouse_position else {
            return;
        };

        if matches!(phase, TouchPhase::Started | TouchPhase::Cancelled) {
            self.reset_scroll_remainders();
        }

        let Some(target) = self.scroll_target_at(x, y) else {
            return;
        };
        let line_step_px = self.scroll_target_line_step_px(&target);
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
        if rounded_delta_px != 0 {
            match target {
                ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::FileList) => {
                    self.dispatch_action(Action::ScrollFileListPx(rounded_delta_px));
                }
                ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::Custom(build)) => {
                    self.dispatch_action(build(rounded_delta_px));
                }
                ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::ViewportLines)
                | ScrollTarget::ViewportFallback => {
                    self.dispatch_action(Action::ScrollViewportPx(rounded_delta_px));
                }
            }
        }

        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.reset_scroll_remainders();
        }
    }

    fn cycle_focus(&mut self) {
        let next = match self.state.overlays.top() {
            Some(OverlaySurface::CompareSheet) => match self.state.focus.current {
                Some(FocusTarget::CompareRepoButton) => Some(FocusTarget::CompareLeftRef),
                Some(FocusTarget::CompareLeftRef) => Some(FocusTarget::CompareRightRef),
                Some(FocusTarget::CompareRightRef) => Some(FocusTarget::CompareStartButton),
                _ => Some(FocusTarget::CompareRepoButton),
            },
            Some(OverlaySurface::RepoPicker | OverlaySurface::RefPicker(_)) => {
                match self.state.focus.current {
                    Some(FocusTarget::PickerInput) => Some(FocusTarget::PickerList),
                    _ => Some(FocusTarget::PickerInput),
                }
            }
            Some(OverlaySurface::CommandPalette) => match self.state.focus.current {
                Some(FocusTarget::CommandPaletteInput) => Some(FocusTarget::CommandPaletteList),
                _ => Some(FocusTarget::CommandPaletteInput),
            },
            Some(OverlaySurface::PullRequestModal) => match self.state.focus.current {
                Some(FocusTarget::PullRequestInput) => Some(FocusTarget::PullRequestConfirm),
                _ => Some(FocusTarget::PullRequestInput),
            },
            Some(OverlaySurface::GitHubAuthModal) => Some(FocusTarget::AuthPrimaryAction),
            None => match self.state.focus.current {
                Some(FocusTarget::TitleBar) => Some(FocusTarget::FileList),
                Some(FocusTarget::FileList) => Some(FocusTarget::DiffViewport),
                Some(FocusTarget::DiffViewport) => Some(FocusTarget::ThemeToggle),
                Some(FocusTarget::ThemeToggle) => Some(FocusTarget::TitleBar),
                Some(FocusTarget::WorkspacePrimaryButton) => Some(FocusTarget::TitleBar),
                _ => Some(if self.state.workspace_mode == WorkspaceMode::Ready {
                    FocusTarget::TitleBar
                } else {
                    FocusTarget::WorkspacePrimaryButton
                }),
            },
        };
        self.dispatch_action(Action::SetFocus(next));
    }

    fn activate_current_focus(&mut self) {
        match self.state.overlays.top() {
            Some(OverlaySurface::CompareSheet) => match self.state.focus.current {
                Some(FocusTarget::CompareRepoButton) => {
                    self.dispatch_action(Action::OpenRepoPicker)
                }
                Some(FocusTarget::CompareLeftRef) => self
                    .dispatch_action(Action::OpenRefPicker(crate::ui::state::CompareField::Left)),
                Some(FocusTarget::CompareRightRef) => self
                    .dispatch_action(Action::OpenRefPicker(crate::ui::state::CompareField::Right)),
                _ => self.dispatch_action(Action::StartCompare),
            },
            Some(OverlaySurface::RepoPicker | OverlaySurface::RefPicker(_))
            | Some(OverlaySurface::CommandPalette) => {
                self.dispatch_action(Action::ConfirmOverlaySelection);
            }
            Some(OverlaySurface::PullRequestModal) => {
                self.dispatch_action(Action::SubmitPullRequest);
            }
            Some(OverlaySurface::GitHubAuthModal) => {
                if self.state.github.auth.device_flow.is_some() {
                    self.dispatch_action(Action::OpenDeviceFlowBrowser);
                } else {
                    self.dispatch_action(Action::StartGitHubDeviceFlow);
                }
            }
            None => match self.state.focus.current {
                Some(FocusTarget::WorkspacePrimaryButton) => {
                    self.dispatch_action(Action::OpenCompareSheet);
                }
                Some(FocusTarget::ThemeToggle) => self.dispatch_action(Action::ToggleThemeMode),
                _ => {}
            },
        }
    }

    fn handle_scrollbar_drag_move(&mut self, mouse_y: f32) {
        let Some(ref drag) = self.scrollbar_drag else {
            return;
        };
        let track_top = drag.track_top;
        let track_h = drag.track_height;
        let thumb_h = drag.thumb_height;
        let grab_offset = drag.grab_offset;
        let content_h = drag.content_height;
        let viewport_h = drag.viewport_height;
        let builder = drag.action_builder.clone();

        // Compute new thumb position
        let thumb_top = (mouse_y - track_top - grab_offset).clamp(0.0, track_h - thumb_h);
        let max_scroll = content_h - viewport_h;
        let scroll_range = track_h - thumb_h;
        let fraction = if scroll_range > 0.0 {
            thumb_top / scroll_range
        } else {
            0.0
        };

        let target_px = (fraction * max_scroll) as u32;

        // Dispatch scroll action based on the builder type
        match builder {
            crate::ui::element::ScrollActionBuilder::FileList => {
                self.dispatch_action(Action::ScrollFileListToPx(target_px));
            }
            crate::ui::element::ScrollActionBuilder::ViewportLines => {
                self.dispatch_action(Action::ScrollViewportTo(target_px));
            }
            crate::ui::element::ScrollActionBuilder::Custom(_) => {}
        }
    }

    fn handle_sidebar_resize_drag_move(&mut self, mouse_x: f32) {
        let Some(ref drag) = self.sidebar_resize_drag else {
            return;
        };

        let target_width = (drag.starting_width + (mouse_x - drag.origin_x))
            .round()
            .max(0.0) as u32;
        self.dispatch_action(Action::SetSidebarWidthPx(target_width));
    }

    fn is_text_focused(&self) -> bool {
        self.state.is_text_focused()
    }

    fn reset_scroll_remainders(&mut self) {
        self.file_list_scroll_remainder_px = 0.0;
        self.overlay_scroll_remainder_px = 0.0;
        self.viewport_scroll_remainder_px = 0.0;
    }

    fn scroll_target_at(&self, x: f32, y: f32) -> Option<ScrollTarget> {
        for region in self.ui_frame.scroll_regions.iter().rev() {
            if region.bounds.contains(x, y) {
                return Some(ScrollTarget::Region(region.action_builder.clone()));
            }
        }

        if self.input_is_blocked_by_overlay(x, y) {
            return None;
        }

        self.ui_frame
            .viewport_rect
            .filter(|rect| rect.contains(x, y))
            .map(|_| ScrollTarget::ViewportFallback)
    }

    fn scroll_target_line_step_px(&self, target: &ScrollTarget) -> f32 {
        match target {
            ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::FileList) => {
                self.state.file_list.row_stride().max(1.0)
            }
            ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::Custom(_)) => {
                self.active_overlay_row_height_px()
            }
            ScrollTarget::Region(crate::ui::element::ScrollActionBuilder::ViewportLines)
            | ScrollTarget::ViewportFallback => self.viewport_runtime.scroll_line_height_px(),
        }
    }

    fn active_overlay_row_height_px(&self) -> f32 {
        match self.state.overlays.top() {
            Some(OverlaySurface::RepoPicker | OverlaySurface::RefPicker(_)) => {
                self.state.overlays.picker.list.row_height_px.max(1) as f32
            }
            Some(OverlaySurface::CommandPalette) => self
                .state
                .overlays
                .command_palette
                .list
                .row_height_px
                .max(1) as f32,
            _ => 36.0,
        }
    }

    fn input_is_blocked_by_overlay(&self, x: f32, y: f32) -> bool {
        self.state.overlays.top().is_some()
            && self
                .ui_frame
                .hits
                .iter()
                .rev()
                .any(|hit| hit.rect.contains(x, y))
    }

    fn handle_key(&mut self, key: &Key) {
        let ctrl = self.modifiers.control_key() || self.modifiers.super_key();
        let shift = self.modifiers.shift_key();

        // Global shortcuts
        if let Key::Character(text) = key {
            let lower = text.to_ascii_lowercase();
            if ctrl {
                match lower.as_str() {
                    "p" => {
                        self.dispatch_action(Action::OpenCommandPalette);
                        return;
                    }
                    "=" | "+" => {
                        self.dispatch_action(Action::IncreaseUiScale);
                        return;
                    }
                    "-" | "_" => {
                        self.dispatch_action(Action::DecreaseUiScale);
                        return;
                    }
                    _ => {}
                }
            }
            // Clipboard shortcuts (when text is focused)
            if ctrl && self.is_text_focused() {
                match lower.as_str() {
                    "a" => {
                        self.dispatch_action(Action::SelectAll);
                        return;
                    }
                    "c" => {
                        self.dispatch_action(Action::Copy);
                        return;
                    }
                    "x" => {
                        self.dispatch_action(Action::Cut);
                        return;
                    }
                    "v" => {
                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                            if let Ok(text) = clipboard.get_text() {
                                self.dispatch_action(Action::Paste(text));
                            }
                        }
                        return;
                    }
                    _ => {}
                }
            }
        }

        match key {
            Key::Named(NamedKey::Escape) => {
                if self.state.overlays.top().is_some() {
                    self.dispatch_action(Action::CloseOverlay);
                }
            }
            Key::Named(NamedKey::Tab) => self.cycle_focus(),
            Key::Named(NamedKey::Enter) => self.activate_current_focus(),

            // Arrow keys: text cursor when text-focused, else overlay/viewport nav
            Key::Named(NamedKey::ArrowLeft) if self.is_text_focused() => {
                let action = match (ctrl, shift) {
                    (true, true) => Action::SelectWordLeft,
                    (true, false) => Action::CursorWordLeft,
                    (false, true) => Action::SelectLeft,
                    (false, false) => Action::CursorLeft,
                };
                self.dispatch_action(action);
            }
            Key::Named(NamedKey::ArrowRight) if self.is_text_focused() => {
                let action = match (ctrl, shift) {
                    (true, true) => Action::SelectWordRight,
                    (true, false) => Action::CursorWordRight,
                    (false, true) => Action::SelectRight,
                    (false, false) => Action::CursorRight,
                };
                self.dispatch_action(action);
            }
            Key::Named(NamedKey::Home) if self.is_text_focused() => {
                self.dispatch_action(if shift {
                    Action::SelectHome
                } else {
                    Action::CursorHome
                });
            }
            Key::Named(NamedKey::End) if self.is_text_focused() => {
                self.dispatch_action(if shift {
                    Action::SelectEnd
                } else {
                    Action::CursorEnd
                });
            }

            Key::Named(NamedKey::ArrowDown) => {
                if self.state.overlays.top().is_some() {
                    self.dispatch_action(Action::MoveOverlaySelection(1));
                } else if self.state.focus.current == Some(FocusTarget::DiffViewport) {
                    self.dispatch_action(Action::ScrollViewportLines(1));
                } else if self.state.workspace_mode == WorkspaceMode::Ready {
                    self.dispatch_action(Action::SelectNextFile);
                }
            }
            Key::Named(NamedKey::ArrowUp) => {
                if self.state.overlays.top().is_some() {
                    self.dispatch_action(Action::MoveOverlaySelection(-1));
                } else if self.state.focus.current == Some(FocusTarget::DiffViewport) {
                    self.dispatch_action(Action::ScrollViewportLines(-1));
                } else if self.state.workspace_mode == WorkspaceMode::Ready {
                    self.dispatch_action(Action::SelectPreviousFile);
                }
            }
            Key::Named(NamedKey::PageDown) if self.state.workspace_mode == WorkspaceMode::Ready => {
                if self.state.focus.current == Some(FocusTarget::DiffViewport) {
                    self.dispatch_action(Action::ScrollViewportPages(1));
                } else {
                    self.dispatch_action(Action::ScrollFileList(10));
                }
            }
            Key::Named(NamedKey::PageUp) if self.state.workspace_mode == WorkspaceMode::Ready => {
                if self.state.focus.current == Some(FocusTarget::DiffViewport) {
                    self.dispatch_action(Action::ScrollViewportPages(-1));
                } else {
                    self.dispatch_action(Action::ScrollFileList(-10));
                }
            }
            Key::Named(NamedKey::Home) if self.state.workspace_mode == WorkspaceMode::Ready => {
                self.dispatch_action(Action::ScrollViewportTo(0));
            }
            Key::Named(NamedKey::End) if self.state.workspace_mode == WorkspaceMode::Ready => {
                self.dispatch_action(Action::ScrollViewportTo(
                    self.state.viewport.max_scroll_top_px(),
                ));
            }
            Key::Named(NamedKey::Backspace) => self.dispatch_action(Action::Backspace),
            Key::Named(NamedKey::Delete) => self.dispatch_action(Action::DeleteForward),
            Key::Character(text) => {
                if !ctrl && !text.chars().all(char::is_control) {
                    self.dispatch_action(Action::InsertText(text.to_string()));
                }
            }
            _ => {}
        }
    }

    fn should_exit(&self) -> bool {
        self.state
            .startup
            .exit_after
            .is_some_and(|exit_after| self.launch_at.elapsed() >= exit_after)
    }
}

impl ApplicationHandler for NativeApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        match event_loop.create_window(self.window_attributes()) {
            Ok(window) => {
                let window = Arc::new(window);
                let size = window.inner_size();
                let scale_factor = window.scale_factor();
                match Renderer::new(window.clone()) {
                    Ok(mut renderer) => {
                        renderer.resize(size.width, size.height, scale_factor);
                        self.renderer = Some(renderer);
                        self.window = Some(window);
                    }
                    Err(error) => {
                        eprintln!("failed to create renderer: {error}");
                        event_loop.exit();
                        return;
                    }
                }
                self.refresh_window_title();
                self.write_dumps_if_needed();
            }
            Err(error) => {
                eprintln!("failed to create native window: {error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window_id() != Some(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                self.write_dumps_if_needed();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let (Some(renderer), Some(window)) =
                    (self.renderer.as_mut(), self.window.as_ref())
                {
                    renderer.resize(size.width, size.height, window.scale_factor());
                }
                self.dumps_dirty = true;
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::RedrawRequested => {
                let frame_started_at = Instant::now();
                let frame = self.build_frame();
                self.ui_frame = frame;
                if let Some(renderer) = self.renderer.as_mut() {
                    let time_seconds = self.launch_at.elapsed().as_secs_f32();
                    match renderer.render(&self.ui_frame.scene, time_seconds) {
                        Ok(frame) => {
                            self.state.debug.last_scene_primitive_count = frame.primitive_count;
                            self.state.debug.last_frame_time_us = frame_started_at
                                .elapsed()
                                .as_micros()
                                .min(u128::from(u64::MAX))
                                as u64;
                        }
                        Err(error) => {
                            eprintln!("render failed: {error}");
                            self.state.last_error = Some(error.to_string());
                        }
                    }
                }
                // Capture scene to PNG if DIFFY_CAPTURE_PATH is set.
                #[cfg(feature = "capture")]
                if let Some(path) = self.capture_pending.take() {
                    let size = self.window.as_ref().map(|w| w.inner_size());
                    let (w, h) = size.map(|s| (s.width, s.height)).unwrap_or((1320, 840));
                    crate::render::capture::scene_to_png(&self.ui_frame.scene, w, h, &path);
                    eprintln!("captured: {}", path.display());
                }

                self.dumps_dirty = true;
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.handle_cursor_moved(position.x as f32, position.y as f32);
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                self.handle_scroll(delta, phase);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some((x, y)) = self.mouse_position {
                    self.handle_left_click(x, y);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                if self.sidebar_resize_drag.take().is_some() {
                    self.runtime
                        .dispatch_all(vec![crate::ui::effects::Effect::SaveSettings(
                            self.state.settings.clone(),
                        )]);
                    self.dumps_dirty = true;
                }
                self.mouse_drag_target = None;
                self.scrollbar_drag = None;
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.handle_key(&event.logical_key);
            }
            WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                self.dispatch_action(Action::InsertText(text));
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.state.update_time(
            self.launch_at
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
        );
        self.process_runtime_events();
        self.write_dumps_if_needed();

        if self.should_exit() {
            if let Some(window) = self.window.as_ref() {
                window.set_visible(false);
            }
            event_loop.exit();
            return;
        }

        let animating = self.state.animation.has_active();
        let cursor_blinking = self.is_text_focused();
        let should_poll = self.state.startup.exit_after.is_some();
        if animating {
            let next = std::time::Instant::now() + std::time::Duration::from_millis(16);
            event_loop.set_control_flow(ControlFlow::WaitUntil(next));
        } else if cursor_blinking {
            // Redraw at ~2fps for cursor blink
            let next = std::time::Instant::now() + std::time::Duration::from_millis(530);
            event_loop.set_control_flow(ControlFlow::WaitUntil(next));
        } else if should_poll {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

#[derive(Debug, Clone)]
enum ScrollTarget {
    Region(crate::ui::element::ScrollActionBuilder),
    ViewportFallback,
}

fn scroll_delta_to_px(delta: MouseScrollDelta, line_step_px: f32) -> f32 {
    match delta {
        MouseScrollDelta::LineDelta(_, y) => -y * line_step_px,
        MouseScrollDelta::PixelDelta(position) => -(position.y as f32),
    }
}

fn scale_text_metrics(
    metrics: crate::render::TextMetrics,
    scale: f32,
) -> crate::render::TextMetrics {
    let scale = scale.clamp(0.7, 1.8);
    crate::render::TextMetrics {
        ui_font_size_px: metrics.ui_font_size_px * scale,
        ui_line_height_px: metrics.ui_line_height_px * scale,
        mono_font_size_px: metrics.mono_font_size_px * scale,
        mono_line_height_px: metrics.mono_line_height_px * scale,
        mono_char_width_px: metrics.mono_char_width_px * scale,
    }
}

fn quantize_scroll_delta_px(remainder_px: &mut f32, delta_px: f32) -> i32 {
    *remainder_px += delta_px;
    let whole_px = remainder_px.trunc() as i32;
    *remainder_px -= whole_px as f32;
    whole_px
}

/// Map a click x-coordinate (relative to text start) to a byte offset in the string.
fn hit_test_text_offset(
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

    // Shape the text into a glyphon buffer and walk glyphs to find the offset
    let metrics = glyphon::Metrics::new(font_size, font_size * 1.2);
    let mut buffer = glyphon::Buffer::new(font_system, metrics);
    let attrs = glyphon::Attrs::new().family(glyphon::Family::SansSerif);
    buffer.set_size(font_system, None, None);
    buffer.set_text(font_system, text, &attrs, glyphon::Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);

    // Walk glyphs to find the click position
    let mut best_offset = text.len();
    let mut best_dist = f32::MAX;

    for run in buffer.layout_runs() {
        // Check position 0 (before first glyph)
        let dist = click_x.abs();
        if dist < best_dist {
            best_dist = dist;
            best_offset = 0;
        }
        for glyph in run.glyphs.iter() {
            // Check left edge of glyph
            let left_dist = (click_x - glyph.x).abs();
            if left_dist < best_dist {
                best_dist = left_dist;
                best_offset = glyph.start;
            }
            // Check right edge of glyph
            let right_dist = (click_x - (glyph.x + glyph.w)).abs();
            if right_dist < best_dist {
                best_dist = right_dist;
                best_offset = glyph.end;
            }
        }
        // Check end of run
        let dist = (click_x - run.line_w).abs();
        if dist < best_dist {
            best_dist = dist;
            best_offset = text.len();
        }
    }

    best_offset
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;
    use winit::dpi::PhysicalPosition;
    use winit::event::{MouseScrollDelta, TouchPhase};
    use winit::keyboard::{Key, ModifiersState};

    use super::{NativeApp, ScrollTarget, quantize_scroll_delta_px, scroll_delta_to_px};
    use crate::app_runtime::{AppRuntime, AppServices};
    use crate::platform::persistence::SettingsStore;
    use crate::ui::actions::Action;
    use crate::ui::state::{
        AppState, FileListEntry, FocusTarget, OverlayEntry, OverlaySurface, WorkspaceMode,
    };

    fn test_app(state: AppState) -> NativeApp {
        let dir = TempDir::new().unwrap();
        let runtime = AppRuntime::new(AppServices::new(SettingsStore::new_in(dir.path())));
        NativeApp::new(state, runtime)
    }

    #[test]
    fn scroll_delta_to_px_preserves_magnitude_and_direction() {
        let line_delta = scroll_delta_to_px(MouseScrollDelta::LineDelta(0.0, 1.5), 20.0);
        assert_eq!(line_delta, -30.0);

        let pixel_delta = scroll_delta_to_px(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -12.5)),
            20.0,
        );
        assert_eq!(pixel_delta, 12.5);
    }

    #[test]
    fn quantize_scroll_delta_px_accumulates_fractional_motion() {
        let mut remainder = 0.0;

        assert_eq!(quantize_scroll_delta_px(&mut remainder, 0.4), 0);
        assert!((remainder - 0.4).abs() < f32::EPSILON);

        assert_eq!(quantize_scroll_delta_px(&mut remainder, 0.8), 1);
        assert!((remainder - 0.2).abs() < f32::EPSILON);

        assert_eq!(quantize_scroll_delta_px(&mut remainder, -0.6), 0);
        assert!((remainder - (-0.4)).abs() < f32::EPSILON);

        assert_eq!(quantize_scroll_delta_px(&mut remainder, -0.8), -1);
        assert!((remainder - (-0.2)).abs() < f32::EPSILON);
    }

    #[test]
    fn file_list_scroll_region_wins_over_viewport_fallback() {
        let mut state = AppState::default();
        state.workspace_mode = WorkspaceMode::Ready;
        state.workspace.files = (0..12)
            .map(|index| FileListEntry {
                path: format!("src/file_{index}.rs"),
                status: "M".to_owned(),
                additions: 1,
                deletions: 0,
                is_binary: false,
            })
            .collect();
        state.workspace.selected_file_index = Some(0);
        state.workspace.selected_file_path = Some("src/file_0.rs".to_owned());

        let mut app = test_app(state);
        app.ui_frame = app.build_frame();

        let region = app
            .ui_frame
            .scroll_regions
            .iter()
            .find(|region| {
                matches!(
                    region.action_builder,
                    crate::ui::element::ScrollActionBuilder::FileList
                )
            })
            .unwrap();
        let x = region.bounds.x + 10.0;
        let y = region.bounds.y + 10.0;

        assert!(matches!(
            app.scroll_target_at(x, y),
            Some(ScrollTarget::Region(
                crate::ui::element::ScrollActionBuilder::FileList
            ))
        ));
    }

    #[test]
    fn file_list_wheel_scroll_moves_sidebar_contents() {
        let mut state = AppState::default();
        state.workspace_mode = WorkspaceMode::Ready;
        state.workspace.files = (0..32)
            .map(|index| FileListEntry {
                path: format!("src/file_{index}.rs"),
                status: "M".to_owned(),
                additions: 1,
                deletions: 0,
                is_binary: false,
            })
            .collect();
        state.workspace.selected_file_index = Some(0);
        state.workspace.selected_file_path = Some("src/file_0.rs".to_owned());

        let mut app = test_app(state);
        app.ui_frame = app.build_frame();

        let region = app
            .ui_frame
            .scroll_regions
            .iter()
            .find(|region| {
                matches!(
                    region.action_builder,
                    crate::ui::element::ScrollActionBuilder::FileList
                )
            })
            .unwrap();
        app.mouse_position = Some((region.bounds.x + 10.0, region.bounds.y + 10.0));

        app.handle_scroll(MouseScrollDelta::LineDelta(0.0, -3.0), TouchPhase::Moved);

        assert!(app.state.file_list.scroll_offset_px > 0.0);
    }

    #[test]
    fn overlay_blocks_viewport_scroll_fallback() {
        let mut state = AppState::default();
        state.workspace_mode = WorkspaceMode::Ready;
        state.workspace.files = vec![FileListEntry {
            path: "src/file_0.rs".to_owned(),
            status: "M".to_owned(),
            additions: 1,
            deletions: 0,
            is_binary: false,
        }];
        state.workspace.selected_file_index = Some(0);
        state.workspace.selected_file_path = Some("src/file_0.rs".to_owned());
        state.overlays.stack.push(OverlayEntry {
            surface: OverlaySurface::CompareSheet,
            focus_return: Some(FocusTarget::TitleBar),
        });

        let mut app = test_app(state);
        app.ui_frame = app.build_frame();
        let overlay_hit = app
            .ui_frame
            .hits
            .iter()
            .rev()
            .find(|hit| matches!(hit.action, Action::CloseOverlay | Action::Noop))
            .expect("overlay hit");
        let x = overlay_hit.rect.x + overlay_hit.rect.width * 0.5;
        let y = overlay_hit.rect.y + overlay_hit.rect.height * 0.5;

        assert!(app.input_is_blocked_by_overlay(x, y));
        assert!(app.scroll_target_at(x, y).is_none());
    }

    #[test]
    fn command_shortcuts_adjust_ui_scale() {
        let mut app = test_app(AppState::default());
        app.modifiers = ModifiersState::SUPER;

        app.handle_key(&Key::Character("=".into()));
        assert_eq!(app.state.settings.ui_scale_pct, 110);

        app.handle_key(&Key::Character("-".into()));
        assert_eq!(app.state.settings.ui_scale_pct, 100);
    }
}

fn init_logging(log_debug: bool) {
    let mut builder = env_logger::Builder::from_default_env();
    if log_debug {
        builder.filter_level(log::LevelFilter::Debug);
    }
    let _ = builder.is_test(false).try_init();
}
