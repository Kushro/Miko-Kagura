//! Persistence of dashboard settings.
//!
//! Every adjustment made in the UI (model, compression, clamping, theme, …) is
//! written to a small JSON file immediately, so nothing is lost however the app
//! exits — including a crash or task-kill. On startup the file becomes the new
//! baseline; CLI flags still override individual fields for that run.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::server::AppState;

/// What was on disk when the app started.
///
/// Used as the fallback for values that can't be read back out of the running
/// state — notably when the model failed to load, which leaves the upscaler
/// holding constructor defaults rather than the user's choice.
static BASELINE: OnceLock<PersistedSettings> = OnceLock::new();

/// Record the settings the app started from. Called once, before the UI runs.
pub fn set_baseline(settings: PersistedSettings) {
    let _ = BASELINE.set(settings);
}

fn baseline() -> PersistedSettings {
    BASELINE.get().cloned().unwrap_or_default()
}

/// Device-mode defaults: a 6.7″ phone at xxhdpi.
pub const DEFAULT_DEVICE_INCHES_TENTHS: u32 = 67;
pub const DEFAULT_DEVICE_DPI: u32 = 480;

/// Valid ranges for the device-mode inputs (inches stored as tenths).
pub const DEVICE_INCHES_TENTHS_RANGE: (u32, u32) = (30, 200);
pub const DEVICE_DPI_RANGE: (u32, u32) = (120, 680);

/// Everything the dashboard can change, in the shape it is saved to disk.
/// `#[serde(default)]` keeps old settings files loadable as fields are added.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct PersistedSettings {
    pub model: String,
    pub scale: i32,
    pub noise: i32,
    pub compress_enabled: bool,
    pub compress_level: u8,
    pub normalize_png: bool,
    pub webp_compat: bool,
    pub webp_max_dimension: u32,
    /// Whether the clamp selector is in Device (inches × DPI) mode.
    pub clamp_device_mode: bool,
    /// Screen diagonal in tenths of an inch (67 = 6.7″).
    pub device_inches_tenths: u32,
    pub device_dpi: u32,
    /// Zoom headroom applied to the screen's long edge, in percent.
    pub clamp_headroom_pct: u32,
    /// Binaries dir explicitly applied by the user; "" keeps auto-detection.
    pub binary_dir: String,
    pub theme: String,
    pub history_max_bytes: u64,
    /// Ids of enabled source plugins (URL endpoints only).
    pub enabled_plugins: Vec<String>,
}

impl Default for PersistedSettings {
    fn default() -> Self {
        let cfg = crate::config::ServerConfig::default();
        Self {
            model: cfg.default_model,
            scale: cfg.default_scale,
            noise: cfg.default_noise,
            compress_enabled: cfg.compress_enabled,
            compress_level: cfg.compress_level,
            normalize_png: cfg.normalize_png,
            webp_compat: cfg.webp_compat,
            webp_max_dimension: cfg.webp_max_dimension,
            clamp_device_mode: false,
            device_inches_tenths: DEFAULT_DEVICE_INCHES_TENTHS,
            device_dpi: DEFAULT_DEVICE_DPI,
            clamp_headroom_pct: crate::clamp::DEFAULT_HEADROOM_PCT,
            binary_dir: String::new(),
            theme: "dark".to_string(),
            history_max_bytes: crate::history::DEFAULT_MAX_BYTES,
            enabled_plugins: Vec::new(),
        }
    }
}

impl PersistedSettings {
    /// Fold the saved settings into the startup config. CLI flags are parsed
    /// afterwards, so they still win for a single run without being persisted.
    pub fn apply_to(&self, cfg: &mut crate::config::ServerConfig) {
        if !self.model.is_empty() {
            cfg.default_model = self.model.clone();
        }
        cfg.default_scale = self.scale;
        cfg.default_noise = self.noise;
        cfg.compress_enabled = self.compress_enabled;
        cfg.compress_level = self.compress_level;
        cfg.normalize_png = self.normalize_png;
        cfg.webp_compat = self.webp_compat;
        // In device mode the pixel value is a derived quantity, so recompute it
        // from the device fields instead of trusting the stored copy — the two
        // can only disagree if the file was edited by hand, and the dashboard
        // would then display a clamp the pipeline isn't actually using.
        cfg.webp_max_dimension = if self.clamp_device_mode {
            crate::clamp::compute(
                self.device_inches_tenths,
                self.device_dpi,
                self.clamp_headroom_pct,
            )
            .clamp_px
        } else {
            self.webp_max_dimension
        };
        if !self.binary_dir.is_empty() {
            cfg.binary_dir = self.binary_dir.clone();
        }
    }
}

/// `%APPDATA%\Miko-Kagura\settings.json`, falling back to a file next to
/// the executable when APPDATA is unavailable (non-Windows / stripped env).
pub fn settings_path() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        if !appdata.is_empty() {
            return PathBuf::from(appdata)
                .join("Miko-Kagura")
                .join("settings.json");
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("miko-kagura-settings.json")))
        .unwrap_or_else(|| PathBuf::from("miko-kagura-settings.json"))
}

