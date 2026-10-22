use std::path::Path;
use std::time::Instant;

use crate::ai::{self, GenerateRequest, StreamMessage};
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

    pub fn generate_commit_message(
        &self,
        request: GenerateCommitMessageRequest,
        emit: &mut dyn FnMut(MutationsEvent) -> bool,
    ) {
        let GenerateCommitMessageRequest {
            generation,
            repository,
            repo_path,
            has_staged,
            provider,
            api_key,
            steering_prompt,
            subject_override,
        } = request;

        let started = Instant::now();
        tracing::info!(
            generation,
            repo = %repo_path.display(),
            has_staged,
            provider = provider.label(),
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
        let rx = ai::run_streaming(GenerateRequest {
            provider,
            api_key: api_key.into_inner(),
            user_message,
            max_tokens: ai::COMMIT_MAX_TOKENS,
        });

        let mut chunk_count: usize = 0;
        let mut byte_count: usize = 0;
        loop {
            match rx.recv() {
                Ok(StreamMessage::Chunk(chunk)) => {
                    chunk_count += 1;
                    byte_count += chunk.len();
                    if !emit(MutationsEvent::CommitMessageChunk {
                        generation,
                        repository,
                        chunk,
                    }) {
                        return;
                    }
                }
                Ok(StreamMessage::Finished) => {
                    tracing::info!(
                        generation,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        chunks = chunk_count,
                        bytes = byte_count,
                        "ai: stream finished"
                    );
                    emit(MutationsEvent::CommitMessageFinished {
                        generation,
                        repository,
                    });
                    return;
                }
                Ok(StreamMessage::Failed(message)) => {
                    tracing::error!(
                        generation,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        chunks = chunk_count,
                        %message,
                        "ai: stream failed"
                    );
                    emit(MutationsEvent::CommitMessageFailed {
                        generation,
                        repository,
                        message,
                    });
                    return;
                }
                Err(_) => {
                    tracing::error!(generation, "ai: worker channel disconnected");
                    emit(MutationsEvent::CommitMessageFailed {
                        generation,
                        repository,
                        message: "AI worker exited unexpectedly".to_owned(),
                    });
                    return;
                }
            }
        }
    }
}

impl MutationsServices {
    pub fn rank_files(&self, job: RankFilesJob) -> std::result::Result<Vec<u32>, String> {
        let RankFilesJob {
            request,
            provider,
            api_key,
        } = job;
        let started = Instant::now();
        let (user_message, sent) = ai::rank::build_rank_message(&request.files);
        if sent == 0 {
            return Err("No files to rank.".to_owned());
        }
        tracing::info!(
            job = request.job.0,
            provider = provider.label(),
            files = request.files.len(),
            sent,
            bytes = user_message.len(),
            "ai.rank: dispatch"
        );
        let rx = ai::run_streaming(GenerateRequest {
            provider,
            api_key: api_key.into_inner(),
            user_message,
            max_tokens: ai::rank::RANK_MAX_TOKENS,
        });
        let mut reply = String::new();
        loop {
            match rx.recv() {
                Ok(StreamMessage::Chunk(chunk)) => reply.push_str(&chunk),
                Ok(StreamMessage::Finished) => break,
                Ok(StreamMessage::Failed(message)) => {
                    tracing::warn!(job = request.job.0, %message, "ai.rank: request failed");
                    return Err(message);
                }
                Err(_) => return Err("AI worker exited unexpectedly".to_owned()),
            }
        }
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
