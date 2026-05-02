use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::core::error::{DiffyError, Result};

#[derive(Debug, Clone)]
pub struct JjCli {
    binary: PathBuf,
    root: PathBuf,
}

impl JjCli {
    pub fn new(root: PathBuf) -> Self {
        Self {
            binary: PathBuf::from("jj"),
            root,
        }
    }

    pub fn with_binary(root: PathBuf, binary: PathBuf) -> Self {
        Self { binary, root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn run(&self, args: &[OsString]) -> Result<String> {
        self.run_inner(args, false)
    }

    pub fn run_ignored_wc(&self, args: &[OsString]) -> Result<String> {
        self.run_inner(args, true)
    }

    fn run_inner(&self, args: &[OsString], ignore_working_copy: bool) -> Result<String> {
        let started = Instant::now();
        let mut command = Command::new(&self.binary);
        command
            .arg("--no-pager")
            .arg("--color=never")
            .arg("--quiet")
            .current_dir(&self.root);
        if ignore_working_copy {
            command.arg("--ignore-working-copy");
        }
        command.args(args);

        let output = command
            .output()
            .map_err(|error| DiffyError::General(format!("failed to run jj: {error}")))?;
        let command_label = command_label(args);
        let elapsed = started.elapsed();
        if output.status.success() {
            tracing::debug!(
                repo = %self.root.display(),
                command = %command_label,
                ignore_working_copy,
                elapsed_ms = elapsed.as_millis(),
                "jj command finished",
            );
            return String::from_utf8(output.stdout).map_err(|error| {
                DiffyError::General(format!("jj emitted non-UTF8 output: {error}"))
            });
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = stderr
            .trim()
            .lines()
            .last()
            .or_else(|| stdout.trim().lines().last())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("jj exited with {}", output.status));
        Err(DiffyError::General(format!(
            "jj {} failed: {detail}",
            command_label
        )))
    }
}

fn command_label(args: &[OsString]) -> String {
    args.iter()
        .take_while(|arg| arg.as_os_str() != "--")
        .map(|arg| sanitize_arg(&arg.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn sanitize_arg(arg: &str) -> String {
    const MAX_ARG_LEN: usize = 96;
    if arg.len() <= MAX_ARG_LEN {
        return arg.to_owned();
    }
    let end = arg
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= MAX_ARG_LEN)
        .last()
        .unwrap_or(0);
    format!("{}...", &arg[..end])
}