/// Load the saved settings, plus a warning to surface if the file existed but
/// couldn't be used — resetting every preference deserves an explanation.
pub fn load() -> (Option<PersistedSettings>, Option<String>) {
    load_from(&settings_path())
}

pub fn load_from(path: &std::path::Path) -> (Option<PersistedSettings>, Option<String>) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        // A missing file is the normal first-run case, not a problem.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (None, None),
        Err(e) => {
            return (
                None,
                Some(format!(
                    "Could not read settings from {} ({e}) — starting from defaults.",
                    path.display()
                )),
            )
        }
    };
    match serde_json::from_str(&text) {
        Ok(s) => (Some(s), None),
        Err(e) => {
            // Keep the unreadable file instead of overwriting it, so it can be
            // inspected or repaired by hand.
            let backup = path.with_extension("json.corrupt");
            let saved = std::fs::rename(path, &backup).is_ok();
            let where_it_went = if saved {
                format!(" It was kept as {}.", backup.display())
            } else {
                String::new()
            };
            (
                None,
                Some(format!(
                    "Settings file at {} is not valid JSON ({e}) — starting from \
                     defaults.{where_it_went}",
                    path.display()
                )),
            )
        }
    }
}

fn save_to(path: &std::path::Path, settings: &PersistedSettings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?;
    // Write-then-rename so an interrupted write can't truncate the file that is
    // already there. The temp name carries the process id because two instances
    // of the server share this path and would otherwise clobber each other's
    // half-written file. `fs::rename` replaces an existing destination on
    // Windows as well as Unix.
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, json)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Snapshot the current runtime state into a serializable settings struct.
pub fn capture(state: &AppState) -> PersistedSettings {
    let baseline = baseline();

    // A model that failed to load leaves the upscaler holding its constructor
    // defaults, not the user's saved choice. Writing those back would quietly
    // destroy the stored model/scale/noise on the first save of the session, so
    // fall back to what was on disk instead.
    let (model, scale, noise) = {
        let up = state.upscaler.lock().unwrap();
        if up.current_config.is_some() {
            (
                up.current_model_name.clone(),
                up.current_scale,
                up.current_noise,
            )
        } else {
            (baseline.model.clone(), baseline.scale, baseline.noise)
        }
    };
    let mut plugins: Vec<String> = state
        .enabled_plugins
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect();
    plugins.sort();
    PersistedSettings {
        model: if model.is_empty() { baseline.model } else { model },
        scale,
        noise,
        compress_enabled: state.compress_enabled.load(Ordering::Relaxed),
        compress_level: state.compress_level.load(Ordering::Relaxed),
        normalize_png: state.normalize_png.load(Ordering::Relaxed),
        webp_compat: state.webp_compat.load(Ordering::Relaxed),
        webp_max_dimension: state.webp_max_dimension.load(Ordering::Relaxed),
        clamp_device_mode: state.clamp_device_mode.load(Ordering::Relaxed),
        device_inches_tenths: state.device_inches_tenths.load(Ordering::Relaxed),
        device_dpi: state.device_dpi.load(Ordering::Relaxed),
        clamp_headroom_pct: state.clamp_headroom_pct.load(Ordering::Relaxed),
        binary_dir: state
            .binary_dir_override
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default(),
        theme: state.theme.lock().unwrap().clone(),
        history_max_bytes: state.history.lock().unwrap().max_bytes(),
        enabled_plugins: plugins,
    }
}

