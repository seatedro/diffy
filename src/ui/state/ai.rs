use crate::actions::AiAction;
use crate::effects::Effect;
use crate::events::AiEvent;

use super::AppState;

pub(super) fn reduce_action(state: &mut AppState, action: AiAction) -> Vec<Effect> {
    state.apply_ai_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: AiEvent) -> Vec<Effect> {
    match event {
        AiEvent::AiKeysLoaded { openai, anthropic } => {
            state.ai_openai_key = openai.unwrap_or_default();
            state.ai_anthropic_key = anthropic.unwrap_or_default();
            Vec::new()
        }
        AiEvent::AiKeysLoadFailed { message } => {
            tracing::warn!("failed to load AI keys from keyring: {message}");
            Vec::new()
        }
        AiEvent::AiKeySaveFailed { message } => {
            state.push_error(&format!("Couldn't save AI key to keyring: {message}"));
            Vec::new()
        }
        AiEvent::CommitMessageChunk { generation, chunk } => {
            if generation == state.ai_generation_id && state.ai_generation_active {
                tracing::trace!(generation, bytes = chunk.len(), "ai: chunk");
                state.commit_editor.append(&chunk);
            } else {
                tracing::debug!(
                    generation,
                    current = state.ai_generation_id,
                    active = state.ai_generation_active,
                    "ai: dropping stale chunk"
                );
            }
            Vec::new()
        }
        AiEvent::CommitMessageGenerationFinished { generation } => {
            if generation == state.ai_generation_id {
                state.ai_generation_active = false;
                tracing::info!(generation, "ai: generation finished");
            } else {
                tracing::debug!(
                    generation,
                    current = state.ai_generation_id,
                    "ai: stale finish event"
                );
            }
            Vec::new()
        }
        AiEvent::CommitMessageGenerationFailed {
            generation,
            message,
        } => {
            if generation == state.ai_generation_id {
                state.ai_generation_active = false;
                state.ai_generation_error = Some(message.clone());
            }
            tracing::error!(generation, %message, "ai: generation failed");
            state.push_error(&format!("Commit message generation failed: {message}"));
            Vec::new()
        }
    }
}
