fn log_dir() -> Option<std::path::PathBuf> {
    crate::platform::startup::app_log_dir()
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

pub fn init_logging(log_debug: bool) {
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
