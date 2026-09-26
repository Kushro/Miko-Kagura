//! The Dioxus desktop dashboard — replaces the Python Textual TUI.
//!
//! Layout mirrors the TUI: a left model-configuration panel, a right tabbed log
//! panel (Requests / Settings / Errors / Storage), and a bottom status bar. UI
//! updates are driven by `ServerEvent`s broadcast from the HTTP server.

use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use dioxus::desktop::{use_wry_event_handler, WindowEvent};
use dioxus::prelude::*;
use tokio::sync::broadcast::error::RecvError;

use crate::app_state;
use crate::cache::{clear_stale_cache, format_bytes, scan_cache, CacheReport};
use crate::clamp::{
    self, DEVICE_PROFILES, HEADROOM_PCT_RANGE, HEADROOM_PRESETS,
};
use crate::config::{get_model, noise_name, supported_models};
use crate::history::{build_comparison, open_in_viewer, open_url, RequestRecord};
use crate::server::{AppState, ServerEvent};
use crate::settings::{DEVICE_DPI_RANGE, DEVICE_INCHES_TENTHS_RANGE};
use crate::upscaler::WEBP_MAX_DIMENSION;

const MAX_ROWS: usize = 50;
const THEMES: &[&str] = &["dark", "light", "catppuccin", "nord", "dracula"];

/// Write the current settings to disk.
///
/// Called after every committed change rather than only on exit, so settings
/// survive a crash or a task-kill as well as a clean close.
fn save_settings() {
    crate::settings::persist(app_state().as_ref());
}

/// Notched DPI values offered under the density slider.
const DPI_PRESETS: &[(u32, &str)] = &[
    (160, "mdpi"),
    (240, "hdpi"),
    (320, "xhdpi"),
    (480, "xxhdpi"),
    (640, "xxxhdpi"),
];

#[derive(Clone, PartialEq)]
struct RequestRow {
    id: u64,
    time: String,
    /// Endpoint label: single / batch / single uri / batch uri.
    source: String,
    /// Batch grouping id, or None for single requests.
    batch_id: Option<u64>,
    model: String,
    scale: i32,
    in_kb: usize,
    out_kb: usize,
    ms: u64,
    /// oxipng compression time in ms, or None when compression was off.
    compress_ms: Option<u64>,
    /// WebP-compat Lanczos3 downscale time in ms, or None when it didn't apply.
    webp_ms: Option<u64>,
}

/// oxipng preset levels exposed in the UI.
const COMPRESS_LEVELS: &[(u8, &str)] = &[
    (0, "Fast"),
    (2, "Default"),
    (4, "Thorough"),
    (6, "Max"),
];

/// Session-history auto-clean budget slider (Storage tab), in MB.
const STORAGE_LIMIT_MIN_MB: u64 = 256;
const STORAGE_LIMIT_MAX_MB: u64 = 4608;
const STORAGE_LIMIT_STEP_MB: u64 = 128;

/// Rough on-disk cost of one chapter of before/after captures, measured in
/// practice (~250 MB). Used only for the "≈ N chapters" hint next to the limit.
const STORAGE_MB_PER_CHAPTER: u64 = 250;

/// Notched presets on the auto-clean slider (value in MB).
const STORAGE_LIMIT_PRESETS: &[(u64, &str)] = &[
    (512, "Lean"),
    (1024, "Balanced"),
    (2048, "Generous"),
    (4096, "Collector"),
];

#[derive(Clone, PartialEq)]
struct LogLine {
    time: String,
    text: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Requests,
    Plugins,
    Settings,
    Errors,
    Storage,
}

/// HH:MM:SS (UTC) label for log timestamps.
fn now_label() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = secs % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

// ── Model actions (shared by switch / scale / noise controls) ─────────

fn switch_model(state: &AppState, key: &str) {
    let cfg = match get_model(key) {
        Some(c) => c,
        None => return,
    };
    let mut up = state.upscaler.lock().unwrap();

    // Preserve current scale/noise when valid for the new model, else default.
    let scale = if cfg.supported_scales.contains(&up.current_scale) {
        up.current_scale
    } else {
        cfg.supported_scales[0]
    };
    let noise = if cfg.supported_noise.contains(&up.current_noise) {
        up.current_noise
    } else {
        cfg.supported_noise[0]
    };

    match up.load_model(key, scale, noise) {
        Ok(()) => {
            let ev = ServerEvent::ModelChanged {
                display_name: up.current_display_name.clone(),
                scale: up.current_scale,
                noise: up.current_noise,
                gpu: up.gpu_name.clone(),
            };
            let info = format!("Switched to {} {scale}x (noise={noise})", cfg.display_name);
            drop(up);
            state.emit(ev);
            state.emit(ServerEvent::Info(info));
            save_settings();
        }
        Err(e) => {
            drop(up);
            state.emit(ServerEvent::Error(format!("Failed to switch model: {e}")));
        }
    }
}

fn cycle_scale(state: &AppState) {
    let mut up = state.upscaler.lock().unwrap();
    let key = up.current_model_name.clone();
    let cfg = match get_model(&key) {
        Some(c) => c,
        None => return,
    };
    let idx = cfg
        .supported_scales
        .iter()
        .position(|&s| s == up.current_scale)
        .unwrap_or(0);
    let new_scale = cfg.supported_scales[(idx + 1) % cfg.supported_scales.len()];
    let noise = up.current_noise;
    match up.load_model(&key, new_scale, noise) {
        Ok(()) => {
            let ev = ServerEvent::ModelChanged {
                display_name: up.current_display_name.clone(),
                scale: up.current_scale,
                noise: up.current_noise,
                gpu: up.gpu_name.clone(),
            };
            drop(up);
            state.emit(ev);
            state.emit(ServerEvent::Info(format!("Scale changed to {new_scale}x")));
            save_settings();
        }
        Err(e) => {
            drop(up);
            state.emit(ServerEvent::Error(format!("Failed to change scale: {e}")));
        }
    }
}

fn cycle_noise(state: &AppState) {
    let mut up = state.upscaler.lock().unwrap();
    let key = up.current_model_name.clone();
    let cfg = match get_model(&key) {
        Some(c) => c,
        None => return,
    };
    let idx = cfg
        .supported_noise
        .iter()
        .position(|&n| n == up.current_noise)
        .unwrap_or(0);
    let new_noise = cfg.supported_noise[(idx + 1) % cfg.supported_noise.len()];
    let scale = up.current_scale;
    match up.load_model(&key, scale, new_noise) {
        Ok(()) => {
            let ev = ServerEvent::ModelChanged {
                display_name: up.current_display_name.clone(),
                scale: up.current_scale,
                noise: up.current_noise,
                gpu: up.gpu_name.clone(),
            };
            drop(up);
            state.emit(ev);
            state.emit(ServerEvent::Info(format!(
                "Noise changed to {}",
                noise_name(new_noise)
            )));
            save_settings();
        }
        Err(e) => {
            drop(up);
            state.emit(ServerEvent::Error(format!("Failed to change noise: {e}")));
        }
    }
}

/// Apply a new binaries directory (from the Settings UI), returning the
/// resolved path and whether the current model's binary was found there.
fn apply_binary_dir(state: &AppState, path: &str) -> (String, bool) {
    let mut up = state.upscaler.lock().unwrap();
    up.set_binary_dir(path);
    let resolved = up.binary_dir().display().to_string();
    let hint = up.missing_binary_hint();
    drop(up);
    match &hint {
        None => state.emit(ServerEvent::Info(format!(
            "Binary directory set to '{resolved}' — binaries found."
        ))),
        Some(msg) => state.emit(ServerEvent::Error(msg.clone())),
    }
    // Save the resolved path, not the raw input: an empty box means "go back
    // to auto-detection", which is what `None` records.
    *state.binary_dir_override.lock().unwrap() = if path.trim().is_empty() {
        None
    } else {
        Some(resolved.clone())
    };
    save_settings();
    (resolved, hint.is_none())
}

/// Push a new clamp value into shared state. Saving is the caller's job, so a
/// live drag doesn't hit the disk on every tick.
fn set_clamp_px(px: u32) {
    app_state()
        .webp_max_dimension
        .store(px.clamp(clamp::CLAMP_MIN_PX, WEBP_MAX_DIMENSION), Ordering::Relaxed);
}

/// Recompute the clamp from the device inputs and apply it. Returns the full
/// breakdown so the caller can log the arithmetic.
fn apply_device_clamp(inches_tenths: u32, dpi: u32, headroom_pct: u32) -> clamp::ClampBreakdown {
    let state = app_state();
    state
        .device_inches_tenths
        .store(inches_tenths, Ordering::Relaxed);
    state.device_dpi.store(dpi, Ordering::Relaxed);
    state
        .clamp_headroom_pct
        .store(headroom_pct, Ordering::Relaxed);
    let breakdown = clamp::compute(inches_tenths, dpi, headroom_pct);
    set_clamp_px(breakdown.clamp_px);
    breakdown
}

// ── Storage / cache scanning ──────────────────────────────────────────

/// Run a blocking cache scan off the UI thread and publish the result.
///
/// The current UI comparison dir is resolved from `RequestHistory` *before*
/// entering `spawn_blocking` so we never hold the history lock across the
/// blocking IO.
async fn scan_once(mut report: Signal<Option<CacheReport>>, mut scanning: Signal<bool>) {
    scanning.set(true);
    let dir = app_state().history.lock().unwrap().current_dir();
    let scanned = tokio::task::spawn_blocking(move || scan_cache(dir.as_deref()))
        .await
        .ok();
    if let Some(r) = scanned {
        report.set(Some(r));
    }
    scanning.set(false);
}