/// Capture + save; failures land in the Errors tab rather than being silent.
pub fn persist(state: &AppState) {
    let snapshot = capture(state);
    if let Err(e) = save_to(&settings_path(), &snapshot) {
        state.emit(crate::server::ServerEvent::Error(format!(
            "Failed to save settings to {}: {e}",
            settings_path().display()
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scratch file inside the OS temp dir — never a real user directory.
    fn temp_settings_path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "miko-kagura-settings-test-{}-{tag}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
    }

    fn customised() -> PersistedSettings {
        PersistedSettings {
            model: "waifu2x".to_string(),
            scale: 2,
            noise: 3,
            compress_enabled: true,
            compress_level: 6,
            normalize_png: false,
            webp_compat: true,
            // Device mode: the stored pixel value is whatever the calculator
            // produced for the device fields below.
            webp_max_dimension: crate::clamp::compute(110, 264, 300).clamp_px,
            clamp_device_mode: true,
            device_inches_tenths: 110,
            device_dpi: 264,
            clamp_headroom_pct: 300,
            binary_dir: "D:/tools/ncnn".to_string(),
            theme: "nord".to_string(),
            history_max_bytes: 2 * 1024 * 1024 * 1024,
            enabled_plugins: vec!["comix".to_string(), "gigaviewer".to_string()],
        }
    }

    #[test]
    fn every_field_survives_a_save_load_cycle() {
        let path = temp_settings_path("roundtrip");
        let original = customised();
        save_to(&path, &original).unwrap();
        let (loaded, warning) = load_from(&path);
        assert!(warning.is_none(), "a valid file should not warn");
        let loaded = loaded.expect("settings should load back");

        assert_eq!(loaded.model, original.model);
        assert_eq!(loaded.scale, original.scale);
        assert_eq!(loaded.noise, original.noise);
        assert_eq!(loaded.compress_enabled, original.compress_enabled);
        assert_eq!(loaded.compress_level, original.compress_level);
        assert_eq!(loaded.normalize_png, original.normalize_png);
        assert_eq!(loaded.webp_compat, original.webp_compat);
        assert_eq!(loaded.webp_max_dimension, original.webp_max_dimension);
        assert_eq!(loaded.clamp_device_mode, original.clamp_device_mode);
        assert_eq!(loaded.device_inches_tenths, original.device_inches_tenths);
        assert_eq!(loaded.device_dpi, original.device_dpi);
        assert_eq!(loaded.clamp_headroom_pct, original.clamp_headroom_pct);
        assert_eq!(loaded.binary_dir, original.binary_dir);
        assert_eq!(loaded.theme, original.theme);
        assert_eq!(loaded.history_max_bytes, original.history_max_bytes);
        assert_eq!(loaded.enabled_plugins, original.enabled_plugins);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn saving_over_an_existing_file_replaces_it() {
        let path = temp_settings_path("overwrite");
        save_to(&path, &PersistedSettings::default()).unwrap();
        let mut second = customised();
        second.theme = "dracula".to_string();
        save_to(&path, &second).unwrap();

        assert_eq!(load_from(&path).0.unwrap().theme, "dracula");
        // The temp file must not be left behind next to the real one.
        let strays: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(strays.is_empty(), "left temp files behind: {strays:?}");

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn settings_written_by_an_older_build_still_load() {
        let path = temp_settings_path("partial");
        // A file from before the clamp/plugin fields existed.
        std::fs::write(
            &path,
            r#"{"model":"realesrgan-anime","scale":4,"theme":"catppuccin"}"#,
        )
        .unwrap();
        let loaded = load_from(&path).0.expect("old settings should still parse");
        assert_eq!(loaded.model, "realesrgan-anime");
        assert_eq!(loaded.scale, 4);
        assert_eq!(loaded.theme, "catppuccin");
        // Missing fields fall back to defaults rather than failing the load.
        let defaults = PersistedSettings::default();
        assert_eq!(loaded.webp_max_dimension, defaults.webp_max_dimension);
        assert_eq!(loaded.clamp_headroom_pct, defaults.clamp_headroom_pct);
        assert!(loaded.enabled_plugins.is_empty());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_corrupt_file_falls_back_to_defaults_instead_of_crashing() {
        let path = temp_settings_path("corrupt");
        std::fs::write(&path, "{not json at all").unwrap();
        let (loaded, warning) = load_from(&path);
        assert!(loaded.is_none());
        // Silently resetting every preference needs an explanation, and the
        // unreadable file is kept rather than overwritten.
        let warning = warning.expect("a corrupt file must be reported");
        assert!(warning.contains("not valid JSON"), "{warning}");
        assert!(path.with_extension("json.corrupt").exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn saved_settings_drive_the_startup_config() {
        let saved = customised();
        let mut cfg = crate::config::ServerConfig::default();
        saved.apply_to(&mut cfg);
        assert_eq!(cfg.default_model, "waifu2x");
        assert_eq!(cfg.default_noise, 3);
        assert_eq!(cfg.compress_level, 6);
        assert!(!cfg.normalize_png);
        assert_eq!(cfg.webp_max_dimension, saved.webp_max_dimension);
        assert_eq!(cfg.binary_dir, "D:/tools/ncnn");
    }

    #[test]
    fn an_empty_binary_dir_keeps_auto_detection() {
        let mut saved = PersistedSettings::default();
        saved.binary_dir = String::new();
        let mut cfg = crate::config::ServerConfig::default();
        let auto = cfg.binary_dir.clone();
        saved.apply_to(&mut cfg);
        assert_eq!(cfg.binary_dir, auto);
    }

    #[test]
    fn device_mode_clamp_matches_what_the_calculator_shows() {
        // A device-mode session must reopen on the same pixel value the UI
        // displayed, or the restored clamp would silently disagree with it.
        let saved = customised();
        let recomputed = crate::clamp::compute(
            saved.device_inches_tenths,
            saved.device_dpi,
            saved.clamp_headroom_pct,
        );
        assert_eq!(saved.webp_max_dimension, recomputed.clamp_px);
    }

    #[test]
    fn device_mode_recomputes_a_hand_edited_pixel_value() {
        let mut saved = customised();
        let correct = saved.webp_max_dimension;
        saved.webp_max_dimension = 999; // as if edited in the JSON by hand
        let mut cfg = crate::config::ServerConfig::default();
        saved.apply_to(&mut cfg);
        assert_eq!(cfg.webp_max_dimension, correct);
    }

    #[test]
    fn pixel_mode_keeps_the_exact_value_the_user_chose() {
        let mut saved = customised();
        saved.clamp_device_mode = false;
        saved.webp_max_dimension = 4096;
        let mut cfg = crate::config::ServerConfig::default();
        saved.apply_to(&mut cfg);
        assert_eq!(cfg.webp_max_dimension, 4096);
    }
}
