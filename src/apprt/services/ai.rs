use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use crate::ai::{self, CliCommand, GenerateRequest, RunOutcome, StreamMessage};
use crate::contracts::mutations::AiBackend;

pub fn complete(
    backend: AiBackend,
    prompt: String,
    max_tokens: u32,
    live: &dyn Fn() -> bool,
    forward: &mut dyn FnMut(String) -> bool,
) -> Result<RunOutcome, String> {
    match backend {
        AiBackend::Api { provider, api_key } => stream_api(
            GenerateRequest {
                provider,
                api_key: api_key.into_inner(),
                user_message: prompt,
                max_tokens,
            },
            live,
            forward,
        ),
        AiBackend::Cli(command) => run_cli(&command, &prompt, live, forward),
    }
}

pub fn complete_text(
    backend: AiBackend,
    prompt: String,
    max_tokens: u32,
    live: &dyn Fn() -> bool,
) -> Result<String, String> {
    let mut reply = String::new();
    let outcome = complete(backend, prompt, max_tokens, live, &mut |chunk| {
        reply.push_str(&chunk);
        true
    })?;
    match outcome {
        RunOutcome::Finished => Ok(reply),
        RunOutcome::Cancelled => Err("The AI request was cancelled.".to_owned()),
    }
}

const API_POLL: Duration = Duration::from_millis(100);

fn stream_api(
    request: GenerateRequest,
    live: &dyn Fn() -> bool,
    forward: &mut dyn FnMut(String) -> bool,
) -> Result<RunOutcome, String> {
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
) -> Result<RunOutcome, String> {
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
