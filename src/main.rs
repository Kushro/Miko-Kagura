// Miko-Kagura — Rust/Dioxus remote upscaling server compatible with KomikkUP.
//
// The HTTP server (axum) runs on a dedicated background thread with its own tokio
// runtime; the Dioxus desktop window owns the main thread and renders the
// dashboard. They share an `Arc<AppState>` and communicate UI updates over a
// tokio broadcast channel.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicU64};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dioxus::desktop::tao::dpi::LogicalSize;
use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;
use tokio::sync::broadcast;

mod cache;
mod clamp;
mod config;
mod history;
mod plugins;
mod server;
mod settings;
mod ui;
mod upscaler;

use config::ServerConfig;
use history::RequestHistory;
use server::{AppState, Stats};
use upscaler::Upscaler;

pub const MAIN_CSS: Asset = asset!("../assets/main.css");
pub const THEME_CSS: Asset = asset!("../assets/themes.css");
pub const TAILWIND_CSS: Asset = asset!("../assets/tailwind.css");

/// Global handle to the shared application state, set once before the UI launches.
static APP_STATE: std::sync::OnceLock<Arc<AppState>> = std::sync::OnceLock::new();

/// Access the shared application state from anywhere in the UI.
pub fn app_state() -> Arc<AppState> {
    APP_STATE
        .get()
        .expect("AppState not initialized")
        .clone()
}

fn main() {
    // Saved settings are the baseline; CLI flags then override them for this
    // run only, so `--port 9000` never rewrites the user's stored preferences.
    let (saved, settings_warning) = settings::load();
    let persisted = saved.unwrap_or_default();
    settings::set_baseline(persisted.clone());
    let cfg = parse_args_over(persisted.clone());

    // Build and prime the upscaler: detect the GPU once, load the default model.
    let mut upscaler = Upscaler::new(&cfg.binary_dir, cfg.gpu_id);
    upscaler.detect_gpu();
    let load_error = upscaler
        .load_model(&cfg.default_model, cfg.default_scale, cfg.default_noise)
        .err();
    // `load_model` only validates the model config (scale/noise) — it doesn't
    // check the CLI binary is actually on disk. Surface that separately so a
    // moved/missing `binaries/` folder is visible at startup, not just as a
    // 500 on the first upscale request.
    let binary_warning = upscaler.missing_binary_hint();

    let (events_tx, _events_rx) = broadcast::channel(256);
    let server_addr = format!("{}:{}", cfg.host, cfg.port);

    // HTTP client for the URL endpoints. A browser-like User-Agent gives a
    // slightly better chance against trivial hotlink/anti-bot checks.
    let http_client = reqwest::Client::builder()
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
        )
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    let mut history = RequestHistory::default();
    history.set_max_bytes(persisted.history_max_bytes);

    // Drop plugin ids that no longer exist in the registry, so an old settings
    // file can't silently "enable" nothing.
    let enabled_plugins: HashSet<String> = persisted
        .enabled_plugins
        .iter()
        .filter(|id| plugins::get(id).is_some())
        .cloned()
        .collect();

    let state = Arc::new(AppState {
        upscaler: Mutex::new(upscaler),
        history: Mutex::new(history),
        stats: Mutex::new(Stats::default()),
        events: events_tx,
        start_time: Instant::now(),
        server_addr: server_addr.clone(),
        compress_enabled: AtomicBool::new(cfg.compress_enabled),
        compress_level: AtomicU8::new(cfg.compress_level),
        normalize_png: AtomicBool::new(cfg.normalize_png),
        webp_compat: AtomicBool::new(cfg.webp_compat),
        webp_max_dimension: AtomicU32::new(cfg.webp_max_dimension),
        clamp_device_mode: AtomicBool::new(persisted.clamp_device_mode),
        device_inches_tenths: AtomicU32::new(persisted.device_inches_tenths),
        device_dpi: AtomicU32::new(persisted.device_dpi),
        clamp_headroom_pct: AtomicU32::new(persisted.clamp_headroom_pct),
        http_client,
        batch_counter: AtomicU64::new(1),
        enabled_plugins: Mutex::new(enabled_plugins),
        binary_dir_override: Mutex::new(
            Some(persisted.binary_dir.clone()).filter(|s| !s.is_empty()),
        ),
        theme: Mutex::new(persisted.theme.clone()),
    });

    // Surface a startup model-load failure in the dashboard once it subscribes.
    if let Some(err) = load_error {
        state.emit(server::ServerEvent::Error(format!(
            "Failed to load default model: {err}"
        )));
    }
    if let Some(warning) = binary_warning {
        state.emit(server::ServerEvent::Error(warning));
    }
    if let Some(warning) = settings_warning {
        state.emit(server::ServerEvent::Error(warning));
    }
    {
        let count = state.enabled_plugins.lock().unwrap().len();
        if count > 0 {
            state.emit(server::ServerEvent::Info(format!(
                "{count} source plugin(s) enabled for URL requests"
            )));
        }
    }

    let _ = APP_STATE.set(state.clone());

    // Start the HTTP server on its own thread/runtime.
    server::spawn_server(state, cfg.host.clone(), cfg.port, cfg.max_size_mb);

    // Launch the desktop dashboard (frameless window with a custom title bar).
    let window = WindowBuilder::new()
        .with_title("Miko-Kagura")
        .with_resizable(true)
        .with_decorations(false)
        .with_inner_size(LogicalSize::new(1120.0, 720.0))
        .with_min_inner_size(LogicalSize::new(820.0, 520.0));

    let desktop_config = Config::new().with_window(window);

    dioxus::LaunchBuilder::desktop()
        .with_cfg(desktop_config)
        .launch(ui::App);
}

