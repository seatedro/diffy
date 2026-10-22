use std::path::Path;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use crate::ai::{self, CliCommand, CliScan, GenerateRequest, RunOutcome, StreamMessage};
use crate::contracts::mutations::{
    AiBackend, GenerateCommitMessageRequest, MutationsEvent, StagedDigest,
};
use crate::contracts::review::Secret;
use crate::core::error::Result;
use crate::core::vcs::discovery;
use crate::platform::secrets::{self, AiKeyKind};

#[derive(Debug, Clone, Copy, Default)]
pub struct MutationsServices;

impl MutationsServices {
    pub fn load_ai_keys(&self) -> Result<(Option<Secret>, Option<Secret>)> {
        let openai = secrets::load_ai_key(AiKeyKind::OpenAi)?.map(Secret::new);
        let anthropic = secrets::load_ai_key(AiKeyKind::Anthropic)?.map(Secret::new);
        Ok((openai, anthropic))
    }

    pub fn save_ai_key(&self, kind: AiKeyKind, value: &Secret) -> Result<()> {
        secrets::save_ai_key(kind, value.expose())
    }

    pub fn clear_ai_key(&self, kind: AiKeyKind) -> Result<()> {
        secrets::clear_ai_key(kind)
    }

    pub fn scan_ai_clis(&self) -> CliScan {
        let started = Instant::now();
        let scan = ai::locate::scan();
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            found = scan.tools.len(),
            "ai.cli: scan finished"
        );
        scan
    }

    pub fn generate_commit_message(
        &self,
        request: GenerateCommitMessageRequest,
        live: &dyn Fn() -> bool,
        emit: &mut dyn FnMut(MutationsEvent) -> bool,
    ) {
        let GenerateCommitMessageRequest {
            generation,
            repository,
            repo_path,
            has_staged,
            backend,
            steering_prompt,
            subject_override,
        } = request;

        let started = Instant::now();
        tracing::info!(
            generation,
            repo = %repo_path.display(),
            has_staged,
            backend = backend.label(),
            "ai: dispatch"
        );

        let diff_text = match read_commit_diff(&repo_path, has_staged) {
            Ok(text) => text,
            Err(error) => {
                tracing::error!(generation, %error, "ai: diff read failed");
                emit(MutationsEvent::CommitMessageFailed {
                    generation,
                    repository,
                    message: format!("failed to read diff: {error}"),
                });
                return;
            }
        };

        if !emit(MutationsEvent::CommitMessageStarted {
            generation,
            staged: StagedDigest::of(diff_text.as_bytes()),
        }) {
            return;
        }
        let compressed =
            ai::diff_compress::build_commit_diff_payload(&diff_text, ai::MAX_DIFF_BYTES);
        tracing::debug!(
            generation,
            raw_bytes = diff_text.len(),
            compressed_bytes = compressed.len(),
            max_bytes = ai::MAX_DIFF_BYTES,
            "ai: diff compressed"
        );

        let user_message =
            ai::build_user_message(&steering_prompt, subject_override.as_deref(), &compressed);
        let mut forward = |chunk: String| {
            emit(MutationsEvent::CommitMessageChunk {
                generation,
                repository,
                chunk,
            })
        };
        let result = match backend {
            AiBackend::Api { provider, api_key } => stream_api(
                GenerateRequest {
                    provider,
                    api_key: api_key.into_inner(),
                    user_message,
                },
                live,
                &mut forward,
            ),
            AiBackend::Cli(command) => run_cli(&command, &user_message, live, &mut forward),
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(RunOutcome::Finished) => {
                tracing::info!(generation, elapsed_ms, "ai: generation finished");
                emit(MutationsEvent::CommitMessageFinished {
                    generation,
                    repository,
                });
            }
            Ok(RunOutcome::Cancelled) => {
                tracing::info!(generation, elapsed_ms, "ai: generation cancelled");
            }
            Err(message) => {
                tracing::error!(generation, elapsed_ms, %message, "ai: generation failed");
                emit(MutationsEvent::CommitMessageFailed {
                    generation,
                    repository,
                    message,
                });
            }
        }
    }
}

const API_POLL: Duration = Duration::from_millis(100);

fn stream_api(
    request: GenerateRequest,
    live: &dyn Fn() -> bool,
    forward: &mut dyn FnMut(String) -> bool,
) -> std::result::Result<RunOutcome, String> {
    let rx = ai::run_streaming(request);
    loop {
        match rx.recv_timeout(API_POLL) {
            Ok(StreamMessage::Chunk(chunk)) => {
                if !live() || !forward(chunk) {
                    return Ok(RunOutcome::Cancelled);
                }
            }
            Ok(StreamMessage::Finished) => return Ok(RunOutcome::Finished),
            Ok(StreamMessage::Failed(message)) => return Err(message),
            Err(RecvTimeoutError::Timeout) if !live() => return Ok(RunOutcome::Cancelled),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err("AI worker exited unexpectedly".to_owned());
            }
        }
    }
}

fn run_cli(
    command: &CliCommand,
    user_message: &str,
    live: &dyn Fn() -> bool,
    forward: &mut dyn FnMut(String) -> bool,
) -> std::result::Result<RunOutcome, String> {
    let workdir = ai::cli::workdir()
        .ok_or_else(|| "couldn't find Diffy's config folder for AI sessions".to_owned())?;
    tracing::info!(
        tool = command.tool.label(),
        program = %command.program.display(),
        workdir = %workdir.display(),
        user_message_bytes = user_message.len(),
        "ai.cli: run"
    );
    ai::cli::run(command, &workdir, user_message, live, forward)
}

impl MutationsServices {
    pub fn staged_digest(&self, repo_path: &Path, has_staged: bool) -> Result<StagedDigest> {
        Ok(StagedDigest::of(
            read_commit_diff(repo_path, has_staged)?.as_bytes(),
        ))
    }
}

fn read_commit_diff(repo_path: &Path, has_staged: bool) -> Result<String> {
    let mut repo = discovery::open_repository(repo_path)?;
    repo.commit_diff(has_staged)
}
