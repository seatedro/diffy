use crate::actions::AiAction;
use crate::effects::Effect;
use crate::events::AiEvent;

use super::*;

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

impl AppState {
    fn set_ai_key(&mut self, kind: AiKeyKind, value: String) -> Vec<Effect> {
        match kind {
            AiKeyKind::OpenAi => self.ai_openai_key = value.clone(),
            AiKeyKind::Anthropic => self.ai_anthropic_key = value.clone(),
        }
        if !self.startup.keyring_enabled {
            return Vec::new();
        }
        if value.is_empty() {
            vec![AiEffect::ClearAiKey { kind }.into()]
        } else {
            vec![AiEffect::SaveAiKey { kind, value }.into()]
        }
    }

    fn clear_ai_key(&mut self, kind: AiKeyKind) -> Vec<Effect> {
        match kind {
            AiKeyKind::OpenAi => {
                self.ai_openai_key.clear();
                self.ai_openai_editing = false;
            }
            AiKeyKind::Anthropic => {
                self.ai_anthropic_key.clear();
                self.ai_anthropic_editing = false;
            }
        }
        if !self.startup.keyring_enabled {
            return Vec::new();
        }
        vec![AiEffect::ClearAiKey { kind }.into()]
    }

    fn set_ai_key_editing(&mut self, kind: AiKeyKind, editing: bool) -> Vec<Effect> {
        let target = match kind {
            AiKeyKind::OpenAi => {
                self.ai_openai_editing = editing;
                FocusTarget::SettingsOpenAiKey
            }
            AiKeyKind::Anthropic => {
                self.ai_anthropic_editing = editing;
                FocusTarget::SettingsAnthropicKey
            }
        };
        if editing {
            self.set_focus(Some(target));
        } else if self.ui.focus.get(&self.store) == Some(target) {
            self.set_focus(None);
        }
        Vec::new()
    }

    pub(super) fn apply_ai_action(&mut self, action: crate::actions::AiAction) -> Vec<Effect> {
        match action {
            crate::actions::AiAction::SetAiKey { kind, value } => self.set_ai_key(kind, value),
            crate::actions::AiAction::ClearAiKey { kind } => self.clear_ai_key(kind),
            crate::actions::AiAction::SetAiKeyEditing { kind, editing } => {
                self.set_ai_key_editing(kind, editing)
            }
            crate::actions::AiAction::GenerateCommitMessage => self.start_generate_commit_message(),
        }
    }

    fn start_generate_commit_message(&mut self) -> Vec<Effect> {
        if self.ai_generation_active {
            return Vec::new();
        }
        let Some(repo_path) = self
            .compare
            .repo_path
            .with(&self.store, |p| p.as_ref().cloned())
        else {
            self.push_error("Open a repository before generating a commit message.");
            return Vec::new();
        };
        let has_staged = self
            .workspace
            .status_file_changes
            .with(&self.store, |changes| {
                changes
                    .iter()
                    .any(|change| change.bucket == crate::core::vcs::model::ChangeBucket::Staged)
            });
        let (provider, api_key) = if !self.ai_anthropic_key.is_empty() {
            (
                crate::ai::Provider::Anthropic,
                self.ai_anthropic_key.clone(),
            )
        } else if !self.ai_openai_key.is_empty() {
            (crate::ai::Provider::OpenAi, self.ai_openai_key.clone())
        } else {
            self.push_error("Add an AI key under Settings \u{2192} Clankers first.");
            return Vec::new();
        };
        let steering_prompt = if self.settings.ai_steering_prompt.trim().is_empty() {
            crate::ai::DEFAULT_STEERING_PROMPT.to_owned()
        } else {
            self.settings.ai_steering_prompt.clone()
        };
        let subject_override = {
            let first_line = self
                .commit_editor
                .text()
                .lines()
                .next()
                .map(ToOwned::to_owned)
                .unwrap_or_default();
            if first_line.trim().is_empty() {
                None
            } else {
                Some(first_line)
            }
        };
        if subject_override.is_some() {
            self.commit_editor.insert_text("\n");
        }
        self.ai_generation_id = self.ai_generation_id.wrapping_add(1);
        self.ai_generation_active = true;
        self.ai_generation_error = None;
        tracing::info!(
            generation = self.ai_generation_id,
            provider = provider.label(),
            model = provider.model_id(),
            has_staged,
            has_subject = subject_override.is_some(),
            steering_prompt_chars = steering_prompt.len(),
            "ai: starting commit message generation"
        );
        vec![
            AiEffect::GenerateCommitMessage(crate::effects::GenerateCommitMessageRequest {
                repo_path,
                has_staged,
                provider,
                api_key,
                steering_prompt,
                subject_override,
                generation: self.ai_generation_id,
            })
            .into(),
        ]
    }
}

impl AppState {
    pub(super) fn ai_key_editable(&self, kind: AiKeyKind) -> bool {
        match kind {
            AiKeyKind::OpenAi => self.ai_openai_key.is_empty() || self.ai_openai_editing,
            AiKeyKind::Anthropic => self.ai_anthropic_key.is_empty() || self.ai_anthropic_editing,
        }
    }
}
