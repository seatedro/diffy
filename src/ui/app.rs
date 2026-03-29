use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
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
}

impl NativeApp {
    fn new(state: AppState, runtime: AppRuntime) -> Self {
        let theme = Theme::for_mode(state.settings.theme_mode);
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
        }
    }

    fn sync_theme(&mut self) {
        self.theme = Theme::for_mode(self.state.settings.theme_mode);
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
            fallback_font_system = glyphon::FontSystem::new();
            &mut fallback_font_system
        };

        let width = size.width.max(1) as f32;
        let height = size.height.max(1) as f32;

        let mut cx = crate::ui::element::ElementContext::new(
            &self.theme,
            scale_factor,
            font_system,
            self.mouse_position,
            &mut self.signal_store,
        )
        .with_focus(self.state.focus.current);

        build_ui_frame(
            &mut self.state,
            &self.theme,
            &mut self.viewport_runtime,
            text_metrics,
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

        if hovered_file != self.state.file_list.hovered_index {
            self.dispatch_action(Action::HoverFile(hovered_file));
        }
        let current_hovered_toast = self.state.toasts.iter().position(|toast| toast.hovered);
        if hovered_toast != current_hovered_toast {
            self.dispatch_action(Action::HoverToast(hovered_toast));
        }

        let hovered_row = self
            .viewport_runtime
            .hit_test_row(&self.state.viewport, x, y);
        if hovered_row != self.state.viewport.hovered_row {
            self.dispatch_action(Action::HoverViewportRow(hovered_row));
        }

        if let Some(window) = self.window.as_ref() {
            let icon = match cursor_hint {
                CursorHint::Default => winit::window::CursorIcon::Default,
                CursorHint::Pointer => winit::window::CursorIcon::Pointer,
                CursorHint::Text => winit::window::CursorIcon::Text,
            };
            window.set_cursor(icon);
        }
    }

    fn handle_scroll(&mut self, delta: MouseScrollDelta) {
        let Some((x, y)) = self.mouse_position else {
            return;
        };
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => {
                if y < 0.0 {
                    3
                } else if y > 0.0 {
                    -3
                } else {
                    0
                }
            }
            MouseScrollDelta::PixelDelta(position) => {
                let amount = (position.y / 36.0).round() as i32;
                if amount == 0 { 0 } else { amount }
            }
        };
        if lines == 0 {
            return;
        }

        // Check scroll regions registered by the element system.
        for region in self.ui_frame.scroll_regions.iter().rev() {
            if region.bounds.contains(x, y) {
                let action = region.action_builder.build(lines);
                self.dispatch_action(action);
                return;
            }
        }

        // Fallback: viewport scroll.
        if self
            .ui_frame
            .viewport_rect
            .is_some_and(|rect| rect.contains(x, y))
        {
            self.dispatch_action(Action::ScrollViewportLines(lines));
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

    fn handle_key(&mut self, key: &Key) {
        if let Key::Character(text) = key {
            let lower = text.to_ascii_lowercase();
            if (self.modifiers.control_key() || self.modifiers.super_key()) && lower == "p" {
                self.dispatch_action(Action::OpenCommandPalette);
                return;
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
            Key::Character(text) => {
                if !(self.modifiers.control_key() || self.modifiers.super_key())
                    && !text.chars().all(char::is_control)
                {
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
                self.dumps_dirty = true;
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.handle_cursor_moved(position.x as f32, position.y as f32);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_scroll(delta);
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
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.handle_key(&event.logical_key);
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
        let should_poll = self.state.startup.exit_after.is_some();
        if animating {
            let next = std::time::Instant::now() + std::time::Duration::from_millis(16);
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

fn init_logging(log_debug: bool) {
    let mut builder = env_logger::Builder::from_default_env();
    if log_debug {
        builder.filter_level(log::LevelFilter::Debug);
    }
    let _ = builder.is_test(false).try_init();
}
