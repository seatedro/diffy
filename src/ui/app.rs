use std::error::Error;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use accesskit::{
    Action as AxAction, ActionData, ActionHandler, ActionRequest, ActivationHandler,
    DeactivationHandler, TreeUpdate,
};
use accesskit_winit::Adapter as AccessibilityAdapter;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Icon, Window, WindowAttributes, WindowId};

use crate::actions::{Action, AppAction, TextEditAction};
use crate::apprt::{AppRuntime, AppServices};
use crate::core::themes::ThemeRegistry;
use crate::effects::{RepositoryEffect, UpdateEffect};
use crate::events::RepositorySyncReason;
use crate::fonts::FontSettings;
use crate::input::InputSystem;
use crate::platform::persistence::SettingsStore;
use crate::platform::startup::StartupOptions;
use crate::render::Renderer;
use crate::ui::components::TooltipState;
use crate::ui::editor::element::EditorElement;
use crate::ui::hud::{HudSample, HudState};
use crate::ui::shell::{UiFrame, build_ui_frame};
use crate::ui::state::{AppState, FocusTarget};
use crate::ui::theme::Theme;

const UPDATE_POLL_INTERVAL: Duration = Duration::from_secs(60 * 60);

pub fn run() -> Result<(), Box<dyn Error>> {
    let startup = StartupOptions::load();
    init_logging(startup.log_debug);
    let keyring_enabled = startup.keyring_enabled;
    let github_token_store = startup.github_token_store;

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);

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
    let wake_proxy = event_loop.create_proxy();
    let runtime = AppRuntime::new(
        AppServices::new(settings_store),
        Some(wake_proxy.clone()),
        keyring_enabled,
        github_token_store,
    );
    runtime.dispatch_all(initial_effects);

    #[cfg(feature = "hot-reload")]
    let hot_reload_pending = {
        let pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        crate::hot_reload::connect(wake_proxy.clone(), pending.clone());
        pending
    };

    let mut app = NativeApp::new(state, runtime, theme_registry, Some(wake_proxy));
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
    font_settings: FontSettings,
    window: Option<Arc<Window>>,
    wake_proxy: Option<EventLoopProxy<()>>,
    accessibility_adapter: Option<AccessibilityAdapter>,
    accessibility_latest_tree: Arc<Mutex<TreeUpdate>>,
    accessibility_action_sender: Sender<ActionRequest>,
    accessibility_actions: Receiver<ActionRequest>,
    ui_frame: UiFrame,
    editor: EditorElement,
    input: InputSystem,
    launch_at: Instant,
    next_update_check_at: Option<Instant>,
    needs_redraw: bool,
    exit_requested: bool,
    has_seen_focus: bool,
    skip_next_focus_regain_rescan: bool,
    rescan_on_next_focus: bool,
    tooltip_state: TooltipState,
    hud: HudState,
    #[cfg(feature = "hot-reload")]
    hot_reload_pending: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl NativeApp {
    fn new(
        state: AppState,
        runtime: AppRuntime,
        theme_registry: ThemeRegistry,
        wake_proxy: Option<EventLoopProxy<()>>,
    ) -> Self {
        let theme = Theme::from_registry(
            &state.settings.theme_name,
            state.settings.theme_mode,
            &theme_registry,
        )
        .with_ui_scale(state.ui_scale_factor());
        let update_polling_enabled = state.update_polling_enabled();
        let font_settings = state.settings.fonts.normalized();
        let (accessibility_action_sender, accessibility_actions) = mpsc::channel();
        Self {
            state,
            theme,
            theme_registry,
            runtime,
            renderer: None,
            font_settings,
            window: None,
            wake_proxy,
            accessibility_adapter: None,
            accessibility_latest_tree: Arc::new(Mutex::new(
                crate::ui::accessibility::empty_tree_update(),
            )),
            accessibility_action_sender,
            accessibility_actions,
            ui_frame: UiFrame::default(),
            input: InputSystem::default(),
            editor: EditorElement::default(),
            launch_at: Instant::now(),
            next_update_check_at: update_polling_enabled
                .then(|| Instant::now() + UPDATE_POLL_INTERVAL),
            needs_redraw: true,
            exit_requested: false,
            has_seen_focus: false,
            skip_next_focus_regain_rescan: true,
            rescan_on_next_focus: false,
            tooltip_state: TooltipState::default(),
            hud: HudState::default(),
            #[cfg(feature = "hot-reload")]
            hot_reload_pending: None,
        }
    }

    fn mark_dirty(&mut self) {
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

    fn paint_debug_overlay(&mut self) {
        use crate::render::{
            BorderPrimitive, FontKind, FontWeight, Rect, RectPrimitive, RoundedRectPrimitive,
            Scene, TextPrimitive,
        };
        use crate::ui::design::{Rad, Sp};
        use crate::ui::hud::BUDGET_120_US;
        use crate::ui::theme::Color;
        use std::sync::Arc;

        if !self.state.debug.overlay_visible.get(&self.state.store) {
            return;
        }
        let renderer = match self.renderer.as_mut() {
            Some(r) => r,
            None => return,
        };
        let window_w = self
            .window
            .as_ref()
            .map(|w| w.inner_size().width as f32)
            .unwrap_or(1320.0);

        let tc = &self.theme.colors;
        let m = &self.theme.metrics;
        let scale = m.ui_scale();
        let font_size = m.mono_font_size;
        let line_height = (font_size * 1.35).round();

        let hud = &self.hud;
        let s = hud.last;
        let cpu_us = hud.cpu_ema_us();
        let to_ms = |us: u64| us as f32 / 1000.0;
        let budget_ms = BUDGET_120_US as f32 / 1000.0;
        let frac = cpu_us as f32 / BUDGET_120_US as f32;

        let lines = [
            (
                FontWeight::Semibold,
                tc.text_strong,
                "DEBUG · 120 budget".to_owned(),
            ),
            (
                FontWeight::Normal,
                tc.text,
                format!("fps   {:>6.1}", hud.fps()),
            ),
            (
                FontWeight::Normal,
                tc.text,
                format!(
                    "cpu   {:>5.2}/{:.2}ms {:>3.0}%",
                    to_ms(cpu_us),
                    budget_ms,
                    frac * 100.0
                ),
            ),
            (
                FontWeight::Normal,
                tc.text_muted,
                format!(
                    "build {:>5.2}  paint {:>5.2}",
                    to_ms(s.build_us),
                    to_ms(s.paint_us)
                ),
            ),
            (
                FontWeight::Normal,
                tc.text_muted,
                format!(
                    "rcpu  {:>5.2}  vsync {:>5.2}",
                    to_ms(s.render_cpu_us),
                    to_ms(s.acquire_us)
                ),
            ),
            (
                FontWeight::Normal,
                tc.text_muted,
                format!("prims {}", s.primitive_count),
            ),
        ];

        let fs = renderer.font_system();
        let mut content_w = 0.0_f32;
        for (weight, _, text) in &lines {
            let w = crate::ui::element::measure_text_width(
                fs,
                text,
                font_size,
                FontKind::Mono,
                *weight,
            );
            content_w = content_w.max(w);
        }

        let pad = (Sp::SM * scale).round();
        let gap = (Sp::XS * scale).round();
        let bar_h = (Sp::XS * scale).round();
        let r = (Rad::MD * scale).round();
        let margin = (Sp::SM * scale).round();
        let graph_h = (line_height * 3.0).round();

        let content_h = lines.len() as f32 * line_height + gap + bar_h + gap + graph_h;
        let panel_w = content_w + pad * 2.0;
        let panel_h = content_h + pad * 2.0;
        let x = (window_w - panel_w - margin).max(margin);
        let y = m.title_bar_height + margin;

        let color_for = |f: f32| {
            if f >= 1.0 {
                tc.status_error
            } else if f >= 0.8 {
                tc.status_warning
            } else {
                tc.line_add
            }
        };

        let scene = &mut self.ui_frame.scene;
        scene.push_z_index(600);
        scene.rounded_rect(RoundedRectPrimitive::uniform(
            Rect {
                x,
                y,
                width: panel_w,
                height: panel_h,
            },
            r,
            tc.elevated_surface,
        ));
        scene.border(BorderPrimitive {
            rect: Rect {
                x,
                y,
                width: panel_w,
                height: panel_h,
            },
            widths: [1.0; 4],
            corner_radii: [r; 4],
            color: tc.border,
        });

        let cx = x + pad;
        let mut cy = y + pad;
        let emit = |scene: &mut Scene, cy: f32, weight: FontWeight, color: Color, text: &str| {
            scene.text(TextPrimitive {
                rect: Rect {
                    x: cx,
                    y: cy,
                    width: content_w,
                    height: line_height,
                },
                text: Arc::from(text),
                color,
                font_size,
                font_kind: FontKind::Mono,
                font_weight: weight,
            });
        };

        for (i, (weight, color, text)) in lines.iter().enumerate() {
            emit(scene, cy, *weight, *color, text);
            cy += line_height;
            if i == 2 {
                let track = Rect {
                    x: cx,
                    y: cy,
                    width: content_w,
                    height: bar_h,
                };
                scene.rounded_rect(RoundedRectPrimitive::uniform(
                    track,
                    bar_h / 2.0,
                    tc.border_variant,
                ));
                let fill_w = (content_w * frac.clamp(0.0, 1.0)).round();
                if fill_w > 0.0 {
                    scene.rounded_rect(RoundedRectPrimitive::uniform(
                        Rect {
                            x: cx,
                            y: cy,
                            width: fill_w,
                            height: bar_h,
                        },
                        bar_h / 2.0,
                        color_for(frac),
                    ));
                }
                cy += bar_h + gap;
            }
        }

        let graph = Rect {
            x: cx,
            y: cy,
            width: content_w,
            height: graph_h,
        };
        scene.rounded_rect(RoundedRectPrimitive::uniform(
            graph,
            (Rad::SM * scale).round(),
            tc.element_background,
        ));
        let cap = hud.history_capacity() as f32;
        let bar_w = content_w / cap;
        let peak = hud.history_peak_us().max(BUDGET_120_US) as f32;
        let unit = graph_h / peak;
        for (i, sample) in hud.samples().enumerate() {
            let h = (sample as f32 * unit).min(graph_h);
            if h <= 0.0 {
                continue;
            }
            scene.rect(RectPrimitive {
                rect: Rect {
                    x: cx + i as f32 * bar_w,
                    y: cy + graph_h - h,
                    width: bar_w.max(1.0),
                    height: h,
                },
                color: color_for(sample as f32 / BUDGET_120_US as f32),
            });
        }
        let budget_y = cy + graph_h - (BUDGET_120_US as f32 * unit);
        scene.rect(RectPrimitive {
            rect: Rect {
                x: cx,
                y: budget_y,
                width: content_w,
                height: 1.0,
            },
            color: tc.text_muted,
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

    fn sync_fonts(&mut self) {
        let font_settings = self.state.settings.fonts.normalized();
        if font_settings == self.font_settings {
            return;
        }

        self.font_settings = font_settings;
        self.state.commit_editor.invalidate_font();
        self.state.review_comment_editor.invalidate_font();
        self.state.steering_prompt_editor.invalidate_font();

        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_font_settings(&self.font_settings);
        }
        self.mark_dirty();
    }

    fn window_attributes(&self) -> WindowAttributes {
        let attrs = Window::default_attributes()
            .with_title(crate::platform::startup::app_display_name())
            .with_inner_size(LogicalSize::new(1320.0, 840.0))
            .with_min_inner_size(LogicalSize::new(640.0, 480.0))
            .with_window_icon(app_window_icon())
            .with_visible(false);
        configure_chrome(attrs)
    }

    fn sync_window_metrics(&mut self, size: PhysicalSize<u32>, scale_factor: f64) {
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.resize(size.width, size.height, scale_factor);
        }
        self.sync_theme();
        self.position_traffic_lights();
        self.mark_dirty();
    }

    #[cfg(target_os = "macos")]
    fn position_traffic_lights(&self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        // Buttons are 12pt, content centerline of our bar is title_bar_height/2
        // (logical points). title_bar_height in theme is already scaled, so
        // divide by ui_scale to get the unscaled bar height in points.
        let bar_h_logical =
            self.theme.metrics.title_bar_height / self.theme.metrics.ui_scale().max(0.01);
        // Match the OS default left inset (NSWindow places the close button
        // around x=8). Tweak if we want them tighter or looser.
        let left_margin = 12.0;
        let target_center_y = bar_h_logical * 0.5;
        crate::platform::macos_window::position_traffic_lights(
            window,
            left_margin,
            target_center_y,
        );
    }

    #[cfg(not(target_os = "macos"))]
    fn position_traffic_lights(&self) {}

    fn window_id(&self) -> Option<WindowId> {
        self.window.as_ref().map(|window| window.id())
    }

    fn refresh_window_title(&self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(&self.state.window_title());
        }
        // Setting the window title resets traffic-light positions on macOS.
        self.position_traffic_lights();
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

    fn create_accessibility_adapter(
        &self,
        event_loop: &ActiveEventLoop,
        window: &Window,
    ) -> AccessibilityAdapter {
        AccessibilityAdapter::with_direct_handlers(
            event_loop,
            window,
            DiffyAccessibilityActivation {
                latest_tree: Arc::clone(&self.accessibility_latest_tree),
            },
            DiffyAccessibilityActions {
                sender: self.accessibility_action_sender.clone(),
                wake_proxy: self.wake_proxy.clone(),
            },
            DiffyAccessibilityDeactivation,
        )
    }

    fn publish_accessibility_update(&mut self) {
        let update = self
            .ui_frame
            .accessibility
            .tree_update(self.state.focus.get(&self.state.store));
        if let Ok(mut latest) = self.accessibility_latest_tree.lock() {
            *latest = update.clone();
        }
        if let Some(adapter) = self.accessibility_adapter.as_mut() {
            adapter.update_if_active(|| update);
        }
    }

    fn process_accessibility_actions(&mut self) {
        let requests: Vec<_> = self.accessibility_actions.try_iter().collect();
        for request in requests {
            self.handle_accessibility_request(request);
        }
    }

    fn handle_accessibility_request(&mut self, request: ActionRequest) {
        if request.target_tree != accesskit::TreeId::ROOT {
            return;
        }

        let Some(action) = self
            .ui_frame
            .accessibility
            .action_for(request.target_node)
            .cloned()
        else {
            return;
        };

        match (request.action, action) {
            (AxAction::Click, crate::ui::accessibility::AccessibilityAction::Click(action)) => {
                self.dispatch_action(action);
                self.mark_dirty();
            }
            (
                AxAction::Focus,
                crate::ui::accessibility::AccessibilityAction::Focus(target)
                | crate::ui::accessibility::AccessibilityAction::TextValue(target),
            ) => {
                self.dispatch_action(AppAction::SetFocus(Some(target)).into());
                self.mark_dirty();
            }
            (
                AxAction::SetValue,
                crate::ui::accessibility::AccessibilityAction::TextValue(target),
            ) => {
                if let Some(ActionData::Value(value)) = request.data {
                    self.dispatch_action(AppAction::SetFocus(Some(target)).into());
                    self.dispatch_action(TextEditAction::SelectAll.into());
                    self.dispatch_action(TextEditAction::Paste(value.into()).into());
                    self.mark_dirty();
                }
            }
            (
                AxAction::ReplaceSelectedText,
                crate::ui::accessibility::AccessibilityAction::TextValue(target),
            ) => {
                if let Some(ActionData::Value(value)) = request.data {
                    self.dispatch_action(AppAction::SetFocus(Some(target)).into());
                    self.dispatch_action(TextEditAction::Paste(value.into()).into());
                    self.mark_dirty();
                }
            }
            (
                AxAction::ScrollDown | AxAction::ScrollUp,
                crate::ui::accessibility::AccessibilityAction::Scroll(builder),
            ) => {
                let delta = if request.action == AxAction::ScrollDown {
                    3
                } else {
                    -3
                };
                self.dispatch_action(builder.build(delta));
                self.mark_dirty();
            }
            (
                AxAction::Focus,
                crate::ui::accessibility::AccessibilityAction::EditorViewport { focus, .. },
            ) => {
                self.dispatch_action(AppAction::SetFocus(Some(focus)).into());
                self.mark_dirty();
            }
            (
                AxAction::ScrollDown | AxAction::ScrollUp,
                crate::ui::accessibility::AccessibilityAction::EditorViewport { scroll, .. },
            ) => {
                let delta = if request.action == AxAction::ScrollDown {
                    3
                } else {
                    -3
                };
                self.dispatch_action(scroll.build(delta));
                self.mark_dirty();
            }
            _ => {}
        }
    }

    fn tick_update_polling(&mut self, now: Instant) {
        if !self.state.update_polling_enabled() {
            self.next_update_check_at = None;
            return;
        }

        match self.next_update_check_at {
            Some(next) if now >= next => {
                self.runtime
                    .dispatch_all(vec![UpdateEffect::CheckForUpdates { silent: true }.into()]);
                self.next_update_check_at = Some(now + UPDATE_POLL_INTERVAL);
            }
            Some(_) => {}
            None => {
                self.next_update_check_at = Some(now + UPDATE_POLL_INTERVAL);
            }
        }
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
            fallback_font_system =
                crate::fonts::new_font_system_with_settings(&self.state.settings.fonts);
            &mut fallback_font_system
        };

        let width = size.width.max(1) as f32;
        let height = size.height.max(1) as f32;
        let ui_scale = self.state.ui_scale_factor();
        let is_maximized = self
            .window
            .as_ref()
            .map(|w| w.is_maximized())
            .unwrap_or(false);

        // Clone the Rc so `cx` can hold `&SignalStore` independently of
        // `self.state` (which we need to borrow mutably for build_ui_frame).
        let store = std::rc::Rc::clone(&self.state.store);

        self.editor.set_mouse_pos(self.input.mouse_position());
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
        let mut frame = subsecond::call(|| {
            build_ui_frame(
                &mut self.state,
                &self.theme,
                &mut self.editor,
                scale_text_metrics(text_metrics, ui_scale),
                width,
                height,
                is_maximized,
                &mut cx,
            )
        });

        #[cfg(not(feature = "hot-reload"))]
        let mut frame = build_ui_frame(
            &mut self.state,
            &self.theme,
            &mut self.editor,
            scale_text_metrics(text_metrics, ui_scale),
            width,
            height,
            is_maximized,
            &mut cx,
        );

        let effects = std::mem::take(&mut frame.effects);
        if !effects.is_empty() {
            self.runtime.dispatch_all(effects);
            self.sync_theme();
            self.refresh_window_title();
            self.sync_window_text_input();
            self.mark_dirty();
        }
        frame
    }

    fn dispatch_action(&mut self, action: Action) {
        if matches!(
            action,
            Action::Repository(crate::actions::RepositoryAction::StageHunk)
                | Action::Repository(crate::actions::RepositoryAction::UnstageHunk)
                | Action::Repository(crate::actions::RepositoryAction::DiscardHunk)
                | Action::Repository(crate::actions::RepositoryAction::StageHunkAt(_))
                | Action::Repository(crate::actions::RepositoryAction::UnstageHunkAt(_))
                | Action::Repository(crate::actions::RepositoryAction::DiscardHunkAt(_))
        ) {
            tracing::info!(?action, "dispatch_action: hunk op");
        }
        if let Action::Window(window_action) = &action {
            self.handle_window_action(window_action.clone());
            return;
        }
        let effects = self.state.apply_action(action);
        self.sync_fonts();
        if let Some(renderer) = self.renderer.as_mut() {
            self.state.commit_editor.flush(renderer.font_system_mut());
            self.state
                .review_comment_editor
                .flush(renderer.font_system_mut());
            self.state
                .steering_prompt_editor
                .flush(renderer.font_system_mut());
        }
        self.runtime.dispatch_all(effects);
        self.sync_theme();
        self.refresh_window_title();
        self.sync_window_text_input();
    }

    fn handle_window_action(&mut self, action: crate::actions::WindowAction) {
        use crate::actions::{ResizeEdge, WindowAction};
        use winit::window::ResizeDirection;
        let Some(window) = self.window.as_ref() else {
            return;
        };
        match action {
            WindowAction::Minimize => window.set_minimized(true),
            WindowAction::ToggleMaximize => window.set_maximized(!window.is_maximized()),
            WindowAction::Close => self.exit_requested = true,
            WindowAction::BeginDrag => {
                let _ = window.drag_window();
            }
            WindowAction::BeginResize(edge) => {
                let dir = match edge {
                    ResizeEdge::North => ResizeDirection::North,
                    ResizeEdge::South => ResizeDirection::South,
                    ResizeEdge::East => ResizeDirection::East,
                    ResizeEdge::West => ResizeDirection::West,
                    ResizeEdge::NorthEast => ResizeDirection::NorthEast,
                    ResizeEdge::NorthWest => ResizeDirection::NorthWest,
                    ResizeEdge::SouthEast => ResizeDirection::SouthEast,
                    ResizeEdge::SouthWest => ResizeDirection::SouthWest,
                };
                let _ = window.drag_resize_window(dir);
            }
        }
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
}

#[cfg(target_os = "macos")]
fn configure_chrome(attrs: WindowAttributes) -> WindowAttributes {
    use winit::platform::macos::WindowAttributesExtMacOS;
    attrs
        .with_titlebar_transparent(true)
        .with_fullsize_content_view(true)
        .with_title_hidden(true)
        .with_movable_by_window_background(false)
}

#[cfg(not(target_os = "macos"))]
fn configure_chrome(attrs: WindowAttributes) -> WindowAttributes {
    attrs.with_decorations(false)
}

fn app_window_icon() -> Option<Icon> {
    #[cfg(target_os = "macos")]
    {
        None
    }
    #[cfg(not(target_os = "macos"))]
    {
        const WINDOW_ICON: &[u8] = include_bytes!("../../assets/packaging/png/diffy-256.png");
        let image = image::load_from_memory(WINDOW_ICON).ok()?.into_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        Icon::from_rgba(rgba, width, height).ok()
    }
}

struct DiffyAccessibilityActivation {
    latest_tree: Arc<Mutex<TreeUpdate>>,
}

impl ActivationHandler for DiffyAccessibilityActivation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.latest_tree.lock().ok().map(|tree| tree.clone())
    }
}

