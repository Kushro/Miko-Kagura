//! Upscaling engine — wraps the NCNN Vulkan CLI binaries via subprocess.
//!
//! Port of the Python `upscaler.py`. The `Upscaler` is cheap to `Clone`: the HTTP
//! handler snapshots it under a short lock, releases the lock, then runs the
//! blocking subprocess on the snapshot so model switches never block in-flight work.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

use image::{DynamicImage, ImageFormat};

use crate::config::{get_model, model_keys_csv, noise_name, ModelConfig, NoiseStyle};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// NCNN Vulkan CLI binaries that support the `-g` flag for GPU listing.
const GPU_DETECT_BINARIES: &[&str] = &[
    "waifu2x-ncnn-vulkan",
    "realcugan-ncnn-vulkan",
    "realesrgan-ncnn-vulkan",
];

/// Image upscaling engine using NCNN CLI binaries.
#[derive(Clone)]
pub struct Upscaler {
    binary_dir: PathBuf,
    pub gpu_id: i32,

    pub current_model_name: String,
    pub current_display_name: String,
    pub current_scale: i32,
    pub current_noise: i32,
    pub current_config: Option<ModelConfig>,
    pub gpu_name: String,
    pub gpu_vendor: String,
}

impl Upscaler {
    pub fn new(binary_dir: &str, gpu_id: i32) -> Self {
        Self {
            binary_dir: resolve_binary_dir(binary_dir),
            gpu_id,
            current_model_name: String::new(),
            current_display_name: String::new(),
            current_scale: 2,
            current_noise: 0,
            current_config: None,
            gpu_name: "Not detected".to_string(),
            gpu_vendor: String::new(),
        }
    }

    /// Load a model, validating scale/noise against its constraints.
    pub fn load_model(&mut self, model: &str, scale: i32, noise: i32) -> Result<(), String> {
        let config = get_model(model).ok_or_else(|| {
            format!("Unknown model '{model}'. Valid: {}", model_keys_csv())
        })?;

        if !config.supported_scales.contains(&scale) {
            return Err(format!(
                "Scale {scale}x not supported for {}. Supported: {:?}",
                config.display_name, config.supported_scales
            ));
        }
        if !config.supported_noise.contains(&noise) {
            return Err(format!(
                "Noise level {noise} not supported for {}. Supported: {:?}",
                config.display_name, config.supported_noise
            ));
        }

        self.current_model_name = config.name.to_string();
        self.current_display_name = config.display_name.to_string();
        self.current_scale = scale;
        self.current_noise = noise;
        self.current_config = Some(config.clone());
        Ok(())
    }

    // ── GPU detection ─────────────────────────────────────────────────

    /// Detect the active GPU once. Cached in `gpu_name` / `gpu_vendor`.
    pub fn detect_gpu(&mut self) {
        for binary_name in GPU_DETECT_BINARIES {
            if let Some(binary) = self.find_binary(binary_name) {
                if let Some(info) = self.detect_gpu_via_ncnn(&binary) {
                    self.gpu_name = info.clone();
                    self.gpu_vendor = classify_vendor(&info);
                    return;
                }
            }
        }

        if let Some(info) = self.detect_gpu_via_vulkaninfo() {
            self.gpu_name = info.clone();
            self.gpu_vendor = classify_vendor(&info);
            return;
        }

        if let Some(info) = detect_gpu_via_windows() {
            self.gpu_name = info.clone();
            self.gpu_vendor = classify_vendor(&info);
            return;
        }

        self.gpu_name = "GPU: not detected (install Vulkan drivers)".to_string();
        self.gpu_vendor = String::new();
    }

