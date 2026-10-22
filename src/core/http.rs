use std::future::Future;
use std::time::Duration;

use crate::core::error::{DiffyError, Result};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .unwrap_or_default()
}

pub(crate) fn block_on<T>(future: impl Future<Output = Result<T>>) -> Result<T> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| DiffyError::General(format!("failed to start HTTP runtime: {error}")))?;
    runtime.block_on(future)
}

/// Whether a failed HTTP status is worth retrying unchanged.
pub(crate) fn status_is_retryable(status: reqwest::StatusCode) -> bool {
    status.is_server_error()
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
}

pub(crate) async fn response_text(response: reqwest::Response, context: &str) -> Result<String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| DiffyError::network(format!("{context} read failed: {error}")))?;
    if status.is_success() {
        Ok(body)
    } else {
        Err(DiffyError::Network {
            details: format!("{context} returned {status}: {body}"),
            retryable: status_is_retryable(status),
        })
    }
}

pub(crate) async fn response_bytes(response: reqwest::Response, context: &str) -> Result<Vec<u8>> {
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|error| DiffyError::network(format!("{context} read failed: {error}")))?;
    if status.is_success() {
        Ok(body.to_vec())
    } else {
        let body = String::from_utf8_lossy(&body);
        Err(DiffyError::Network {
            details: format!("{context} returned {status}: {body}"),
            retryable: status_is_retryable(status),
        })
    }
}
