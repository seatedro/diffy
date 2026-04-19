use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use futures::StreamExt;
use llm::builder::{LLMBackend, LLMBuilder};
use llm::chat::ChatMessage;

pub const OPENAI_MODEL: &str = "gpt-5.4-nano";
pub const ANTHROPIC_MODEL: &str = "claude-haiku-4-5";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAi,
    Anthropic,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::Anthropic => "Anthropic",
        }
    }

    pub fn model_id(self) -> &'static str {
        match self {
            Self::OpenAi => OPENAI_MODEL,
            Self::Anthropic => ANTHROPIC_MODEL,
        }
    }

    fn backend(self) -> LLMBackend {
        match self {
            Self::OpenAi => LLMBackend::OpenAI,
            Self::Anthropic => LLMBackend::Anthropic,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GenerateRequest {
    pub provider: Provider,
    pub api_key: String,
    pub user_message: String,
}

#[derive(Debug, Clone)]
pub enum StreamMessage {
    Chunk(String),
    Finished,
    Failed(String),
}

fn key_fingerprint(key: &str) -> String {
    let len = key.chars().count();
    if len == 0 {
        return String::from("<empty>");
    }
    if len <= 8 {
        return format!("{}…(len={len})", "*".repeat(len.min(4)));
    }
    let prefix: String = key.chars().take(4).collect();
    let suffix: String = key.chars().skip(len - 4).collect();
    format!("{prefix}…{suffix}(len={len})")
}

pub fn run_streaming(request: GenerateRequest) -> Receiver<StreamMessage> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            let _ = tx.send(StreamMessage::Failed(
                "failed to start tokio runtime".to_owned(),
            ));
            return;
        };
        runtime.block_on(drive_stream(request, tx));
    });
    rx
}

async fn drive_stream(request: GenerateRequest, tx: Sender<StreamMessage>) {
    let GenerateRequest {
        provider,
        api_key,
        user_message,
    } = request;

    // Keys pasted from browser/terminals sometimes carry trailing whitespace
    // or newlines. reqwest rejects these outright with an opaque
    // "builder error" when used as an auth header, so strip them here.
    let original_len = api_key.len();
    let api_key = api_key.trim().to_owned();
    let fingerprint = key_fingerprint(&api_key);

    tracing::info!(
        provider = provider.label(),
        model = provider.model_id(),
        api_key_len = api_key.len(),
        original_len,
        api_key_fingerprint = %fingerprint,
        user_message_bytes = user_message.len(),
        "ai.stream: building client"
    );

    if api_key.is_empty() {
        let _ = tx.send(StreamMessage::Failed(format!(
            "{} API key is empty",
            provider.label()
        )));
        return;
    }

    let llm = match LLMBuilder::new()
        .backend(provider.backend())
        .api_key(api_key)
        .model(provider.model_id())
        .reasoning(false)
        .max_tokens(1024)
        .build()
    {
        Ok(llm) => llm,
        Err(error) => {
            tracing::error!(
                provider = provider.label(),
                %error,
                "ai.stream: builder failed"
            );
            let _ = tx.send(StreamMessage::Failed(format!(
                "failed to build {} client: {error}",
                provider.label()
            )));
            return;
        }
    };

    let messages = vec![
        ChatMessage::user()
            .content(user_message)
            .build(),
    ];

    tracing::debug!(provider = provider.label(), "ai.stream: opening chat stream");
    let stream = match llm.chat_stream(&messages).await {
        Ok(stream) => stream,
        Err(error) => {
            tracing::error!(
                provider = provider.label(),
                %error,
                "ai.stream: chat_stream failed"
            );
            let _ = tx.send(StreamMessage::Failed(format!("chat request failed: {error}")));
            return;
        }
    };

    let mut stream = stream;
    let mut tokens: usize = 0;
    let mut bytes: usize = 0;
    while let Some(item) = stream.next().await {
        match item {
            Ok(token) => {
                tokens += 1;
                bytes += token.len();
                if tx.send(StreamMessage::Chunk(token)).is_err() {
                    tracing::warn!(
                        provider = provider.label(),
                        tokens,
                        "ai.stream: receiver dropped, stopping"
                    );
                    return;
                }
            }
            Err(error) => {
                tracing::error!(
                    provider = provider.label(),
                    tokens,
                    %error,
                    "ai.stream: token error"
                );
                let _ = tx.send(StreamMessage::Failed(format!("stream error: {error}")));
                return;
            }
        }
    }

    tracing::debug!(
        provider = provider.label(),
        tokens,
        bytes,
        "ai.stream: completed"
    );
    let _ = tx.send(StreamMessage::Finished);
}