struct DiffyAccessibilityActions {
    sender: Sender<ActionRequest>,
    wake_proxy: Option<EventLoopProxy<()>>,
}

impl ActionHandler for DiffyAccessibilityActions {
    fn do_action(&mut self, request: ActionRequest) {
        if self.sender.send(request).is_ok() {
            if let Some(wake_proxy) = &self.wake_proxy {
                let _ = wake_proxy.send_event(());
            }
        }
    }
}

struct DiffyAccessibilityDeactivation;

impl DeactivationHandler for DiffyAccessibilityDeactivation {
    fn deactivate_accessibility(&mut self) {}
}

impl ApplicationHandler for NativeApp {
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        self.process_accessibility_actions();
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
                let accessibility_adapter = self.create_accessibility_adapter(event_loop, &window);
                match Renderer::new(window.clone(), &self.font_settings) {
                    Ok(mut renderer) => {
                        renderer.resize(size.width, size.height, scale_factor);
                        self.renderer = Some(renderer);
                        self.accessibility_adapter = Some(accessibility_adapter);
                        window.set_visible(true);
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
                self.position_traffic_lights();
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

        if let (Some(adapter), Some(window)) =
            (self.accessibility_adapter.as_mut(), self.window.as_ref())
        {
            adapter.process_event(window, &event);
        }

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Focused(true) => {
                if self.rescan_on_next_focus
                    && let Some(path) = self.state.compare.repo_path.get(&self.state.store)
                {
                    self.runtime.dispatch_all(vec![
                        RepositoryEffect::SyncRepository {
                            path,
                            reason: RepositorySyncReason::Rescan,
                            reporter_generation: None,
                        }
                        .into(),
                    ]);
                }
                self.has_seen_focus = true;
                self.rescan_on_next_focus = false;
                self.mark_dirty();
            }
            WindowEvent::Focused(false) => {
                if self.has_seen_focus {
                    self.rescan_on_next_focus = !self.skip_next_focus_regain_rescan;
                    self.skip_next_focus_regain_rescan = false;
                }
                self.mark_dirty();
            }
            WindowEvent::Resized(size) => {
                let scale_factor = self
                    .window
                    .as_ref()
                    .map(|window| window.scale_factor())
                    .unwrap_or(1.0);
                self.sync_window_metrics(size, scale_factor);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let size = self
                    .window
                    .as_ref()
                    .map(|window| window.inner_size())
                    .unwrap_or_else(|| PhysicalSize::new(0, 0));
                self.sync_window_metrics(size, scale_factor);
            }
            WindowEvent::RedrawRequested => {
                let frame_started_at = Instant::now();
                let frame_interval_us = self.hud.frame_started(frame_started_at);
                let now_ms = self.launch_at.elapsed().as_millis() as u64;
                self.tooltip_state.tick(now_ms);
                let build_started_at = Instant::now();
                let frame = self.build_frame();
                self.ui_frame = frame;
                let build_us = build_started_at.elapsed().as_micros() as u64;
                let paint_started_at = Instant::now();
                self.paint_tooltip();
                self.paint_debug_overlay();
                self.publish_accessibility_update();
                for (target, editor) in [
                    (FocusTarget::CommitEditor, &mut self.state.commit_editor),
                    (
                        FocusTarget::ReviewCommentEditor,
                        &mut self.state.review_comment_editor,
                    ),
                    (
                        FocusTarget::SettingsSteeringPrompt,
                        &mut self.state.steering_prompt_editor,
                    ),
                ] {
                    if let Some(ha) = self
                        .ui_frame
                        .text_input_hit_areas
                        .iter()
                        .find(|ha| ha.focus_target == target)
                    {
                        let w = ha.text_width;
                        let h = ha.text_height;
                        let fs = ha.font_size;
                        if let Some(renderer) = self.renderer.as_mut() {
                            editor.set_font_size(renderer.font_system_mut(), fs);
                            editor.sync_size(renderer.font_system_mut(), w, h);
                            editor.flush(renderer.font_system_mut());
                        }
                    }
                }
                let paint_us = paint_started_at.elapsed().as_micros() as u64;
                if let Some(renderer) = self.renderer.as_mut() {
                    let time_seconds = self.launch_at.elapsed().as_secs_f32();
                    let editors: [Option<&crate::editor::Editor>; 3] = [
                        Some(&self.state.commit_editor),
                        Some(&self.state.steering_prompt_editor),
                        Some(&self.state.review_comment_editor),
                    ];
                    match renderer.render(&self.ui_frame.scene, time_seconds, &editors) {
                        Ok(frame) => {
                            self.hud.record(HudSample {
                                build_us,
                                paint_us,
                                render_cpu_us: frame.cpu_us,
                                acquire_us: frame.acquire_us,
                                present_us: frame.present_us,
                                primitive_count: frame.primitive_count,
                                frame_interval_us,
                            });
                        }
                        Err(error) => {
                            eprintln!("render failed: {error}");
                            self.state
                                .last_error
                                .set(&self.state.store, Some(error.to_string()));
                        }
                    }
                }
                self.needs_redraw = false;
                self.state.store.clear_dirty();
            }
            event => {
                let outcome = self.input.handle_window_event(
                    &mut self.state,
                    &mut self.ui_frame,
                    &self.editor,
                    self.renderer.as_mut().map(Renderer::font_system),
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
        if self.exit_requested {
            event_loop.exit();
            return;
        }
        let now = Instant::now();
        self.process_accessibility_actions();
        self.tick_update_polling(now);
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

        let tooltip_was_visible = self.tooltip_state.visible;
        let now_ms = self
            .launch_at
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        self.tooltip_state.tick(now_ms);
        let tooltip_changed = self.tooltip_state.visible != tooltip_was_visible;

        let animating = self.state.animation.has_active();
        let syntax_pack_installing = self.state.syntax_pack_install_active();
        let cursor_blink_changed = self.state.cursor_blink_epoch() != prior_cursor_blink_epoch;
        let debug_overlay = self.state.debug.overlay_visible.get(&self.state.store);
        let next_wake = if debug_overlay {
            Some(now + Duration::from_millis(8))
        } else if animating || syntax_pack_installing {
            Some(now + Duration::from_millis(16))
        } else {
            let next_cursor_blink = self
                .state
                .next_cursor_blink_at_ms()
                .map(|ms| self.launch_at + std::time::Duration::from_millis(ms));
            let next_toast_expiry = self
                .state
                .next_toast_expiry_at_ms()
                .map(|ms| self.launch_at + std::time::Duration::from_millis(ms));
            let next_compare_progress_reveal =
                self.state
                    .compare_progress
                    .with(&self.state.store, |progress| {
                        progress.as_ref().and_then(|progress| {
                            (now_ms < progress.reveal_at_ms).then(|| {
                                self.launch_at
                                    + std::time::Duration::from_millis(progress.reveal_at_ms)
                            })
                        })
                    });
            let next_tooltip = if !self.tooltip_state.text.is_empty() && !self.tooltip_state.visible
            {
                Some(
                    self.launch_at
                        + std::time::Duration::from_millis(self.tooltip_state.show_at_ms),
                )
            } else {
                None
            };
            [
                next_cursor_blink,
                next_toast_expiry,
                next_compare_progress_reveal,
                next_tooltip,
                self.next_update_check_at,
            ]
            .into_iter()
            .flatten()
            .min()
        };

        if let Some(next) = next_wake {
            event_loop.set_control_flow(ControlFlow::WaitUntil(next));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }

        if let Some(window) = self.window.as_ref()
            && (self.needs_redraw
                || self.state.store.any_dirty()
                || animating
                || syntax_pack_installing
                || cursor_blink_changed
                || tooltip_changed
                || debug_overlay)
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
    use crate::core::compare::{CompareMode, CompareOutput};
    use crate::core::forge::github::DeviceFlowState;
    use crate::core::themes::ThemeRegistry;
    use crate::core::vcs::model::{PublishAction, PublishActionKind, PublishPlan};
    use tempfile::TempDir;
    use winit::dpi::PhysicalPosition;
    use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase};
    use winit::keyboard::{ModifiersState, NamedKey};

    use super::NativeApp;
    use crate::apprt::{AppRuntime, AppServices};
    use crate::input::{
        InputEvent, InputOutcome, KeyChord, KeyKind, quantize_scroll_delta_px, scroll_delta_to_px,
    };
    use crate::platform::persistence::SettingsStore;
    use crate::platform::startup::GitHubTokenStore;
    use crate::ui::state::{
        AppState, AppView, FileListEntry, FocusTarget, OverlayEntry, OverlaySurface,
        SettingsSection, WorkspaceMode, WorkspaceSource,
    };

    fn test_app(state: AppState) -> NativeApp {
        let dir = TempDir::new().unwrap();
        let runtime = AppRuntime::new(
            AppServices::new(SettingsStore::new_in(dir.path())),
            None,
            true,
            GitHubTokenStore::Keyring,
        );
        NativeApp::new(state, runtime, ThemeRegistry::load(), None)
    }

    fn route_input_event(app: &mut NativeApp, event: InputEvent) -> InputOutcome {
        app.input.handle_input_event_for_test(
            &mut app.state,
            &mut app.ui_frame,
            &app.editor,
            app.renderer
                .as_mut()
                .map(crate::render::Renderer::font_system),
            app.window.as_ref(),
            &mut app.tooltip_state,
            app.launch_at,
            event,
        )
    }

    fn dispatch_input_event(app: &mut NativeApp, event: InputEvent) {
        let outcome = route_input_event(app, event);
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

    fn named_keypress(named: NamedKey, modifiers: ModifiersState) -> InputEvent {
        InputEvent::KeyPress(KeyChord {
            logical: KeyKind::Named(named),
            physical: None,
            modifiers,
            repeat: false,
        })
    }

    fn publish_action(label: &str) -> PublishAction {
        PublishAction {
            label: label.to_owned(),
            description: format!("{label} description"),
            kind: PublishActionKind::PushRef {
                remote: "origin".to_owned(),
                refspec: "main".to_owned(),
                force_with_lease: false,
            },
            disabled_reason: None,
            change_id_token: None,
        }
    }

    fn compare_file(path: &str) -> carbon::FileDiff {
        carbon::FileDiff {
            old_path: Some(path.to_owned()),
            new_path: Some(path.to_owned()),
            ..carbon::FileDiff::default()
        }
    }

    fn file_header_point(app: &NativeApp, path: &str) -> (f32, f32) {
        let viewport = app.ui_frame.viewport_rect.expect("viewport rect");
        let width = viewport.width.max(1.0).round() as u32;
        let height = viewport.height.max(1.0).round() as u32;
        for y_offset in 0..height {
            let y = viewport.y + y_offset as f32 + 0.5;
            for x_offset in (0..width).step_by(8) {
                let x = viewport.x + x_offset as f32 + 0.5;
                if app.editor.file_header_path_at(x, y).as_deref() == Some(path) {
                    return (x, y);
                }
            }
        }
        panic!("missing file header for {path}");
    }

    #[test]
    fn scroll_delta_to_px_preserves_magnitude_and_direction() {
        let line_delta = scroll_delta_to_px(MouseScrollDelta::LineDelta(0.0, 1.5), 20.0, 1.0);
        assert_eq!(line_delta, -30.0);

        let pixel_delta = scroll_delta_to_px(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -12.5)),
            20.0,
            1.0,
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
        let state = AppState::default();
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state.workspace.files.set(
            &state.store,
            (0..32)
                .map(|index| FileListEntry {
                    path: format!("src/file_{index}.rs").into(),
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
        let state = AppState::default();
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state.workspace.files.set(
            &state.store,
            (0..32)
                .map(|index| FileListEntry {
                    path: format!("src/file_{index}.rs").into(),
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
        let state = AppState::default();
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state.workspace.files.set(
            &state.store,
            vec![FileListEntry {
                path: "src/file_0.rs".into(),
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
        state.apply_action(crate::actions::OverlayAction::OpenRepoPicker);
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
        state.apply_action(crate::actions::EditorAction::OpenSearch);
        let mut app = test_app(state);

        dispatch_input_event(&mut app, keypress("p", ModifiersState::SUPER));

        assert_eq!(
            app.state.overlays_top(),
            Some(OverlaySurface::CommandPalette)
        );
    }

    #[test]
    fn clicking_file_row_selects_exact_file() {
        let state = AppState::default();
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state.workspace.files.set(
            &state.store,
            vec![
                FileListEntry {
                    path: "src/ui/state/mod.rs".into(),
                },
                FileListEntry {
                    path: "src/ui/state/text_edit.rs".into(),
                },
            ],
        );
        state.workspace.compare_output.set(
            &state.store,
            Some(CompareOutput {
                carbon: carbon::DiffDocument {
                    files: vec![
                        compare_file("src/ui/state/mod.rs"),
                        compare_file("src/ui/state/text_edit.rs"),
                    ],
                },
                ..CompareOutput::default()
            }),
        );
        state
            .workspace
            .selected_file_index
            .set(&state.store, Some(0));
        state
            .workspace
            .selected_file_path
            .set(&state.store, Some("src/ui/state/mod.rs".to_owned()));

        let mut app = test_app(state);
        app.ui_frame = app.build_frame();
        let hit = app
            .ui_frame
            .hits
            .iter()
            .rev()
            .find(|hit| matches!(hit.identity, Some(crate::ui::element::HitIdentity::File(1))))
            .expect("text_edit.rs file row hit");
        let x = hit.rect.x + hit.rect.width * 0.5;
        let y = hit.rect.y + hit.rect.height * 0.5;

        dispatch_input_event(&mut app, InputEvent::PointerMoved { x, y });
        dispatch_input_event(
            &mut app,
            InputEvent::PointerButton {
                button: MouseButton::Left,
                state: ElementState::Pressed,
            },
        );

        assert_eq!(
            app.state.workspace.selected_file_path.get(&app.state.store),
            Some("src/ui/state/text_edit.rs".to_owned())
        );
        assert_eq!(
            app.state.focus.get(&app.state.store),
            Some(FocusTarget::FileList)
        );
    }

    #[test]
    fn clicking_continuous_file_header_selects_exact_file() {
        let mut state = AppState::default();
        state.settings.continuous_scroll = true;
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state
            .workspace
            .source
            .set(&state.store, WorkspaceSource::Compare);
        state.workspace.files.set(
            &state.store,
            vec![
                FileListEntry {
                    path: "src/ui/state/mod.rs".into(),
                },
                FileListEntry {
                    path: "src/ui/state/text_edit.rs".into(),
                },
            ],
        );
        state.workspace.compare_output.set(
            &state.store,
            Some(CompareOutput {
                carbon: carbon::DiffDocument {
                    files: vec![
                        compare_file("src/ui/state/mod.rs"),
                        compare_file("src/ui/state/text_edit.rs"),
                    ],
                },
                ..CompareOutput::default()
            }),
        );
        state
            .workspace
            .selected_file_index
            .set(&state.store, Some(0));
        state
            .workspace
            .selected_file_path
            .set(&state.store, Some("src/ui/state/mod.rs".to_owned()));

        let mut app = test_app(state);
        app.ui_frame = app.build_frame();
        let (x, y) = file_header_point(&app, "src/ui/state/text_edit.rs");

        dispatch_input_event(&mut app, InputEvent::PointerMoved { x, y });
        dispatch_input_event(
            &mut app,
            InputEvent::PointerButton {
                button: MouseButton::Left,
                state: ElementState::Pressed,
            },
        );

        assert_eq!(
            app.state.workspace.selected_file_path.get(&app.state.store),
            Some("src/ui/state/text_edit.rs".to_owned())
        );
        assert_eq!(
            app.state.focus.get(&app.state.store),
            Some(FocusTarget::Editor)
        );

        app.ui_frame = app.build_frame();
        assert_eq!(
            app.state.workspace.selected_file_path.get(&app.state.store),
            Some("src/ui/state/text_edit.rs".to_owned())
        );
    }

    #[test]
    fn escape_closes_search_while_search_input_focused() {
        let mut state = AppState::default();
        state.apply_action(crate::actions::EditorAction::OpenSearch);
        let mut app = test_app(state);

        dispatch_input_event(
            &mut app,
            named_keypress(NamedKey::Escape, ModifiersState::empty()),
        );

        assert!(!app.state.editor.search.open.get(&app.state.store));
        assert_eq!(
            app.state.focus.get(&app.state.store),
            Some(FocusTarget::Editor)
        );
    }

    #[test]
    fn question_mark_toggles_keyboard_shortcuts_overlay() {
        let mut app = test_app(AppState::default());

        dispatch_input_event(&mut app, keypress("?", ModifiersState::empty()));
        assert_eq!(app.state.app_view.get(&app.state.store), AppView::Settings);
        assert_eq!(
            app.state.settings_section.get(&app.state.store),
            SettingsSection::Keymaps
        );
    }

    #[test]
    fn compare_menu_number_key_selects_compare_mode() {
        let state = AppState::default();
        state.overlays.stack.update(&state.store, |stack| {
            stack.push(OverlayEntry {
                surface: OverlaySurface::CompareMenu,
                focus_return: None,
            });
        });
        let mut app = test_app(state);

        dispatch_input_event(&mut app, keypress("2", ModifiersState::empty()));

        assert_eq!(
            app.state.compare.mode.get(&app.state.store),
            CompareMode::TwoDot
        );
        assert_eq!(app.state.overlays_top(), None);
    }

    #[test]
    fn sidebar_tab_keys_switch_files_and_commits() {
        let repo_dir = TempDir::new().unwrap();
        let state = AppState::default();
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state
            .workspace
            .source
            .set(&state.store, crate::ui::state::WorkspaceSource::Compare);
        state.workspace.compare_history_pending.set(
            &state.store,
            Some(crate::effects::CompareHistoryRequest {
                repo_path: repo_dir.path().to_path_buf(),
                left_ref: "main".to_owned(),
                right_ref: "HEAD".to_owned(),
            }),
        );
        let mut app = test_app(state);

        dispatch_input_event(&mut app, keypress("C", ModifiersState::empty()));
        assert_eq!(
            app.state.file_list.tab.get(&app.state.store),
            crate::ui::state::SidebarTab::Commits
        );

        dispatch_input_event(&mut app, keypress("F", ModifiersState::empty()));
        assert_eq!(
            app.state.file_list.tab.get(&app.state.store),
            crate::ui::state::SidebarTab::Files
        );
    }

    #[test]
    fn escape_restores_compare_from_commit_drilldown() {
        let state = AppState::default();
        state.workspace.pre_drill_compare.set(
            &state.store,
            Some(("main".to_owned(), "HEAD".to_owned(), CompareMode::TwoDot)),
        );
        let mut app = test_app(state);

        let outcome = route_input_event(
            &mut app,
            named_keypress(NamedKey::Escape, ModifiersState::empty()),
        );

        assert_eq!(
            outcome.actions,
            vec![crate::actions::CompareAction::ClearSidebarCommit.into()]
        );
    }

    #[test]
    fn publish_menu_enter_dispatches_primary_action() {
        let primary = publish_action("Publish primary");
        let state = AppState::default();
        state.repository.publish_plan.set(
            &state.store,
            Some(PublishPlan {
                primary: primary.clone(),
                alternatives: vec![publish_action("Publish alternate")],
            }),
        );
        state.overlays.stack.update(&state.store, |stack| {
            stack.push(OverlayEntry {
                surface: OverlaySurface::PublishMenu,
                focus_return: None,
            });
        });
        let mut app = test_app(state);

        let outcome = route_input_event(
            &mut app,
            named_keypress(NamedKey::Enter, ModifiersState::empty()),
        );

        assert_eq!(
            outcome.actions,
            vec![crate::actions::RepositoryAction::Publish(primary).into()]
        );
    }

    #[test]
    fn account_menu_number_keys_dispatch_menu_actions() {
        let state = AppState::default();
        state.overlays.stack.update(&state.store, |stack| {
            stack.push(OverlayEntry {
                surface: OverlaySurface::AccountMenu,
                focus_return: None,
            });
        });
        let mut app = test_app(state);

        let outcome = route_input_event(&mut app, keypress("1", ModifiersState::empty()));
        assert_eq!(
            outcome.actions,
            vec![crate::actions::SettingsAction::OpenSettings.into()]
        );

        let outcome = route_input_event(&mut app, keypress("2", ModifiersState::empty()));
        assert_eq!(
            outcome.actions,
            vec![crate::actions::GitHubAction::SignOutGitHub.into()]
        );
    }

    #[test]
    fn github_auth_modal_keys_copy_code_and_open_browser() {
        let state = AppState::default();
        state.github.auth.device_flow.set(
            &state.store,
            Some(DeviceFlowState {
                device_code: "device".to_owned(),
                user_code: "ABCD-1234".to_owned(),
                verification_uri: "https://github.com/login/device".to_owned(),
                interval: 5,
            }),
        );
        state.overlays.stack.update(&state.store, |stack| {
            stack.push(OverlayEntry {
                surface: OverlaySurface::GitHubAuthModal,
                focus_return: None,
            });
        });
        let mut app = test_app(state);

        let outcome = route_input_event(&mut app, keypress("c", ModifiersState::empty()));
        assert_eq!(
            outcome.actions,
            vec![crate::actions::AppAction::CopyText("ABCD-1234".to_owned()).into()]
        );

        let outcome = route_input_event(&mut app, keypress("o", ModifiersState::empty()));
        assert_eq!(
            outcome.actions,
            vec![crate::actions::GitHubAction::OpenDeviceFlowBrowser.into()]
        );
    }

    #[test]
    fn confirmation_overlay_keys_confirm_and_cancel() {
        let state = AppState::default();
        state.overlays.stack.update(&state.store, |stack| {
            stack.push(OverlayEntry {
                surface: OverlaySurface::Confirmation,
                focus_return: None,
            });
        });
        let mut app = test_app(state);

        let outcome = route_input_event(&mut app, keypress("y", ModifiersState::empty()));
        assert_eq!(
            outcome.actions,
            vec![crate::actions::OverlayAction::ConfirmOverlaySelection.into()]
        );

        let outcome = route_input_event(&mut app, keypress("n", ModifiersState::empty()));
        assert_eq!(
            outcome.actions,
            vec![crate::actions::OverlayAction::CloseOverlay.into()]
        );

        let outcome = route_input_event(
            &mut app,
            named_keypress(NamedKey::Enter, ModifiersState::empty()),
        );
        assert_eq!(
            outcome.actions,
            vec![crate::actions::OverlayAction::ConfirmOverlaySelection.into()]
        );
    }

    #[test]
    fn publish_action_closes_publish_menu() {
        let repo_dir = TempDir::new().unwrap();
        let primary = publish_action("Publish primary");
        let state = AppState::default();
        state
            .compare
            .repo_path
            .set(&state.store, Some(repo_dir.path().to_path_buf()));
        state.overlays.stack.update(&state.store, |stack| {
            stack.push(OverlayEntry {
                surface: OverlaySurface::PublishMenu,
                focus_return: None,
            });
        });

        let mut app = test_app(state);
        let effects = app
            .state
            .apply_action(crate::actions::RepositoryAction::Publish(primary));

        assert_eq!(app.state.overlays_top(), None);
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn row_cursor_keys_move_visible_editor_cursor() {
        let state = AppState::default();
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state.focus.set(&state.store, Some(FocusTarget::Editor));
        state.editor.visible_row_start.set(&state.store, Some(4));
        state.editor.visible_row_end.set(&state.store, Some(8));
        let mut app = test_app(state);

        dispatch_input_event(&mut app, keypress("J", ModifiersState::empty()));
        assert_eq!(app.state.editor.hovered_row.get(&app.state.store), Some(4));

        dispatch_input_event(&mut app, keypress("J", ModifiersState::empty()));
        assert_eq!(app.state.editor.hovered_row.get(&app.state.store), Some(5));

        dispatch_input_event(&mut app, keypress("K", ModifiersState::empty()));
        assert_eq!(app.state.editor.hovered_row.get(&app.state.store), Some(4));
    }

    #[test]
    fn line_selection_keys_dispatch_current_line_actions() {
        let state = AppState::default();
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state
            .workspace
            .source
            .set(&state.store, crate::ui::state::WorkspaceSource::Status);
        state.focus.set(&state.store, Some(FocusTarget::Editor));
        let mut app = test_app(state);

        let toggle = route_input_event(&mut app, keypress("v", ModifiersState::empty()));
        assert_eq!(
            toggle.actions,
            vec![crate::actions::RepositoryAction::ToggleCurrentLineSelection.into()]
        );

        let range = route_input_event(&mut app, keypress("V", ModifiersState::empty()));
        assert_eq!(
            range.actions,
            vec![crate::actions::RepositoryAction::ToggleCurrentLineSelectionRange.into()]
        );
    }

    #[test]
    fn review_comment_editor_keyboard_submit_and_cancel() {
        let state = AppState::default();
        state
            .focus
            .set(&state.store, Some(FocusTarget::ReviewCommentEditor));
        let mut app = test_app(state);

        let submit = route_input_event(
            &mut app,
            named_keypress(NamedKey::Enter, ModifiersState::SUPER),
        );
        assert_eq!(
            submit.actions,
            vec![crate::actions::GitHubAction::SubmitReviewComment.into()]
        );

        let cancel = route_input_event(
            &mut app,
            named_keypress(NamedKey::Escape, ModifiersState::empty()),
        );
        assert_eq!(
            cancel.actions,
            vec![crate::actions::GitHubAction::CancelReviewComment.into()]
        );
    }

    #[test]
    fn settings_number_and_navigation_keys_switch_sections() {
        let state = AppState::default();
        state.app_view.set(&state.store, AppView::Settings);
        let mut app = test_app(state);

        dispatch_input_event(&mut app, keypress("3", ModifiersState::empty()));
        assert_eq!(
            app.state.settings_section.get(&app.state.store),
            SettingsSection::Behavior
        );

        dispatch_input_event(&mut app, keypress("j", ModifiersState::empty()));
        assert_eq!(
            app.state.settings_section.get(&app.state.store),
            SettingsSection::Keymaps
        );

        dispatch_input_event(
            &mut app,
            named_keypress(NamedKey::ArrowUp, ModifiersState::empty()),
        );
        assert_eq!(
            app.state.settings_section.get(&app.state.store),
            SettingsSection::Behavior
        );
    }

    #[test]
    fn settings_control_keys_dispatch_existing_actions() {
        let state = AppState::default();
        state.app_view.set(&state.store, AppView::Settings);
        let mut app = test_app(state);

        dispatch_input_event(&mut app, keypress("w", ModifiersState::empty()));
        assert!(app.state.editor.wrap_enabled.get(&app.state.store));

        dispatch_input_event(&mut app, keypress("c", ModifiersState::empty()));
        assert!(app.state.settings.continuous_scroll);

        let outcome = route_input_event(&mut app, keypress("u", ModifiersState::empty()));
        assert_eq!(
            outcome.actions,
            vec![crate::actions::UpdateAction::CheckForUpdates.into()]
        );
    }

    #[test]
    fn keymap_rebind_overrides_default_shortcut() {
        let state = AppState::default();
        state.app_view.set(&state.store, AppView::Settings);
        state
            .settings_section
            .set(&state.store, SettingsSection::Keymaps);
        state.keymap_capture.set(
            &state.store,
            Some(crate::input::ShortcutCommand::ToggleWrap),
        );
        let mut app = test_app(state);

        dispatch_input_event(&mut app, keypress("z", ModifiersState::empty()));
        assert_eq!(app.state.settings.keymap_overrides.len(), 1);

        dispatch_input_event(&mut app, keypress("w", ModifiersState::empty()));
        assert!(!app.state.editor.wrap_enabled.get(&app.state.store));

        dispatch_input_event(&mut app, keypress("z", ModifiersState::empty()));
        assert!(app.state.editor.wrap_enabled.get(&app.state.store));
    }

    #[test]
    fn vim_focus_keys_switch_file_list_and_editor_focus() {
        let state = AppState::default();
        state.workspace_mode.set(&state.store, WorkspaceMode::Ready);
        state.focus.set(&state.store, Some(FocusTarget::FileList));
        let mut app = test_app(state);

        dispatch_input_event(&mut app, keypress("l", ModifiersState::empty()));
        assert_eq!(
            app.state.focus.get(&app.state.store),
            Some(FocusTarget::Editor)
        );

        dispatch_input_event(&mut app, keypress("h", ModifiersState::empty()));
        assert_eq!(
            app.state.focus.get(&app.state.store),
            Some(FocusTarget::FileList)
        );
    }

    #[test]
    fn ime_commit_inserts_once_into_text_field() {
        let mut state = AppState::default();
        state.apply_action(crate::actions::EditorAction::OpenSearch);
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

fn logging_env_base() -> String {
    std::env::var("RUST_LOG")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "info".to_owned())
}

fn stdout_logging_filter(log_debug: bool) -> tracing_subscriber::EnvFilter {
    use tracing_subscriber::EnvFilter;

    if log_debug {
        return EnvFilter::new("debug");
    }

    // Keep difftastic internals and Diffy's difftastic diagnostics out of
    // stdout during normal runs. DIFFY_LOG_DEBUG=1 still enables them.
    let base = logging_env_base();
    let filter =
        format!("{base},difftastic=off,vendored_difftastic=off,difft=off,diffy::difftastic=off");

    EnvFilter::try_new(filter).unwrap_or_else(|_| {
        EnvFilter::new(
            "info,difftastic=off,vendored_difftastic=off,difft=off,diffy::difftastic=off",
        )
    })
}

fn file_logging_filter(log_debug: bool) -> tracing_subscriber::EnvFilter {
    use tracing_subscriber::EnvFilter;

    if log_debug {
        return EnvFilter::new("debug");
    }

    // Log concise Diffy-owned difftastic diagnostics to the file log, but keep
    // vendored difftastic's chatty info-level internals disabled unless debug is
    // explicitly requested.
    let base = logging_env_base();
    let filter =
        format!("{base},difftastic=off,vendored_difftastic=off,difft=off,diffy::difftastic=info");

    EnvFilter::try_new(filter).unwrap_or_else(|_| {
        EnvFilter::new(
            "info,difftastic=off,vendored_difftastic=off,difft=off,diffy::difftastic=info",
        )
    })
}

fn init_logging(log_debug: bool) {
    use std::sync::Mutex;
    use tracing_subscriber::Layer;
    use tracing_subscriber::fmt;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let stdout_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .with_filter(stdout_logging_filter(log_debug));

    let (file_layer, file_path) = log_dir()
        .and_then(|dir| {
            std::fs::create_dir_all(&dir).ok()?;
            let path = dir.join("diffy.log");
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .ok()?;
            let layer = fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(file))
                .with_filter(file_logging_filter(log_debug));
            Some((layer, path))
        })
        .map(|(l, p)| (Some(l), Some(p)))
        .unwrap_or((None, None));

    tracing_subscriber::registry()
        .with(stdout_layer)
        .with(file_layer)
        .init();

    if let Some(path) = file_path {
        tracing::info!(path = %path.display(), "logging initialized");
    } else {
        tracing::warn!("logging: unable to open log file, falling back to stdout only");
    }
}
