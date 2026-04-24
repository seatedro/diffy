use std::future::Future;

use crate::core::error::{DiffyError, Result};

pub(crate) fn block_on<T>(future: impl Future<Output = Result<T>>) -> Result<T> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| DiffyError::Http(format!("failed to start HTTP runtime: {error}")))?;
    runtime.block_on(future)
}

pub(crate) async fn response_text(response: reqwest::Response, context: &str) -> Result<String> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| DiffyError::Http(format!("{context} read failed: {error}")))?;
    if status.is_success() {
        Ok(body)
    } else {
        Err(DiffyError::Http(format!(
            "{context} returned {status}: {body}"
        )))
    }
}

pub(crate) async fn response_bytes(response: reqwest::Response, context: &str) -> Result<Vec<u8>> {
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|error| DiffyError::Http(format!("{context} read failed: {error}")))?;
    if status.is_success() {
        Ok(body.to_vec())
    } else {
        let body = String::from_utf8_lossy(&body);
        Err(DiffyError::Http(format!(
            "{context} returned {status}: {body}"
        )))
    }
}