/// Minimal command-line argument parsing (mirrors the Python CLI flags),
/// layered over whatever was saved from the last session.
fn parse_args_over(saved: settings::PersistedSettings) -> ServerConfig {
    let mut cfg = ServerConfig::default();
    saved.apply_to(&mut cfg);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].clone();
        let value = args.get(i + 1).cloned();
        let mut consumed_value = true;
        match arg.as_str() {
            "--host" => {
                if let Some(v) = value {
                    cfg.host = v;
                }
            }
            "--port" => {
                if let Some(p) = value.and_then(|v| v.parse().ok()) {
                    cfg.port = p;
                }
            }
            "--model" => {
                if let Some(v) = value {
                    cfg.default_model = v;
                }
            }
            "--scale" | "-s" => {
                if let Some(p) = value.and_then(|v| v.parse().ok()) {
                    cfg.default_scale = p;
                }
            }
            "--noise" | "-n" => {
                if let Some(p) = value.and_then(|v| v.parse().ok()) {
                    cfg.default_noise = p;
                }
            }
            "--binary-dir" => {
                if let Some(v) = value {
                    cfg.binary_dir = v;
                }
            }
            "--max-size" => {
                if let Some(p) = value.and_then(|v| v.parse().ok()) {
                    cfg.max_size_mb = p;
                }
            }
            "--gpu-id" => {
                if let Some(p) = value.and_then(|v| v.parse().ok()) {
                    cfg.gpu_id = p;
                }
            }
            "--compress" | "-C" => {
                cfg.compress_enabled = true;
                consumed_value = false; // flag, no value
            }
            "--compress-level" => {
                if let Some(p) = value.and_then(|v| v.parse().ok()) {
                    cfg.compress_level = p;
                }
            }
            "--no-normalize" => {
                cfg.normalize_png = false;
                consumed_value = false; // flag, no value
            }
            "--no-webp-compat" => {
                cfg.webp_compat = false;
                consumed_value = false; // flag, no value
            }
            "--webp-max-dimension" => {
                if let Some(p) = value.and_then(|v| v.parse().ok()) {
                    cfg.webp_max_dimension = p;
                }
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => {
                consumed_value = false;
            }
        }
        i += if consumed_value { 2 } else { 1 };
    }
    cfg
}

fn print_help() {
    println!(
        "Miko-Kagura — remote image upscaler\n\n\
         Usage: miko-kagura [OPTIONS]\n\n\
         Options:\n\
         \x20 --host <ADDR>        Bind address (default: 0.0.0.0)\n\
         \x20 --port <PORT>        Bind port (default: 8282)\n\
         \x20 --model <NAME>       Default model (default: realcugan-se)\n\
         \x20 --scale, -s <N>      Upscale factor (default: 2)\n\
         \x20 --noise, -n <N>      Denoise level 0-4 (default: 0)\n\
         \x20 --binary-dir <DIR>   Path to ncnn-vulkan binaries (default: auto-detected\n\
         \x20                      'binaries' folder next to the executable or cwd;\n\
         \x20                      also changeable from the dashboard's Settings tab)\n\
         \x20 --max-size <MB>      Max request body size in MB (default: 20)\n\
         \x20 --gpu-id <N>         GPU device index (-1 = auto)\n\
         \x20 --no-normalize       Disable PNG normalisation (send raw NCNN output)\n\
         \x20 --no-webp-compat     Disable WebP output clamping\n\
         \x20 --webp-max-dimension <N>  Per-dimension clamp in px (default: 16383)\n\n\
         Models: waifu2x, waifu2x-upconv7, waifu2x-photo,\n\
         \x20       realcugan-se, realcugan-pro, realcugan-nose,\n\
         \x20       realesrgan-anime, realesrgan-photo\n\n\
         Dashboard settings (model, clamp, compression, plugins, theme) are saved\n\
         automatically to:\n\
         \x20 {}\n\
         and reloaded at startup. The flags above override the saved values for\n\
         one run without changing the file.",
        settings::settings_path().display()
    );
}