/// Fire-and-forget wrapper around [`scan_once`] for use inside event handlers.
fn trigger_scan(report: Signal<Option<CacheReport>>, scanning: Signal<bool>) {
    spawn(async move { scan_once(report, scanning).await });
}

/// Apply a new session-history byte budget. Eviction of old records (and their
/// file deletion) happens off the UI thread; the panel is re-scanned after.
async fn apply_history_limit(
    mb: u64,
    report: Signal<Option<CacheReport>>,
    scanning: Signal<bool>,
) {
    let bytes = mb * 1024 * 1024;
    let res = tokio::task::spawn_blocking(move || {
        app_state().history.lock().unwrap().set_max_bytes(bytes)
    })
    .await
    .ok();
    if let Some((freed, evicted)) = res {
        if evicted > 0 {
            app_state().emit(ServerEvent::Info(format!(
                "History limit set to {} — auto-cleaned {} old records ({} freed)",
                format_bytes(bytes),
                evicted,
                format_bytes(freed)
            )));
        } else {
            app_state().emit(ServerEvent::Info(format!(
                "History limit set to {}",
                format_bytes(bytes)
            )));
        }
        save_settings();
    }
    scan_once(report, scanning).await;
}

// ── Root component ────────────────────────────────────────────────────

#[component]
pub fn App() -> Element {
    let mut active_display =
        use_signal(|| app_state().upscaler.lock().unwrap().current_display_name.clone());
    let mut active_scale = use_signal(|| app_state().upscaler.lock().unwrap().current_scale);
    let mut active_noise = use_signal(|| app_state().upscaler.lock().unwrap().current_noise);
    let mut gpu_name = use_signal(|| app_state().upscaler.lock().unwrap().gpu_name.clone());

    let mut requests = use_signal(Vec::<RequestRow>::new);
    let mut settings_log = use_signal(Vec::<LogLine>::new);
    let mut error_log = use_signal(Vec::<LogLine>::new);
    let mut total_requests = use_signal(|| 0u64);
    let mut active_tab = use_signal(|| Tab::Requests);
    let mut compare = use_signal(|| None as Option<RequestRecord>);
    let mut theme = use_signal(|| app_state().theme.lock().unwrap().clone());

    // Compression state — mirrors the AtomicBool/AtomicU8 in AppState.
    let mut compress_enabled =
        use_signal(|| app_state().compress_enabled.load(Ordering::Relaxed));
    let mut compress_level =
        use_signal(|| app_state().compress_level.load(Ordering::Relaxed));

    // Advanced state.
    let mut normalize_png =
        use_signal(|| app_state().normalize_png.load(Ordering::Relaxed));
    let mut webp_compat =
        use_signal(|| app_state().webp_compat.load(Ordering::Relaxed));
    // Owned by `ClampSelector`, which mutates them; App only reads for display.
    let webp_max_dim = use_signal(|| app_state().webp_max_dimension.load(Ordering::Relaxed));
    let clamp_device_mode =
        use_signal(|| app_state().clamp_device_mode.load(Ordering::Relaxed));
    let device_inches = use_signal(|| app_state().device_inches_tenths.load(Ordering::Relaxed));
    let device_dpi = use_signal(|| app_state().device_dpi.load(Ordering::Relaxed));
    let headroom_pct = use_signal(|| app_state().clamp_headroom_pct.load(Ordering::Relaxed));

    // Binary directory — where NCNN CLI binaries are resolved from.
    let mut binary_dir_display =
        use_signal(|| app_state().upscaler.lock().unwrap().binary_dir().display().to_string());
    let mut binaries_ok = use_signal(|| app_state().upscaler.lock().unwrap().binaries_ok());
    let mut binary_dir_input = use_signal(|| binary_dir_display.read().clone());

    let server_addr = use_hook(|| app_state().server_addr.clone());

    // Settings are written on every committed change, but a close is the last
    // chance to catch anything still only in memory — including closes that
    // bypass our title bar (Alt+F4, taskbar, session logout).
    use_wry_event_handler(move |event, _| {
        if let dioxus::desktop::tao::event::Event::WindowEvent { event, .. } = event {
            if matches!(event, WindowEvent::CloseRequested | WindowEvent::Destroyed) {
                save_settings();
            }
        }
    });

    // Subscribe to server events and fan them out into the UI signals.
    use_future(move || async move {
        let mut rx = app_state().events.subscribe();
        loop {
            match rx.recv().await {
                Ok(ServerEvent::Request(rec)) => {
                    let mut rows = requests.write();
                    rows.push(RequestRow {
                        id: rec.id,
                        time: rec.time_label.clone(),
                        source: rec.source.label().to_string(),
                        batch_id: rec.batch_id,
                        model: rec.model.clone(),
                        scale: rec.scale,
                        in_kb: rec.size_in / 1024,
                        out_kb: rec.size_out / 1024,
                        ms: rec.time_ms,
                        compress_ms: rec.compress_ms,
                        webp_ms: rec.webp_ms,
                    });
                    while rows.len() > MAX_ROWS {
                        rows.remove(0);
                    }
                    drop(rows);
                    total_requests += 1;
                }
                Ok(ServerEvent::Info(msg)) => {
                    settings_log.write().push(LogLine {
                        time: now_label(),
                        text: msg,
                    });
                }
                Ok(ServerEvent::Error(msg)) => {
                    error_log.write().push(LogLine {
                        time: now_label(),
                        text: msg,
                    });
                }
                Ok(ServerEvent::ModelChanged {
                    display_name,
                    scale,
                    noise,
                    gpu,
                }) => {
                    active_display.set(display_name);
                    active_scale.set(scale);
                    active_noise.set(noise);
                    gpu_name.set(gpu);
                    // Different models can need different CLI binaries, so
                    // re-check readiness whenever the active model changes.
                    binaries_ok.set(app_state().upscaler.lock().unwrap().binaries_ok());
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });

    let theme_class = format!("theme-{}", theme.read());
    let display = active_display.read().clone();
    let scale = *active_scale.read();
    let noise = *active_noise.read();
    let gpu = gpu_name.read().clone();
    let tab = *active_tab.read();

    rsx! {
        document::Stylesheet { href: crate::TAILWIND_CSS }
        document::Stylesheet { href: crate::MAIN_CSS }
        document::Stylesheet { href: crate::THEME_CSS }
        document::Link { rel: "stylesheet", href: "https://fonts.bunny.net/css?family=inter:wght@400;500;600;700;800&display=swap" }
        document::Link { rel: "stylesheet", href: "https://fonts.bunny.net/css?family=jetbrains-mono:400;500;700;800&display=swap" }
        document::Link { rel: "stylesheet", href: "https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.5.1/css/all.min.css" }

        div {
            class: "flex flex-col h-screen text-white select-none {theme_class}",
            style: "background-color: var(--color-black);",

            super::title_bar::TitleBar {}

            // ── Missing-binaries alert ─────────────────────────────────
            if !*binaries_ok.read() {
                div {
                    class: "flex items-center gap-2 mx-4 mt-3 px-4 py-2 rounded-lg bg-red-500/15 border border-red-500/30 text-red-300 text-sm flex-shrink-0",
                    i { class: "fa-solid fa-triangle-exclamation flex-shrink-0" }
                    span {
                        "Upscaler binaries not found under "
                        span { class: "font-mono text-red-200", "{binary_dir_display}" }
                        ". Fix the path below (Advanced → Binary Directory)."
                    }
                }
            }

            // Main content: model panel + log panel
            div {
                class: "flex flex-1 min-h-0 gap-4 p-4",

                // ── Left: model configuration ─────────────────────────
                div {
                    class: "flex flex-col w-80 flex-shrink-0 bg-white/5 border border-white/10 rounded-xl p-5 overflow-y-auto",

                    h2 {
                        class: "text-sm font-bold tracking-widest text-white/90 uppercase mb-4",
                        style: "font-family: 'JetBrains Mono', monospace;",
                        "Model Configuration"
                    }

                    p { class: "text-base font-semibold text-green-400", "Active: {display} {scale}x" }
                    p { class: "text-sm text-slate-300 mt-1", "Scale: {scale}x  |  noise={noise}" }
                    p { class: "text-sm text-slate-400 mt-1", i { class: "fa-solid fa-microchip mr-2 text-slate-500" } "{gpu}" }

                    div { class: "h-px bg-white/10 my-4" }

                    p {
                        class: "text-xs font-semibold uppercase tracking-wider text-slate-500 mb-2",
                        "Switch Model"
                    }
                    div { class: "flex flex-col gap-1.5",
                        for (idx, m) in supported_models().iter().enumerate() {
                            {
                                let active = display == m.display_name;
                                let active_class = if active {
                                    "bg-indigo-500/20 text-indigo-300 border-indigo-500/30"
                                } else {
                                    "bg-white/5 text-slate-300 border-white/10 hover:bg-white/10 hover:text-white"
                                };
                                let scales = m
                                    .supported_scales
                                    .iter()
                                    .map(|s| s.to_string())
                                    .collect::<Vec<_>>()
                                    .join("/");
                                let key = m.name;
                                rsx! {
                                    button {
                                        key: "{m.name}",
                                        class: "flex items-center justify-between text-left text-sm px-3 py-2 rounded-lg border transition-all duration-150 cursor-pointer {active_class}",
                                        onclick: move |_| switch_model(app_state().as_ref(), key),
                                        span {
                                            span { class: "text-slate-500 mr-2", "[{idx + 1}]" }
                                            "{m.display_name}"
                                        }
                                        span { class: "text-xs text-slate-500", "{scales}x" }
                                    }
                                }
                            }
                        }
                    }

                    div { class: "h-px bg-white/10 my-4" }

                    p {
                        class: "text-xs font-semibold uppercase tracking-wider text-slate-500 mb-2",
                        "Adjust"
                    }
                    div { class: "flex gap-2",
                        button {
                            class: "flex-1 inline-flex items-center justify-center gap-2 text-sm px-3 py-2 rounded-lg bg-white/10 hover:bg-white/15 text-white border border-white/10 transition-all cursor-pointer",
                            onclick: move |_| cycle_scale(app_state().as_ref()),
                            i { class: "fa-solid fa-expand text-xs" }
                            "Scale"
                        }
                        button {
                            class: "flex-1 inline-flex items-center justify-center gap-2 text-sm px-3 py-2 rounded-lg bg-white/10 hover:bg-white/15 text-white border border-white/10 transition-all cursor-pointer",
                            onclick: move |_| cycle_noise(app_state().as_ref()),
                            i { class: "fa-solid fa-wand-magic-sparkles text-xs" }
                            "Noise"
                        }
                    }

                    // ── Compression ───────────────────────────────────
                    div { class: "h-px bg-white/10 my-4" }

                    p {
                        class: "text-xs font-semibold uppercase tracking-wider text-slate-500 mb-2",
                        "Compression"
                    }

                    // Enable / disable toggle
                    {
                        let enabled = *compress_enabled.read();
                        let (toggle_cls, icon_cls, label) = if enabled {
                            (
                                "bg-green-500/20 text-green-300 border-green-500/30 hover:bg-green-500/30",
                                "fa-solid fa-compress",
                                "Enabled — lossless oxipng",
                            )
                        } else {
                            (
                                "bg-white/5 text-slate-400 border-white/10 hover:bg-white/10 hover:text-white",
                                "fa-solid fa-compress",
                                "Disabled",
                            )
                        };
                        rsx! {
                            button {
                                class: "w-full flex items-center justify-between text-sm px-3 py-2 rounded-lg border transition-all duration-150 cursor-pointer {toggle_cls}",
                                onclick: move |_| {
                                    let new_val = !*compress_enabled.read();
                                    app_state().compress_enabled.store(new_val, Ordering::Relaxed);
                                    compress_enabled.set(new_val);
                                    app_state().emit(ServerEvent::Info(format!(
                                        "Compression {}", if new_val { "enabled" } else { "disabled" }
                                    )));
                                    save_settings();
                                },
                                span { class: "inline-flex items-center gap-2",
                                    i { class: "{icon_cls} text-xs" }
                                    "{label}"
                                }
                                // Pill indicator
                                span {
                                    class: if enabled {
                                        "text-xs font-semibold px-2 py-0.5 rounded-full bg-green-500 text-white"
                                    } else {
                                        "text-xs font-semibold px-2 py-0.5 rounded-full bg-white/10 text-slate-500"
                                    },
                                    if enabled { "ON" } else { "OFF" }
                                }
                            }
                        }
                    }

                    // Level selector — visible only when enabled
                    if *compress_enabled.read() {
                        div { class: "flex gap-1.5 mt-2",
                            for (lvl, name) in COMPRESS_LEVELS {
                                {
                                    let lvl = *lvl;
                                    let name = *name;
                                    let active = *compress_level.read() == lvl;
                                    let cls = if active {
                                        "bg-indigo-500/20 text-indigo-300 border-indigo-500/30"
                                    } else {
                                        "bg-white/5 text-slate-400 border-white/10 hover:bg-white/10 hover:text-white"
                                    };
                                    rsx! {
                                        button {
                                            key: "{lvl}",
                                            class: "flex-1 text-xs px-2 py-1.5 rounded-lg border transition-all cursor-pointer {cls}",
                                            title: "oxipng preset {lvl}",
                                            onclick: move |_| {
                                                app_state().compress_level.store(lvl, Ordering::Relaxed);
                                                compress_level.set(lvl);
                                                app_state().emit(ServerEvent::Info(
                                                    format!("Compression level: {name} (preset {lvl})")
                                                ));
                                                save_settings();
                                            },
                                            "{name}"
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // ── Advanced ──────────────────────────────────────
                    div { class: "h-px bg-white/10 my-4" }

                    p {
                        class: "text-xs font-semibold uppercase tracking-wider text-slate-500 mb-2",
                        "Advanced"
                    }

                    // PNG normalisation toggle
                    {
                        let norm_on = *normalize_png.read();
                        let (row_cls, pill_cls, pill_label) = if norm_on {
                            (
                                "bg-blue-500/10 text-blue-300 border-blue-500/20 hover:bg-blue-500/20",
                                "text-xs font-semibold px-2 py-0.5 rounded-full bg-blue-500 text-white",
                                "ON",
                            )
                        } else {
                            (
                                "bg-white/5 text-slate-400 border-white/10 hover:bg-white/10 hover:text-white",
                                "text-xs font-semibold px-2 py-0.5 rounded-full bg-white/10 text-slate-500",
                                "OFF",
                            )
                        };
                        rsx! {
                            button {
                                class: "w-full flex items-center justify-between text-sm px-3 py-2 rounded-lg border transition-all duration-150 cursor-pointer {row_cls}",
                                title: "Re-encode NCNN output as clean 8-bit RGB/RGBA PNG.\nPrevents 'BitmapFactory returned null bitmap' on Android — Coil rejects 16-bit, palette, or grayscale+alpha PNGs that NCNN sometimes emits.",
                                onclick: move |_| {
                                    let new_val = !*normalize_png.read();
                                    app_state().normalize_png.store(new_val, Ordering::Relaxed);
                                    normalize_png.set(new_val);
                                    app_state().emit(ServerEvent::Info(format!(
                                        "PNG normalisation {}",
                                        if new_val { "enabled" } else { "disabled (raw NCNN output)" }
                                    )));
                                    save_settings();
                                },
                                span { class: "flex flex-col items-start gap-0.5",
                                    span { class: "inline-flex items-center gap-2",
                                        i { class: "fa-solid fa-file-image text-xs" }
                                        "PNG Normalisation"
                                    }
                                    span { class: "text-xs text-slate-500 font-normal",
                                        "Fix Android BitmapFactory null bitmap"
                                    }
                                }
                                span { class: "{pill_cls}", "{pill_label}" }
                            }
                        }
                    }

                    // WebP compatibility toggle
                    {
                        let on = *webp_compat.read();
                        let max_dim = *webp_max_dim.read();
                        let (row_cls, pill_cls, pill_label) = if on {
                            (
                                "bg-teal-500/10 text-teal-300 border-teal-500/20 hover:bg-teal-500/20 mt-2",
                                "text-xs font-semibold px-2 py-0.5 rounded-full bg-teal-500 text-white",
                                "ON",
                            )
                        } else {
                            (
                                "bg-white/5 text-slate-400 border-white/10 hover:bg-white/10 hover:text-white mt-2",
                                "text-xs font-semibold px-2 py-0.5 rounded-full bg-white/10 text-slate-500",
                                "OFF",
                            )
                        };
                        rsx! {
                            button {
                                class: "w-full flex items-center justify-between text-sm px-3 py-2 rounded-lg border transition-all duration-150 cursor-pointer {row_cls}",
                                title: "Clamp upscaled output to a per-dimension maximum (WebP's hard limit is 16383px).\nOversized pages (e.g. tall webtoons) are downscaled proportionally with Lanczos3 so WebP encoding on the client doesn't break. No-op for images already within the limit.",
                                onclick: move |_| {
                                    let new_val = !*webp_compat.read();
                                    app_state().webp_compat.store(new_val, Ordering::Relaxed);
                                    webp_compat.set(new_val);
                                    app_state().emit(ServerEvent::Info(format!(
                                        "WebP compatibility {}",
                                        if new_val { "enabled" } else { "disabled" }
                                    )));
                                    save_settings();
                                },
                                span { class: "flex flex-col items-start gap-0.5",
                                    span { class: "inline-flex items-center gap-2",
                                        i { class: "fa-solid fa-ruler-combined text-xs" }
                                        "WebP Compatibility"
                                    }
                                    span { class: "text-xs text-slate-500 font-normal",
                                        "Clamp output to {max_dim}px max"
                                    }
                                }
                                span { class: "{pill_cls}", "{pill_label}" }
                            }
                        }
                    }

                    // Clamp selector — only meaningful while the toggle above is on.
                    if *webp_compat.read() {
                        ClampSelector {
                            webp_max_dim,
                            clamp_device_mode,
                            device_inches,
                            device_dpi,
                            headroom_pct,
                        }
                    }

                    // ── Binary directory ───────────────────────────────
                    div { class: "h-px bg-white/10 my-4" }

                    p {
                        class: "text-xs font-semibold uppercase tracking-wider text-slate-500 mb-2",
                        "Binary Directory"
                    }

                    div {
                        class: "flex items-center justify-between gap-2 text-xs text-slate-400 mb-1.5",
                        span {
                            class: "truncate",
                            title: "{binary_dir_display}",
                            i {
                                class: if *binaries_ok.read() {
                                    "fa-solid fa-circle-check text-green-400 mr-1.5"
                                } else {
                                    "fa-solid fa-circle-exclamation text-red-400 mr-1.5"
                                }
                            }
                            "{binary_dir_display}"
                        }
                        span {
                            class: if *binaries_ok.read() {
                                "text-xs font-semibold px-2 py-0.5 rounded-full bg-green-500 text-white flex-shrink-0"
                            } else {
                                "text-xs font-semibold px-2 py-0.5 rounded-full bg-red-500 text-white flex-shrink-0"
                            },
                            if *binaries_ok.read() { "OK" } else { "MISSING" }
                        }
                    }

                    div { class: "flex gap-1.5",
                        input {
                            r#type: "text",
                            class: "flex-1 min-w-0 text-xs px-2 py-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-200 placeholder-slate-600 focus:outline-none focus:border-indigo-500/50",
                            placeholder: "Path to ncnn-vulkan binaries…",
                            value: "{binary_dir_input}",
                            oninput: move |evt| binary_dir_input.set(evt.value()),
                        }
                        button {
                            class: "text-xs px-2.5 py-1.5 rounded-lg bg-white/10 hover:bg-white/15 text-white border border-white/10 transition-all cursor-pointer flex-shrink-0",
                            title: "Browse for folder",
                            onclick: move |_| {
                                spawn(async move {
                                    let picked = tokio::task::spawn_blocking(|| {
                                        rfd::FileDialog::new()
                                            .set_title("Select ncnn-vulkan binaries folder")
                                            .pick_folder()
                                    })
                                    .await
                                    .ok()
                                    .flatten();
                                    if let Some(path) = picked {
                                        let (resolved, ok) = apply_binary_dir(
                                            app_state().as_ref(),
                                            &path.display().to_string(),
                                        );
                                        binary_dir_display.set(resolved.clone());
                                        binary_dir_input.set(resolved);
                                        binaries_ok.set(ok);
                                    }
                                });
                            },
                            i { class: "fa-solid fa-folder-open" }
                        }
                    }

                    button {
                        class: "w-full mt-1.5 text-xs px-3 py-1.5 rounded-lg bg-indigo-500/20 hover:bg-indigo-500/30 text-indigo-300 border border-indigo-500/30 transition-all cursor-pointer",
                        onclick: move |_| {
                            let path = binary_dir_input.read().clone();
                            let (resolved, ok) = apply_binary_dir(app_state().as_ref(), &path);
                            binary_dir_display.set(resolved.clone());
                            binary_dir_input.set(resolved);
                            binaries_ok.set(ok);
                        },
                        "Apply"
                    }
                }

                // ── Right: tabbed log panel ───────────────────────────
                div {
                    class: "flex flex-col flex-1 min-w-0 bg-white/5 border border-white/10 rounded-xl overflow-hidden",

                    // Tab bar
                    div { class: "flex items-center gap-1 px-3 pt-3 border-b border-white/5",
                        TabButton { label: "Requests", icon: "fa-solid fa-list", active: tab == Tab::Requests,
                            onselect: move |_| active_tab.set(Tab::Requests) }
                        TabButton { label: "Plugins", icon: "fa-solid fa-puzzle-piece", active: tab == Tab::Plugins,
                            onselect: move |_| active_tab.set(Tab::Plugins) }
                        TabButton { label: "Settings", icon: "fa-solid fa-gear", active: tab == Tab::Settings,
                            onselect: move |_| active_tab.set(Tab::Settings) }
                        TabButton { label: "Errors", icon: "fa-solid fa-triangle-exclamation", active: tab == Tab::Errors,
                            onselect: move |_| active_tab.set(Tab::Errors) }
                        TabButton { label: "Storage", icon: "fa-solid fa-hard-drive", active: tab == Tab::Storage,
                            onselect: move |_| active_tab.set(Tab::Storage) }

                        div { class: "flex-1" }

                        // Theme cycle
                        button {
                            class: "w-8 h-8 mb-1 flex items-center justify-center rounded-lg text-slate-400 hover:text-white hover:bg-white/10 transition-colors cursor-pointer",
                            title: "Cycle theme",
                            onclick: move |_| {
                                let cur = theme.read().clone();
                                let idx = THEMES.iter().position(|&t| t == cur).unwrap_or(0);
                                let next = THEMES[(idx + 1) % THEMES.len()].to_string();
                                *app_state().theme.lock().unwrap() = next.clone();
                                theme.set(next);
                                save_settings();
                            },
                            i { class: "fa-solid fa-palette text-sm" }
                        }
                        // Clear active log
                        button {
                            class: "w-8 h-8 mb-1 flex items-center justify-center rounded-lg text-slate-400 hover:text-white hover:bg-white/10 transition-colors cursor-pointer",
                            title: "Clear current view",
                            onclick: move |_| match *active_tab.read() {
                                Tab::Requests => requests.write().clear(),
                                Tab::Settings => settings_log.write().clear(),
                                Tab::Errors => error_log.write().clear(),
                                // Storage and Plugins have their own actions;
                                // the eraser is a no-op there.
                                Tab::Storage | Tab::Plugins => {}
                            },
                            i { class: "fa-solid fa-eraser text-sm" }
                        }
                    }

                    // Tab content
                    div { class: "flex-1 min-h-0 overflow-y-auto p-3",
                        match tab {
                            Tab::Requests => rsx! { RequestsTable { rows: requests, compare } },
                            Tab::Plugins => rsx! { PluginsPanel {} },
                            Tab::Settings => rsx! { LogView { lines: settings_log, color: "text-green-400", empty: "No settings activity yet." } },
                            Tab::Errors => rsx! { LogView { lines: error_log, color: "text-red-400", empty: "No errors." } },
                            Tab::Storage => rsx! { StoragePanel { requests, compare } },
                        }
                    }
                }
            }

            // ── Bottom status bar ─────────────────────────────────────
            div {
                class: "flex items-center gap-4 h-9 px-4 bg-black/40 border-t border-white/5 flex-shrink-0 text-xs",
                span { class: "inline-flex items-center gap-2 text-slate-300",
                    span { class: "w-1.5 h-1.5 rounded-full bg-green-500" }
                    "Server: "
                    span { class: "font-semibold text-white", "{server_addr}" }
                }
                span { class: "text-slate-500", "|" }
                span { class: "text-slate-300", "Requests: {total_requests}" }
            }
        }

        // ── Comparison modal ──────────────────────────────────────────
        if let Some(rec) = compare.read().clone() {
            CompareModal { record: rec, on_close: move |_| compare.set(None) }
        }
    }
}

// ── Sub-components ─────────────────────────────────────────────────────

/// The maximum-dimension control.
///
/// Two ways to say the same thing: a raw pixel slider (unchanged from before)
/// or a device calculator that derives the pixel value from a screen diagonal,
/// pixel density and zoom headroom. Device mode always shows the arithmetic and
/// the pixel result it lands on, and switching back to Pixels keeps that value
/// — the two modes are views of one number, not separate settings.
#[component]
fn ClampSelector(
    webp_max_dim: Signal<u32>,
    clamp_device_mode: Signal<bool>,
    device_inches: Signal<u32>,
    device_dpi: Signal<u32>,
    headroom_pct: Signal<u32>,
) -> Element {
    let mut webp_max_dim = webp_max_dim;
    let mut clamp_device_mode = clamp_device_mode;
    let mut device_inches = device_inches;
    let mut device_dpi = device_dpi;
    let mut headroom_pct = headroom_pct;

    let device_mode = *clamp_device_mode.read();
    let inches = *device_inches.read();
    let dpi = *device_dpi.read();
    let headroom = *headroom_pct.read();
    let breakdown = clamp::compute(inches, dpi, headroom);
    // What the pipeline will actually use. Normally identical to the computed
    // value in device mode, but a `--webp-max-dimension` flag wins for the run.
    let applied_px = *webp_max_dim.read();

    // Recompute + apply after any device-input change, keeping the pixel
    // readout in sync so switching modes never surprises the user.
    //
    // Each field is optional and unspecified ones are read from the signals
    // rather than from a render-time snapshot: handlers outlive the render that
    // created them, so a snapshot taken before a sibling slider moved would
    // write that stale value back over the newer one.
    let mut recompute = move |new_inches: Option<u32>,
                              new_dpi: Option<u32>,
                              new_headroom: Option<u32>,
                              log: bool| {
        let inches = new_inches.unwrap_or_else(|| *device_inches.read());
        let dpi = new_dpi.unwrap_or_else(|| *device_dpi.read());
        let headroom = new_headroom.unwrap_or_else(|| *headroom_pct.read());
        if let Some(v) = new_inches {
            device_inches.set(v);
        }
        if let Some(v) = new_dpi {
            device_dpi.set(v);
        }
        if let Some(v) = new_headroom {
            headroom_pct.set(v);
        }
        let b = apply_device_clamp(inches, dpi, headroom);
        webp_max_dim.set(b.clamp_px);
        if log {
            let capped = if b.hit_container_limit {
                format!(" (capped from {}px by the WebP limit)", b.raw_clamp_px)
            } else {
                String::new()
            };
            app_state().emit(ServerEvent::Info(format!(
                "Clamp from device: {} → {} → {}px{capped}",
                b.device_label(),
                b.resolution_label(),
                b.clamp_px
            )));
            save_settings();
        }
    };

    rsx! {
        div { class: "flex flex-col gap-2 mt-2 px-1",

            // Mode switch — the clamp is one value, expressed two ways.
            div { class: "flex gap-1 p-0.5 rounded-lg bg-white/5 border border-white/10",
                {
                    let (px_cls, dev_cls) = if device_mode {
                        (
                            "flex-1 text-xs px-2 py-1 rounded-md text-slate-400 hover:text-white transition-all cursor-pointer",
                            "flex-1 text-xs px-2 py-1 rounded-md bg-teal-500/25 text-teal-200 font-semibold transition-all cursor-pointer",
                        )
                    } else {
                        (
                            "flex-1 text-xs px-2 py-1 rounded-md bg-teal-500/25 text-teal-200 font-semibold transition-all cursor-pointer",
                            "flex-1 text-xs px-2 py-1 rounded-md text-slate-400 hover:text-white transition-all cursor-pointer",
                        )
                    };
                    rsx! {
                        button {
                            class: "{px_cls}",
                            title: "Set the clamp directly in pixels.",
                            onclick: move |_| {
                                app_state().clamp_device_mode.store(false, Ordering::Relaxed);
                                clamp_device_mode.set(false);
                                save_settings();
                            },
                            i { class: "fa-solid fa-ruler-horizontal mr-1.5 text-[10px]" }
                            "Pixels"
                        }
                        button {
                            class: "{dev_cls}",
                            title: "Derive the clamp from a screen's diagonal and pixel density.",
                            onclick: move |_| {
                                app_state().clamp_device_mode.store(true, Ordering::Relaxed);
                                clamp_device_mode.set(true);
                                // Adopting device mode immediately applies its
                                // computed value, so the readout never lies.
                                recompute(None, None, None, true);
                            },
                            i { class: "fa-solid fa-mobile-screen mr-1.5 text-[10px]" }
                            "Device"
                        }
                    }
                }
            }

            if device_mode {
                // ── Screen diagonal ───────────────────────────────────
                div { class: "flex items-center justify-between text-xs text-slate-400",
                    span { "Screen size" }
                    span { class: "font-mono text-teal-300", "{inches / 10}.{inches % 10}\"" }
                }
                input {
                    r#type: "range",
                    class: "w-full accent-teal-400 cursor-pointer",
                    min: "{DEVICE_INCHES_TENTHS_RANGE.0}",
                    max: "{DEVICE_INCHES_TENTHS_RANGE.1}",
                    step: "1",
                    value: "{inches}",
                    oninput: move |evt| {
                        if let Ok(v) = evt.value().parse::<u32>() {
                            recompute(Some(v), None, None, false);
                        }
                    },
                    onchange: move |evt| {
                        if let Ok(v) = evt.value().parse::<u32>() {
                            recompute(Some(v), None, None, true);
                        }
                    },
                }
                // Device presets, positioned along the slider they snap to.
                div { class: "relative h-7 mx-1.5",
                    for profile in DEVICE_PROFILES {
                        {
                            let pct = profile.inches_tenths.saturating_sub(DEVICE_INCHES_TENTHS_RANGE.0) as f64
                                * 100.0
                                / (DEVICE_INCHES_TENTHS_RANGE.1 - DEVICE_INCHES_TENTHS_RANGE.0) as f64;
                            let active = inches == profile.inches_tenths && dpi == profile.dpi;
                            let label_cls = if active {
                                "text-[9px] font-semibold text-teal-300 whitespace-nowrap"
                            } else {
                                "text-[9px] text-slate-500 group-hover:text-slate-300 whitespace-nowrap"
                            };
                            let (p_inches, p_dpi) = (profile.inches_tenths, profile.dpi);
                            rsx! {
                                button {
                                    key: "{profile.label}",
                                    class: "absolute top-0 -translate-x-1/2 flex flex-col items-center gap-0.5 cursor-pointer group",
                                    style: "left: {pct:.1}%;",
                                    title: "{profile.note}",
                                    onclick: move |_| {
                                        recompute(Some(p_inches), Some(p_dpi), None, true);
                                    },
                                    span { class: "w-px h-2 bg-slate-500 group-hover:bg-slate-300" }
                                    span { class: "{label_cls}", "{profile.label}" }
                                }
                            }
                        }
                    }
                }

                // ── Pixel density ─────────────────────────────────────
                div { class: "flex items-center justify-between text-xs text-slate-400 mt-1",
                    span { "Pixel density" }
                    span { class: "font-mono text-teal-300", "{dpi} dpi" }
                }
                input {
                    r#type: "range",
                    class: "w-full accent-teal-400 cursor-pointer",
                    min: "{DEVICE_DPI_RANGE.0}",
                    max: "{DEVICE_DPI_RANGE.1}",
                    step: "1",
                    value: "{dpi}",
                    oninput: move |evt| {
                        if let Ok(v) = evt.value().parse::<u32>() {
                            recompute(None, Some(v), None, false);
                        }
                    },
                    onchange: move |evt| {
                        if let Ok(v) = evt.value().parse::<u32>() {
                            recompute(None, Some(v), None, true);
                        }
                    },
                }
                div { class: "relative h-7 mx-1.5",
                    for (value, name) in DPI_PRESETS {
                        {
                            let value = *value;
                            let name = *name;
                            let pct = (value.saturating_sub(DEVICE_DPI_RANGE.0)) as f64 * 100.0
                                / (DEVICE_DPI_RANGE.1 - DEVICE_DPI_RANGE.0) as f64;
                            let label_cls = if dpi == value {
                                "text-[9px] font-semibold text-teal-300"
                            } else {
                                "text-[9px] text-slate-500 group-hover:text-slate-300"
                            };
                            rsx! {
                                button {
                                    key: "{value}",
                                    class: "absolute top-0 -translate-x-1/2 flex flex-col items-center gap-0.5 cursor-pointer group",
                                    style: "left: {pct:.1}%;",
                                    title: "{value} dpi",
                                    onclick: move |_| {
                                        recompute(None, Some(value), None, true);
                                    },
                                    span { class: "w-px h-2 bg-slate-500 group-hover:bg-slate-300" }
                                    span { class: "{label_cls}", "{name}" }
                                }
                            }
                        }
                    }
                }

                // ── Zoom headroom ─────────────────────────────────────
                div { class: "flex items-center justify-between text-xs text-slate-400 mt-1",
                    span {
                        title: "Extra resolution kept beyond the screen's long edge, so pinch-zoom still has pixels to show.",
                        "Zoom headroom"
                    }
                    span { class: "font-mono text-teal-300", "{headroom}%" }
                }
                div { class: "flex gap-1",
                    for (pct, name) in HEADROOM_PRESETS {
                        {
                            let pct = *pct;
                            let name = *name;
                            let cls = if headroom == pct {
                                "flex-1 text-[10px] px-1 py-1 rounded-lg border bg-teal-500/20 text-teal-300 border-teal-500/30 transition-all cursor-pointer"
                            } else {
                                "flex-1 text-[10px] px-1 py-1 rounded-lg border bg-white/5 text-slate-400 border-white/10 hover:bg-white/10 hover:text-white transition-all cursor-pointer"
                            };
                            rsx! {
                                button {
                                    key: "{pct}",
                                    class: "{cls}",
                                    title: "{pct}% of the screen's long edge",
                                    onclick: move |_| {
                                        recompute(None, None, Some(pct), true);
                                    },
                                    "{name}"
                                }
                            }
                        }
                    }
                }
                input {
                    r#type: "range",
                    class: "w-full accent-teal-400 cursor-pointer",
                    min: "{HEADROOM_PCT_RANGE.0}",
                    max: "{HEADROOM_PCT_RANGE.1}",
                    step: "10",
                    value: "{headroom}",
                    oninput: move |evt| {
                        if let Ok(v) = evt.value().parse::<u32>() {
                            recompute(None, None, Some(v), false);
                        }
                    },
                    onchange: move |evt| {
                        if let Ok(v) = evt.value().parse::<u32>() {
                            recompute(None, None, Some(v), true);
                        }
                    },
                }

                // ── The arithmetic, shown rather than hidden ──────────
                div { class: "flex flex-col gap-1 p-2.5 rounded-lg bg-black/30 border border-white/10 font-mono text-[10px] leading-relaxed",
                    div { class: "flex justify-between text-slate-400",
                        span { "{breakdown.device_label()}" }
                        span { "{breakdown.diagonal_px}px diag" }
                    }
                    div { class: "text-slate-400", "{breakdown.resolution_label()}" }
                    div { class: "text-slate-500",
                        "long edge {breakdown.long_edge_px} × {headroom}% = {breakdown.raw_clamp_px}px"
                    }
                    div { class: "flex justify-between items-center pt-1 border-t border-white/10",
                        span { class: "text-slate-400", "Clamp" }
                        span { class: "text-teal-300 font-bold text-xs", "{breakdown.clamp_px}px" }
                    }
                    if breakdown.hit_container_limit {
                        div { class: "text-amber-400",
                            i { class: "fa-solid fa-triangle-exclamation mr-1" }
                            "capped by WebP's {WEBP_MAX_DIMENSION}px limit"
                        }
                    }
                    // `--webp-max-dimension` overrides the computed value for
                    // the run. Say so rather than displaying a number the
                    // pipeline isn't using.
                    if applied_px != breakdown.clamp_px {
                        div { class: "flex justify-between items-center text-amber-400",
                            span {
                                title: "A --webp-max-dimension flag overrides the device calculation for this run. Move any slider to go back to the computed value.",
                                i { class: "fa-solid fa-triangle-exclamation mr-1" }
                                "overridden, in use"
                            }
                            span { class: "font-bold text-xs", "{applied_px}px" }
                        }
                    }
                }
            } else {
                // ── Raw pixel slider ──────────────────────────────────
                div { class: "flex items-center justify-between text-xs text-slate-400",
                    span { "Max dimension" }
                    span { class: "font-mono text-teal-300", "{webp_max_dim}px" }
                }
                input {
                    r#type: "range",
                    class: "w-full accent-teal-400 cursor-pointer",
                    min: "{clamp::CLAMP_MIN_PX}",
                    max: "{WEBP_MAX_DIMENSION}",
                    step: "1",
                    value: "{webp_max_dim}",
                    // Live-update while dragging (instant effect on the next request).
                    oninput: move |evt| {
                        if let Ok(v) = evt.value().parse::<u32>() {
                            set_clamp_px(v);
                            webp_max_dim.set(v);
                        }
                    },
                    // Log and save the committed value once the user releases the
                    // slider, instead of on every drag tick.
                    onchange: move |evt| {
                        if let Ok(v) = evt.value().parse::<u32>() {
                            app_state().emit(ServerEvent::Info(format!(
                                "Max dimension set to {v}px"
                            )));
                            save_settings();
                        }
                    },
                }
                div { class: "flex items-center justify-between",
                    span { class: "text-xs text-slate-600", "{clamp::CLAMP_MIN_PX}px" }
                    button {
                        class: "text-xs px-2 py-1 rounded-lg bg-white/5 hover:bg-white/10 text-slate-400 hover:text-white border border-white/10 transition-all cursor-pointer",
                        disabled: *webp_max_dim.read() == WEBP_MAX_DIMENSION,
                        onclick: move |_| {
                            set_clamp_px(WEBP_MAX_DIMENSION);
                            webp_max_dim.set(WEBP_MAX_DIMENSION);
                            app_state().emit(ServerEvent::Info(format!(
                                "Max dimension reset to default ({WEBP_MAX_DIMENSION}px)"
                            )));
                            save_settings();
                        },
                        i { class: "fa-solid fa-rotate-left mr-1.5 text-[10px]" }
                        "Reset to default"
                    }
                    span { class: "text-xs text-slate-600", "{WEBP_MAX_DIMENSION}px" }
                }
            }
        }
    }
}

#[component]
fn TabButton(label: String, icon: String, active: bool, onselect: EventHandler<()>) -> Element {
    let cls = if active {
        "bg-indigo-500/20 text-indigo-300"
    } else {
        "text-slate-400 hover:text-white hover:bg-white/5"
    };
    rsx! {
        button {
            class: "inline-flex items-center gap-2 text-sm font-medium px-3 py-2 mb-1 rounded-t-lg transition-colors cursor-pointer {cls}",
            onclick: move |_| onselect.call(()),
            i { class: "{icon} text-xs" }
            "{label}"
        }
    }
}

/// Per-source plugins for the URL endpoints.
///
/// Every plugin ships disabled: they rewrite image bytes, so turning one on for
/// a source that doesn't need it would be worse than useless. Each entry states
/// what the source does and which domains it covers, so the choice is informed
/// rather than a name and a switch.
#[component]
fn PluginsPanel() -> Element {
    let mut enabled =
        use_signal(|| app_state().enabled_plugins.lock().unwrap().clone());

    let enabled_now = enabled.read().clone();
    let active_count = enabled_now.len();
    let total = crate::plugins::all().len();

    rsx! {
        div { class: "flex flex-col gap-4",

            div { class: "flex items-start justify-between gap-3",
                div { class: "flex flex-col gap-1",
                    h3 {
                        class: "text-sm font-bold tracking-widest text-white/90 uppercase",
                        style: "font-family: 'JetBrains Mono', monospace;",
                        "Source Plugins"
                    }
                    p { class: "text-xs text-slate-400 max-w-2xl",
                        "Some sources ship pages the reader has to repair before they can be \
                         viewed — tiles shuffled around a grid, or bytes XORed against a key. \
                         When this server downloads the image itself that repair never happens, \
                         so the upscaler would sharpen a scrambled page. These plugins redo the \
                         work server-side."
                    }
                }
                span {
                    class: if active_count > 0 {
                        "text-xs font-semibold px-2.5 py-1 rounded-full bg-indigo-500 text-white flex-shrink-0"
                    } else {
                        "text-xs font-semibold px-2.5 py-1 rounded-full bg-white/10 text-slate-500 flex-shrink-0"
                    },
                    "{active_count} / {total} on"
                }
            }

            // Scope note — the single most important thing to know about these.
            div { class: "flex items-start gap-2 px-3 py-2 rounded-lg bg-amber-500/10 border border-amber-500/20 text-xs text-amber-200/90",
                i { class: "fa-solid fa-circle-info mt-0.5 flex-shrink-0" }
                span {
                    "Plugins run only for "
                    span { class: "font-mono text-amber-100", "single uri" }
                    " and "
                    span { class: "font-mono text-amber-100", "batch uri" }
                    " requests, and only for their own domains. Images the reader uploads \
                     directly are already decoded, so they are never touched."
                }
            }

            div { class: "flex flex-col gap-2",
                for plugin in crate::plugins::all() {
                    {
                        let id = plugin.id;
                        let on = enabled_now.contains(id);
                        let (card_cls, pill_cls, pill_label) = if on {
                            (
                                "flex flex-col gap-2 p-3 rounded-xl border bg-indigo-500/10 border-indigo-500/30",
                                "text-xs font-semibold px-2 py-0.5 rounded-full bg-indigo-500 text-white flex-shrink-0",
                                "ON",
                            )
                        } else {
                            (
                                "flex flex-col gap-2 p-3 rounded-xl border bg-white/5 border-white/10",
                                "text-xs font-semibold px-2 py-0.5 rounded-full bg-white/10 text-slate-500 flex-shrink-0",
                                "OFF",
                            )
                        };
                        rsx! {
                            div { key: "{id}", class: "{card_cls}",
                                button {
                                    class: "flex items-start justify-between gap-3 text-left cursor-pointer",
                                    onclick: move |_| {
                                        let state = app_state();
                                        let now_on = {
                                            let mut set = state.enabled_plugins.lock().unwrap();
                                            if set.contains(id) {
                                                set.remove(id);
                                                false
                                            } else {
                                                set.insert(id.to_string());
                                                true
                                            }
                                        };
                                        enabled.set(state.enabled_plugins.lock().unwrap().clone());
                                        state.emit(ServerEvent::Info(format!(
                                            "Plugin '{id}' {}",
                                            if now_on { "enabled" } else { "disabled" }
                                        )));
                                        save_settings();
                                    },
                                    span { class: "flex flex-col gap-1 min-w-0",
                                        span { class: "text-sm font-semibold text-white", "{plugin.name}" }
                                        span { class: "text-xs text-slate-400 leading-relaxed", "{plugin.description}" }
                                    }
                                    span { class: "{pill_cls}", "{pill_label}" }
                                }
                                div { class: "flex items-start gap-2 text-[11px] text-slate-500",
                                    i { class: "fa-solid fa-globe mt-0.5 flex-shrink-0" }
                                    span { class: "min-w-0", "{plugin.sources}" }
                                }
                                div { class: "flex flex-wrap gap-1",
                                    for host in plugin.hosts {
                                        span {
                                            key: "{host}",
                                            class: "text-[10px] font-mono px-1.5 py-0.5 rounded bg-black/30 text-slate-500 border border-white/5",
                                            "{host}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A single summary tile in the Storage panel (session / stale / total).
#[component]
fn StatCard(label: String, value: String, sub: String, accent: String, loading: bool) -> Element {
    rsx! {
        div { class: "flex-1 min-w-0 bg-white/5 border border-white/10 rounded-xl p-4",
            p { class: "text-xs font-semibold uppercase tracking-wider text-slate-500 mb-1", "{label}" }
            if loading {
                p {
                    class: "text-2xl font-bold text-slate-500",
                    style: "font-family: 'JetBrains Mono', monospace;",
                    i { class: "fa-solid fa-spinner fa-spin text-lg" }
                }
            } else {
                p {
                    class: "text-2xl font-bold {accent}",
                    style: "font-family: 'JetBrains Mono', monospace;",
                    "{value}"
                }
            }
            if !sub.is_empty() {
                p { class: "text-xs text-slate-400 mt-1 truncate", title: "{sub}", "{sub}" }
            }
        }
    }
}

/// Disk-usage overview and cache-clearing actions.
///
/// `requests` and `compare` are threaded in so that clearing the session
/// history can also wipe the visible request table and dismiss any open
/// comparison modal (both reference PNGs that get deleted).
#[component]
fn StoragePanel(
    requests: Signal<Vec<RequestRow>>,
    compare: Signal<Option<RequestRecord>>,
) -> Element {
    let report = use_signal(|| None as Option<CacheReport>);
    let scanning = use_signal(|| false);
    // True while a destructive operation is running.
    let mut busy = use_signal(|| false);
    // Which action is awaiting its second (confirming) click, if any.
    let mut confirming = use_signal(|| None as Option<&'static str>);
    // Session-history byte budget, in MB (mirrors RequestHistory::max_bytes).
    let mut limit_mb =
        use_signal(|| app_state().history.lock().unwrap().max_bytes() / (1024 * 1024));

    // Scan once when the panel mounts (it only mounts while the tab is active).
    use_future(move || scan_once(report, scanning));

    let rep = report.read().clone();
    let scanning_now = *scanning.read();
    let busy_now = *busy.read();
    let locked = scanning_now || busy_now;
    let loading = scanning_now && rep.is_none();
    let confirming_now = *confirming.read();

    // Derive display strings for the three summary cards.
    let (session_val, session_sub, stale_val, stale_sub, total_val, stale_accent) = match &rep {
        Some(r) => (
            format_bytes(r.session_bytes),
            format!("{} files", r.session_files),
            format_bytes(r.stale_bytes),
            format!("{} dirs · {} files", r.stale_dirs, r.stale_files),
            format_bytes(r.total_bytes()),
            if r.stale_bytes > 0 { "text-amber-300" } else { "text-white" },
        ),
        None => (
            "—".to_string(),
            String::new(),
            "—".to_string(),
            String::new(),
            "—".to_string(),
            "text-white",
        ),
    };

    let session_confirm = confirming_now == Some("session");
    let (session_btn_cls, session_icon, session_text) = if session_confirm {
        (
            "bg-red-500/20 text-red-300 border-red-500/40 hover:bg-red-500/30",
            "fa-solid fa-triangle-exclamation",
            "Confirm — clear history?",
        )
    } else {
        (
            "bg-white/5 text-slate-300 border-white/10 hover:bg-white/10 hover:text-white",
            "fa-solid fa-trash-can",
            "Clear session history",
        )
    };

    // Auto-clean limit derivations.
    let limit_now = *limit_mb.read();
    let limit_label = format_bytes(limit_now * 1024 * 1024);
    let chapters_est =
        ((limit_now + STORAGE_MB_PER_CHAPTER / 2) / STORAGE_MB_PER_CHAPTER).max(1);
    let usage_pct = match &rep {
        Some(r) if limit_now > 0 => {
            (r.session_bytes as f64 * 100.0 / (limit_now * 1024 * 1024) as f64).min(100.0)
        }
        _ => 0.0,
    };
    let usage_bar_cls = if usage_pct >= 90.0 {
        "bg-red-400"
    } else if usage_pct >= 70.0 {
        "bg-amber-400"
    } else {
        "bg-indigo-400"
    };

    let stale_confirm = confirming_now == Some("stale");
    let (stale_btn_cls, stale_icon, stale_text) = if stale_confirm {
        (
            "bg-red-500/20 text-red-300 border-red-500/40 hover:bg-red-500/30",
            "fa-solid fa-triangle-exclamation",
            "Confirm — delete stale cache?",
        )
    } else {
        (
            "bg-white/5 text-slate-300 border-white/10 hover:bg-white/10 hover:text-white",
            "fa-solid fa-broom",
            "Clean stale cache",
        )
    };

    rsx! {
        div { class: "flex flex-col gap-4",

            // Header + Refresh
            div { class: "flex items-center justify-between",
                h3 {
                    class: "text-sm font-bold tracking-widest text-white/90 uppercase",
                    style: "font-family: 'JetBrains Mono', monospace;",
                    "Disk Usage"
                }
                button {
                    class: "inline-flex items-center gap-2 text-xs px-3 py-1.5 rounded-lg bg-white/5 hover:bg-white/10 text-slate-300 hover:text-white border border-white/10 transition-all cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed",
                    disabled: locked,
                    onclick: move |_| {
                        confirming.set(None);
                        trigger_scan(report, scanning);
                    },
                    i {
                        class: if scanning_now { "fa-solid fa-rotate fa-spin text-xs" } else { "fa-solid fa-rotate text-xs" }
                    }
                    "Refresh"
                }
            }

            // Summary cards
            div { class: "flex gap-3",
                StatCard {
                    label: "Session history",
                    value: session_val,
                    sub: session_sub,
                    accent: "text-white",
                    loading,
                }
                StatCard {
                    label: "Stale cache",
                    value: stale_val,
                    sub: stale_sub,
                    accent: stale_accent,
                    loading,
                }
                StatCard {
                    label: "Total",
                    value: total_val,
                    sub: String::new(),
                    accent: "text-indigo-300",
                    loading,
                }
            }

            div { class: "h-px bg-white/10" }

            // ── Auto-clean limit ──────────────────────────────────────
            div { class: "flex flex-col gap-2",
                div { class: "flex items-center justify-between gap-2",
                    p { class: "text-xs text-slate-400",
                        i { class: "fa-solid fa-scale-balanced mr-2 text-slate-500" }
                        "Auto-clean limit — oldest captures are deleted automatically once the session history exceeds it."
                    }
                    span { class: "text-xs font-mono text-indigo-300 flex-shrink-0",
                        "{limit_label} · ≈{chapters_est} chapters"
                    }
                }

                // Usage vs limit bar.
                div {
                    class: "h-1.5 rounded-full bg-white/5 overflow-hidden",
                    title: "Session history usage vs limit",
                    div {
                        class: "h-full rounded-full transition-all duration-300 {usage_bar_cls}",
                        style: "width: {usage_pct:.1}%;",
                    }
                }

                input {
                    r#type: "range",
                    class: "w-full accent-indigo-400 cursor-pointer",
                    min: "{STORAGE_LIMIT_MIN_MB}",
                    max: "{STORAGE_LIMIT_MAX_MB}",
                    step: "{STORAGE_LIMIT_STEP_MB}",
                    value: "{limit_now}",
                    disabled: busy_now,
                    // Live display while dragging; the budget applies on release.
                    oninput: move |evt| {
                        if let Ok(v) = evt.value().parse::<u64>() {
                            limit_mb.set(v);
                        }
                    },
                    onchange: move |evt| {
                        if let Ok(v) = evt.value().parse::<u64>() {
                            limit_mb.set(v);
                            spawn(async move { apply_history_limit(v, report, scanning).await });
                        }
                    },
                }

                // Notched presets under the slider, clickable to snap.
                div { class: "relative h-7 mx-1.5",
                    for (mb, name) in STORAGE_LIMIT_PRESETS {
                        {
                            let mb = *mb;
                            let name = *name;
                            let pct = (mb - STORAGE_LIMIT_MIN_MB) as f64 * 100.0
                                / (STORAGE_LIMIT_MAX_MB - STORAGE_LIMIT_MIN_MB) as f64;
                            let active = limit_now == mb;
                            let label_cls = if active {
                                "text-[10px] font-semibold text-indigo-300"
                            } else {
                                "text-[10px] text-slate-500 group-hover:text-slate-300"
                            };
                            let size_label = format_bytes(mb * 1024 * 1024);
                            rsx! {
                                button {
                                    key: "{mb}",
                                    class: "absolute top-0 -translate-x-1/2 flex flex-col items-center gap-0.5 cursor-pointer group",
                                    style: "left: {pct:.1}%;",
                                    title: "{size_label}",
                                    disabled: busy_now,
                                    onclick: move |_| {
                                        limit_mb.set(mb);
                                        spawn(async move { apply_history_limit(mb, report, scanning).await });
                                    },
                                    span { class: "w-px h-2 bg-slate-500 group-hover:bg-slate-300" }
                                    span { class: "{label_cls}", "{name}" }
                                }
                            }
                        }
                    }
                }
            }

            div { class: "h-px bg-white/10" }

            // ── Clear session history ─────────────────────────────────
            div { class: "flex flex-col gap-1.5",
                p { class: "text-xs text-slate-400",
                    i { class: "fa-solid fa-clock-rotate-left mr-2 text-slate-500" }
                    "Before/after PNGs kept this session — they back the per-request comparison view."
                }
                button {
                    class: "w-full flex items-center justify-center gap-2 text-sm px-3 py-2 rounded-lg border transition-all duration-150 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed {session_btn_cls}",
                    disabled: locked,
                    onclick: move |_| {
                        if *confirming.read() == Some("session") {
                            confirming.set(None);
                            busy.set(true);
                            spawn(async move {
                                let res = tokio::task::spawn_blocking(|| {
                                    app_state().history.lock().unwrap().clear_session()
                                })
                                .await
                                .ok();
                                if let Some((bytes, _files)) = res {
                                    requests.write().clear();
                                    compare.set(None);
                                    app_state().emit(ServerEvent::Info(format!(
                                        "Session history cleared — {} freed",
                                        format_bytes(bytes)
                                    )));
                                }
                                busy.set(false);
                                scan_once(report, scanning).await;
                            });
                        } else {
                            confirming.set(Some("session"));
                        }
                    },
                    i { class: "{session_icon} text-xs" }
                    "{session_text}"
                }
            }

            // ── Clean stale cache ─────────────────────────────────────
            div { class: "flex flex-col gap-1.5",
                p { class: "text-xs text-slate-400",
                    i { class: "fa-solid fa-ghost mr-2 text-slate-500" }
                    "Orphaned temp folders left in %TEMP% by previous sessions — safe to delete."
                }
                button {
                    class: "w-full flex items-center justify-center gap-2 text-sm px-3 py-2 rounded-lg border transition-all duration-150 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed {stale_btn_cls}",
                    disabled: locked,
                    onclick: move |_| {
                        if *confirming.read() == Some("stale") {
                            confirming.set(None);
                            busy.set(true);
                            spawn(async move {
                                let dir = app_state().history.lock().unwrap().current_dir();
                                let res = tokio::task::spawn_blocking(move || {
                                    clear_stale_cache(dir.as_deref())
                                })
                                .await
                                .ok();
                                if let Some((bytes, dirs, errors)) = res {
                                    app_state().emit(ServerEvent::Info(format!(
                                        "Stale cache cleaned — {} freed, {} dirs removed",
                                        format_bytes(bytes),
                                        dirs
                                    )));
                                    if !errors.is_empty() {
                                        let summary = errors
                                            .iter()
                                            .take(3)
                                            .cloned()
                                            .collect::<Vec<_>>()
                                            .join("; ");
                                        app_state().emit(ServerEvent::Error(format!(
                                            "Stale cache: {} error(s) — {}",
                                            errors.len(),
                                            summary
                                        )));
                                    }
                                }
                                busy.set(false);
                                scan_once(report, scanning).await;
                            });
                        } else {
                            confirming.set(Some("stale"));
                        }
                    },
                    i { class: "{stale_icon} text-xs" }
                    "{stale_text}"
                }
            }
        }
    }
}

#[component]
fn RequestsTable(rows: Signal<Vec<RequestRow>>, compare: Signal<Option<RequestRecord>>) -> Element {
    let data = rows.read();
    if data.is_empty() {
        return rsx! {
            div { class: "h-full flex flex-col items-center justify-center text-slate-600 gap-3",
                i { class: "fa-solid fa-inbox text-3xl" }
                p { class: "text-sm", "No upscale requests yet — waiting for the reader to connect." }
            }
        };
    }
    rsx! {
        table { class: "w-full text-sm border-collapse",
            thead {
                tr { class: "text-left text-xs uppercase tracking-wider text-slate-500",
                    th { class: "py-2 px-2 font-semibold", "Time" }
                    th { class: "py-2 px-2 font-semibold", "Source" }
                    th { class: "py-2 px-2 font-semibold", "Batch" }
                    th { class: "py-2 px-2 font-semibold", "Model" }
                    th { class: "py-2 px-2 font-semibold", "Scale" }
                    th { class: "py-2 px-2 font-semibold", "In" }
                    th { class: "py-2 px-2 font-semibold", "Out" }
                    th { class: "py-2 px-2 font-semibold", "Upscale" }
                    th { class: "py-2 px-2 font-semibold", "Cmp ms" }
                    th { class: "py-2 px-2 font-semibold", "WebP ms" }
                }
            }
            tbody {
                for (i, row) in data.iter().enumerate() {
                    {
                        let zebra = if i % 2 == 0 { "bg-white/[0.02]" } else { "" };
                        let id = row.id;
                        let cmp_cell = match row.compress_ms {
                            Some(ms) => format!("{ms}"),
                            None => "—".to_string(),
                        };
                        let cmp_class = if row.compress_ms.is_some() {
                            "py-1.5 px-2 text-green-400 font-mono"
                        } else {
                            "py-1.5 px-2 text-slate-600 font-mono"
                        };
                        let webp_cell = match row.webp_ms {
                            Some(ms) => format!("{ms}"),
                            None => "-".to_string(),
                        };
                        let webp_class = if row.webp_ms.is_some() {
                            "py-1.5 px-2 text-teal-300 font-mono"
                        } else {
                            "py-1.5 px-2 text-slate-600 font-mono"
                        };
                        let batch_cell = match row.batch_id {
                            Some(b) => format!("#{b}"),
                            None => "—".to_string(),
                        };
                        let batch_class = if row.batch_id.is_some() {
                            "py-1.5 px-2 text-amber-300 font-mono"
                        } else {
                            "py-1.5 px-2 text-slate-600 font-mono"
                        };
                        let is_uri = row.source.contains("uri");
                        let source_class = if is_uri {
                            "py-1.5 px-2 text-cyan-300"
                        } else {
                            "py-1.5 px-2 text-slate-300"
                        };
                        rsx! {
                            tr {
                                key: "{row.id}",
                                class: "cursor-pointer hover:bg-indigo-500/10 transition-colors {zebra}",
                                title: "Open before/after comparison",
                                onclick: move |_| {
                                    if let Some(rec) = app_state().history.lock().unwrap().get(id) {
                                        compare.set(Some(rec));
                                    }
                                },
                                td { class: "py-1.5 px-2 text-slate-400 font-mono", "{row.time}" }
                                td { class: "{source_class}", "{row.source}" }
                                td { class: "{batch_class}", "{batch_cell}" }
                                td { class: "py-1.5 px-2 text-white", "{row.model}" }
                                td { class: "py-1.5 px-2 text-slate-300", "{row.scale}x" }
                                td { class: "py-1.5 px-2 text-slate-300", "{row.in_kb}KB" }
                                td { class: "py-1.5 px-2 text-slate-300", "{row.out_kb}KB" }
                                td { class: "py-1.5 px-2 text-indigo-300 font-mono", "{row.ms}" }
                                td { class: "{cmp_class}", "{cmp_cell}" }
                                td { class: "{webp_class}", "{webp_cell}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn LogView(lines: Signal<Vec<LogLine>>, color: String, empty: String) -> Element {
    let data = lines.read();
    if data.is_empty() {
        return rsx! {
            div { class: "h-full flex items-center justify-center text-slate-600 text-sm", "{empty}" }
        };
    }
    rsx! {
        div { class: "flex flex-col gap-0.5 font-mono text-xs",
            for (i, line) in data.iter().enumerate() {
                div { key: "{i}", class: "flex gap-3 py-0.5",
                    span { class: "text-slate-600 flex-shrink-0", "{line.time}" }
                    span { class: "{color}", "{line.text}" }
                }
            }
        }
    }
}

#[component]
fn CompareModal(record: RequestRecord, on_close: EventHandler<()>) -> Element {
    let r = record.clone();
    let sbs_rec = record.clone();
    let before_rec = record.clone();
    let after_rec = record.clone();

    rsx! {
        div {
            class: "fixed inset-0 bg-black/80 z-50 flex items-center justify-center",
            onclick: move |_| on_close.call(()),

            div {
                class: "bg-neutral-900 border border-white/10 rounded-xl shadow-2xl w-[480px] max-w-[90%] mx-4 p-6",
                onclick: move |evt| evt.stop_propagation(),

                div { class: "flex items-center justify-between mb-4",
                    h2 { class: "text-lg font-semibold text-white",
                        "Request #{r.id} — {r.model} {r.scale}x noise={r.noise}"
                    }
                    button {
                        class: "w-8 h-8 flex items-center justify-center rounded-lg hover:bg-white/10 text-slate-500 hover:text-white transition-colors cursor-pointer",
                        onclick: move |_| on_close.call(()),
                        i { class: "fa-solid fa-xmark" }
                    }
                }

                div { class: "flex flex-col gap-1 text-sm text-slate-300 mb-5",
                    p { "Before: {r.width_in}x{r.height_in}  ({r.size_in / 1024} KB)" }
                    p {
                        "After:  {r.width_out}x{r.height_out}  ({r.size_out / 1024} KB)  ~{r.scale_factor():.2}x"
                    }
                    p { class: "text-slate-400", "Processed in {r.time_ms} ms" }
                    if let Some(cms) = r.compress_ms {
                        p { class: "text-green-400",
                            i { class: "fa-solid fa-compress mr-1.5 text-xs" }
                            "Compressed in {cms} ms"
                        }
                    }
                    if let Some(url) = r.origin_url.clone() {
                        p {
                            class: "text-slate-500 text-xs break-all",
                            title: "{url}",
                            i { class: "fa-solid fa-link mr-1.5" }
                            "{url}"
                        }
                    }
                }

                div { class: "flex gap-3",
                    button {
                        class: "flex-1 inline-flex items-center justify-center gap-2 text-sm px-4 py-2 rounded-lg bg-indigo-500 hover:bg-indigo-400 text-white shadow-lg shadow-indigo-500/20 transition-all cursor-pointer",
                        onclick: move |_| {
                            let rec = sbs_rec.clone();
                            spawn(async move {
                                let rec2 = rec.clone();
                                let built = tokio::task::spawn_blocking(move || build_comparison(&rec2))
                                    .await
                                    .ok()
                                    .flatten();
                                match built {
                                    Some(path) => {
                                        app_state().history.lock().unwrap().set_compare_path(rec.id, path.clone());
                                        open_in_viewer(&path);
                                    }
                                    None => app_state()
                                        .emit(ServerEvent::Error("Could not build comparison image".to_string())),
                                }
                            });
                        },
                        i { class: "fa-solid fa-images text-xs" }
                        "Side-by-side"
                    }
                    button {
                        class: "inline-flex items-center justify-center gap-2 text-sm px-4 py-2 rounded-lg bg-white/10 hover:bg-white/15 text-white border border-white/10 transition-all cursor-pointer",
                        onclick: move |_| open_in_viewer(&before_rec.input_path),
                        "Before"
                    }
                    button {
                        class: "inline-flex items-center justify-center gap-2 text-sm px-4 py-2 rounded-lg bg-white/10 hover:bg-white/15 text-white border border-white/10 transition-all cursor-pointer",
                        onclick: move |_| open_in_viewer(&after_rec.output_path),
                        "After"
                    }
                    if let Some(url) = record.origin_url.clone() {
                        button {
                            class: "inline-flex items-center justify-center gap-2 text-sm px-4 py-2 rounded-lg bg-white/10 hover:bg-white/15 text-white border border-white/10 transition-all cursor-pointer",
                            title: "Open the source image URL the server downloaded",
                            onclick: move |_| open_url(&url),
                            i { class: "fa-solid fa-up-right-from-square text-xs" }
                            "Origin"
                        }
                    }
                }
            }
        }
    }
}
