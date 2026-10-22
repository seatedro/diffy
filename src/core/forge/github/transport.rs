use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{
    ACCEPT, AUTHORIZATION, CONTENT_TYPE, ETAG, HeaderMap, IF_NONE_MATCH, LINK, RETRY_AFTER,
    USER_AGENT,
};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::core::error::{DiffyError, Result};
use crate::core::http;

const GRAPHQL_URL: &str = "https://api.github.com/graphql";
const AGENT: &str = "diffy/0.1";
const API_VERSION: &str = "2026-03-10";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(60);
const CACHE_CAPACITY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Media {
    Legacy,
    Versioned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quota {
    Core,
    Graphql,
}

#[derive(Debug, Clone)]
pub(super) struct Page {
    pub body: Arc<str>,
    pub next: bool,
}

struct Pause {
    credential: u64,
    quota: Quota,
    until: SystemTime,
}

struct Cached {
    credential: u64,
    url: String,
    etag: String,
    page: Page,
    used: u64,
}

struct Cache {
    entries: Vec<Cached>,
    clock: u64,
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

static PAUSES: Mutex<Vec<Pause>> = Mutex::new(Vec::new());
static CACHE: Mutex<Cache> = Mutex::new(Cache {
    entries: Vec::new(),
    clock: 0,
});

pub(super) fn get(token: &str, url: &str, media: Media, context: &str) -> Result<Page> {
    let credential = credential(token);
    paused(credential, Quota::Core, SystemTime::now())?;
    let cached = cache_lookup(credential, url);
    let etag = cached.as_ref().map(|(etag, _)| etag.as_str());
    let reply = execute(token, Method::GET, url, media, None, etag, context)?;
    settle(credential, Quota::Core, &reply, &[])?;
    if reply.status == StatusCode::NOT_MODIFIED
        && let Some((_, page)) = cached
    {
        return Ok(page);
    }
    let etag = header(&reply.headers, ETAG).map(ToOwned::to_owned);
    let next = has_next_page(&reply.headers);
    let body: Arc<str> = success_body(reply, context)?.into();
    let page = Page { body, next };
    if let Some(etag) = etag {
        cache_store(credential, url, etag, page.clone());
    }
    Ok(page)
}

pub(super) fn send(
    token: &str,
    method: Method,
    url: &str,
    body: Option<String>,
    context: &str,
) -> Result<String> {
    let credential = credential(token);
    let reply = execute(token, method, url, Media::Versioned, body, None, context)?;
    settle(credential, Quota::Core, &reply, &[])?;
    success_body(reply, context)
}

pub(super) fn graphql_query(token: &str, query: &str, variables: Value) -> Result<Value> {
    paused(credential(token), Quota::Graphql, SystemTime::now())?;
    graphql(token, query, variables)
}

pub(super) fn graphql_mutation(token: &str, query: &str, variables: Value) -> Result<Value> {
    graphql(token, query, variables)
}

fn graphql(token: &str, query: &str, variables: Value) -> Result<Value> {
    let context = "GitHub GraphQL request";
    let credential = credential(token);
    let body = json!({ "query": query, "variables": variables }).to_string();
    let reply = execute(
        token,
        Method::POST,
        GRAPHQL_URL,
        Media::Versioned,
        Some(body),
        None,
        context,
    )?;
    let value = if reply.status.is_success() {
        serde_json::from_str::<Value>(&reply.body)?
    } else {
        Value::Null
    };
    settle(credential, Quota::Graphql, &reply, graphql_errors(&value))?;
    if !reply.status.is_success() {
        return Err(failure(&reply, context));
    }
    Ok(value)
}

pub(super) fn graphql_errors(value: &Value) -> &[Value] {
    value
        .get("errors")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn execute(
    token: &str,
    method: Method,
    url: &str,
    media: Media,
    body: Option<String>,
    etag: Option<&str>,
    context: &str,
) -> Result<Reply> {
    http::block_on(async {
        let mut request = http::client()
            .request(method, url)
            .timeout(REQUEST_TIMEOUT)
            .header(USER_AGENT, AGENT);
        request = match media {
            Media::Legacy => request.header(ACCEPT, "application/vnd.github.v3+json"),
            Media::Versioned => request
                .header(ACCEPT, "application/vnd.github+json")
                .header("X-GitHub-Api-Version", API_VERSION),
        };
        if !token.is_empty() {
            request = request.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        if let Some(etag) = etag {
            request = request.header(IF_NONE_MATCH, etag);
        }
        if let Some(body) = body {
            request = request.header(CONTENT_TYPE, "application/json").body(body);
        }
        let response = request
            .send()
            .await
            .map_err(|error| DiffyError::network(format!("{context} failed: {error}")))?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .text()
            .await
            .map_err(|error| DiffyError::network(format!("{context} read failed: {error}")))?;
        Ok(Reply {
            status,
            headers,
            body,
        })
    })
}

fn success_body(reply: Reply, context: &str) -> Result<String> {
    if reply.status.is_success() {
        Ok(reply.body)
    } else {
        Err(failure(&reply, context))
    }
}

fn failure(reply: &Reply, context: &str) -> DiffyError {
    DiffyError::Network {
        details: format!("{context} returned {}: {}", reply.status, reply.body),
        retryable: http::status_is_retryable(reply.status),
    }
}

fn settle(credential: u64, quota: Quota, reply: &Reply, errors: &[Value]) -> Result<()> {
    let now = SystemTime::now();
    let mut pauses = PAUSES.lock().unwrap_or_else(PoisonError::into_inner);
    pauses.retain(|pause| !(pause.credential == credential && pause.quota == quota));
    if is_rate_limited(reply.status, &reply.headers, &reply.body, errors) {
        let until = retry_at(&reply.headers, now);
        pauses.push(Pause {
            credential,
            quota,
            until,
        });
        return Err(rate_limit_error(until, now));
    }
    if header(&reply.headers, "x-ratelimit-remaining") == Some("0")
        && let Some(until) = reset_at(&reply.headers).filter(|until| *until > now)
    {
        pauses.push(Pause {
            credential,
            quota,
            until,
        });
    }
    Ok(())
}

fn paused(credential: u64, quota: Quota, now: SystemTime) -> Result<()> {
    let pauses = PAUSES.lock().unwrap_or_else(PoisonError::into_inner);
    match pauses
        .iter()
        .find(|pause| pause.credential == credential && pause.quota == quota)
    {
        Some(pause) if pause.until > now => Err(rate_limit_error(pause.until, now)),
        _ => Ok(()),
    }
}

fn is_rate_limited(status: StatusCode, headers: &HeaderMap, body: &str, errors: &[Value]) -> bool {
    let exhausted = header(headers, "x-ratelimit-remaining") == Some("0");
    status == StatusCode::TOO_MANY_REQUESTS
        || (!errors.is_empty()
            && (exhausted
                || errors.iter().any(|error| {
                    error.get("type").and_then(Value::as_str) == Some("RATE_LIMITED")
                        || error
                            .get("message")
                            .and_then(Value::as_str)
                            .is_some_and(mentions_rate_limit_exceeded)
                })))
        || (status == StatusCode::FORBIDDEN
            && (exhausted
                || headers.contains_key(RETRY_AFTER)
                || body.to_ascii_lowercase().contains("rate limit")))
}

fn mentions_rate_limit_exceeded(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("rate limit exceeded") || message.contains("rate limit already exceeded")
}

fn retry_at(headers: &HeaderMap, now: SystemTime) -> SystemTime {
    if let Some(seconds) = header(headers, RETRY_AFTER).and_then(|value| value.parse::<u64>().ok())
    {
        return now + Duration::from_secs(seconds);
    }
    reset_at(headers)
        .filter(|until| *until > now)
        .unwrap_or(now + RATE_LIMIT_COOLDOWN)
}

fn reset_at(headers: &HeaderMap) -> Option<SystemTime> {
    header(headers, "x-ratelimit-reset")
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| UNIX_EPOCH + Duration::from_secs(seconds))
}

fn rate_limit_error(until: SystemTime, now: SystemTime) -> DiffyError {
    let minutes = until
        .duration_since(now)
        .unwrap_or_default()
        .as_secs()
        .div_ceil(60)
        .max(1);
    DiffyError::network_fatal(format!(
        "GitHub rate limit reached. Try again in {minutes} min."
    ))
}

fn has_next_page(headers: &HeaderMap) -> bool {
    header(headers, LINK).is_some_and(|link| link.contains("rel=\"next\""))
}

fn header(headers: &HeaderMap, name: impl reqwest::header::AsHeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn credential(token: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    hasher.finish()
}

fn cache_lookup(credential: u64, url: &str) -> Option<(String, Page)> {
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    cache.clock += 1;
    let clock = cache.clock;
    let entry = cache
        .entries
        .iter_mut()
        .find(|entry| entry.credential == credential && entry.url == url)?;
    entry.used = clock;
    Some((entry.etag.clone(), entry.page.clone()))
}

fn cache_store(credential: u64, url: &str, etag: String, page: Page) {
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    cache.clock += 1;
    let used = cache.clock;
    cache
        .entries
        .retain(|entry| !(entry.credential == credential && entry.url == url));
    if cache.entries.len() >= CACHE_CAPACITY
        && let Some(oldest) = cache
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| entry.used)
            .map(|(index, _)| index)
    {
        cache.entries.swap_remove(oldest);
    }
    cache.entries.push(Cached {
        credential,
        url: url.to_owned(),
        etag,
        page,
        used,
    });
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderValue;

    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(*name, HeaderValue::from_static(value));
        }
        headers
    }

    #[test]
    fn rate_limits_are_recognized_across_rest_and_graphql() {
        let none = HeaderMap::new();
        assert!(is_rate_limited(
            StatusCode::TOO_MANY_REQUESTS,
            &none,
            "",
            &[]
        ));
        assert!(is_rate_limited(
            StatusCode::FORBIDDEN,
            &headers(&[("x-ratelimit-remaining", "0")]),
            "",
            &[]
        ));
        assert!(is_rate_limited(
            StatusCode::FORBIDDEN,
            &none,
            "You have exceeded a secondary rate limit.",
            &[]
        ));
        assert!(is_rate_limited(
            StatusCode::OK,
            &none,
            "",
            &[json!({ "type": "RATE_LIMITED", "message": "limited" })]
        ));
        assert!(!is_rate_limited(
            StatusCode::FORBIDDEN,
            &none,
            "Resource not accessible",
            &[]
        ));
        assert!(!is_rate_limited(
            StatusCode::OK,
            &none,
            "",
            &[json!({ "type": "FORBIDDEN", "message": "denied" })]
        ));
    }

    #[test]
    fn retry_time_prefers_retry_after_then_reset_then_cooldown() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        assert_eq!(
            retry_at(&headers(&[("retry-after", "30")]), now),
            now + Duration::from_secs(30)
        );
        assert_eq!(
            retry_at(&headers(&[("x-ratelimit-reset", "1600")]), now),
            UNIX_EPOCH + Duration::from_secs(1_600)
        );
        assert_eq!(
            retry_at(&headers(&[("x-ratelimit-reset", "900")]), now),
            now + RATE_LIMIT_COOLDOWN
        );
    }

    #[test]
    fn link_header_marks_the_next_page() {
        assert!(has_next_page(&headers(&[(
            "link",
            "<https://example.test/a?page=2>; rel=\"next\", <https://example.test/a?page=3>; rel=\"last\""
        )])));
        assert!(!has_next_page(&headers(&[(
            "link",
            "<https://example.test/a?page=1>; rel=\"prev\""
        )])));
        assert!(!has_next_page(&HeaderMap::new()));
    }
}
