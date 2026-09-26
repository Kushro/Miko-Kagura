//! Server configuration and the registry of supported upscaling models.
//!
//! Port of the Python `config.py`. The model registry is the source of truth for
//! which CLI binary, model directory, scales, and noise levels each model uses.

use std::sync::OnceLock;

/// How the noise / model-name argument is passed to the NCNN binary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoiseStyle {
    /// `-n 0`, `-n 1`, `-n 2`, `-n 3` (waifu2x).
    Numeric,
    /// `-n no-denoise`, `-n denoise1x`, … (realcugan).
    Cugan,
    /// `-n <model-name>` — realesrgan uses `-n` for model selection, not noise.
    Realesrgan,
}

/// Definition of one available upscaling model.
#[derive(Clone, Debug)]
pub struct ModelConfig {
    pub name: &'static str,
    pub display_name: &'static str,
    pub cli_binary: &'static str,
    pub supported_scales: Vec<i32>,
    pub supported_noise: Vec<i32>,
    /// Model subdirectory relative to the binary's parent (passed via `-m`).
    pub models_dir: &'static str,
    pub noise_style: NoiseStyle,
    /// For realesrgan-style binaries: `-n` selects the model name from this list.
    pub model_names: Vec<&'static str>,
    /// syncgap mode for realcugan (`-c 0..3`); negative means "do not pass".
    pub syncgap: i32,
}

impl ModelConfig {
    fn new(
        name: &'static str,
        display_name: &'static str,
        cli_binary: &'static str,
        supported_scales: &[i32],
        supported_noise: &[i32],
        models_dir: &'static str,
        noise_style: NoiseStyle,
    ) -> Self {
        Self {
            name,
            display_name,
            cli_binary,
            supported_scales: supported_scales.to_vec(),
            supported_noise: supported_noise.to_vec(),
            models_dir,
            noise_style,
            model_names: Vec::new(),
            syncgap: -1,
        }
    }

    fn with_syncgap(mut self, syncgap: i32) -> Self {
        self.syncgap = syncgap;
        self
    }

    fn with_model_names(mut self, names: &[&'static str]) -> Self {
        self.model_names = names.to_vec();
        self
    }
}

/// Ordered registry of all supported models. Order matters — the UI numbers the
/// model switch buttons by position.
pub fn supported_models() -> &'static [ModelConfig] {
    static MODELS: OnceLock<Vec<ModelConfig>> = OnceLock::new();
    MODELS.get_or_init(|| {
        vec![
            // ── Waifu2x ──────────────────────────────────────────────
            ModelConfig::new(
                "waifu2x",
                "Waifu2x CUNet",
                "waifu2x-ncnn-vulkan",
                &[1, 2],
                &[0, 1, 2, 3],
                "models-cunet",
                NoiseStyle::Numeric,
            ),
            ModelConfig::new(
                "waifu2x-upconv7",
                "Waifu2x UpConv7",
                "waifu2x-ncnn-vulkan",
                &[2],
                &[0, 1, 2, 3],
                "models-upconv_7_anime_style_art_rgb",
                NoiseStyle::Numeric,
            ),
            ModelConfig::new(
                "waifu2x-photo",
                "Waifu2x Photo",
                "waifu2x-ncnn-vulkan",
                &[2],
                &[0, 1, 2, 3],
                "models-upconv_7_photo",
                NoiseStyle::Numeric,
            ),
            // ── Real-CUGAN ───────────────────────────────────────────
            ModelConfig::new(
                "realcugan-se",
                "Real-CUGAN SE",
                "realcugan-ncnn-vulkan",
                &[2, 3, 4],
                &[0, 1, 2, 3],
                "models-se",
                NoiseStyle::Cugan,
            )
            .with_syncgap(3),
            ModelConfig::new(
                "realcugan-pro",
                "Real-CUGAN Pro",
                "realcugan-ncnn-vulkan",
                &[2, 3],
                &[0, 3],
                "models-pro",
                NoiseStyle::Cugan,
            )
            .with_syncgap(3),
            ModelConfig::new(
                "realcugan-nose",
                "Real-CUGAN Nose",
                "realcugan-ncnn-vulkan",
                &[2],
                &[0],
                "models-nose",
                NoiseStyle::Cugan,
            )
            .with_syncgap(3),
            // ── Real-ESRGAN ──────────────────────────────────────────
            ModelConfig::new(
                "realesrgan-anime",
                "Real-ESRGAN Anime",
                "realesrgan-ncnn-vulkan",
                &[2, 3, 4],
                &[0],
                "models",
                NoiseStyle::Realesrgan,
            )
            .with_model_names(&["realesr-animevideov3", "realesrgan-x4plus-anime"]),
            ModelConfig::new(
                "realesrgan-photo",
                "Real-ESRGAN Photo",
                "realesrgan-ncnn-vulkan",
                &[4],
                &[0],
                "models",
                NoiseStyle::Realesrgan,
            )
            .with_model_names(&["realesrgan-x4plus"]),
        ]
    })
}

/// Look up a model definition by key.
pub fn get_model(name: &str) -> Option<&'static ModelConfig> {
    supported_models().iter().find(|m| m.name == name)
}

/// Comma-separated list of valid model keys (for error messages).
pub fn model_keys_csv() -> String {
    supported_models()
        .iter()
        .map(|m| m.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Display name for a Real-CUGAN noise level.
pub fn noise_name(noise: i32) -> &'static str {
    match noise {
        0 => "no-denoise",
        1 => "denoise1x",
        2 => "denoise2x",
        3 => "denoise3x",
        4 => "conservative",
        _ => "no-denoise",
    }
}

/// Runtime configuration for the upscaler server.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub default_model: String,
    pub default_scale: i32,
    pub default_noise: i32,
    pub binary_dir: String,
    pub max_size_mb: usize,
    pub gpu_id: i32,
    /// Start with oxipng lossless compression enabled. Default: false.
    pub compress_enabled: bool,
    /// oxipng preset level 0–6 for startup. Default: 2.
    pub compress_level: u8,
    /// Normalize NCNN output to clean 8-bit RGB/RGBA PNG before sending.
    /// Prevents `BitmapFactory returned null bitmap` on Android (Coil/BitmapFactory
    /// rejects 16-bit, palette, or grayscale+alpha PNGs that NCNN can emit).
    /// Default: true (strongly recommended).
    pub normalize_png: bool,
    /// Clamp upscaled output to WebP's 16383px per-dimension maximum, downscaling
    /// proportionally (Lanczos3) when exceeded. Prevents oversized pages from
    /// breaking WebP encoding on the client. Default: true.
    pub webp_compat: bool,
    /// Per-dimension clamp applied when `webp_compat` is on. Configurable down
    /// from the hard WebP container limit ([`crate::upscaler::WEBP_MAX_DIMENSION`])
    /// to trade a bit of resolution for smaller output. Default: the hard limit.
    pub webp_max_dimension: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8282,
            default_model: "realcugan-se".to_string(),
            default_scale: 2,
            default_noise: 0,
            binary_dir: "binaries".to_string(),
            max_size_mb: 20,
            gpu_id: -1, // -1 = auto-select first available GPU
            compress_enabled: false,
            compress_level: 2,
            normalize_png: true, // on by default — fixes Android BitmapFactory decoding
            webp_compat: true,   // on by default — clamps to WebP's 16383px limit
            webp_max_dimension: crate::upscaler::WEBP_MAX_DIMENSION,
        }
    }
}
