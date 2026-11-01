//! Polite image downloads for the URL endpoints.
//!
//! The server fetches pages over the user's own home connection, so the aim
//! isn't to spread load across IPs but to stay under a source's rate limit: a
//! cap on simultaneous downloads per host, and retries of transient failures
//! with exponential backoff and jitter that honour `Retry-After`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Simultaneous downloads allowed per host. A reader's batch is a handful of
/// pages, so this barely slows it while keeping bursts below typical CDN
/// rate limits.
pub const PER_HOST_LIMIT: usize = 4;
/// Total tries per download, including the first.
const MAX_ATTEMPTS: u32 = 3;
const BASE_DELAY: Duration = Duration::from_millis(500);
/// Longest wait before a retry. A `Retry-After` beyond this isn't worth
/// holding the reader for, so the response is returned as-is instead.
const MAX_DELAY: Duration = Duration::from_secs(10);

/// Hands out per-host download slots.
#[derive(Default)]
pub struct HostLimiter {
    hosts: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl HostLimiter {
    /// Wait for a download slot on `url`'s host. The slot is held until the
    /// returned permit is dropped.
    pub async fn acquire(&self, url: &str) -> OwnedSemaphorePermit {
        let host = reqwest::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| url.to_string());
        let semaphore = {
            let mut hosts = self.hosts.lock().unwrap();
            // Forget hosts nobody is downloading from; a held permit keeps its
            // semaphore's count above one.
            hosts.retain(|_, s| Arc::strong_count(s) > 1);
            hosts
                .entry(host)
                .or_insert_with(|| Arc::new(Semaphore::new(PER_HOST_LIMIT)))
                .clone()
        };
        semaphore.acquire_owned().await.expect("host semaphores are never closed")
    }
}

/// A completed response, body included.
pub struct Fetched {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

/// Send the request `build` produces and read its body, retrying timeouts,
/// connection and transfer errors, and 408/429/5xx responses. `on_retry` is
/// told why and how long before each retry. The last response is returned
/// whatever its status; only a transport error on the last try is an `Err`.
pub async fn fetch_with_retry(
    build: impl Fn() -> reqwest::RequestBuilder,
    mut on_retry: impl FnMut(&str, Duration),
) -> Result<Fetched, reqwest::Error> {
    let mut attempt = 1;
    loop {
        let (reason, delay) = match send(build()).await {
            Ok(f) => match retry_delay(&f, attempt) {
                Some(delay) => (format!("HTTP {}", f.status.as_u16()), delay),
                None => return Ok(f),
            },
            Err(e) if attempt < MAX_ATTEMPTS && is_transient(&e) => (e.to_string(), backoff(attempt)),
            Err(e) => return Err(e),
        };
        on_retry(&reason, delay);
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}

async fn send(request: reqwest::RequestBuilder) -> Result<Fetched, reqwest::Error> {
    let resp = request.send().await?;
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = resp.bytes().await?;
    Ok(Fetched { status, headers, body })
}

fn is_transient(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect() || e.is_body() || e.is_request()
}

/// How long to wait before retrying `f`, or `None` if it should be returned.
fn retry_delay(f: &Fetched, attempt: u32) -> Option<Duration> {
    let retryable = matches!(f.status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504);
    // A Cloudflare challenge won't clear by asking again.
    let challenged = f.headers.contains_key("cf-mitigated");
    if !retryable || challenged || attempt >= MAX_ATTEMPTS {
        return None;
    }
    let requested = f
        .headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    match requested {
        Some(d) if d > MAX_DELAY => None,
        Some(d) => Some(d),
        None => Some(backoff(attempt)),
    }
}

/// Exponential backoff with equal jitter: half the step is fixed, half random,
/// so retries from a burst don't land on the host together.
fn backoff(attempt: u32) -> Duration {
    let step = (BASE_DELAY * 2u32.pow(attempt - 1)).min(MAX_DELAY);
    let half = step / 2;
    half + half.mul_f64(fastrand::f64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::http::HeaderMap as AxumHeaders;
    use axum::routing::get;
    use axum::Router;

    /// A test response: status and headers for the nth call.
    type Respond = fn(usize) -> (u16, Vec<(&'static str, &'static str)>);

    /// Serve `respond(call_number)` on a local port and return its URL and
    /// call counter.
    async fn serve(respond: Respond) -> (String, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let app = Router::new().route(
            "/img",
            get(move || {
                let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                async move {
                    let (status, pairs) = respond(n);
                    let mut headers = AxumHeaders::new();
                    for (k, v) in pairs {
                        headers.insert(k, v.parse().unwrap());
                    }
                    (axum::http::StatusCode::from_u16(status).unwrap(), headers, "body")
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/img"), calls)
    }

    async fn fetch(url: &str) -> (Fetched, usize) {
        let client = reqwest::Client::new();
        let mut retries = 0;
        let f = fetch_with_retry(|| client.get(url), |_, _| retries += 1)
            .await
            .unwrap();
        (f, retries)
    }

    #[tokio::test]
    async fn a_rate_limited_download_is_retried_after_the_requested_wait() {
        let (url, calls) = serve(|n| {
            if n == 1 {
                (429, vec![("retry-after", "0")])
            } else {
                (200, vec![])
            }
        })
        .await;
        let (f, retries) = fetch(&url).await;
        assert_eq!(f.status, StatusCode::OK);
        assert_eq!(&f.body[..], b"body");
        assert_eq!((calls.load(Ordering::SeqCst), retries), (2, 1));
    }

    #[tokio::test]
    async fn persistent_server_errors_stop_after_the_last_attempt() {
        let (url, calls) = serve(|_| (503, vec![])).await;
        let (f, retries) = fetch(&url).await;
        assert_eq!(f.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!((calls.load(Ordering::SeqCst), retries), (MAX_ATTEMPTS as usize, 2));
    }

    #[tokio::test]
    async fn challenges_blocks_and_long_waits_are_not_retried() {
        for respond in [
            (|_| (503, vec![("cf-mitigated", "challenge")])) as Respond,
            |_| (403, vec![]),
            |_| (429, vec![("retry-after", "60")]),
        ] {
            let (url, calls) = serve(respond).await;
            let (_, retries) = fetch(&url).await;
            assert_eq!((calls.load(Ordering::SeqCst), retries), (1, 0));
        }
    }

    #[tokio::test]
    async fn the_host_limit_applies_per_host() {
        let limiter = HostLimiter::default();
        let mut held = Vec::new();
        for _ in 0..PER_HOST_LIMIT {
            held.push(limiter.acquire("https://cdn.example/a.png").await);
        }
        let wait = Duration::from_millis(50);
        let same_host = tokio::time::timeout(wait, limiter.acquire("https://cdn.example/b.png"));
        assert!(same_host.await.is_err());
        let other_host = tokio::time::timeout(wait, limiter.acquire("https://img.example/c.png"));
        assert!(other_host.await.is_ok());

        held.pop();
        let freed = tokio::time::timeout(wait, limiter.acquire("https://cdn.example/b.png"));
        assert!(freed.await.is_ok());
    }

    #[test]
    fn backoff_grows_and_stays_bounded() {
        for attempt in 1..=6 {
            let step = (BASE_DELAY * 2u32.pow(attempt - 1)).min(MAX_DELAY);
            let d = backoff(attempt);
            assert!(d >= step / 2 && d <= step, "attempt {attempt}: {d:?}");
        }
    }
}
