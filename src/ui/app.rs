use std::error::Error;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::app_runtime::{AppRuntime, AppServices};
use crate::platform::automation::{ErrorDump, FilesDump, StateDump, write_json};
use crate::platform::persistence::SettingsStore;
use crate::platform::startup::StartupOptions;
use crate::render::renderer::Renderer;
use crate::render::scene::{RectPrimitive, Scene, TextPrimitive};
use crate::ui::state::AppState;
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
    renderer: Renderer,
    window: Option<Window>,
    launch_at: Instant,
    dumps_dirty: bool,
}

impl NativeApp {
    fn new(state: AppState, runtime: AppRuntime) -> Self {
        Self {
            state,
            theme: Theme::default_dark(),
            runtime,
            renderer: Renderer::default(),
            window: None,
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
        self.window.as_ref().map(Window::id)
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

    fn build_scene(&self) -> Scene {
        let mut scene = Scene::default();
        scene.rect(RectPrimitive {
            x: 0.0,
            y: 0.0,
            width: 1280.0,
            height: 800.0,
            color: self.theme.colors.app_bg,
        });
        scene.text(TextPrimitive {
            x: 24.0,
            y: 28.0,
            text: self.state.window_title(),
            color: self.theme.colors.text_strong,
            font_size: 18.0,
        });
        scene.text(TextPrimitive {
            x: 24.0,
            y: 56.0,
            text: format!(
                "repo status: {:?} | compare status: {:?} | files: {}",
                self.state.repository.status,
                self.state.workspace.status,
                self.state.workspace.files.len()
            ),
            color: self.theme.colors.text_muted,
            font_size: 14.0,
        });
        if let Some(message) = self.state.last_error.as_deref() {
            scene.text(TextPrimitive {
                x: 24.0,
                y: 84.0,
                text: format!("error: {message}"),
                color: self.theme.colors.line_del,
                font_size: 14.0,
            });
        }
        scene
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
                let size = window.inner_size();
                self.renderer.resize(size.width, size.height);
                self.window = Some(window);
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
                self.renderer.resize(size.width, size.height);
                self.dumps_dirty = true;
            }
            WindowEvent::RedrawRequested => {
                let frame_started_at = Instant::now();
                let scene = self.build_scene();
                let frame = self.renderer.render(&scene);
                self.state.debug.last_scene_primitive_count = frame.primitive_count;
                self.state.debug.last_frame_time_us = frame_started_at
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX))
                    as u64;
                self.dumps_dirty = true;
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
