use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::actions::Action;
use crate::apprt::{AppRuntime, AppServices};
use crate::core::themes::ThemeRegistry;
use crate::effects::Effect;
use crate::events::RepositorySyncReason;
use crate::input::InputSystem;
use crate::platform::automation::{ErrorDump, FilesDump, StateDump, write_json};
use crate::platform::persistence::SettingsStore;
use crate::platform::startup::StartupOptions;
use crate::render::Renderer;
use crate::ui::components::TooltipState;
use crate::ui::editor::element::EditorElement;
use crate::ui::shell::{UiFrame, build_ui_frame};
use crate::ui::state::{AppState, FocusTarget};
use crate::ui::theme::Theme;

pub fn run() -> Result<(), Box<dyn Error>> {
    let startup = StartupOptions::load();
    init_logging(startup.log_debug);
    let should_poll = startup.exit_after().is_some();

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(if should_poll {
        ControlFlow::Poll
    } else {
        ControlFlow::Wait
    });

    let settings_store = SettingsStore::new_default();
    let settings = settings_store.load()?;
    let (mut state, initial_effects) = AppState::bootstrap(startup, settings);
    let theme_registry = ThemeRegistry::load();
    state.theme_names = theme_registry.names().map(str::to_owned).collect();
    state.theme_variants = state
        .theme_names
        .iter()
        .map(|n| theme_registry.variant(n))
        .collect();
    let runtime = AppRuntime::new(
        AppServices::new(settings_store),
        Some(event_loop.create_proxy()),
    );
    runtime.dispatch_all(initial_effects);

    #[cfg(feature = "hot-reload")]
    let hot_reload_pending = {
        let pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        crate::hot_reload::connect(event_loop.create_proxy(), pending.clone());
        pending
    };

    let mut app = NativeApp::new(state, runtime, theme_registry);
    #[cfg(feature = "hot-reload")]
    {
        app.hot_reload_pending = Some(hot_reload_pending);
    }
    event_loop.run_app(&mut app)?;
    Ok(())
}

