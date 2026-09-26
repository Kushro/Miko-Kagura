//! axum HTTP server for remote image upscaling.
//!
//! Endpoints mirror the Python FastAPI server:
//!   POST /upscale  — PNG bytes in, upscaled PNG bytes out
//!   GET  /status   — active model + stats
//!   GET  /health   — liveness check
//!   POST /model    — change the active model at runtime

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;

use crate::config::get_model;
use crate::history::{RequestHistory, RequestRecord, RequestSource};
use crate::upscaler::{enforce_webp_limit, normalize_png_bytes, Upscaler};

/// Server-side counters.
#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub requests_processed: u64,
    pub bytes_processed: u64,
}

/// Updates pushed from the server to the UI.
#[derive(Clone, Debug)]
pub enum ServerEvent {
    Request(RequestRecord),
    Error(String),
    Info(String),
    ModelChanged {
        display_name: String,
        scale: i32,
        noise: i32,
        gpu: String,
    },
}

/// Shared application state. Held in an `Arc` by both the HTTP server thread and
/// the Dioxus UI.
pub struct AppState {
    pub upscaler: Mutex<Upscaler>,
    pub history: Mutex<RequestHistory>,
    pub stats: Mutex<Stats>,
    pub events: broadcast::Sender<ServerEvent>,
    pub start_time: Instant,
    pub server_addr: String,
    /// Whether oxipng lossless compression is applied after upscaling.
    pub compress_enabled: AtomicBool,
    /// oxipng preset level 0 (fast) … 6 (max). Default 2.
    pub compress_level: AtomicU8,
    /// Normalize NCNN output to clean 8-bit RGB/RGBA PNG (Android BitmapFactory fix).
    /// Default true — prevents Coil from crashing on exotic PNG sub-formats.
    pub normalize_png: AtomicBool,
    /// Clamp output to WebP's 16383px per-dimension maximum. Default true.
    pub webp_compat: AtomicBool,
    /// Per-dimension clamp applied when `webp_compat` is on. Configurable from
    /// the dashboard; always clamped to `WEBP_MAX_DIMENSION` at use.
    pub webp_max_dimension: AtomicU32,
    /// Whether the clamp is being driven by the device calculator rather than
    /// the raw pixel slider. Presentation only — `webp_max_dimension` is still
    /// the single value the pipeline reads.
    pub clamp_device_mode: AtomicBool,
    /// Screen diagonal in tenths of an inch, for the device calculator.
    pub device_inches_tenths: AtomicU32,
    pub device_dpi: AtomicU32,
    pub clamp_headroom_pct: AtomicU32,
    /// Shared HTTP client for the URL endpoints (connection-pooled).
    pub http_client: reqwest::Client,
    /// Monotonic id handed to each batch request so its entries can be grouped.
    pub batch_counter: AtomicU64,
    /// Ids of source plugins the user enabled. Empty by default — every plugin
    /// ships disabled and only ever runs for the URL endpoints.
    pub enabled_plugins: Mutex<HashSet<String>>,
    /// Binaries directory explicitly chosen in the UI, for persistence. `None`
    /// means "keep auto-detecting".
    pub binary_dir_override: Mutex<Option<String>>,
    /// Active dashboard theme, mirrored here so it can be saved.
    pub theme: Mutex<String>,
}

impl AppState {
    /// Convenience: broadcast an event, ignoring "no subscribers" errors.
    pub fn emit(&self, event: ServerEvent) {
        let _ = self.events.send(event);
    }
}

// ── Request / response bodies ─────────────────────────────────────────

#[derive(Deserialize)]
pub struct ModelChangeRequest {
    pub model: String,
    #[serde(default = "default_scale")]
    pub scale: i32,
    #[serde(default)]
    pub noise: i32,
}

fn default_scale() -> i32 {
    2
}

#[derive(Serialize)]
struct StatusResponse {
    model: Option<String>,
    model_display_name: Option<String>,
    scale: Option<i32>,
    noise: Option<i32>,
    gpu: Option<String>,
    uptime_seconds: u64,
    requests_processed: u64,
    bytes_processed: u64,
    binary_dir: String,
    binaries_ready: bool,
}

#[derive(Serialize)]
struct ModelChangeResponse {
    model: String,
    model_display_name: String,
    scale: i32,
    noise: i32,
    loaded: bool,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    upscaler_ready: bool,
}

