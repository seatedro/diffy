use std::env;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

use crate::core::compare::{CompareMode, LayoutMode, RendererKind};

const DEFAULT_GITHUB_CLIENT_ID: &str = "Ov23lijXMwtY1XmHedUM";

#[derive(Debug, Clone, Parser, PartialEq, Eq)]
#[command(name = "diffy", about = "Native desktop diff viewer")]
pub struct Args {
    #[arg(long, value_name = "PATH")]
    pub repo: Option<PathBuf>,

    #[arg(long)]
    pub left: Option<String>,

    #[arg(long)]
    pub right: Option<String>,

    #[arg(long = "compare-mode", value_parser = parse_compare_mode)]
    pub compare_mode: Option<CompareMode>,

    #[arg(long, value_parser = parse_layout_mode)]
    pub layout: Option<LayoutMode>,

    #[arg(long, value_parser = parse_renderer_kind)]
    pub renderer: Option<RendererKind>,

    #[arg(long = "file-index")]
    pub file_index: Option<usize>,

    #[arg(long = "file-path")]
    pub file_path: Option<String>,

    #[arg(long = "open-pr")]
    pub open_pr: Option<String>,

    #[arg(long = "exit-after-ms")]
    pub exit_after_ms: Option<u64>,

    #[arg(long = "hidden-window", default_value_t = false)]
    pub hidden_window: bool,

    #[arg(long = "dump-state-json", value_name = "PATH")]
    pub dump_state_json: Option<PathBuf>,

    #[arg(long = "dump-files-json", value_name = "PATH")]
    pub dump_files_json: Option<PathBuf>,

    #[arg(long = "dump-errors-json", value_name = "PATH")]
    pub dump_errors_json: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupOptions {
    pub args: Args,
    pub github_token: Option<String>,
    pub github_client_id: String,
    pub log_debug: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct StartupEnvDefaults {
    repo: Option<PathBuf>,
    left: Option<String>,
    right: Option<String>,
    compare_mode: Option<CompareMode>,
    layout: Option<LayoutMode>,
    renderer: Option<RendererKind>,
    file_index: Option<usize>,
    file_path: Option<String>,
    open_pr: Option<String>,
    exit_after_ms: Option<u64>,
}

impl StartupEnvDefaults {
    fn load<F>(get_env: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        Self {
            repo: get_env("DIFFY_START_REPO").map(PathBuf::from),
            left: get_env("DIFFY_START_LEFT"),
            right: get_env("DIFFY_START_RIGHT"),
            compare_mode: parse_env_with(&get_env, "DIFFY_START_COMPARE_MODE", parse_compare_mode),
            layout: parse_env_with(&get_env, "DIFFY_START_LAYOUT", parse_layout_mode),
            renderer: parse_env_with(&get_env, "DIFFY_START_RENDERER", parse_renderer_kind),
            file_index: parse_env_with(&get_env, "DIFFY_START_FILE_INDEX", parse_file_index),
            file_path: get_env("DIFFY_START_FILE_PATH"),
            open_pr: get_env("DIFFY_START_OPEN_PR"),
            exit_after_ms: parse_env_with(&get_env, "DIFFY_EXIT_AFTER_MS", parse_exit_after_ms),
        }
    }

    fn apply(self, mut args: Args) -> Args {
        if args.repo.is_none() {
            args.repo = self.repo;
        }
        if args.left.is_none() {
            args.left = self.left;
        }
        if args.right.is_none() {
            args.right = self.right;
        }
        if args.compare_mode.is_none() {
            args.compare_mode = self.compare_mode;
        }
        if args.layout.is_none() {
            args.layout = self.layout;
        }
        if args.renderer.is_none() {
            args.renderer = self.renderer;
        }
        if args.file_index.is_none() {
            args.file_index = self.file_index;
        }
        if args.file_path.is_none() {
            args.file_path = self.file_path;
        }
        if args.open_pr.is_none() {
            args.open_pr = self.open_pr;
        }
        if args.exit_after_ms.is_none() {
            args.exit_after_ms = self.exit_after_ms;
        }
        args
    }
}

impl StartupOptions {
    pub fn load() -> Self {
        let env_defaults = StartupEnvDefaults::load(env_var);
        Self::from_parts(
            env_defaults.apply(Args::parse()),
            env_var("GITHUB_TOKEN"),
            env_var("DIFFY_GITHUB_CLIENT_ID")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_GITHUB_CLIENT_ID.to_owned()),
            env_flag("DIFFY_LOG_DEBUG"),
        )
    }

    pub fn from_parts(
        args: Args,
        github_token: Option<String>,
        github_client_id: String,
        log_debug: bool,
    ) -> Self {
        Self {
            args,
            github_token: github_token.filter(|value| !value.is_empty()),
            github_client_id,
            log_debug,
        }
    }

    pub fn exit_after(&self) -> Option<Duration> {
        self.args.exit_after_ms.map(Duration::from_millis)
    }

    pub fn hidden_window(&self) -> bool {
        self.args.hidden_window
    }

    pub fn wants_compare(&self, mode: CompareMode, left_ref: &str, right_ref: &str) -> bool {
        if self.args.open_pr.is_some() {
            return true;
        }

        match mode {
            CompareMode::SingleCommit => !left_ref.is_empty() || !right_ref.is_empty(),
            CompareMode::TwoDot | CompareMode::ThreeDot => {
                !left_ref.is_empty() && !right_ref.is_empty()
            }
        }
    }
}

fn parse_compare_mode(value: &str) -> Result<CompareMode, String> {
    value.parse().map_err(str::to_owned)
}

fn parse_layout_mode(value: &str) -> Result<LayoutMode, String> {
    value.parse().map_err(str::to_owned)
}

fn parse_renderer_kind(value: &str) -> Result<RendererKind, String> {
    value.parse().map_err(str::to_owned)
}

fn parse_file_index(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| "invalid file index".to_owned())
}