struct NativeApp {
    state: AppState,
    theme: Theme,
    theme_registry: ThemeRegistry,
    runtime: AppRuntime,
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    ui_frame: UiFrame,
    editor: EditorElement,
    input: InputSystem,
    launch_at: Instant,
    dumps_dirty: bool,
    #[cfg(feature = "capture")]
    capture_pending: Option<std::path::PathBuf>,
    needs_redraw: bool,
    tooltip_state: TooltipState,
    #[cfg(feature = "hot-reload")]
    hot_reload_pending: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl NativeApp {
    fn new(state: AppState, runtime: AppRuntime, theme_registry: ThemeRegistry) -> Self {
        let theme = Theme::from_registry(
            &state.settings.theme_name,
            state.settings.theme_mode,
            &theme_registry,
        )
        .with_ui_scale(state.ui_scale_factor());
        #[cfg(feature = "capture")]
        let capture_pending = std::env::var("DIFFY_CAPTURE_PATH")
            .ok()
            .map(std::path::PathBuf::from);
        Self {
            state,
            theme,
            theme_registry,
            runtime,
            renderer: None,
            window: None,
            ui_frame: UiFrame::default(),
            input: InputSystem::default(),
            editor: EditorElement::default(),
            launch_at: Instant::now(),
            dumps_dirty: true,
            #[cfg(feature = "capture")]
            capture_pending,
            needs_redraw: true,
            tooltip_state: TooltipState::default(),
            #[cfg(feature = "hot-reload")]
            hot_reload_pending: None,
        }
    }

    fn mark_dirty(&mut self) {
        self.dumps_dirty = true;
        self.needs_redraw = true;
    }

    fn paint_tooltip(&mut self) {
        use crate::render::{
            BorderPrimitive, FontKind, FontWeight, Rect, RoundedRectPrimitive, ShadowPrimitive,
            TextPrimitive,
        };
        use crate::ui::design::{Rad, Shadow, Sp};
        use std::sync::Arc;

        if !self.tooltip_state.visible || self.tooltip_state.text.is_empty() {
            return;
        }
        let renderer = match self.renderer.as_mut() {
            Some(r) => r,
            None => return,
        };
        let window_size = self
            .window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or(winit::dpi::PhysicalSize::new(1320, 840));
        let window_w = window_size.width as f32;
        let window_h = window_size.height as f32;
        let tc = &self.theme.colors;
        let m = &self.theme.metrics;
        let scale = m.ui_scale();
        let font_size = m.ui_small_font_size;
        let text_w = crate::ui::element::measure_text_width(
            renderer.font_system(),
            &self.tooltip_state.text,
            font_size,
            FontKind::Ui,
            FontWeight::Normal,
        );
        let px = (Sp::MD * scale).round();
        let py = (Sp::SM * scale).round();
        let w = text_w + px * 2.0;
        let h = font_size + py * 2.0;
        let r = (Rad::MD * scale).round();
        let gap = (Sp::XS * scale).round();
        let margin = (Sp::XS * scale).round();
        let x = self.tooltip_state.x.min(window_w - w - margin).max(margin);
        let y = (self.tooltip_state.y + gap).min(window_h - h - margin);
        let rect = Rect {
            x,
            y,
            width: w,
            height: h,
        };
        let scene = &mut self.ui_frame.scene;

        scene.push_z_index(500);
        for layer in Shadow::TOOLTIP {
            scene.shadow(ShadowPrimitive {
                rect,
                blur_radius: layer.blur,
                corner_radius: r,
                offset: [0.0, layer.offset_y],
                color: crate::ui::theme::Color::rgba(0, 0, 0, layer.alpha),
            });
        }
        scene.rounded_rect(RoundedRectPrimitive::uniform(rect, r, tc.elevated_surface));
        scene.border(BorderPrimitive {
            rect,
            widths: [1.0; 4],
            corner_radii: [r; 4],
            color: tc.border,
        });
        let line_height = font_size * 1.35;
        scene.text(TextPrimitive {
            rect: Rect {
                x: x + px,
                y: y + ((h - line_height) / 2.0).round(),
                width: text_w + px,
                height: line_height,
            },
            text: Arc::from(self.tooltip_state.text.as_str()),
            color: tc.text,
            font_size,
            font_kind: FontKind::Ui,
            font_weight: FontWeight::Normal,
        });
        scene.pop_z_index();
    }

    fn sync_theme(&mut self) {
        let dpi = self
            .renderer
            .as_ref()
            .map(|r| r.scale_factor() as f32)
            .unwrap_or(1.0);
        self.theme = Theme::from_registry(
            &self.state.settings.theme_name,
            self.state.settings.theme_mode,
            &self.theme_registry,
        )
        .with_ui_scale(self.state.ui_scale_factor() * dpi);
    }

    fn window_attributes(&self) -> WindowAttributes {
        Window::default_attributes()
            .with_title(self.state.window_title())
            .with_visible(!self.state.startup.hidden_window)
            .with_inner_size(LogicalSize::new(1320.0, 840.0))
            .with_min_inner_size(LogicalSize::new(640.0, 480.0))
    }

    fn window_id(&self) -> Option<WindowId> {
        self.window.as_ref().map(|window| window.id())
    }

    fn refresh_window_title(&self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(&self.state.window_title());
        }
    }