/// `POST /upscale/url` body — a single image URL the server will download.
/// `headers` carries the manga source's HTTP headers (User-Agent, Referer…) so the
/// download isn't rejected by anti-hotlinking; older clients simply omit it.
#[derive(Deserialize)]
struct UrlRequest {
    url: String,
    #[serde(default)]
    headers: std::collections::HashMap<String, String>,
}

/// `POST /upscale/batch` body — base64-encoded images.
#[derive(Deserialize)]
struct BatchImagesRequest {
    images: Vec<String>,
}

/// `POST /upscale/batch/url` body — image URLs the server downloads in parallel.
#[derive(Deserialize)]
struct BatchUrlsRequest {
    urls: Vec<String>,
    #[serde(default)]
    headers: std::collections::HashMap<String, String>,
}

/// One entry of a batch response — success carries the base64 PNG, failure the error.
#[derive(Serialize)]
struct BatchItem {
    index: usize,
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_ms: Option<u64>,
}

impl BatchItem {
    fn ok(index: usize, o: &UpscaleOutcome) -> Self {
        Self {
            index,
            success: true,
            image: Some(BASE64.encode(&o.bytes)),
            error: None,
            width: Some(o.width),
            height: Some(o.height),
            time_ms: Some(o.upscale_ms),
        }
    }

    fn err(index: usize, msg: String) -> Self {
        Self {
            index,
            success: false,
            image: None,
            error: Some(msg),
            width: None,
            height: None,
            time_ms: None,
        }
    }
}

#[derive(Serialize)]
struct BatchResponse {
    batch_id: u64,
    results: Vec<BatchItem>,
}

/// Result of upscaling one image, shared by every endpoint.
struct UpscaleOutcome {
    bytes: Vec<u8>,
    model: String,
    scale: i32,
    noise: i32,
    upscale_ms: u64,
    compress_ms: Option<u64>,
    width: u32,
    height: u32,
}

// ── Server bootstrap ──────────────────────────────────────────────────

/// Build the router and serve forever. Runs on its own tokio runtime/thread.
pub async fn serve(state: Arc<AppState>, host: String, port: u16, max_size_mb: usize) {
    let max_bytes = max_size_mb.saturating_mul(1024 * 1024).max(1024 * 1024);

    // Batched image uploads carry several base64 images, so allow a much larger
    // body on that route than the single-image limit.
    let batch_bytes = max_bytes.saturating_mul(32);

    let app = Router::new()
        .route("/upscale", post(upscale))
        .route("/upscale/url", post(upscale_url))
        .route(
            "/upscale/batch",
            post(upscale_batch).layer(DefaultBodyLimit::max(batch_bytes)),
        )
        .route("/upscale/batch/url", post(upscale_batch_url))
        .route("/status", get(status))
        .route("/health", get(health))
        .route("/model", post(change_model))
        .layer(DefaultBodyLimit::max(max_bytes))
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    let addr = format!("{host}:{port}");
    match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => {
            state.emit(ServerEvent::Info(format!("Server listening on {addr}")));
            if let Err(e) = axum::serve(listener, app).await {
                state.emit(ServerEvent::Error(format!("Server error: {e}")));
            }
        }
        Err(e) => {
            state.emit(ServerEvent::Error(format!("Failed to bind {addr}: {e}")));
        }
    }
}

// ── Handlers ──────────────────────────────────────────────────────────

/// Snapshot the active upscaler under a short lock, or 503 if no model is loaded.
/// The active model can change between requests and we must not hold the lock
/// during the subprocess, so each request works off its own clone.
fn snapshot_upscaler(state: &Arc<AppState>) -> Result<Upscaler, (StatusCode, String)> {
    let guard = state.upscaler.lock().unwrap();
    if guard.current_config.is_none() {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "No model loaded".to_string()));
    }
    // Fail fast with a clear message instead of letting every endpoint
    // discover this only after running (or downloading for URL endpoints).
    if let Some(hint) = guard.missing_binary_hint() {
        return Err((StatusCode::SERVICE_UNAVAILABLE, hint));
    }
    Ok(guard.clone())
}

