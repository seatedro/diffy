use std::path::Path;
use std::time::Instant;

use crate::ai::{self, CliScan, RunOutcome};
use crate::contracts::mutations::{
    GenerateCommitMessageRequest, MutationsEvent, RankFilesJob, StagedDigest,
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
        let result = super::ai::complete(
            backend,
            user_message,
            ai::COMMIT_MAX_TOKENS,
            live,
            &mut forward,
        );
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

impl MutationsServices {
    pub fn rank_files(&self, job: RankFilesJob) -> std::result::Result<Vec<u32>, String> {
        let RankFilesJob { request, backend } = job;
        let started = Instant::now();
        let (user_message, sent) = ai::rank::build_rank_message(&request.files);
        if sent == 0 {
            return Err("No files to rank.".to_owned());
        }
        tracing::info!(
            job = request.job.0,
            backend = backend.label(),
            files = request.files.len(),
            sent,
            bytes = user_message.len(),
            "ai.rank: dispatch"
        );
        let reply =
            super::ai::complete_text(backend, user_message, ai::rank::RANK_MAX_TOKENS, &|| true)
                .inspect_err(|message| {
                    tracing::warn!(job = request.job.0, %message, "ai.rank: request failed");
                })?;
        let ranking = ai::rank::parse_ranking(&reply, sent);
        tracing::info!(
            job = request.job.0,
            elapsed_ms = started.elapsed().as_millis() as u64,
            ranked = ranking.as_ref().map_or(0, Vec::len),
            sent,
            "ai.rank: finished"
        );
        ranking
    }

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
