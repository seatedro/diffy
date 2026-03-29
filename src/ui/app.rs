use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::app_runtime::{AppRuntime, AppServices};
use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
use crate::platform::automation::{ErrorDump, FilesDump, StateDump, write_json};
use crate::platform::persistence::SettingsStore;
use crate::platform::startup::StartupOptions;
use crate::render::{
    BorderPrimitive, FontKind, Rect, RectPrimitive, Renderer, RoundedRectPrimitive, Scene,
    ShadowPrimitive, TextMetrics, TextPrimitive,
};
use crate::ui::actions::Action;
use crate::ui::diff_viewport::runtime::{DiffViewportRuntime, ViewportDocument};
use crate::ui::state::{AppState, FocusTarget, Screen, ToastKind};
use crate::ui::theme::{Color, Theme};

#[derive(Debug, Clone)]
struct HitRegion {
    rect: Rect,
    action: Action,
    hover_file_index: Option<usize>,
}

#[derive(Debug, Clone, Default)]
struct UiFrame {
    scene: Scene,
    hits: Vec<HitRegion>,
    file_list_rect: Option<Rect>,
    viewport_rect: Option<Rect>,
}

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
    launch_at: Instant,
    dumps_dirty: bool,
}

impl NativeApp {
    fn new(state: AppState, runtime: AppRuntime) -> Self {
        Self {
            state,
            theme: Theme::default_dark(),
            runtime,
            renderer: None,
            window: None,
            ui_frame: UiFrame::default(),
            viewport_runtime: DiffViewportRuntime::default(),
            mouse_position: None,
            launch_at: Instant::now(),
            dumps_dirty: true,
        }
    }

    fn window_attributes(&self) -> WindowAttributes {
        Window::default_attributes()
            .with_title(self.state.window_title())
            .with_visible(!self.state.startup.hidden_window)
            .with_inner_size(LogicalSize::new(1280.0, 800.0))
            .with_min_inner_size(LogicalSize::new(960.0, 640.0))
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
            .unwrap_or_else(|| winit::dpi::PhysicalSize::new(1280, 800));
        let text_metrics = self
            .renderer
            .as_ref()
            .map(Renderer::text_metrics)
            .unwrap_or_default();
        build_ui_frame(
            &mut self.state,
            &self.theme,
            &mut self.viewport_runtime,
            text_metrics,
            size.width.max(1) as f32,
            size.height.max(1) as f32,
        )
    }