/// Upscale one image end-to-end: run NCNN, optionally normalise + compress, then
/// record the request in history/stats tagged with its originating endpoint and
/// batch id. Returns the final PNG plus metadata, or an error string (already
/// logged to the Errors list).
async fn run_upscale(
    state: &Arc<AppState>,
    snapshot: Upscaler,
    input: Vec<u8>,
    source: RequestSource,
    batch_id: Option<u64>,
    origin_url: Option<String>,
) -> Result<UpscaleOutcome, String> {
    let model = snapshot.current_model_name.clone();
    let scale = snapshot.current_scale;
    let noise = snapshot.current_noise;

    let start = Instant::now();
    let input_for_proc = input.clone();
    let result = tokio::task::spawn_blocking(move || snapshot.process(&input_for_proc)).await;

    let upscaled = match result {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(e)) => {
            state.emit(ServerEvent::Error(format!("{model}: {e}")));
            return Err(e);
        }
        Err(e) => {
            state.emit(ServerEvent::Error(format!("{model}: worker panicked: {e}")));
            return Err("Worker thread failed".to_string());
        }
    };

    let upscale_ms = start.elapsed().as_millis() as u64;

    // ── Optional WebP compatibility (clamp to 16383px) ────────────────
    // WebP can't hold a dimension above 16383px; oversized upscales (e.g. tall
    // webtoon pages) break WebP encoding on the client. Downscale proportionally
    // to fit, preserving aspect ratio. No-op (no copy) when already within limits.
    let mut webp_ms: Option<u64> = None;
    let upscaled = if state.webp_compat.load(Ordering::Relaxed) {
        let webp_start = Instant::now();
        let limit = state.webp_max_dimension.load(Ordering::Relaxed);
        match enforce_webp_limit(&upscaled, limit) {
            Some((bytes, (fw, fh, tw, th))) => {
                webp_ms = Some(webp_start.elapsed().as_millis() as u64);
                state.emit(ServerEvent::Info(format!(
                    "WebP compat: downscaled {fw}x{fh} → {tw}x{th} (≤{limit}px)"
                )));
                bytes
            }
            None => upscaled,
        }
    } else {
        upscaled
    };

    // ── Optional PNG normalisation (Android BitmapFactory fix) ────────
    // NCNN binaries can emit 16-bit, palette, or grayscale+alpha PNGs that
    // Android's BitmapFactory (and Coil) refuse to decode. Re-encoding through
    // the `image` crate guarantees a clean 8-bit RGB/RGBA PNG.
    let upscaled = if state.normalize_png.load(Ordering::Relaxed) {
        normalize_png_bytes(&upscaled)
    } else {
        upscaled
    };

    // ── Optional oxipng lossless compression ──────────────────────────
    let (final_out, compress_ms) = if state.compress_enabled.load(Ordering::Relaxed) {
        let level = state.compress_level.load(Ordering::Relaxed);
        let opts = oxipng::Options::from_preset(level);
        let upscaled_clone = upscaled.clone(); // keep `upscaled` for the fallback path
        let compress_start = Instant::now();
        match tokio::task::spawn_blocking(move || {
            oxipng::optimize_from_memory(&upscaled_clone, &opts)
        })
        .await
        {
            Ok(Ok(compressed)) => {
                let cms = compress_start.elapsed().as_millis() as u64;
                (compressed, Some(cms))
            }
            Ok(Err(e)) => {
                state.emit(ServerEvent::Error(format!("oxipng failed, sending uncompressed: {e}")));
                (upscaled, None)
            }
            Err(e) => {
                state.emit(ServerEvent::Error(format!("oxipng worker panicked: {e}")));
                (upscaled, None)
            }
        }
    } else {
        (upscaled, None)
    };

    let (width, height) = image_dimensions(&final_out);

    let record = {
        let mut hist = state.history.lock().unwrap();
        hist.add(
            &input, &final_out, &model, scale, noise, upscale_ms, compress_ms, webp_ms, source,
            batch_id, origin_url,
        )
    };

    {
        let mut stats = state.stats.lock().unwrap();
        stats.requests_processed += 1;
        stats.bytes_processed += input.len() as u64;
    }

    state.emit(ServerEvent::Request(record));

    Ok(UpscaleOutcome {
        bytes: final_out,
        model,
        scale,
        noise,
        upscale_ms,
        compress_ms,
        width,
        height,
    })
}

/// Build the binary PNG response (with the X-Upscale-* headers) for a single image.
fn binary_response(o: UpscaleOutcome) -> Response {
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        .header("X-Upscale-Model", &o.model)
        .header("X-Upscale-Scale", o.scale.to_string())
        .header("X-Upscale-Noise", o.noise.to_string())
        .header("X-Process-Time-Ms", o.upscale_ms.to_string());
    if let Some(cms) = o.compress_ms {
        builder = builder.header("X-Compress-Time-Ms", cms.to_string());
    }
    builder.body(Body::from(o.bytes)).unwrap()
}

