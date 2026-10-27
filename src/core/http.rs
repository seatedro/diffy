use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;

use crate::core::error::{DiffyError, Result};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const RUNTIME_THREADS: usize = 2;

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static RUNTIME: OnceLock<std::result::Result<tokio::runtime::Runtime, String>> = OnceLock::new();

pub(crate) fn client() -> reqwest::Client {
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .pool_idle_timeout(POOL_IDLE_TIMEOUT)
                .build()
                .unwrap_or_default()
        })
        .clone()
}

pub(crate) fn runtime() -> Result<&'static tokio::runtime::Runtime> {
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(RUNTIME_THREADS)
                .thread_name("diffy-http")
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| DiffyError::General(format!("failed to start HTTP runtime: {error}")))
}

pub(crate) fn block_on<T>(future: impl Future<Output = Result<T>>) -> Result<T> {
    runtime()?.block_on(future)
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