    fn detect_gpu_via_ncnn(&self, binary: &Path) -> Option<String> {
        let output = new_command(binary).arg("-g").output().ok()?;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));

        // NCNN vulkan binaries list devices as "[0 AMD Radeon RX 9070 XT]  queueC=..."
        // (repeated per capability line), so collect unique device indices in order.
        let mut gpus: Vec<(u32, String)> = Vec::new();
        for line in text.lines() {
            if let Some((idx, name)) = parse_gpu_bracket(line) {
                if !gpus.iter().any(|(i, _)| *i == idx) {
                    gpus.push((idx, name));
                }
            }
        }
        // Some builds instead print "GPU 0: <name>".
        if gpus.is_empty() {
            for (i, line) in text.lines().enumerate() {
                if let Some(rest) = parse_gpu_line(line) {
                    gpus.push((i as u32, rest));
                }
            }
        }
        if !gpus.is_empty() {
            gpus.sort_by_key(|(i, _)| *i);
            let pos = gpus
                .iter()
                .position(|(i, _)| *i as i32 == self.gpu_id)
                .unwrap_or(0);
            return Some(gpus[pos].1.clone());
        }

        // Fallback: any line that mentions a known GPU vendor keyword.
        for line in text.lines() {
            let low = line.to_lowercase();
            if ["radeon", "geforce", "rtx", "gtx", "arc", "nvidia", "amd", "intel"]
                .iter()
                .any(|kw| low.contains(kw))
            {
                return Some(line.trim().to_string());
            }
        }
        None
    }

    fn detect_gpu_via_vulkaninfo(&self) -> Option<String> {
        // Prefer the bundled vulkaninfo.exe next to our binaries, then PATH.
        let mut candidates: Vec<PathBuf> = Vec::new();
        candidates.push(self.binary_dir.join("vulkan").join("vulkaninfo.exe"));
        candidates.push(PathBuf::from("vulkaninfo"));

        for cmd in candidates {
            if cmd.components().count() > 1 && !cmd.exists() {
                continue;
            }
            let output = match new_command(&cmd).arg("--summary").output() {
                Ok(o) => o,
                Err(_) => continue,
            };
            let text = String::from_utf8_lossy(&output.stdout);
            // "GPU id = 0 (AMD Radeon RX 9070 XT)"
            let mut gpus: Vec<String> = Vec::new();
            for line in text.lines() {
                if let Some(name) = parse_vulkaninfo_line(line) {
                    gpus.push(name);
                }
            }
            if !gpus.is_empty() {
                let idx = if self.gpu_id >= 0 && (self.gpu_id as usize) < gpus.len() {
                    self.gpu_id as usize
                } else {
                    0
                };
                return Some(gpus[idx].clone());
            }
        }
        None
    }

    // ── Binary resolution ─────────────────────────────────────────────

    /// Find a CLI binary by name, checking `binary_dir/<subdir>/<name>.exe`,
    /// then `binary_dir/<name>.exe`, then PATH.
    fn find_binary(&self, name: &str) -> Option<PathBuf> {
        let subdir = match name {
            "waifu2x-ncnn-vulkan" => "waifu2x",
            "realcugan-ncnn-vulkan" => "realcugan",
            "realesrgan-ncnn-vulkan" => "realesrgan",
            other => other.trim_end_matches("-ncnn-vulkan"),
        };

        let exts: &[&str] = if cfg!(windows) { &[".exe", ""] } else { &[""] };

        let candidate_dir = self.binary_dir.join(subdir);
        if candidate_dir.is_dir() {
            for ext in exts {
                let candidate = candidate_dir.join(format!("{name}{ext}"));
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
        for ext in exts {
            let candidate = self.binary_dir.join(format!("{name}{ext}"));
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    }

    fn noise_arg(&self, noise: i32, config: &ModelConfig) -> String {
        match config.noise_style {
            NoiseStyle::Cugan => noise_name(noise).to_string(),
            _ => noise.to_string(),
        }
    }

    /// The directory currently used to resolve NCNN CLI binaries.
    pub fn binary_dir(&self) -> &Path {
        &self.binary_dir
    }

    /// Re-resolve the binaries directory from a new path — e.g. set from the
    /// Settings UI. Accepts the same relative/absolute/portable resolution as
    /// startup (see [`resolve_binary_dir`]).
    pub fn set_binary_dir(&mut self, dir: &str) {
        self.binary_dir = resolve_binary_dir(dir);
    }

    /// Whether the CLI binary required by the currently loaded model can
    /// actually be found under `binary_dir`. `load_model` only validates the
    /// model *config* (scale/noise) — it doesn't check the binary is on disk —
    /// so this is the real "is upscaling possible right now" check. Used by
    /// `/health` and the dashboard alert banner.
    pub fn binaries_ok(&self) -> bool {
        match &self.current_config {
            Some(config) => self.find_binary(config.cli_binary).is_some(),
            None => false,
        }
    }

    /// Human-readable explanation for the dashboard/health alert when
    /// [`Self::binaries_ok`] is false. `None` when everything's fine.
    pub fn missing_binary_hint(&self) -> Option<String> {
        let config = self.current_config.as_ref()?;
        if self.find_binary(config.cli_binary).is_some() {
            return None;
        }
        Some(format!(
            "Binary '{}' not found under '{}'. Set the correct folder in Settings → \
             Binary Directory, or place the ncnn-vulkan binaries there \
             (waifu2x/, realcugan/, realesrgan/ subfolders).",
            config.cli_binary,
            self.binary_dir.display()
        ))
    }

    // ── Processing ────────────────────────────────────────────────────

    /// Process image bytes and return upscaled PNG bytes.
    pub fn process(&self, image_bytes: &[u8]) -> Result<Vec<u8>, String> {
        let config = self
            .current_config
            .as_ref()
            .ok_or_else(|| "No model loaded. Call load_model() first.".to_string())?;

        let binary = self.find_binary(config.cli_binary).ok_or_else(|| {
            self.missing_binary_hint().unwrap_or_else(|| {
                format!(
                    "Binary '{}' not found under '{}'.",
                    config.cli_binary,
                    self.binary_dir.display()
                )
            })
        })?;

        // Unique temp working directory.
        let tmpdir = make_temp_dir()?;
        let in_path = tmpdir.join("input.png");
        let out_path = tmpdir.join("output.png");

        let result = (|| -> Result<Vec<u8>, String> {
            // Normalize input to a clean 8-bit RGB/RGBA PNG.
            let img = image::load_from_memory(image_bytes)
                .map_err(|e| format!("Could not decode input image: {e}"))?;
            let normalized = to_rgb_or_rgba(img);
            normalized
                .save_with_format(&in_path, ImageFormat::Png)
                .map_err(|e| format!("Could not write input image: {e}"))?;

            let mut cmd = new_command(&binary);
            cmd.arg("-i").arg(&in_path);
            cmd.arg("-o").arg(&out_path);
            cmd.arg("-s").arg(self.current_scale.to_string());

            // Noise / model-name argument.
            match config.noise_style {
                NoiseStyle::Realesrgan => {
                    if let Some(model_name) = config.model_names.first() {
                        cmd.arg("-n").arg(model_name);
                    }
                }
                _ => {
                    cmd.arg("-n").arg(self.noise_arg(self.current_noise, config));
                }
            }

            // Model directory: -m <path>, next to the binary.
            let models_path = binary
                .parent()
                .map(|p| p.join(config.models_dir))
                .unwrap_or_else(|| PathBuf::from(config.models_dir));
            if models_path.is_dir() {
                cmd.arg("-m").arg(&models_path);
            }

            // Real-CUGAN syncgap mode.
            if config.syncgap >= 0 {
                cmd.arg("-c").arg(config.syncgap.to_string());
            }

            // GPU selection.
            if self.gpu_id >= 0 {
                cmd.arg("-g").arg(self.gpu_id.to_string());
            }

            // Tile size for VRAM management.
            if self.gpu_vendor == "amd" {
                cmd.arg("-j").arg("2:4:2");
            } else {
                cmd.arg("-j").arg("2:4:4");
            }

            let output = cmd
                .output()
                .map_err(|e| format!("Failed to run upscaler: {e}"))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let low = stderr.to_lowercase();
                if low.contains("vulkan") && (low.contains("failed") || low.contains("error")) {
                    return Err(format!(
                        "Vulkan initialization failed. Install up-to-date GPU drivers \
                         (AMD Adrenalin / NVIDIA Game Ready / Intel Arc) or the Vulkan \
                         runtime.\nOriginal error: {stderr}"
                    ));
                }
                return Err(format!(
                    "Upscaler failed (exit {}): {stderr}",
                    output.status.code().unwrap_or(-1)
                ));
            }

            if !out_path.exists() {
                return Err("Upscaler produced no output file".to_string());
            }

            // Return the raw NCNN output. The caller (server.rs) applies
            // normalize_png_bytes() and/or oxipng compression as configured.
            std::fs::read(&out_path)
                .map_err(|e| format!("Could not read output image: {e}"))
        })();

        // Always clean up the temp dir.
        let _ = std::fs::remove_dir_all(&tmpdir);
        result
    }
}

// ── Free helpers ──────────────────────────────────────────────────────

/// Build a `Command` with the no-console-window flag on Windows.
fn new_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Resolve the binaries directory.
///
/// Order:
///   1. `explicit` (the `--binary-dir` flag, or a path set from the Settings
///      UI), if it points at an existing directory — checked relative to the
///      current working directory when not absolute.
///   2. A `binaries/` folder discovered by walking up from the running
///      executable's own directory, all the way to the filesystem root. This
///      is what makes the server portable: whichever nested `target/...`
///      layout cargo or `dx build` produced (e.g.
///      `target/dx/<name>/release/windows/app/`), and wherever the whole
///      project folder gets moved or renamed, a `binaries/` folder living at
///      the project root is still found — no compile-time path is ever
///      baked in.
///   3. `binaries/` under the current working directory (covers `cargo run`
///      / `dx serve`, where cwd is already the project root).
///
/// Falls back to a bare `binaries` (or the given `explicit` path) if nothing
/// is found, so downstream error messages at least show what was tried.
fn resolve_binary_dir(explicit: &str) -> PathBuf {
    if !explicit.is_empty() {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            return absolutize(p);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(Path::to_path_buf);
        while let Some(d) = dir {
            let candidate = d.join("binaries");
            if candidate.is_dir() {
                return absolutize(candidate);
            }
            dir = d.parent().map(Path::to_path_buf);
        }
    }

    let cwd_candidate = std::env::current_dir().unwrap_or_default().join("binaries");
    if cwd_candidate.is_dir() {
        return absolutize(cwd_candidate);
    }

    if explicit.is_empty() {
        PathBuf::from("binaries")
    } else {
        PathBuf::from(explicit)
    }
}

/// Resolve a path to an absolute one, stripping the Windows `\\?\` verbatim
/// prefix. The ncnn CLI binaries mishandle verbatim paths when concatenating
/// the model directory with a filename, so we must hand them plain paths.
fn absolutize(p: PathBuf) -> PathBuf {
    let abs = std::fs::canonicalize(&p).unwrap_or(p);
    let s = abs.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        abs
    }
}