/// A downloaded image plus the response headers, which some source plugins need
/// (Comix ships its descrambling seed there rather than in the URL).
pub struct DownloadedImage {
    pub bytes: Vec<u8>,
    /// Response headers with lowercased names.
    pub headers: HashMap<String, String>,
}

/// Download an image, logging (and returning) a clear error on failure. HTTP
/// 403/503 responses are flagged as likely Cloudflare / anti-bot blocks.
/// `headers` are the manga source's own headers forwarded by the client — applying
/// them defeats most anti-hotlinking checks; invalid entries are skipped.
///
/// Enabled plugins may contribute headers of their own for hosts they claim;
/// the client's values always win, since the reader knows its session best.
async fn download_image(
    state: &Arc<AppState>,
    url: &str,
    headers: &HashMap<String, String>,
) -> Result<DownloadedImage, String> {
    // A fragment is a client-side marker (tile order, XOR key) and must not be
    // sent to the server — plugins read it from the original URL instead.
    let fetch_url = url.split('#').next().unwrap_or(url);
    let mut request = state.http_client.get(fetch_url);

    for (name, value) in plugin_request_headers(state, url) {
        if headers.keys().any(|k| k.eq_ignore_ascii_case(&name)) {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            reqwest::header::HeaderName::try_from(name.as_str()),
            reqwest::header::HeaderValue::try_from(value.as_str()),
        ) {
            request = request.header(n, v);
        }
    }
    for (name, value) in headers {
        if let (Ok(n), Ok(v)) = (
            reqwest::header::HeaderName::try_from(name.as_str()),
            reqwest::header::HeaderValue::try_from(value.as_str()),
        ) {
            request = request.header(n, v);
        }
    }
    let resp = match request.send().await {
        Ok(r) => r,
        Err(e) => {
            let msg = format!("Download failed for {url}: {e}");
            state.emit(ServerEvent::Error(msg.clone()));
            return Err(msg);
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let hint = if matches!(status.as_u16(), 403 | 429 | 503) {
            " (likely Cloudflare / anti-bot block)"
        } else {
            ""
        };
        let msg = format!("Download blocked for {url}: HTTP {}{hint}", status.as_u16());
        state.emit(ServerEvent::Error(msg.clone()));
        return Err(msg);
    }

    let resp_headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str().to_string(), v.to_string())))
        .collect();

    match resp.bytes().await {
        Ok(b) => Ok(DownloadedImage {
            bytes: b.to_vec(),
            headers: resp_headers,
        }),
        Err(e) => {
            let msg = format!("Download read failed for {url}: {e}");
            state.emit(ServerEvent::Error(msg.clone()));
            Err(msg)
        }
    }
}

/// Headers the enabled plugins want added for this URL's host.
fn plugin_request_headers(state: &Arc<AppState>, url: &str) -> Vec<(String, String)> {
    let enabled = state.enabled_plugins.lock().unwrap();
    if enabled.is_empty() {
        return Vec::new();
    }
    crate::plugins::matching(url)
        .into_iter()
        .filter(|p| enabled.contains(p.id))
        .flat_map(|p| (p.request_headers)(url))
        .collect()
}

/// Run every enabled plugin that claims this URL's host over the downloaded
/// bytes, in registry order.
///
/// This is deliberately non-fatal: a source that changed its scrambling should
/// degrade to "upscaled a scrambled page" with a visible error, not a failed
/// request, so the reader still gets an image back.
fn apply_plugins(state: &Arc<AppState>, url: &str, image: DownloadedImage) -> Vec<u8> {
    let enabled = state.enabled_plugins.lock().unwrap().clone();
    if enabled.is_empty() {
        return image.bytes;
    }
    // Descrambling decodes and re-encodes the page, so this is deliberately
    // kept off the async worker threads by the caller.
    let mut bytes = image.bytes;
    let mut handled = false;
    for plugin in crate::plugins::transformers_for(url) {
        if !enabled.contains(plugin.id) {
            continue;
        }
        let transform = match plugin.transform {
            Some(t) => t,
            None => continue,
        };
        let input = crate::plugins::PluginInput {
            fragment: crate::plugins::fragment_of(url),
            response_headers: &image.headers,
            bytes: &bytes,
        };
        match transform(&input) {
            Ok(crate::plugins::PluginOutput::Rewritten { bytes: new, note }) => {
                state.emit(ServerEvent::Info(format!("Plugin {note}")));
                bytes = new;
                handled = true;
            }
            Ok(crate::plugins::PluginOutput::Unchanged) => {}
            Err(e) => {
                state.emit(ServerEvent::Error(format!(
                    "Plugin '{}' failed for {url}: {e} — upscaling the image as downloaded",
                    plugin.id
                )));
                handled = true;
            }
        }
    }

    // A page that advertises itself as scrambled but that nothing claimed would
    // otherwise be upscaled scrambled, with nothing in the log to explain the
    // mess. Usually means the source serves images from a CDN this plugin's
    // host list doesn't cover.
    if !handled && crate::plugins::looks_scrambled(url) {
        state.emit(ServerEvent::Error(format!(
            "{url} looks scrambled but no enabled plugin handled it — the image \
             will be upscaled as-is. Check the Plugins tab for its source."
        )));
    }
    bytes
}