    fn sync_window_text_input(&self) {
        if let Some(window) = self.window.as_ref() {
            // winit disables IME/text input by default. Mirror Diffy's focus state so
            // picker/search fields and the commit editor receive translated text.
            window.set_ime_allowed(self.state.is_text_focused());
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
        self.sync_window_text_input();
        self.mark_dirty();
    }

    fn write_dumps_if_needed(&mut self) {
        if !self.dumps_dirty {
            return;
        }

        if self.state.startup.hidden_window {
            let frame = self.build_frame();
            self.state.store.write(
                self.state.debug.last_scene_primitive_count,
                frame.scene.len(),
            );
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
            .as_mut()
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

        // Clone the Rc so `cx` can hold `&SignalStore` independently of
        // `self.state` (which we need to borrow mutably for build_ui_frame).
        let store = std::rc::Rc::clone(&self.state.store);

        let mut cx = crate::ui::element::ElementContext::new(
            &self.theme,
            scale_factor,
            font_system,
            self.input.mouse_position(),
            &store,
        )
        .with_focus(store.read(self.state.focus))
        .with_clock(self.state.clock_ms);
        cx.debug_wireframe = std::env::var("DIFFY_DEBUG_WIREFRAME").is_ok();

        #[cfg(feature = "hot-reload")]
        return subsecond::call(|| {
            build_ui_frame(
                &mut self.state,
                &self.theme,
                &mut self.editor,
                scale_text_metrics(text_metrics, ui_scale),
                width,
                height,
                &mut cx,
            )
        });

        #[cfg(not(feature = "hot-reload"))]
        build_ui_frame(
            &mut self.state,
            &self.theme,
            &mut self.editor,
            scale_text_metrics(text_metrics, ui_scale),
            width,
            height,
            &mut cx,
        )
    }

    fn dispatch_action(&mut self, action: Action) {
        if matches!(
            action,
            Action::StageHunk
                | Action::UnstageHunk
                | Action::DiscardHunk
                | Action::StageHunkAt(_)
                | Action::UnstageHunkAt(_)
                | Action::DiscardHunkAt(_)
        ) {
            tracing::info!(?action, "dispatch_action: hunk op");
        }
        let effects = self.state.apply_action(action);
        if let Some(renderer) = self.renderer.as_mut() {
            self.state.commit_editor.flush(renderer.font_system_mut());
        }
        self.runtime.dispatch_all(effects);
        self.sync_theme();
        self.refresh_window_title();
        self.sync_window_text_input();
    }

    fn apply_input_outcome(&mut self, outcome: crate::input::InputOutcome) {
        for action in outcome.actions {
            self.dispatch_action(action);
        }
        if !outcome.effects.is_empty() {
            self.runtime.dispatch_all(outcome.effects);
        }
        if outcome.dirty {
            self.mark_dirty();
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
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        self.process_runtime_events();
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

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
                self.sync_theme();
                self.refresh_window_title();
                self.sync_window_text_input();
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
            WindowEvent::Focused(true) => {
                if let Some(path) = self.state.compare.repo_path.get(&self.state.store) {
                    self.runtime.dispatch_all(vec![Effect::SyncRepository {
                        path,
                        reason: RepositorySyncReason::Rescan,
                    }]);
                }
                self.mark_dirty();
            }
            WindowEvent::Resized(size) => {
                if let (Some(renderer), Some(window)) =
                    (self.renderer.as_mut(), self.window.as_ref())
                {
                    renderer.resize(size.width, size.height, window.scale_factor());
                }
                self.sync_theme();
                self.mark_dirty();
            }
            WindowEvent::RedrawRequested => {
                let frame_started_at = Instant::now();
                let now_ms = self.launch_at.elapsed().as_millis() as u64;
                self.tooltip_state.tick(now_ms);
                let frame = self.build_frame();
                self.ui_frame = frame;
                self.paint_tooltip();
                if let Some(ha) = self
                    .ui_frame
                    .text_input_hit_areas
                    .iter()
                    .find(|ha| ha.focus_target == FocusTarget::CommitEditor)
                {
                    let w = ha.text_width;
                    let h = ha.text_height;
                    let fs = ha.font_size;
                    if let Some(renderer) = self.renderer.as_mut() {
                        self.state
                            .commit_editor
                            .set_font_size(renderer.font_system_mut(), fs);
                        self.state
                            .commit_editor
                            .sync_size(renderer.font_system_mut(), w, h);
                    }
                }
                if let Some(renderer) = self.renderer.as_mut() {
                    let time_seconds = self.launch_at.elapsed().as_secs_f32();
                    match renderer.render(
                        &self.ui_frame.scene,
                        time_seconds,
                        Some(&self.state.commit_editor),
                    ) {
                        Ok(frame) => {
                            let store = &self.state.store;
                            store.write(
                                self.state.debug.last_scene_primitive_count,
                                frame.primitive_count,
                            );
                            store.write(
                                self.state.debug.last_frame_time_us,
                                frame_started_at
                                    .elapsed()
                                    .as_micros()
                                    .min(u128::from(u64::MAX))
                                    as u64,
                            );
                        }
                        Err(error) => {
                            eprintln!("render failed: {error}");
                            self.state
                                .last_error
                                .set(&self.state.store, Some(error.to_string()));
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
                self.needs_redraw = false;
                self.state.store.clear_dirty();
            }
            event => {
                let outcome = self.input.handle_window_event(
                    &mut self.state,
                    &mut self.ui_frame,
                    &self.editor,
                    self.renderer.as_mut(),
                    self.window.as_ref(),
                    &mut self.tooltip_state,
                    self.launch_at,
                    event,
                );
                if let Some(outcome) = outcome {
                    self.apply_input_outcome(outcome);
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let prior_cursor_blink_epoch = self.state.cursor_blink_epoch();
        self.state.update_time(
            self.launch_at
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
        );
        self.process_runtime_events();

        #[cfg(feature = "hot-reload")]
        if let Some(pending) = &self.hot_reload_pending {
            if pending.swap(false, std::sync::atomic::Ordering::AcqRel) {
                self.sync_theme();
                self.mark_dirty();
            }
        }

        self.write_dumps_if_needed();

        if self.should_exit() {
            if let Some(window) = self.window.as_ref() {
                window.set_visible(false);
            }
            event_loop.exit();
            return;
        }

        let tooltip_was_visible = self.tooltip_state.visible;
        let now_ms = self
            .launch_at
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        self.tooltip_state.tick(now_ms);
        let tooltip_changed = self.tooltip_state.visible != tooltip_was_visible;

        let animating = self.state.animation.has_active();
        let cursor_blink_changed = self.state.cursor_blink_epoch() != prior_cursor_blink_epoch;
        let should_poll = self.state.startup.exit_after.is_some();
        let next_wake = if animating {
            Some(std::time::Instant::now() + std::time::Duration::from_millis(16))
        } else {
            let next_cursor_blink = self
                .state
                .next_cursor_blink_at_ms()
                .map(|ms| self.launch_at + std::time::Duration::from_millis(ms));
            let next_toast_expiry = self
                .state
                .next_toast_expiry_at_ms()
                .map(|ms| self.launch_at + std::time::Duration::from_millis(ms));
            let next_tooltip = if !self.tooltip_state.text.is_empty() && !self.tooltip_state.visible
            {
                Some(
                    self.launch_at
                        + std::time::Duration::from_millis(self.tooltip_state.show_at_ms),
                )
            } else {
                None
            };
            [next_cursor_blink, next_toast_expiry, next_tooltip]
                .into_iter()
                .flatten()
                .min()
        };

        if should_poll {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else if let Some(next) = next_wake {
            event_loop.set_control_flow(ControlFlow::WaitUntil(next));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }

        if let Some(window) = self.window.as_ref()
            && (self.needs_redraw
                || self.state.store.any_dirty()
                || animating
                || cursor_blink_changed
                || tooltip_changed)
        {
            window.request_redraw();
        }
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

#[cfg(test)]
mod tests {
    use crate::core::themes::ThemeRegistry;
    use tempfile::TempDir;
    use winit::dpi::PhysicalPosition;
    use winit::event::{MouseScrollDelta, TouchPhase};
    use winit::keyboard::ModifiersState;

    use super::NativeApp;
    use crate::actions::Action;
    use crate::apprt::{AppRuntime, AppServices};
    use crate::input::{
        InputEvent, KeyChord, KeyKind, quantize_scroll_delta_px, scroll_delta_to_px,
    };
    use crate::platform::persistence::SettingsStore;
    use crate::ui::state::{
        AppState, FileListEntry, FocusTarget, OverlayEntry, OverlaySurface, WorkspaceMode,
    };

    fn test_app(state: AppState) -> NativeApp {
        let dir = TempDir::new().unwrap();
        let runtime = AppRuntime::new(AppServices::new(SettingsStore::new_in(dir.path())), None);
        NativeApp::new(state, runtime, ThemeRegistry::load())
    }

    fn dispatch_input_event(app: &mut NativeApp, event: InputEvent) {
        let outcome = app.input.handle_input_event_for_test(
            &mut app.state,
            &mut app.ui_frame,
            &app.editor,
            app.renderer.as_mut(),
            app.window.as_ref(),
            &mut app.tooltip_state,
            app.launch_at,
            event,
        );
        app.apply_input_outcome(outcome);
    }

    fn keypress(text: impl Into<String>, modifiers: ModifiersState) -> InputEvent {
        InputEvent::KeyPress(KeyChord {
            logical: KeyKind::Character(text.into()),
            physical: None,
            modifiers,
            repeat: false,
        })
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
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state.workspace.files.set(
            &state.store,
            (0..32)
                .map(|index| FileListEntry {
                    path: format!("src/file_{index}.rs"),
                    status: "M".to_owned(),
                    additions: 1,
                    deletions: 0,
                    is_binary: false,
                })
                .collect(),
        );
        state
            .workspace
            .selected_file_index
            .set(&state.store, Some(0));
        state
            .workspace
            .selected_file_path
            .set(&state.store, Some("src/file_0.rs".to_owned()));

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

        dispatch_input_event(&mut app, InputEvent::PointerMoved { x, y });
        dispatch_input_event(
            &mut app,
            InputEvent::Wheel {
                delta: MouseScrollDelta::LineDelta(0.0, -2.0),
                phase: TouchPhase::Moved,
            },
        );

        assert!(app.state.file_list.scroll_offset_px.get(&app.state.store) > 0.0);
        assert_eq!(app.state.editor.scroll_top_px.get(&app.state.store), 0);
    }

    #[test]
    fn file_list_wheel_scroll_moves_sidebar_contents() {
        let mut state = AppState::default();
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state.workspace.files.set(
            &state.store,
            (0..32)
                .map(|index| FileListEntry {
                    path: format!("src/file_{index}.rs"),
                    status: "M".to_owned(),
                    additions: 1,
                    deletions: 0,
                    is_binary: false,
                })
                .collect(),
        );
        state
            .workspace
            .selected_file_index
            .set(&state.store, Some(0));
        state
            .workspace
            .selected_file_path
            .set(&state.store, Some("src/file_0.rs".to_owned()));

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
        dispatch_input_event(&mut app, InputEvent::PointerMoved { x, y });
        dispatch_input_event(
            &mut app,
            InputEvent::Wheel {
                delta: MouseScrollDelta::LineDelta(0.0, -3.0),
                phase: TouchPhase::Moved,
            },
        );

        assert!(app.state.file_list.scroll_offset_px.get(&app.state.store) > 0.0);
    }

    #[test]
    fn overlay_blocks_viewport_scroll_fallback() {
        let mut state = AppState::default();
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state.workspace.files.set(
            &state.store,
            vec![FileListEntry {
                path: "src/file_0.rs".to_owned(),
                status: "M".to_owned(),
                additions: 1,
                deletions: 0,
                is_binary: false,
            }],
        );
        state
            .workspace
            .selected_file_index
            .set(&state.store, Some(0));
        state
            .workspace
            .selected_file_path
            .set(&state.store, Some("src/file_0.rs".to_owned()));
        state.overlays.stack.update(&state.store, |stack| {
            stack.push(OverlayEntry {
                surface: OverlaySurface::GitHubAuthModal,
                focus_return: Some(FocusTarget::TitleBar),
            });
        });

        let mut app = test_app(state);
        app.ui_frame = app.build_frame();
        let overlay_hit = app
            .ui_frame
            .hits
            .iter()
            .rev()
            .find(|hit| {
                matches!(
                    hit.identity,
                    Some(crate::ui::element::HitIdentity::OverlayBackdrop)
                )
            })
            .expect("overlay hit");
        let x = overlay_hit.rect.x + overlay_hit.rect.width * 0.5;
        let y = overlay_hit.rect.y + overlay_hit.rect.height * 0.5;

        dispatch_input_event(&mut app, InputEvent::PointerMoved { x, y });
        dispatch_input_event(
            &mut app,
            InputEvent::Wheel {
                delta: MouseScrollDelta::LineDelta(0.0, -3.0),
                phase: TouchPhase::Moved,
            },
        );

        assert_eq!(app.state.editor.scroll_top_px.get(&app.state.store), 0);
        assert_eq!(
            app.state.file_list.scroll_offset_px.get(&app.state.store),
            0.0
        );
    }

    #[test]
    fn command_shortcuts_adjust_ui_scale() {
        let mut app = test_app(AppState::default());
        dispatch_input_event(&mut app, keypress("=", ModifiersState::SUPER));
        assert_eq!(app.state.settings.ui_scale_pct, 110);

        dispatch_input_event(&mut app, keypress("-", ModifiersState::SUPER));
        assert_eq!(app.state.settings.ui_scale_pct, 100);
    }

    #[test]
    fn space_in_picker_input_inserts_text() {
        let mut state = AppState::default();
        state.apply_action(crate::actions::Action::OpenRepoPicker);
        let mut app = test_app(state);
        let before = app
            .state
            .overlays
            .picker
            .query
            .with(&app.state.store, |q| q.clone());

        dispatch_input_event(&mut app, InputEvent::TextInput(" ".to_owned()));

        assert_eq!(
            app.state
                .overlays
                .picker
                .query
                .with(&app.state.store, |q| q.clone()),
            format!("{before} ")
        );
    }

    #[test]
    fn command_palette_shortcut_still_works_while_text_focused() {
        let mut state = AppState::default();
        state.apply_action(crate::actions::Action::OpenSearch);
        let mut app = test_app(state);

        dispatch_input_event(&mut app, keypress("p", ModifiersState::SUPER));

        assert_eq!(
            app.state.overlays_top(),
            Some(OverlaySurface::CommandPalette)
        );
    }

    #[test]
    fn ime_commit_inserts_once_into_text_field() {
        let mut state = AppState::default();
        state.apply_action(crate::actions::Action::OpenSearch);
        let mut app = test_app(state);

        dispatch_input_event(
            &mut app,
            InputEvent::ImePreedit("ni".to_owned(), Some((2, 2))),
        );
        dispatch_input_event(&mut app, InputEvent::TextInput("に".to_owned()));

        assert_eq!(
            app.state
                .editor
                .search
                .query
                .with(&app.state.store, |s| s.clone()),
            "に"
        );
    }
}

fn log_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|h| h.join("Library/Logs/diffy"))
    }
    #[cfg(target_os = "linux")]
    {
        dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .map(|base| base.join("diffy"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        dirs::data_local_dir().map(|base| base.join("diffy").join("logs"))
    }
}

fn init_logging(log_debug: bool) {
    use std::sync::Mutex;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = if log_debug {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    let stdout_layer = fmt::layer().with_writer(std::io::stdout);

    let (file_layer, file_path) = log_dir()
        .and_then(|dir| {
            std::fs::create_dir_all(&dir).ok()?;
            let path = dir.join("diffy.log");
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .ok()?;
            let layer = fmt::layer().with_ansi(false).with_writer(Mutex::new(file));
            Some((layer, path))
        })
        .map(|(l, p)| (Some(l), Some(p)))
        .unwrap_or((None, None));

    tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();

    if let Some(path) = file_path {
        tracing::info!(path = %path.display(), "logging initialized");
    } else {
        tracing::warn!("logging: unable to open log file, falling back to stdout only");
    }
}
