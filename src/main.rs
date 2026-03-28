#[cfg(feature = "qt")]
use std::cell::RefCell;
#[cfg(feature = "qt")]
use std::collections::HashSet;
#[cfg(feature = "qt")]
use std::env;
#[cfg(feature = "qt")]
use std::path::{Path, PathBuf};
#[cfg(feature = "qt")]
use std::time::Duration;

#[cfg(feature = "qt")]
use cstr::cstr;
#[cfg(feature = "qt")]
use diffy::app::controller::DiffController;
#[cfg(feature = "qt")]
use diffy::app::surface::item::DiffSurfaceItem;
#[cfg(feature = "qt")]
use diffy::app::theme::ThemeProvider;
#[cfg(feature = "qt")]
use qmetaobject::prelude::*;
#[cfg(feature = "qt")]
use qmetaobject::{QObjectPinned, QVariant, qml_register_type, single_shot};

#[cfg(feature = "qt")]
#[derive(Clone, Default)]
struct StartupConfig {
    repo_path: Option<String>,
    left_ref: Option<String>,
    right_ref: Option<String>,
    compare_mode: Option<String>,
    layout_mode: Option<String>,
    renderer: Option<String>,
    start_compare: bool,
    selected_file_index: Option<i32>,
    selected_file_path: Option<String>,
    require_results: bool,
    print_file_list: bool,
    exit_after_ms: Option<u64>,
}

#[cfg(feature = "qt")]
impl StartupConfig {
    fn from_env() -> Self {
        Self {
            repo_path: env_var("DIFFY_START_REPO"),
            left_ref: env_var("DIFFY_START_LEFT"),
            right_ref: env_var("DIFFY_START_RIGHT"),
            compare_mode: env_var("DIFFY_START_COMPARE_MODE"),
            layout_mode: env_var("DIFFY_START_LAYOUT"),
            renderer: env_var("DIFFY_START_RENDERER"),
            start_compare: env_flag("DIFFY_START_COMPARE"),
            selected_file_index: parse_i32("DIFFY_START_FILE_INDEX"),
            selected_file_path: env_var("DIFFY_START_FILE_PATH"),
            require_results: env_flag("DIFFY_REQUIRE_RESULTS"),
            print_file_list: env_flag("DIFFY_PRINT_FILE_LIST"),
            exit_after_ms: parse_non_negative_ms("DIFFY_EXIT_AFTER_MS"),
        }
    }

    fn needs_post_compare_work(&self) -> bool {
        self.start_compare
            || self.selected_file_index.is_some()
            || self.selected_file_path.is_some()
            || self.require_results
            || self.print_file_list
    }
}

#[cfg(feature = "qt")]
fn main() {
    let _ = env_logger::try_init();

    qml_register_type::<DiffSurfaceItem>(cstr!("Diffy.Native"), 1, 0, cstr!("DiffSurface"));

    let engine = Box::leak(Box::new(QmlEngine::new()));
    add_qml_import_paths(engine);

    let theme = Box::leak(Box::new(RefCell::new(ThemeProvider::default())));
    let controller = Box::leak(Box::new(RefCell::new(DiffController::default())));

    engine.set_object_property("theme".into(), unsafe { QObjectPinned::new(theme) });
    engine.set_object_property("diffController".into(), unsafe {
        QObjectPinned::new(controller)
    });

    let main_qml = discover_main_qml().unwrap_or_else(|| {
        eprintln!(
            "Could not locate qml/Main.qml. Checked DIFFY_REPO_ROOT, CARGO_MANIFEST_DIR, current directory, and executable ancestors."
        );
        std::process::exit(1);
    });
    engine.load_file(QString::from(main_qml.to_string_lossy().as_ref()));

    let startup = StartupConfig::from_env();
    if let Err(error) = apply_startup_config(controller, &startup) {
        eprintln!("Startup automation failed: {error}");
        std::process::exit(1);
    }

    if startup.needs_post_compare_work() {
        schedule_post_compare_work(controller, startup.clone(), 160);
    }

    if let Some(exit_after_ms) = startup.exit_after_ms {
        schedule_exit(engine, exit_after_ms);
    }

    engine.exec();
}

#[cfg(feature = "qt")]
fn add_qml_import_paths(engine: &mut QmlEngine) {
    for prefix in split_env_paths("QT_ADDITIONAL_PACKAGES_PREFIX_PATH") {
        let qml_path = [
            prefix.join("qml"),
            prefix.join("lib/qt6/qml"),
            prefix.join("lib/qt-6/qml"),
        ]
        .into_iter()
        .find(|path| path.exists());
        if let Some(path) = qml_path {
            engine.add_import_path(QString::from(path.to_string_lossy().as_ref()));
        }
    }

    for import_path in split_env_paths("QML2_IMPORT_PATH") {
        if import_path.exists() {
            engine.add_import_path(QString::from(import_path.to_string_lossy().as_ref()));
        }
    }

    for import_path in split_env_paths("QML_IMPORT_PATH") {
        if import_path.exists() {
            engine.add_import_path(QString::from(import_path.to_string_lossy().as_ref()));
        }
    }
}