/// Decode image dimensions; (0, 0) if the bytes can't be parsed.
fn image_dimensions(bytes: &[u8]) -> (u32, u32) {
    match image::load_from_memory(bytes) {
        Ok(img) => (img.width(), img.height()),
        Err(_) => (0, 0),
    }
}

/// `POST /upscale` — single raw image in, upscaled PNG out.
async fn upscale(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    if body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Empty request body".to_string()));
    }
    let snapshot = snapshot_upscaler(&state)?;
    match run_upscale(&state, snapshot, body.to_vec(), RequestSource::Single, None, None).await {
        Ok(o) => Ok(binary_response(o)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Upscaling failed: {e}"))),
    }
}

/// Download, then hand the bytes to any enabled source plugin before upscaling.
/// Plugin work is image decode/encode, so it runs on the blocking pool.
async fn download_and_unscramble(
    state: &Arc<AppState>,
    url: &str,
    headers: &HashMap<String, String>,
) -> Result<Vec<u8>, String> {
    let image = download_image(state, url, headers).await?;
    let state = state.clone();
    let url = url.to_string();
    tokio::task::spawn_blocking(move || apply_plugins(&state, &url, image))
        .await
        .map_err(|e| format!("Plugin worker failed: {e}"))
}

/// `POST /upscale/url` — the server downloads the image, then upscales it.
async fn upscale_url(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UrlRequest>,
) -> Result<Response, (StatusCode, String)> {
    let snapshot = snapshot_upscaler(&state)?;
    let bytes = download_and_unscramble(&state, &req.url, &req.headers)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    match run_upscale(
        &state,
        snapshot,
        bytes,
        RequestSource::SingleUrl,
        None,
        Some(req.url),
    )
    .await
    {
        Ok(o) => Ok(binary_response(o)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Upscaling failed: {e}"))),
    }
}

/// `POST /upscale/batch` — many base64 images in, JSON of per-item results out.
async fn upscale_batch(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchImagesRequest>,
) -> Result<Json<BatchResponse>, (StatusCode, String)> {
    if req.images.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Empty batch".to_string()));
    }
    let snapshot = snapshot_upscaler(&state)?;
    let batch_id = state.batch_counter.fetch_add(1, Ordering::Relaxed);
    state.emit(ServerEvent::Info(format!(
        "Batch #{batch_id}: {} image(s)",
        req.images.len()
    )));

    let mut results = Vec::with_capacity(req.images.len());
    for (i, b64) in req.images.iter().enumerate() {
        match BASE64.decode(b64.as_bytes()) {
            Ok(bytes) => {
                let r = run_upscale(
                    &state,
                    snapshot.clone(),
                    bytes,
                    RequestSource::Batch,
                    Some(batch_id),
                    None,
                )
                .await;
                results.push(match r {
                    Ok(o) => BatchItem::ok(i, &o),
                    Err(e) => BatchItem::err(i, e),
                });
            }
            Err(e) => {
                let msg = format!("Batch #{batch_id}[{i}]: invalid base64: {e}");
                state.emit(ServerEvent::Error(msg.clone()));
                results.push(BatchItem::err(i, msg));
            }
        }
    }
    Ok(Json(BatchResponse { batch_id, results }))
}