    fn dispatch_action(&mut self, action: Action) {
        let effects = self.state.apply_action(action);
        self.runtime.dispatch_all(effects);
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
            if hit.hover_file_index.is_some() {
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
        let hovered = self
            .ui_frame
            .hits
            .iter()
            .rev()
            .find(|hit| hit.rect.contains(x, y))
            .and_then(|hit| hit.hover_file_index);
        if hovered != self.state.file_list.hovered_index {
            self.dispatch_action(Action::HoverFile(hovered));
        }

        let hovered_row = self
            .viewport_runtime
            .hit_test_row(&self.state.viewport, x, y);
        if hovered_row != self.state.viewport.hovered_row {
            self.dispatch_action(Action::HoverViewportRow(hovered_row));
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

        if self
            .ui_frame
            .file_list_rect
            .is_some_and(|rect| rect.contains(x, y))
        {
            self.dispatch_action(Action::ScrollFileList(lines));
        } else if self
            .ui_frame
            .viewport_rect
            .is_some_and(|rect| rect.contains(x, y))
        {
            self.dispatch_action(Action::ScrollViewportLines(lines));
        }
    }

    fn move_overlay_selection(&mut self, delta: i32) {
        if self.state.overlay.ref_suggestions.is_empty() {
            return;
        }
        let current = self.state.overlay.selected_index as i32;
        let max = self.state.overlay.ref_suggestions.len().saturating_sub(1) as i32;
        self.state.overlay.selected_index = (current + delta).clamp(0, max) as usize;
        self.dumps_dirty = true;
    }

    fn cycle_focus(&mut self) {
        let next = match self.state.current_screen {
            Screen::Welcome => Some(FocusTarget::OpenRepositoryButton),
            Screen::Compare => match self.state.focus.current {
                Some(FocusTarget::LeftRef) => Some(FocusTarget::RightRef),
                Some(FocusTarget::RightRef) => Some(FocusTarget::StartCompare),
                _ => Some(FocusTarget::LeftRef),
            },
            Screen::Diff => match self.state.focus.current {
                Some(FocusTarget::FileList) => Some(FocusTarget::DiffViewport),
                _ => Some(FocusTarget::FileList),
            },
        };
        self.dispatch_action(Action::SetFocus(next));
    }

    fn activate_current_focus(&mut self) {
        match self.state.current_screen {
            Screen::Welcome => self.dispatch_action(Action::OpenRepositoryDialog),
            Screen::Compare => {
                if !self.state.overlay.ref_suggestions.is_empty()
                    && self.state.overlay.active_field.is_some()
                {
                    self.dispatch_action(Action::SelectRefSuggestion(
                        self.state.overlay.selected_index,
                    ));
                } else {
                    self.dispatch_action(Action::StartCompare);
                }
            }
            Screen::Diff => {}
        }
    }

    fn handle_key(&mut self, key: &Key) {
        match key {
            Key::Named(NamedKey::ArrowDown) => match self.state.current_screen {
                Screen::Compare if !self.state.overlay.ref_suggestions.is_empty() => {
                    self.move_overlay_selection(1);
                }
                Screen::Diff if self.state.focus.current == Some(FocusTarget::DiffViewport) => {
                    self.dispatch_action(Action::ScrollViewportLines(1));
                }
                Screen::Diff => self.dispatch_action(Action::SelectNextFile),
                _ => {}
            },
            Key::Named(NamedKey::ArrowUp) => match self.state.current_screen {
                Screen::Compare if !self.state.overlay.ref_suggestions.is_empty() => {
                    self.move_overlay_selection(-1);
                }
                Screen::Diff if self.state.focus.current == Some(FocusTarget::DiffViewport) => {
                    self.dispatch_action(Action::ScrollViewportLines(-1));
                }
                Screen::Diff => self.dispatch_action(Action::SelectPreviousFile),
                _ => {}
            },
            Key::Named(NamedKey::PageDown) if self.state.current_screen == Screen::Diff => {
                if self.state.focus.current == Some(FocusTarget::DiffViewport) {
                    self.dispatch_action(Action::ScrollViewportPages(1));
                } else {
                    self.dispatch_action(Action::ScrollFileList(10));
                }
            }
            Key::Named(NamedKey::PageUp) if self.state.current_screen == Screen::Diff => {
                if self.state.focus.current == Some(FocusTarget::DiffViewport) {
                    self.dispatch_action(Action::ScrollViewportPages(-1));
                } else {
                    self.dispatch_action(Action::ScrollFileList(-10));
                }
            }
            Key::Named(NamedKey::Home) if self.state.current_screen == Screen::Diff => {
                self.dispatch_action(Action::ScrollViewportTo(0));
            }
            Key::Named(NamedKey::End) if self.state.current_screen == Screen::Diff => {
                self.dispatch_action(Action::ScrollViewportTo(
                    self.state.viewport.max_scroll_top_px(),
                ));
            }
            Key::Named(NamedKey::Tab) => self.cycle_focus(),
            Key::Named(NamedKey::Enter) => self.activate_current_focus(),
            Key::Named(NamedKey::Escape) => {
                self.state.overlay.active_field = None;
                self.state.overlay.ref_suggestions.clear();
                self.dumps_dirty = true;
            }
            Key::Named(NamedKey::Backspace) => self.dispatch_action(Action::Backspace),
            Key::Character(text) => {
                if !text.chars().all(char::is_control) {
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
            WindowEvent::RedrawRequested => {
                let frame_started_at = Instant::now();
                let frame = self.build_frame();
                self.ui_frame = frame;
                if let Some(renderer) = self.renderer.as_mut() {
                    match renderer.render(&self.ui_frame.scene) {
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
        self.process_runtime_events();
        self.write_dumps_if_needed();

        if self.should_exit() {
            if let Some(window) = self.window.as_ref() {
                window.set_visible(false);
            }
            event_loop.exit();
            return;
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

fn build_ui_frame(
    state: &mut AppState,
    theme: &Theme,
    viewport_runtime: &mut DiffViewportRuntime,
    text_metrics: TextMetrics,
    width: f32,
    height: f32,
) -> UiFrame {
    let mut frame = UiFrame::default();
    frame.scene.rect(RectPrimitive {
        rect: Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        },
        color: theme.colors.app_bg,
    });

    match state.current_screen {
        Screen::Welcome => build_welcome_frame(&mut frame, state, theme, width, height),
        Screen::Compare => build_compare_frame(&mut frame, state, theme, width, height),
        Screen::Diff => build_diff_frame(
            &mut frame,
            state,
            theme,
            viewport_runtime,
            text_metrics,
            width,
            height,
        ),
    }

    draw_toasts(&mut frame, state, theme, width, height);
    frame
}

fn draw_toasts(frame: &mut UiFrame, state: &AppState, theme: &Theme, width: f32, height: f32) {
    let toast_width = 360.0_f32.min((width - 32.0).max(200.0));
    let toast_height = 54.0;
    let gap = 10.0;
    for (offset, (index, toast)) in state.toasts.iter().enumerate().rev().enumerate() {
        let rect = Rect {
            x: width - toast_width - 16.0,
            y: height - 16.0 - toast_height - offset as f32 * (toast_height + gap),
            width: toast_width,
            height: toast_height,
        };
        let color = match toast.kind {
            ToastKind::Info => theme.colors.panel_strong,
            ToastKind::Error => Color::rgba(0x52, 0x2c, 0x30, 0xff),
        };
        draw_surface(frame, rect, color, theme.colors.border_soft);
        draw_text(
            &mut frame.scene,
            pad_rect(rect, 16.0, 14.0, 16.0, 14.0),
            &toast.message,
            theme.colors.text_strong,
            14.0,
            FontKind::Ui,
        );
        frame.hits.push(HitRegion {
            rect,
            action: Action::DismissToast(index),
            hover_file_index: None,
        });
    }
}

fn draw_surface(frame: &mut UiFrame, rect: Rect, fill: Color, border: Color) {
    frame.scene.shadow(ShadowPrimitive {
        rect,
        blur_radius: 10.0,
        color: Color::rgba(0, 0, 0, 70),
    });
    frame.scene.rounded_rect(RoundedRectPrimitive {
        rect,
        radius: 12.0,
        color: fill,
    });
    frame.scene.border(BorderPrimitive {
        rect,
        width: 1.0,
        color: border,
    });
}

fn draw_button(
    frame: &mut UiFrame,
    rect: Rect,
    label: &str,
    action: Action,
    theme: &Theme,
    active: bool,
    focused: bool,
) {
    let fill = if active {
        theme.colors.accent
    } else if focused {
        theme.colors.panel_strong
    } else {
        theme.colors.panel
    };
    draw_surface(frame, rect, fill, theme.colors.border_soft);
    draw_text(
        &mut frame.scene,
        pad_rect(rect, 14.0, 12.0, 14.0, 12.0),
        label,
        theme.colors.text_strong,
        14.0,
        FontKind::Ui,
    );
    frame.hits.push(HitRegion {
        rect,
        action,
        hover_file_index: None,
    });
}

fn draw_input(
    frame: &mut UiFrame,
    rect: Rect,
    label: &str,
    value: &str,
    placeholder: &str,
    focus_target: FocusTarget,
    theme: &Theme,
    focused: bool,
) {
    let fill = if focused {
        theme.colors.panel_strong
    } else {
        theme.colors.panel
    };
    draw_surface(frame, rect, fill, theme.colors.border_soft);
    draw_text(
        &mut frame.scene,
        Rect {
            x: rect.x + 14.0,
            y: rect.y + 8.0,
            width: rect.width - 28.0,
            height: 16.0,
        },
        label,
        theme.colors.text_muted,
        12.0,
        FontKind::Ui,
    );
    let display = if value.is_empty() { placeholder } else { value };
    draw_text(
        &mut frame.scene,
        Rect {
            x: rect.x + 14.0,
            y: rect.y + 24.0,
            width: rect.width - 28.0,
            height: 20.0,
        },
        display,
        if value.is_empty() {
            Color::rgba(
                theme.colors.text_muted.r,
                theme.colors.text_muted.g,
                theme.colors.text_muted.b,
                180,
            )
        } else {
            theme.colors.text_strong
        },
        14.0,
        FontKind::Mono,
    );
    frame.hits.push(HitRegion {
        rect,
        action: Action::SetFocus(Some(focus_target)),
        hover_file_index: None,
    });
}

fn draw_text(
    scene: &mut Scene,
    rect: Rect,
    text: &str,
    color: Color,
    font_size: f32,
    font_kind: FontKind,
) {
    scene.text(TextPrimitive {
        rect,
        text: text.to_owned(),
        color,
        font_size,
        font_kind,
    });
}

fn pad_rect(rect: Rect, left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect {
        x: rect.x + left,
        y: rect.y + top,
        width: (rect.width - left - right).max(0.0),
        height: (rect.height - top - bottom).max(0.0),
    }
}

fn rect_from_layout(layout: &taffy::Layout) -> Rect {
    Rect {
        x: layout.location.x,
        y: layout.location.y,
        width: layout.size.width,
        height: layout.size.height,
    }
}

fn build_welcome_frame(
    frame: &mut UiFrame,
    state: &AppState,
    theme: &Theme,
    width: f32,
    height: f32,
) {
    let content = layout_welcome(width, height, state.settings.recent_repos.len());
    draw_surface(
        frame,
        content,
        theme.colors.canvas,
        theme.colors.border_soft,
    );

    draw_text(
        &mut frame.scene,
        Rect {
            x: content.x + 24.0,
            y: content.y + 24.0,
            width: content.width - 48.0,
            height: 34.0,
        },
        "diffy native",
        theme.colors.text_strong,
        28.0,
        FontKind::Ui,
    );
    draw_text(
        &mut frame.scene,
        Rect {
            x: content.x + 24.0,
            y: content.y + 62.0,
            width: content.width - 48.0,
            height: 18.0,
        },
        "Open a repository and jump into a native Rust shell.",
        theme.colors.text_muted,
        14.0,
        FontKind::Ui,
    );

    let open_button = Rect {
        x: content.x + 24.0,
        y: content.y + 104.0,
        width: 220.0,
        height: 44.0,
    };
    draw_button(
        frame,
        open_button,
        "Open Repository",
        Action::OpenRepositoryDialog,
        theme,
        true,
        state.focus.current == Some(FocusTarget::OpenRepositoryButton),
    );

    draw_text(
        &mut frame.scene,
        Rect {
            x: content.x + 24.0,
            y: content.y + 168.0,
            width: content.width - 48.0,
            height: 18.0,
        },
        "Recent Repositories",
        theme.colors.text_muted,
        12.0,
        FontKind::Ui,
    );

    let mut y = content.y + 194.0;
    for repo in state.settings.recent_repos.iter().take(6) {
        let rect = Rect {
            x: content.x + 24.0,
            y,
            width: content.width - 48.0,
            height: 36.0,
        };
        draw_surface(frame, rect, theme.colors.panel, theme.colors.border_soft);
        draw_text(
            &mut frame.scene,
            pad_rect(rect, 14.0, 10.0, 14.0, 10.0),
            &repo.display().to_string(),
            theme.colors.text_strong,
            13.0,
            FontKind::Ui,
        );
        frame.hits.push(HitRegion {
            rect,
            action: Action::OpenRepository(repo.clone()),
            hover_file_index: None,
        });
        y += 42.0;
    }
}

fn build_compare_frame(
    frame: &mut UiFrame,
    state: &AppState,
    theme: &Theme,
    width: f32,
    height: f32,
) {
    let (header, card) = layout_compare(width, height);
    draw_surface(frame, header, theme.colors.canvas, theme.colors.border_soft);
    draw_surface(frame, card, theme.colors.canvas, theme.colors.border_soft);

    let repo_label = state
        .compare
        .repo_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "No repository selected".to_owned());
    draw_text(
        &mut frame.scene,
        Rect {
            x: header.x + 20.0,
            y: header.y + 16.0,
            width: header.width - 200.0,
            height: 24.0,
        },
        &repo_label,
        theme.colors.text_strong,
        18.0,
        FontKind::Ui,
    );
    draw_text(
        &mut frame.scene,
        Rect {
            x: header.x + 20.0,
            y: header.y + 44.0,
            width: header.width - 200.0,
            height: 18.0,
        },
        "Choose refs, compare mode, and renderer.",
        theme.colors.text_muted,
        13.0,
        FontKind::Ui,
    );
    let open_repo_button = Rect {
        x: header.x + header.width - 180.0,
        y: header.y + 16.0,
        width: 160.0,
        height: 40.0,
    };
    draw_button(
        frame,
        open_repo_button,
        "Open Repository",
        Action::OpenRepositoryDialog,
        theme,
        false,
        false,
    );

    let inner = pad_rect(card, 24.0, 24.0, 24.0, 24.0);
    let field_gap = 16.0;
    let field_width = ((inner.width - field_gap) / 2.0).max(180.0);
    let left_rect = Rect {
        x: inner.x,
        y: inner.y + 8.0,
        width: field_width,
        height: 58.0,
    };
    let right_rect = Rect {
        x: left_rect.x + field_width + field_gap,
        y: left_rect.y,
        width: field_width,
        height: 58.0,
    };
    draw_input(
        frame,
        left_rect,
        "Left ref",
        &state.compare.left_ref,
        "main",
        FocusTarget::LeftRef,
        theme,
        state.focus.current == Some(FocusTarget::LeftRef),
    );
    draw_input(
        frame,
        right_rect,
        "Right ref",
        &state.compare.right_ref,
        "feature",
        FocusTarget::RightRef,
        theme,
        state.focus.current == Some(FocusTarget::RightRef),
    );

    let mode_row = Rect {
        x: inner.x,
        y: inner.y + 92.0,
        width: inner.width,
        height: 40.0,
    };
    let layout_row = Rect {
        x: inner.x,
        y: inner.y + 146.0,
        width: inner.width,
        height: 40.0,
    };
    let renderer_row = Rect {
        x: inner.x,
        y: inner.y + 200.0,
        width: inner.width,
        height: 40.0,
    };
    draw_segmented_compare_mode(frame, mode_row, state, theme);
    draw_segmented_layout(frame, layout_row, state, theme);
    draw_segmented_renderer(frame, renderer_row, state, theme);

    if let Some(message) = state.compare_form.validation_message.as_deref() {
        draw_text(
            &mut frame.scene,
            Rect {
                x: inner.x,
                y: inner.y + 254.0,
                width: inner.width,
                height: 18.0,
            },
            message,
            theme.colors.line_del,
            13.0,
            FontKind::Ui,
        );
    }

    let start_button = Rect {
        x: inner.x,
        y: card.bottom() - 68.0,
        width: 180.0,
        height: 44.0,
    };
    draw_button(
        frame,
        start_button,
        if state.workspace.status == crate::ui::state::AsyncStatus::Loading {
            "Comparing..."
        } else {
            "Start Compare"
        },
        Action::StartCompare,
        theme,
        true,
        state.focus.current == Some(FocusTarget::StartCompare),
    );

    let suggestion_anchor = match state.overlay.active_field {
        Some(crate::ui::state::CompareField::Left) => Some(left_rect),
        Some(crate::ui::state::CompareField::Right) => Some(right_rect),
        None => None,
    };
    if let Some(anchor) = suggestion_anchor {
        draw_ref_suggestions(frame, state, theme, anchor);
    }
}

fn build_diff_frame(
    frame: &mut UiFrame,
    state: &mut AppState,
    theme: &Theme,
    viewport_runtime: &mut DiffViewportRuntime,
    text_metrics: TextMetrics,
    width: f32,
    height: f32,
) {
    let (toolbar, sidebar, viewport_panel) = layout_diff(width, height);
    frame.file_list_rect = Some(sidebar);

    draw_surface(
        frame,
        toolbar,
        theme.colors.canvas,
        theme.colors.border_soft,
    );
    draw_surface(
        frame,
        sidebar,
        theme.colors.canvas,
        theme.colors.border_soft,
    );
    draw_surface(
        frame,
        viewport_panel,
        theme.colors.canvas,
        theme.colors.border_soft,
    );

    draw_text(
        &mut frame.scene,
        Rect {
            x: toolbar.x + 20.0,
            y: toolbar.y + 14.0,
            width: toolbar.width - 200.0,
            height: 22.0,
        },
        &state.window_title(),
        theme.colors.text_strong,
        16.0,
        FontKind::Ui,
    );
    draw_text(
        &mut frame.scene,
        Rect {
            x: toolbar.x + 20.0,
            y: toolbar.y + 38.0,
            width: toolbar.width - 200.0,
            height: 18.0,
        },
        &format!(
            "{} files   {} -> {}",
            state.workspace.files.len(),
            state.compare.resolved_left.as_deref().unwrap_or("?"),
            state.compare.resolved_right.as_deref().unwrap_or("?")
        ),
        theme.colors.text_muted,
        12.0,
        FontKind::Ui,
    );

    let wrap_button = Rect {
        x: toolbar.right() - 144.0,
        y: toolbar.y + 14.0,
        width: 124.0,
        height: 38.0,
    };
    let split_button = Rect {
        x: wrap_button.x - 100.0,
        y: wrap_button.y,
        width: 88.0,
        height: 38.0,
    };
    let unified_button = Rect {
        x: split_button.x - 100.0,
        y: wrap_button.y,
        width: 88.0,
        height: 38.0,
    };
    draw_button(
        frame,
        unified_button,
        "Unified",
        Action::SetLayoutMode(LayoutMode::Unified),
        theme,
        state.compare.layout == LayoutMode::Unified,
        false,
    );
    draw_button(
        frame,
        split_button,
        "Split",
        Action::SetLayoutMode(LayoutMode::Split),
        theme,
        state.compare.layout == LayoutMode::Split,
        false,
    );
    draw_button(
        frame,
        wrap_button,
        if state.viewport.wrap_enabled {
            "Wrap On"
        } else {
            "Wrap Off"
        },
        Action::ToggleWrap,
        theme,
        state.viewport.wrap_enabled,
        false,
    );

    draw_file_list(frame, state, theme, sidebar);
    let viewport_bounds = pad_rect(viewport_panel, 10.0, 10.0, 10.0, 10.0);
    let document = match state.workspace.active_file.as_ref() {
        Some(active_file) if active_file.file.is_binary => ViewportDocument::Binary {
            path: &active_file.path,
        },
        Some(active_file) => ViewportDocument::Text {
            compare_generation: state.workspace.compare_generation,
            file_index: active_file.index,
            path: &active_file.path,
            doc: &active_file.render_doc,
        },
        None => ViewportDocument::Empty,
    };
    viewport_runtime.prepare(&mut state.viewport, document, viewport_bounds, text_metrics);
    frame.viewport_rect = Some(viewport_runtime.body_bounds());
    viewport_runtime.paint(&mut frame.scene, theme, &state.viewport, document);
}

fn draw_segmented_compare_mode(frame: &mut UiFrame, rect: Rect, state: &AppState, theme: &Theme) {
    let labels = [
        ("Single Commit", CompareMode::SingleCommit),
        ("Two Dot", CompareMode::TwoDot),
        ("Three Dot", CompareMode::ThreeDot),
    ];
    let gap = 10.0;
    let width = (rect.width - gap * (labels.len() as f32 - 1.0)) / labels.len() as f32;
    for (index, (label, mode)) in labels.into_iter().enumerate() {
        let button = Rect {
            x: rect.x + index as f32 * (width + gap),
            y: rect.y,
            width,
            height: rect.height,
        };
        draw_button(
            frame,
            button,
            label,
            Action::SetCompareMode(mode),
            theme,
            state.compare.mode == mode,
            false,
        );
    }
}

fn draw_segmented_layout(frame: &mut UiFrame, rect: Rect, state: &AppState, theme: &Theme) {
    let labels = [
        ("Unified", LayoutMode::Unified),
        ("Split", LayoutMode::Split),
    ];
    let gap = 10.0;
    let width = (rect.width - gap) / 2.0;
    for (index, (label, layout)) in labels.into_iter().enumerate() {
        let button = Rect {
            x: rect.x + index as f32 * (width + gap),
            y: rect.y,
            width,
            height: rect.height,
        };
        draw_button(
            frame,
            button,
            label,
            Action::SetLayoutMode(layout),
            theme,
            state.compare.layout == layout,
            false,
        );
    }
}

fn draw_segmented_renderer(frame: &mut UiFrame, rect: Rect, state: &AppState, theme: &Theme) {
    let labels = [
        ("Built-in", RendererKind::Builtin),
        ("Difftastic", RendererKind::Difftastic),
    ];
    let gap = 10.0;
    let width = (rect.width - gap) / 2.0;
    for (index, (label, renderer)) in labels.into_iter().enumerate() {
        let button = Rect {
            x: rect.x + index as f32 * (width + gap),
            y: rect.y,
            width,
            height: rect.height,
        };
        draw_button(
            frame,
            button,
            label,
            Action::SetRenderer(renderer),
            theme,
            state.compare.renderer == renderer,
            false,
        );
    }
}

fn draw_ref_suggestions(frame: &mut UiFrame, state: &AppState, theme: &Theme, anchor: Rect) {
    if state.overlay.ref_suggestions.is_empty() {
        return;
    }
    let height = 12.0 + state.overlay.ref_suggestions.len() as f32 * 30.0;
    let panel = Rect {
        x: anchor.x,
        y: anchor.bottom() + 8.0,
        width: anchor.width,
        height,
    };
    draw_surface(frame, panel, theme.colors.panel, theme.colors.border_soft);
    let mut y = panel.y + 6.0;
    for (index, suggestion) in state.overlay.ref_suggestions.iter().enumerate() {
        let row = Rect {
            x: panel.x + 6.0,
            y,
            width: panel.width - 12.0,
            height: 24.0,
        };
        if index == state.overlay.selected_index {
            frame.scene.rounded_rect(RoundedRectPrimitive {
                rect: row,
                radius: 8.0,
                color: theme.colors.selection_bg,
            });
        }
        draw_text(
            &mut frame.scene,
            pad_rect(row, 10.0, 5.0, 10.0, 5.0),
            &suggestion.label,
            theme.colors.text_strong,
            13.0,
            FontKind::Mono,
        );
        frame.hits.push(HitRegion {
            rect: row,
            action: Action::SelectRefSuggestion(index),
            hover_file_index: None,
        });
        y += 30.0;
    }
}

fn draw_file_list(frame: &mut UiFrame, state: &AppState, theme: &Theme, sidebar: Rect) {
    let header = Rect {
        x: sidebar.x + 12.0,
        y: sidebar.y + 12.0,
        width: sidebar.width - 24.0,
        height: 42.0,
    };
    draw_text(
        &mut frame.scene,
        header,
        &format!("Changed Files ({})", state.workspace.files.len()),
        theme.colors.text_strong,
        16.0,
        FontKind::Ui,
    );

    let body = Rect {
        x: sidebar.x + 12.0,
        y: sidebar.y + 56.0,
        width: sidebar.width - 24.0,
        height: sidebar.height - 68.0,
    };
    frame.scene.clip(body);
    let row_height = state.file_list.row_height;
    let visible = (body.height / row_height).ceil().max(1.0) as usize;
    let max_start = state.workspace.files.len().saturating_sub(visible);
    let start = state.file_list.scroll_offset.min(max_start);
    let end = (start + visible + 1).min(state.workspace.files.len());
    for index in start..end {
        let file = &state.workspace.files[index];
        let row = Rect {
            x: body.x,
            y: body.y + (index - start) as f32 * row_height,
            width: body.width,
            height: row_height - 4.0,
        };
        let is_selected = state.workspace.selected_file_index == Some(index);
        let is_hovered = state.file_list.hovered_index == Some(index);
        if is_selected || is_hovered {
            frame.scene.rounded_rect(RoundedRectPrimitive {
                rect: row,
                radius: 8.0,
                color: if is_selected {
                    theme.colors.selection_bg
                } else {
                    theme.colors.panel
                },
            });
        }
        draw_text(
            &mut frame.scene,
            Rect {
                x: row.x + 10.0,
                y: row.y + 6.0,
                width: row.width - 100.0,
                height: 16.0,
            },
            &file.path,
            theme.colors.text_strong,
            13.0,
            FontKind::Ui,
        );
        draw_text(
            &mut frame.scene,
            Rect {
                x: row.x + 10.0,
                y: row.y + 20.0,
                width: row.width - 20.0,
                height: 12.0,
            },
            &format!(
                "{}   +{}   -{}",
                file.status, file.additions, file.deletions
            ),
            theme.colors.text_muted,
            11.0,
            FontKind::Ui,
        );
        frame.hits.push(HitRegion {
            rect: row,
            action: Action::SelectFile(index),
            hover_file_index: Some(index),
        });
    }
    frame.scene.pop_clip();
}

fn layout_welcome(width: f32, height: f32, recent_count: usize) -> Rect {
    let mut tree = taffy::TaffyTree::<()>::new();
    let content_height = 220.0 + recent_count.min(6) as f32 * 42.0;
    let content = tree
        .new_leaf(taffy::Style {
            size: taffy::Size {
                width: taffy::prelude::length((width - 80.0).min(760.0).max(420.0)),
                height: taffy::prelude::length(content_height.min((height - 80.0).max(240.0))),
            },
            ..Default::default()
        })
        .unwrap();
    let root = tree
        .new_with_children(
            taffy::Style {
                size: taffy::Size {
                    width: taffy::prelude::length(width),
                    height: taffy::prelude::length(height),
                },
                justify_content: Some(taffy::JustifyContent::Center),
                align_items: Some(taffy::AlignItems::Center),
                ..Default::default()
            },
            &[content],
        )
        .unwrap();
    tree.compute_layout(
        root,
        taffy::Size {
            width: taffy::AvailableSpace::Definite(width),
            height: taffy::AvailableSpace::Definite(height),
        },
    )
    .unwrap();
    rect_from_layout(tree.layout(content).unwrap())
}

fn layout_compare(width: f32, height: f32) -> (Rect, Rect) {
    let mut tree = taffy::TaffyTree::<()>::new();
    let header = tree
        .new_leaf(taffy::Style {
            size: taffy::Size {
                width: taffy::prelude::auto(),
                height: taffy::prelude::length(80.0),
            },
            ..Default::default()
        })
        .unwrap();
    let card = tree
        .new_leaf(taffy::Style {
            flex_grow: 1.0,
            size: taffy::Size {
                width: taffy::prelude::auto(),
                height: taffy::prelude::auto(),
            },
            ..Default::default()
        })
        .unwrap();
    let root = tree
        .new_with_children(
            taffy::Style {
                size: taffy::Size {
                    width: taffy::prelude::length(width),
                    height: taffy::prelude::length(height),
                },
                padding: taffy::Rect {
                    left: taffy::prelude::length(24.0),
                    right: taffy::prelude::length(24.0),
                    top: taffy::prelude::length(24.0),
                    bottom: taffy::prelude::length(24.0),
                },
                flex_direction: taffy::FlexDirection::Column,
                gap: taffy::Size {
                    width: taffy::prelude::length(0.0),
                    height: taffy::prelude::length(16.0),
                },
                ..Default::default()
            },
            &[header, card],
        )
        .unwrap();
    tree.compute_layout(
        root,
        taffy::Size {
            width: taffy::AvailableSpace::Definite(width),
            height: taffy::AvailableSpace::Definite(height),
        },
    )
    .unwrap();
    (
        rect_from_layout(tree.layout(header).unwrap()),
        rect_from_layout(tree.layout(card).unwrap()),
    )
}

fn layout_diff(width: f32, height: f32) -> (Rect, Rect, Rect) {
    let mut tree = taffy::TaffyTree::<()>::new();
    let toolbar = tree
        .new_leaf(taffy::Style {
            size: taffy::Size {
                width: taffy::prelude::auto(),
                height: taffy::prelude::length(68.0),
            },
            ..Default::default()
        })
        .unwrap();
    let sidebar = tree
        .new_leaf(taffy::Style {
            size: taffy::Size {
                width: taffy::prelude::length(360.0),
                height: taffy::prelude::auto(),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .unwrap();
    let preview = tree
        .new_leaf(taffy::Style {
            flex_grow: 1.0,
            size: taffy::Size {
                width: taffy::prelude::auto(),
                height: taffy::prelude::auto(),
            },
            ..Default::default()
        })
        .unwrap();
    let body = tree
        .new_with_children(
            taffy::Style {
                flex_grow: 1.0,
                flex_direction: taffy::FlexDirection::Row,
                gap: taffy::Size {
                    width: taffy::prelude::length(16.0),
                    height: taffy::prelude::length(0.0),
                },
                ..Default::default()
            },
            &[sidebar, preview],
        )
        .unwrap();
    let root = tree
        .new_with_children(
            taffy::Style {
                size: taffy::Size {
                    width: taffy::prelude::length(width),
                    height: taffy::prelude::length(height),
                },
                padding: taffy::Rect {
                    left: taffy::prelude::length(16.0),
                    right: taffy::prelude::length(16.0),
                    top: taffy::prelude::length(16.0),
                    bottom: taffy::prelude::length(16.0),
                },
                flex_direction: taffy::FlexDirection::Column,
                gap: taffy::Size {
                    width: taffy::prelude::length(0.0),
                    height: taffy::prelude::length(16.0),
                },
                ..Default::default()
            },
            &[toolbar, body],
        )
        .unwrap();
    tree.compute_layout(
        root,
        taffy::Size {
            width: taffy::AvailableSpace::Definite(width),
            height: taffy::AvailableSpace::Definite(height),
        },
    )
    .unwrap();
    (
        rect_from_layout(tree.layout(toolbar).unwrap()),
        rect_from_layout(tree.layout(sidebar).unwrap()),
        rect_from_layout(tree.layout(preview).unwrap()),
    )
}