#[cfg(feature = "qt")]
fn discover_main_qml() -> Option<PathBuf> {
    let mut visited = HashSet::new();
    let mut roots = Vec::new();

    if let Some(root) = env::var_os("DIFFY_REPO_ROOT") {
        push_ancestors(&mut roots, PathBuf::from(root));
    }
    push_ancestors(&mut roots, PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    if let Ok(current_dir) = env::current_dir() {
        push_ancestors(&mut roots, current_dir);
    }
    if let Ok(current_exe) = env::current_exe() {
        push_ancestors(&mut roots, current_exe);
    }

    for root in roots {
        if !visited.insert(root.clone()) {
            continue;
        }
        let candidate = root.join("qml/Main.qml");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

#[cfg(feature = "qt")]
fn push_ancestors(paths: &mut Vec<PathBuf>, start: PathBuf) {
    paths.extend(start.ancestors().take(8).map(Path::to_path_buf));
}

#[cfg(feature = "qt")]
fn apply_startup_config(
    controller: &RefCell<DiffController>,
    startup: &StartupConfig,
) -> Result<(), String> {
    let mut controller = controller.borrow_mut();

    if let Some(repo_path) = startup.repo_path.as_deref()
        && !controller.open_repository(QString::from(repo_path))
    {
        return Err(non_empty_error(controller.get_error_message().to_string()));
    }

    if let Some(left_ref) = startup.left_ref.as_deref() {
        controller.set_left_ref(QString::from(left_ref));
    }
    if let Some(right_ref) = startup.right_ref.as_deref() {
        controller.set_right_ref(QString::from(right_ref));
    }
    if let Some(compare_mode) = startup.compare_mode.as_deref() {
        controller.set_compare_mode(QString::from(compare_mode));
    }
    if let Some(layout_mode) = startup.layout_mode.as_deref() {
        controller.set_layout_mode(QString::from(layout_mode));
    }
    if let Some(renderer) = startup.renderer.as_deref() {
        controller.set_renderer(QString::from(renderer));
    }

    if startup.start_compare {
        controller.compare();
        return Ok(());
    }

    if let Some(index) = startup.selected_file_index {
        controller.select_file(index);
    }
    if let Some(path) = startup.selected_file_path.as_deref() {
        let _ = select_file_by_path(&mut controller, path);
    }
    if startup.print_file_list {
        print_file_list(&controller);
    }
    if startup.require_results {
        assert_results_ready(&controller)?;
    }

    Ok(())
}

#[cfg(feature = "qt")]
fn schedule_post_compare_work(
    controller: &'static RefCell<DiffController>,
    startup: StartupConfig,
    remaining_attempts: u32,
) {
    single_shot(Duration::from_millis(50), move || {
        let mut controller_ref = controller.borrow_mut();

        if controller_ref.get_comparing() || controller_ref.get_pull_request_loading() {
            drop(controller_ref);
            if remaining_attempts > 0 {
                schedule_post_compare_work(controller, startup.clone(), remaining_attempts - 1);
            } else if startup.require_results {
                eprintln!("Startup automation failed: compare did not finish in time");
                std::process::exit(1);
            }
            return;
        }

        if let Some(index) = startup.selected_file_index {
            controller_ref.select_file(index);
        }
        if let Some(path) = startup.selected_file_path.as_deref() {
            let _ = select_file_by_path(&mut controller_ref, path);
        }
        if startup.print_file_list {
            print_file_list(&controller_ref);
        }
        if startup.require_results
            && let Err(error) = assert_results_ready(&controller_ref)
        {
            eprintln!("Startup automation failed: {error}");
            std::process::exit(1);
        }
    });
}

#[cfg(feature = "qt")]
fn schedule_exit(engine: &'static QmlEngine, exit_after_ms: u64) {
    single_shot(Duration::from_millis(exit_after_ms), move || {
        engine.quit();
    });
}

#[cfg(feature = "qt")]
fn assert_results_ready(controller: &DiffController) -> Result<(), String> {
    if controller.get_files().is_empty() {
        return Err(non_empty_error(controller.get_error_message().to_string()));
    }
    if !controller.get_selected_file_render_ready()
        || controller.get_selected_file_render_line_count() == 0
    {
        return Err(non_empty_error(controller.get_error_message().to_string()));
    }
    Ok(())
}

#[cfg(feature = "qt")]
fn select_file_by_path(controller: &mut DiffController, selected_file_path: &str) -> bool {
    let files = controller.get_files();
    for (index, file) in files.into_iter().enumerate() {
        let file_path = file
            .to_qvariantmap()
            .value(QString::from("path"), QVariant::default())
            .to_qstring()
            .to_string();
        if file_path == selected_file_path || file_path.ends_with(selected_file_path) {
            controller.select_file(index as i32);
            return true;
        }
    }
    false
}

#[cfg(feature = "qt")]
fn print_file_list(controller: &DiffController) {
    for (index, file) in controller.get_files().into_iter().enumerate() {
        let path = file
            .to_qvariantmap()
            .value(QString::from("path"), QVariant::default())
            .to_qstring()
            .to_string();
        println!("DIFFY_FILE index={index} path={path}");
    }
}

#[cfg(feature = "qt")]
fn split_env_paths(name: &str) -> Vec<PathBuf> {
    env::var_os(name)
        .map(|value| env::split_paths(&value).collect())
        .unwrap_or_default()
}

#[cfg(feature = "qt")]
fn env_var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(feature = "qt")]
fn env_flag(name: &str) -> bool {
    env_var(name)
        .map(|value| {
            let value = value.to_ascii_lowercase();
            value != "0" && value != "false" && value != "no"
        })
        .unwrap_or(false)
}

#[cfg(feature = "qt")]
fn parse_i32(name: &str) -> Option<i32> {
    env_var(name)?.parse().ok()
}

#[cfg(feature = "qt")]
fn parse_non_negative_ms(name: &str) -> Option<u64> {
    let value: i64 = env_var(name)?.parse().ok()?;
    (value >= 0).then_some(value as u64)
}

#[cfg(feature = "qt")]
fn non_empty_error(error: String) -> String {
    if error.trim().is_empty() {
        "operation failed".to_owned()
    } else {
        error
    }
}

#[cfg(not(feature = "qt"))]
fn main() {
    eprintln!("diffy was built without the `qt` feature");
}
