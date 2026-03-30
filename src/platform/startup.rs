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

impl StartupOptions {
    pub fn load() -> Self {
        Self::from_parts(
            Args::parse(),
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
    use std::path::PathBuf;

    use clap::Parser;

    use super::{Args, StartupOptions};
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
}