/// Windows GPU detection via PowerShell WMI query.
#[cfg(windows)]
fn detect_gpu_via_windows() -> Option<String> {
    let output = new_command(Path::new("powershell"))
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

#[cfg(not(windows))]
fn detect_gpu_via_windows() -> Option<String> {
    None
}

fn parse_gpu_bracket(line: &str) -> Option<(u32, String)> {
    // Match "[<idx> <name>] …"
    let t = line.trim();
    if !t.starts_with('[') {
        return None;
    }
    let close = t.find(']')?;
    let inner = &t[1..close];
    let space = inner.find(' ')?;
    let (num, name) = inner.split_at(space);
    let idx: u32 = num.trim().parse().ok()?;
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some((idx, name.to_string()))
    }
}

fn parse_gpu_line(line: &str) -> Option<String> {
    // Match "GPU <n>: <rest>"
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("GPU")?;
    let rest = rest.trim_start();
    let colon = rest.find(':')?;
    let (num, after) = rest.split_at(colon);
    if !num.trim().chars().all(|c| c.is_ascii_digit()) || num.trim().is_empty() {
        return None;
    }
    let value = after[1..].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_vulkaninfo_line(line: &str) -> Option<String> {
    // Match "GPU id = <n> (<name>)"
    let trimmed = line.trim();
    if !trimmed.starts_with("GPU id") {
        return None;
    }
    let open = trimmed.find('(')?;
    let close = trimmed.rfind(')')?;
    if close <= open + 1 {
        return None;
    }
    Some(trimmed[open + 1..close].trim().to_string())
}

fn classify_vendor(gpu_name: &str) -> String {
    let low = gpu_name.to_lowercase();
    let any = |kws: &[&str]| kws.iter().any(|k| low.contains(k));
    if any(&["amd", "radeon", "ati", "navi", "vega", "polaris"]) {
        "amd".to_string()
    } else if any(&["nvidia", "geforce", "rtx", "gtx", "quadro"]) {
        "nvidia".to_string()
    } else if any(&["intel", "arc", "uhd", "iris"]) {
        "intel".to_string()
    } else if any(&["apple", " m1", " m2", " m3", " m4"]) {
        "apple".to_string()
    } else if any(&["mali", "adreno", "qualcomm"]) {
        "qualcomm".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Convert an image to 8-bit RGB, or RGBA when it carries transparency.
fn to_rgb_or_rgba(img: DynamicImage) -> DynamicImage {
    if img.color().has_alpha() {
        DynamicImage::ImageRgba8(img.to_rgba8())
    } else {
        DynamicImage::ImageRgb8(img.to_rgb8())
    }
}

/// Re-encode PNG bytes as a clean 8-bit RGB/RGBA PNG.
///
/// NCNN binaries can emit PNG variants (16-bit, palette, grayscale) that
/// Android's BitmapFactory refuses to decode. Round-tripping through the `image`
/// crate guarantees a widely-decodable 8-bit PNG. Falls back to the raw bytes if
/// the re-encode fails for any reason.
pub fn normalize_png_bytes(data: &[u8]) -> Vec<u8> {
    match image::load_from_memory(data) {
        Ok(img) => {
            let normalized = to_rgb_or_rgba(img);
            let mut buf = Cursor::new(Vec::new());
            match normalized.write_to(&mut buf, ImageFormat::Png) {
                Ok(()) => buf.into_inner(),
                Err(_) => data.to_vec(),
            }
        }
        Err(_) => data.to_vec(),
    }
}

/// Hard per-dimension maximum for the WebP container (2^14 − 1). This is a
/// container format limit, not a preference — [`enforce_webp_limit`]'s `limit`
/// argument is always clamped to this, however it's configured.
pub const WEBP_MAX_DIMENSION: u32 = 16383;

/// If `data` exceeds `limit` on either axis, return a proportionally
/// downscaled clean 8-bit PNG (Lanczos3) plus `(from_w, from_h, to_w, to_h)`
/// for logging. Returns `None` when the image is already within `limit` — the
/// caller then keeps the original bytes, so no copy or re-encode happens on
/// the common path.
///
/// `limit` is clamped to `1..=WEBP_MAX_DIMENSION` — callers may expose a
/// smaller configurable clamp (e.g. to save bandwidth), but can never exceed
/// the hard WebP container limit.
///
/// Quality note: the model already ran on the full-resolution input; we only
/// trim the excess beyond `limit`. For images just over the limit this is a
/// small reduction and effectively supersampling, so detail is preserved.
pub fn enforce_webp_limit(data: &[u8], limit: u32) -> Option<(Vec<u8>, (u32, u32, u32, u32))> {
    let limit = limit.clamp(1, WEBP_MAX_DIMENSION);
    let img = image::load_from_memory(data).ok()?;
    let (w, h) = (img.width(), img.height());
    if w.max(h) <= limit {
        return None;
    }

    let scale = limit as f64 / w.max(h) as f64;
    // floor + clamp guarantees both axes land at or under the limit.
    let nw = ((w as f64 * scale).floor() as u32).clamp(1, limit);
    let nh = ((h as f64 * scale).floor() as u32).clamp(1, limit);

    let resized = img.resize_exact(nw, nh, image::imageops::FilterType::Lanczos3);
    let clean = to_rgb_or_rgba(resized);
    let mut buf = Cursor::new(Vec::new());
    clean.write_to(&mut buf, ImageFormat::Png).ok()?;
    Some((buf.into_inner(), (w, h, nw, nh)))
}

/// Create a unique temporary working directory.
fn make_temp_dir() -> Result<PathBuf, String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "{}{}-{nanos}",
        crate::cache::CACHE_PREFIX,
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create temp dir: {e}"))?;
    Ok(dir)
}