fn parse_exit_after_ms(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| "invalid exit duration".to_owned())
}

fn parse_env_with<T, FEnv, FParse>(get_env: &FEnv, name: &str, parser: FParse) -> Option<T>
where
    FEnv: Fn(&str) -> Option<String>,
    FParse: Fn(&str) -> Result<T, String>,
{
    let value = get_env(name)?;
    match parser(&value) {
        Ok(parsed) => Some(parsed),
        Err(error) => {
            eprintln!("Ignoring {name}: {error}");
            None
        }
    }
}

fn env_var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn env_flag(name: &str) -> bool {
    env_var(name)
        .map(|value| {
            let value = value.to_ascii_lowercase();
            value != "0" && value != "false" && value != "no"
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use clap::Parser;

    use super::{Args, StartupEnvDefaults, StartupOptions};
    use crate::core::compare::{CompareMode, LayoutMode, RendererKind};

    #[test]
    fn parses_cli_contract() {
        let args = Args::parse_from([
            "diffy",
            "--repo",
            "C:\\work\\demo",
            "--left",
            "main",
            "--right",
            "feature",
            "--compare-mode",
            "three-dot",
            "--layout",
            "split",
            "--renderer",
            "difftastic",
            "--file-index",
            "3",
            "--file-path",
            "src/main.rs",
            "--open-pr",
            "https://github.com/owner/repo/pull/42",
            "--exit-after-ms",
            "25",
            "--hidden-window",
            "--dump-state-json",
            "state.json",
            "--dump-files-json",
            "files.json",
            "--dump-errors-json",
            "errors.json",
        ]);

        assert_eq!(args.repo.unwrap(), PathBuf::from("C:\\work\\demo"));
        assert_eq!(args.left.as_deref(), Some("main"));
        assert_eq!(args.right.as_deref(), Some("feature"));
        assert_eq!(args.compare_mode, Some(CompareMode::ThreeDot));
        assert_eq!(args.layout, Some(LayoutMode::Split));
        assert_eq!(args.renderer, Some(RendererKind::Difftastic));
        assert_eq!(args.file_index, Some(3));
        assert!(args.hidden_window);
    }

    #[test]
    fn startup_env_overrides_are_preserved() {
        let options = StartupOptions::from_parts(
            Args::parse_from(["diffy"]),
            Some("token".to_owned()),
            "client".to_owned(),
            true,
        );

        assert_eq!(options.github_token.as_deref(), Some("token"));
        assert_eq!(options.github_client_id, "client");
        assert!(options.log_debug);
    }

    #[test]
    fn startup_env_defaults_fill_missing_args() {
        let env = HashMap::from([
            ("DIFFY_START_REPO", "C:\\work\\demo"),
            ("DIFFY_START_LEFT", "main"),
            ("DIFFY_START_RIGHT", "feature"),
            ("DIFFY_START_COMPARE_MODE", "three-dot"),
            ("DIFFY_START_LAYOUT", "split"),
            ("DIFFY_START_RENDERER", "difftastic"),
            ("DIFFY_START_FILE_INDEX", "3"),
            ("DIFFY_START_FILE_PATH", "src/main.rs"),
            (
                "DIFFY_START_OPEN_PR",
                "https://github.com/owner/repo/pull/42",
            ),
            ("DIFFY_EXIT_AFTER_MS", "25"),
        ]);
        let args = StartupEnvDefaults::load(|name| env.get(name).map(|value| (*value).to_owned()))
            .apply(Args::parse_from(["diffy"]));

        assert_eq!(args.repo, Some(PathBuf::from("C:\\work\\demo")));
        assert_eq!(args.left.as_deref(), Some("main"));
        assert_eq!(args.right.as_deref(), Some("feature"));
        assert_eq!(args.compare_mode, Some(CompareMode::ThreeDot));
        assert_eq!(args.layout, Some(LayoutMode::Split));
        assert_eq!(args.renderer, Some(RendererKind::Difftastic));
        assert_eq!(args.file_index, Some(3));
        assert_eq!(args.file_path.as_deref(), Some("src/main.rs"));
        assert_eq!(
            args.open_pr.as_deref(),
            Some("https://github.com/owner/repo/pull/42")
        );
        assert_eq!(args.exit_after_ms, Some(25));
    }

    #[test]
    fn startup_env_defaults_do_not_override_cli_args() {
        let env = HashMap::from([
            ("DIFFY_START_REPO", "C:\\work\\env"),
            ("DIFFY_START_LEFT", "env-left"),
            ("DIFFY_START_RIGHT", "env-right"),
            ("DIFFY_START_COMPARE_MODE", "three-dot"),
            ("DIFFY_START_LAYOUT", "split"),
            ("DIFFY_START_RENDERER", "difftastic"),
            ("DIFFY_START_FILE_INDEX", "7"),
            ("DIFFY_START_FILE_PATH", "env.rs"),
            ("DIFFY_START_OPEN_PR", "https://github.com/owner/repo/pull/7"),
            ("DIFFY_EXIT_AFTER_MS", "99"),
        ]);
        let args = StartupEnvDefaults::load(|name| env.get(name).map(|value| (*value).to_owned()))
            .apply(Args::parse_from([
                "diffy",
                "--repo",
                "C:\\work\\cli",
                "--left",
                "cli-left",
                "--right",
                "cli-right",
                "--compare-mode",
                "two-dot",
                "--layout",
                "unified",
                "--renderer",
                "builtin",
                "--file-index",
                "2",
                "--file-path",
                "cli.rs",
                "--open-pr",
                "https://github.com/owner/repo/pull/2",
                "--exit-after-ms",
                "10",
            ]));

        assert_eq!(args.repo, Some(PathBuf::from("C:\\work\\cli")));
        assert_eq!(args.left.as_deref(), Some("cli-left"));
        assert_eq!(args.right.as_deref(), Some("cli-right"));
        assert_eq!(args.compare_mode, Some(CompareMode::TwoDot));
        assert_eq!(args.layout, Some(LayoutMode::Unified));
        assert_eq!(args.renderer, Some(RendererKind::Builtin));
        assert_eq!(args.file_index, Some(2));
        assert_eq!(args.file_path.as_deref(), Some("cli.rs"));
        assert_eq!(
            args.open_pr.as_deref(),
            Some("https://github.com/owner/repo/pull/2")
        );
        assert_eq!(args.exit_after_ms, Some(10));
    }
}
