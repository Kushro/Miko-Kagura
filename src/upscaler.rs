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
        // Prefer the bundled vulkaninfo next to our binaries, then PATH.
        let bundled = if cfg!(windows) { "vulkaninfo.exe" } else { "vulkaninfo" };
        let mut candidates: Vec<PathBuf> = Vec::new();
        candidates.push(self.binary_dir.join("vulkan").join(bundled));
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

    /// Settings that decide how the engine is invoked. Jobs with equal keys can
    /// share one engine run.
    pub fn engine_key(&self) -> (PathBuf, i32, String, i32, i32, String) {
        (
            self.binary_dir.clone(),
            self.gpu_id,
            self.current_model_name.clone(),
            self.current_scale,
            self.current_noise,
            self.gpu_vendor.clone(),
        )
    }

    /// Process image bytes and return upscaled PNG bytes.
    pub fn process(&self, image_bytes: &[u8]) -> Result<Vec<u8>, String> {
        self.process_batch(&[image_bytes])?
            .pop()
            .unwrap_or_else(|| Err("Upscaler produced no output file".to_string()))
    }

    /// Process several images in a single engine run, so Vulkan and the model
    /// are initialised once for all of them. Returns one result per input, in
    /// order; an input that can't be decoded fails on its own. The outer `Err`
    /// means the engine run itself failed, which affects every input.
    pub fn process_batch(&self, inputs: &[&[u8]]) -> Result<Vec<Result<Vec<u8>, String>>, String> {
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

        // Unique temp working directory. A single image uses the engine's file
        // mode; several use directory mode, which pipelines load/process/save.
        let tmpdir = make_temp_dir()?;
        let dir_mode = inputs.len() > 1;
        let (in_target, out_target) = if dir_mode {
            (tmpdir.join("in"), tmpdir.join("out"))
        } else {
            (tmpdir.join("input.png"), tmpdir.join("output.png"))
        };
        let file_name = |i: usize| format!("{i:04}.png");
        let in_path = |i: usize| if dir_mode { in_target.join(file_name(i)) } else { in_target.clone() };
        let out_path = |i: usize| if dir_mode { out_target.join(file_name(i)) } else { out_target.clone() };

        let result = (|| -> Result<Vec<Result<Vec<u8>, String>>, String> {
            if dir_mode {
                for dir in [&in_target, &out_target] {
                    std::fs::create_dir_all(dir)
                        .map_err(|e| format!("Could not create temp dir: {e}"))?;
                }
            }

            // Normalize each input to a clean 8-bit RGB/RGBA PNG. Inputs that
            // already are one are written as-is, skipping a decode and encode.
            let prepared: Vec<Result<(), String>> = inputs
                .iter()
                .enumerate()
                .map(|(i, bytes)| {
                    if is_plain_png(bytes) {
                        return std::fs::write(in_path(i), bytes)
                            .map_err(|e| format!("Could not write input image: {e}"));
                    }
                    let img = image::load_from_memory(bytes)
                        .map_err(|e| format!("Could not decode input image: {e}"))?;
                    to_rgb_or_rgba(img)
                        .save_with_format(in_path(i), ImageFormat::Png)
                        .map_err(|e| format!("Could not write input image: {e}"))
                })
                .collect();
            if prepared.iter().all(Result::is_err) {
                return Ok(prepared.into_iter().map(|r| r.map(|()| Vec::new())).collect());
            }

            let mut cmd = new_command(&binary);
            cmd.arg("-i").arg(&in_target);
            cmd.arg("-o").arg(&out_target);
            cmd.arg("-s").arg(self.current_scale.to_string());
            if dir_mode {
                cmd.arg("-f").arg("png");
            }

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

            // Return the raw NCNN output. The caller (server.rs) applies
            // normalize_png_bytes() and/or oxipng compression as configured.
            Ok(prepared
                .into_iter()
                .enumerate()
                .map(|(i, prep)| {
                    prep?;
                    let out = out_path(i);
                    if !out.exists() {
                        return Err("Upscaler produced no output file".to_string());
                    }
                    std::fs::read(&out).map_err(|e| format!("Could not read output image: {e}"))
                })
                .collect())
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
///   3. Outside Windows, `$XDG_DATA_HOME/miko-kagura/binaries` — the writable
///      place for engines when the app itself is installed read-only
///      (AppImage, .deb, AUR Lite).
///   4. `binaries/` under the current working directory (covers `cargo run`
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

    #[cfg(not(windows))]
    {
        if let Some(dir) = crate::settings::xdg_app_dir("XDG_DATA_HOME", ".local/share") {
            let candidate = dir.join("binaries");
            if candidate.is_dir() {
                return absolutize(candidate);
            }
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

/// Width and height read from the image header, without decoding pixels.
/// `(0, 0)` if the bytes aren't a recognised image.
pub fn image_dimensions(data: &[u8]) -> (u32, u32) {
    image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok())
        .unwrap_or((0, 0))
}

/// Whether `data` is a PNG that is already 8-bit RGB or RGBA — the form the
/// engines and Android's BitmapFactory both read as-is. Checks only the
/// signature and the IHDR chunk (bit depth at byte 24, colour type at 25).
pub fn is_plain_png(data: &[u8]) -> bool {
    data.len() > 25
        && data.starts_with(b"\x89PNG\r\n\x1a\n")
        && &data[12..16] == b"IHDR"
        && data[24] == 8
        && matches!(data[25], 2 | 6)
}

/// Re-encode PNG bytes as a clean 8-bit RGB/RGBA PNG.
///
/// NCNN binaries can emit PNG variants (16-bit, palette, grayscale) that
/// Android's BitmapFactory refuses to decode. Round-tripping through the `image`
/// crate guarantees a widely-decodable 8-bit PNG. Bytes that already are one
/// (the usual engine output) are returned untouched, and so are bytes the
/// re-encode fails on.
pub fn normalize_png_bytes(data: Vec<u8>) -> Vec<u8> {
    if is_plain_png(&data) {
        return data;
    }
    match image::load_from_memory(&data) {
        Ok(img) => {
            let normalized = to_rgb_or_rgba(img);
            let mut buf = Cursor::new(Vec::new());
            match normalized.write_to(&mut buf, ImageFormat::Png) {
                Ok(()) => buf.into_inner(),
                Err(_) => data,
            }
        }
        Err(_) => data,
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
pub fn enforce_webp_limit(data: &[u8], limit: u32) -> Option<(Vec<u8>, Resize)> {
    let limit = limit.clamp(1, WEBP_MAX_DIMENSION);
    // The header is enough to rule out the common case without decoding.
    let (w, h) = image_dimensions(data);
    if w > 0 && w.max(h) <= limit {
        return None;
    }
    let img = image::load_from_memory(data).ok()?;
    let (resized, dims) = fit_within(&img, limit)?;
    let clean = to_rgb_or_rgba(resized);
    let mut buf = Cursor::new(Vec::new());
    clean.write_to(&mut buf, ImageFormat::Png).ok()?;
    Some((buf.into_inner(), dims))
}

/// A resize as `(from_w, from_h, to_w, to_h)`, for logging.
pub type Resize = (u32, u32, u32, u32);

/// Proportionally downscale (Lanczos3) `img` so neither axis exceeds `limit`,
/// returning it with `(from_w, from_h, to_w, to_h)`; `None` if it already fits.
fn fit_within(img: &DynamicImage, limit: u32) -> Option<(DynamicImage, Resize)> {
    let (w, h) = (img.width(), img.height());
    if w.max(h) <= limit {
        return None;
    }

    let scale = limit as f64 / w.max(h) as f64;
    // floor + clamp guarantees both axes land at or under the limit.
    let nw = ((w as f64 * scale).floor() as u32).clamp(1, limit);
    let nh = ((h as f64 * scale).floor() as u32).clamp(1, limit);

    let resized = img.resize_exact(nw, nh, image::imageops::FilterType::Lanczos3);
    Some((resized, (w, h, nw, nh)))
}

/// Encode an image as lossy WebP at `quality` (1–100), first downscaling it
/// to fit `limit` (itself capped at [`WEBP_MAX_DIMENSION`], which WebP can't
/// exceed). Also returns the resize as `(from_w, from_h, to_w, to_h)`, if any.
pub fn encode_webp(
    data: &[u8],
    limit: u32,
    quality: u8,
) -> Result<(Vec<u8>, Option<Resize>), String> {
    let limit = limit.clamp(1, WEBP_MAX_DIMENSION);
    let img = image::load_from_memory(data).map_err(|e| format!("Could not decode image: {e}"))?;
    let (img, resized) = match fit_within(&img, limit) {
        Some((img, dims)) => (img, Some(dims)),
        None => (img, None),
    };
    let quality = quality.clamp(1, 100) as f32;
    let encoded = match to_rgb_or_rgba(img) {
        DynamicImage::ImageRgba8(buf) => webp::Encoder::from_rgba(&buf, buf.width(), buf.height())
            .encode_simple(false, quality),
        other => {
            let buf = other.to_rgb8();
            webp::Encoder::from_rgb(&buf, buf.width(), buf.height()).encode_simple(false, quality)
        }
    }
    .map_err(|e| format!("WebP encoding failed: {e:?}"))?;
    Ok((encoded.to_vec(), resized))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(img: DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, format).unwrap();
        buf.into_inner()
    }

    fn rgb(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(image::RgbImage::new(w, h))
    }

    #[test]
    fn only_8bit_rgb_and_rgba_pngs_count_as_plain() {
        assert!(is_plain_png(&encode(rgb(4, 3), ImageFormat::Png)));
        let rgba = DynamicImage::ImageRgba8(image::RgbaImage::new(4, 3));
        assert!(is_plain_png(&encode(rgba, ImageFormat::Png)));

        let gray = DynamicImage::ImageLuma8(image::GrayImage::new(4, 3));
        assert!(!is_plain_png(&encode(gray, ImageFormat::Png)));
        let deep = DynamicImage::ImageRgb16(image::ImageBuffer::new(4, 3));
        assert!(!is_plain_png(&encode(deep, ImageFormat::Png)));
        assert!(!is_plain_png(&encode(rgb(4, 3), ImageFormat::Jpeg)));
        assert!(!is_plain_png(b"\x89PNG"));
    }

    #[test]
    fn normalizing_keeps_plain_pngs_and_converts_the_rest() {
        let plain = encode(rgb(4, 3), ImageFormat::Png);
        assert_eq!(normalize_png_bytes(plain.clone()), plain);

        let deep = DynamicImage::ImageRgb16(image::ImageBuffer::new(4, 3));
        let converted = normalize_png_bytes(encode(deep, ImageFormat::Png));
        assert!(is_plain_png(&converted));
        assert_eq!(image_dimensions(&converted), (4, 3));
    }

    #[test]
    fn dimensions_come_from_the_header() {
        assert_eq!(image_dimensions(&encode(rgb(7, 5), ImageFormat::Png)), (7, 5));
        assert_eq!(image_dimensions(&encode(rgb(7, 5), ImageFormat::Jpeg)), (7, 5));
        assert_eq!(image_dimensions(b"not an image"), (0, 0));
    }

    #[test]
    fn webp_output_is_lossy_webp_clamped_to_the_limit() {
        let png = encode(rgb(40, 10), ImageFormat::Png);
        let (webp, resized) = encode_webp(&png, 100, 90).unwrap();
        assert_eq!(image::guess_format(&webp).unwrap(), ImageFormat::WebP);
        assert_eq!(image_dimensions(&webp), (40, 10));
        assert!(resized.is_none());

        let (webp, resized) = encode_webp(&png, 20, 90).unwrap();
        assert_eq!(resized, Some((40, 10, 20, 5)));
        assert_eq!(image_dimensions(&webp), (20, 5));

        let rgba = DynamicImage::ImageRgba8(image::RgbaImage::new(6, 4));
        let (webp, _) = encode_webp(&encode(rgba, ImageFormat::Png), 100, 90).unwrap();
        assert!(image::load_from_memory(&webp).unwrap().color().has_alpha());

        assert!(encode_webp(b"not an image", 100, 90).is_err());
    }

    #[test]
    fn webp_clamp_only_resizes_oversized_images() {
        let png = encode(rgb(40, 10), ImageFormat::Png);
        assert!(enforce_webp_limit(&png, 40).is_none());
        let (bytes, dims) = enforce_webp_limit(&png, 20).unwrap();
        assert_eq!(dims, (40, 10, 20, 5));
        assert_eq!(image_dimensions(&bytes), (20, 5));
    }
}
