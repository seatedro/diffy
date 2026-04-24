use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use serde_json::{Value, json};

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
        return format!("{}...(len={len})", "*".repeat(len.min(4)));
    }
    let prefix: String = key.chars().take(4).collect();
    let suffix: String = key.chars().skip(len - 4).collect();
    format!("{prefix}...{suffix}(len={len})")
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

    // Keys pasted from browser/terminals sometimes carry trailing whitespace.
    // Strip before building auth headers so failures are legible.
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

    let client = reqwest::Client::new();
    if let Err(error) = match provider {
        Provider::OpenAi => stream_openai(&client, &api_key, &user_message, &tx).await,
        Provider::Anthropic => stream_anthropic(&client, &api_key, &user_message, &tx).await,
    } {
        tracing::error!(
            provider = provider.label(),
            %error,
            "ai.stream: request failed"
        );
        let _ = tx.send(StreamMessage::Failed(error));
    }
}

async fn stream_openai(
    client: &reqwest::Client,
    api_key: &str,
    user_message: &str,
    tx: &Sender<StreamMessage>,
) -> Result<(), String> {
    let body = json!({
        "model": OPENAI_MODEL,
        "messages": [
            { "role": "user", "content": user_message }
        ],
        "max_completion_tokens": 1024,
        "stream": true,
        "stream_options": { "include_usage": true }
    });

    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", &format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .body(body.to_string())
        .send()
        .await
        .map_err(|error| format!("OpenAI chat request failed: {error}"))?;
    let response = require_success(response, "OpenAI").await?;

    stream_sse_response(response, tx, parse_openai_event).await
}

async fn stream_anthropic(
    client: &reqwest::Client,
    api_key: &str,
    user_message: &str,
    tx: &Sender<StreamMessage>,
) -> Result<(), String> {
    let body = json!({
        "model": ANTHROPIC_MODEL,
        "messages": [
            { "role": "user", "content": user_message }
        ],
        "system": "You are a helpful assistant.",
        "max_tokens": 1024,
        "temperature": 0.7,
        "stream": true
    });

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .body(body.to_string())
        .send()
        .await
        .map_err(|error| format!("Anthropic chat request failed: {error}"))?;
    let response = require_success(response, "Anthropic").await?;

    stream_sse_response(response, tx, parse_anthropic_event).await
}

async fn require_success(
    response: reqwest::Response,
    provider: &str,
) -> Result<reqwest::Response, String> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let body = response
        .text()
        .await
        .unwrap_or_else(|error| format!("failed to read error body: {error}"));
    Err(format!("{provider} API returned {status}: {body}"))
}

async fn stream_sse_response(
    mut response: reqwest::Response,
    tx: &Sender<StreamMessage>,
    parse_event: fn(&str) -> Result<Option<String>, String>,
) -> Result<(), String> {
    let mut buffer = Vec::new();
    let mut chunks = 0_usize;
    let mut bytes = 0_usize;

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("stream read failed: {error}"))?
    {
        buffer.extend_from_slice(&chunk);
        while let Some((pos, separator_len)) = event_boundary(&buffer) {
            let raw_event = String::from_utf8(buffer[..pos].to_vec())
                .map_err(|error| format!("stream event was not utf-8: {error}"))?;
            buffer.drain(..pos + separator_len);
            if !emit_stream_event(&raw_event, tx, parse_event, &mut chunks, &mut bytes)? {
                return Ok(());
            }
        }
    }

    if !buffer.is_empty() {
        let raw_event = String::from_utf8(buffer)
            .map_err(|error| format!("stream event was not utf-8: {error}"))?;
        let _ = emit_stream_event(&raw_event, tx, parse_event, &mut chunks, &mut bytes)?;
    }

    tracing::debug!(chunks, bytes, "ai.stream: completed");
    let _ = tx.send(StreamMessage::Finished);
    Ok(())
}

fn emit_stream_event(
    raw_event: &str,
    tx: &Sender<StreamMessage>,
    parse_event: fn(&str) -> Result<Option<String>, String>,
    chunks: &mut usize,
    bytes: &mut usize,
) -> Result<bool, String> {
    if raw_event.trim().is_empty() {
        return Ok(true);
    }
    if let Some(chunk) = parse_event(raw_event)? {
        *chunks += 1;
        *bytes += chunk.len();
        if tx.send(StreamMessage::Chunk(chunk)).is_err() {
            tracing::warn!(chunks = *chunks, "ai.stream: receiver dropped, stopping");
            return Ok(false);
        }
    }
    Ok(true)
}

fn event_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|window| window == b"\n\n");
    let crlf = buffer.windows(4).position(|window| window == b"\r\n\r\n");
    match (lf, crlf) {
        (Some(a), Some(b)) if a <= b => Some((a, 2)),
        (Some(_), Some(b)) => Some((b, 4)),
        (Some(a), None) => Some((a, 2)),
        (None, Some(b)) => Some((b, 4)),
        (None, None) => None,
    }
}

fn parse_openai_event(event: &str) -> Result<Option<String>, String> {
    for line in event.lines().map(str::trim) {
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        if data == "[DONE]" {
            return Ok(None);
        }
        let value: Value = serde_json::from_str(data)
            .map_err(|error| format!("failed to decode OpenAI stream event: {error}"))?;
        let content = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("delta"))
            .and_then(|delta| delta.get("content"))
            .and_then(Value::as_str);
        if let Some(content) = content.filter(|content| !content.is_empty()) {
            return Ok(Some(content.to_owned()));
        }
    }
    Ok(None)
}

fn parse_anthropic_event(event: &str) -> Result<Option<String>, String> {
    for line in event.lines().map(str::trim) {
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        let value: Value = serde_json::from_str(data)
            .map_err(|error| format!("failed to decode Anthropic stream event: {error}"))?;
        let event_type = value.get("type").and_then(Value::as_str);
        if event_type != Some("content_block_delta") {
            continue;
        }
        let text = value
            .get("delta")
            .and_then(|delta| delta.get("text"))
            .and_then(Value::as_str);
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            return Ok(Some(text.to_owned()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{parse_anthropic_event, parse_openai_event};

    #[test]
    fn parses_openai_text_delta() {
        let event = r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;
        assert_eq!(parse_openai_event(event).unwrap().as_deref(), Some("hello"));
    }

    #[test]
    fn parses_anthropic_text_delta() {
        let event = r#"event: content_block_delta
data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}
"#;
        assert_eq!(
            parse_anthropic_event(event).unwrap().as_deref(),
            Some("hello")
        );
    }
}