/// `POST /upscale/batch/url` — many URLs in; downloaded in parallel then upscaled.
async fn upscale_batch_url(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchUrlsRequest>,
) -> Result<Json<BatchResponse>, (StatusCode, String)> {
    if req.urls.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Empty batch".to_string()));
    }
    let snapshot = snapshot_upscaler(&state)?;
    let batch_id = state.batch_counter.fetch_add(1, Ordering::Relaxed);
    let n = req.urls.len();
    state.emit(ServerEvent::Info(format!(
        "Batch #{batch_id}: downloading {n} URL(s)"
    )));

    // Download all images in parallel, preserving request order.
    let headers = Arc::new(req.headers);
    let mut set = tokio::task::JoinSet::new();
    for (i, url) in req.urls.into_iter().enumerate() {
        let state = state.clone();
        let headers = headers.clone();
        set.spawn(async move {
            let r = download_and_unscramble(&state, &url, &headers).await;
            (i, url, r)
        });
    }
    let mut downloaded: Vec<Option<(String, Result<Vec<u8>, String>)>> =
        (0..n).map(|_| None).collect();
    while let Some(joined) = set.join_next().await {
        if let Ok((i, url, r)) = joined {
            downloaded[i] = Some((url, r));
        }
    }

    // Upscale the successfully downloaded images.
    let mut results = Vec::with_capacity(n);
    for (i, slot) in downloaded.into_iter().enumerate() {
        let item = match slot {
            Some((url, Ok(bytes))) => {
                match run_upscale(
                    &state,
                    snapshot.clone(),
                    bytes,
                    RequestSource::BatchUrl,
                    Some(batch_id),
                    Some(url),
                )
                .await
                {
                    Ok(o) => BatchItem::ok(i, &o),
                    Err(e) => BatchItem::err(i, e),
                }
            }
            Some((_, Err(e))) => BatchItem::err(i, e), // already logged in download_image
            None => BatchItem::err(i, "download task dropped".to_string()),
        };
        results.push(item);
    }
    Ok(Json(BatchResponse { batch_id, results }))
}

async fn status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let (model, display, scale, noise, gpu, binary_dir, binaries_ready) = {
        let up = state.upscaler.lock().unwrap();
        let binary_dir = up.binary_dir().display().to_string();
        let binaries_ready = up.binaries_ok();
        if up.current_config.is_some() {
            (
                Some(up.current_model_name.clone()),
                Some(up.current_display_name.clone()),
                Some(up.current_scale),
                Some(up.current_noise),
                Some(up.gpu_name.clone()),
                binary_dir,
                binaries_ready,
            )
        } else {
            (None, None, None, None, None, binary_dir, binaries_ready)
        }
    };
    let stats = *state.stats.lock().unwrap();

    Json(StatusResponse {
        model,
        model_display_name: display,
        scale,
        noise,
        gpu,
        uptime_seconds: state.start_time.elapsed().as_secs(),
        requests_processed: stats.requests_processed,
        bytes_processed: stats.bytes_processed,
        binary_dir,
        binaries_ready,
    })
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // `current_config.is_some()` only means a model was *selected* — it says
    // nothing about whether its CLI binary actually exists on disk. Check both,
    // so a moved/missing `binaries/` folder shows up here instead of the app
    // reporting a healthy server that 500s on every request.
    let up = state.upscaler.lock().unwrap();
    let ready = up.current_config.is_some() && up.binaries_ok();
    Json(HealthResponse {
        status: "ok",
        upscaler_ready: ready,
    })
}

async fn change_model(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ModelChangeRequest>,
) -> Result<Json<ModelChangeResponse>, (StatusCode, String)> {
    if get_model(&body.model).is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Unknown model '{}'. Valid: {}",
                body.model,
                crate::config::model_keys_csv()
            ),
        ));
    }

    let (display, scale, noise, gpu) = {
        let mut up = state.upscaler.lock().unwrap();
        up.load_model(&body.model, body.scale, body.noise)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to load model: {e}")))?;
        (
            up.current_display_name.clone(),
            up.current_scale,
            up.current_noise,
            up.gpu_name.clone(),
        )
    };

    state.emit(ServerEvent::ModelChanged {
        display_name: display.clone(),
        scale,
        noise,
        gpu,
    });
    // A model switched over HTTP is as much a preference as one switched in the
    // dashboard, so it is saved the same way.
    crate::settings::persist(&state);

    Ok(Json(ModelChangeResponse {
        model: body.model,
        model_display_name: display,
        scale,
        noise,
        loaded: true,
    }))
}

/// Spawn the HTTP server on a dedicated thread with its own tokio runtime.
pub fn spawn_server(state: Arc<AppState>, host: String, port: u16, max_size_mb: usize) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                state.emit(ServerEvent::Error(format!("Failed to start runtime: {e}")));
                return;
            }
        };
        rt.block_on(serve(state, host, port, max_size_mb));
    });
}
