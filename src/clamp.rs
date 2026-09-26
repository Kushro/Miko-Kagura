//! Turning a device's physical screen into a sensible per-dimension clamp.
//!
//! The raw pixel slider stays the source of truth — this module only computes a
//! *recommended* pixel value from a screen diagonal and pixel density, so the
//! clamp can be reasoned about in device terms ("6.7 inch phone at 480 dpi")
//! instead of a bare number. Every intermediate value is exposed so the UI can
//! show the arithmetic rather than hide it behind a magic result.

use crate::upscaler::WEBP_MAX_DIMENSION;

/// Lower bound of the pixel slider (also the floor of any computed clamp).
pub const CLAMP_MIN_PX: u32 = 256;

/// Zoom headroom presets, in percent of the screen's long edge.
///
/// A page rendered exactly at the screen's long edge looks correct until the
/// reader pinch-zooms, at which point there are no spare pixels. 200% is the
/// sweet spot for manga: sharp at 2x zoom without doubling the payload again.
pub const HEADROOM_PRESETS: &[(u32, &str)] = &[
    (100, "Fit"),
    (150, "Light"),
    (200, "Balanced"),
    (300, "Pixel-peep"),
];

pub const DEFAULT_HEADROOM_PCT: u32 = 200;
pub const HEADROOM_PCT_RANGE: (u32, u32) = (100, 400);

/// A common device shape used as a notch on the inches/DPI sliders.
pub struct DeviceProfile {
    pub label: &'static str,
    /// Screen diagonal in tenths of an inch (61 = 6.1 inch).
    pub inches_tenths: u32,
    pub dpi: u32,
    pub note: &'static str,
}

/// Notched presets, ordered by screen size — the UI draws them under the slider.
pub const DEVICE_PROFILES: &[DeviceProfile] = &[
    DeviceProfile {
        label: "Compact",
        inches_tenths: 54,
        dpi: 476,
        note: "5.4\" phone, e.g. iPhone 13 mini / Zenfone",
    },
    DeviceProfile {
        label: "Phone",
        inches_tenths: 61,
        dpi: 460,
        note: "6.1\" flagship, e.g. Pixel 8 / iPhone 16",
    },
    DeviceProfile {
        label: "Phablet",
        inches_tenths: 67,
        dpi: 480,
        note: "6.7\" large phone, e.g. Galaxy S/Ultra, Pixel Pro",
    },
    DeviceProfile {
        label: "Reader",
        inches_tenths: 78,
        dpi: 300,
        note: "7.8\" e-ink reader, e.g. Boox / Kobo Libra",
    },
    DeviceProfile {
        label: "Tablet",
        inches_tenths: 110,
        dpi: 264,
        note: "11\" tablet, e.g. iPad Pro / Tab S9",
    },
    DeviceProfile {
        label: "Big tablet",
        inches_tenths: 129,
        dpi: 264,
        note: "12.9\" tablet — closest to print page size",
    },
];

/// Aspect ratio (long:short) assumed for a given diagonal.
///
/// Phones are tall and narrow, tablets and e-readers are closer to paper. The
/// ratio only shifts the split between long and short edge; both are shown in
/// the UI so a wrong guess is visible rather than silent.
pub fn aspect_ratio_for(inches_tenths: u32) -> (u32, u32) {
    match inches_tenths {
        0..=69 => (195, 90),   // 19.5:9 — modern phone
        70..=89 => (43, 32),   // 4:3-ish — e-ink readers
        _ => (16, 10),         // 16:10 — tablets
    }
}

/// The full derivation from device dimensions to a clamp value, so the UI can
/// display each step instead of just the answer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClampBreakdown {
    pub inches_tenths: u32,
    pub dpi: u32,
    pub headroom_pct: u32,
    /// Diagonal in pixels: inches × dpi.
    pub diagonal_px: u32,
    pub short_edge_px: u32,
    pub long_edge_px: u32,
    pub aspect: (u32, u32),
    /// Long edge × headroom, before the hard-limit clamp.
    pub raw_clamp_px: u32,
    /// Final value written to the clamp — `raw_clamp_px` bounded to the slider
    /// range. Equal to `raw_clamp_px` unless clamped.
    pub clamp_px: u32,
    /// True when the WebP container limit, not the device, decided the value.
    pub hit_container_limit: bool,
}

impl ClampBreakdown {
    /// "6.7\" @ 480dpi" — compact device label.
    pub fn device_label(&self) -> String {
        format!(
            "{}.{}\" @ {}dpi",
            self.inches_tenths / 10,
            self.inches_tenths % 10,
            self.dpi
        )
    }

    /// "1484 × 3216 px (19.5:9)" — the derived screen resolution.
    pub fn resolution_label(&self) -> String {
        let (a, b) = self.aspect;
        // Ratios are stored scaled (195:90) to keep them integral; print the
        // reduced decimal form the user recognises from spec sheets.
        let trim = |v: u32| {
            if a >= 100 || b >= 100 {
                format!("{:.1}", v as f64 / 10.0)
                    .trim_end_matches(".0")
                    .to_string()
            } else {
                v.to_string()
            }
        };
        format!(
            "{} × {} px ({}:{})",
            self.short_edge_px,
            self.long_edge_px,
            trim(a),
            trim(b)
        )
    }
}

/// Compute the recommended clamp for a device. Inputs are clamped to their
/// valid ranges, so out-of-range UI state can never produce a nonsense result.
pub fn compute(inches_tenths: u32, dpi: u32, headroom_pct: u32) -> ClampBreakdown {
    let inches_tenths = inches_tenths.clamp(
        crate::settings::DEVICE_INCHES_TENTHS_RANGE.0,
        crate::settings::DEVICE_INCHES_TENTHS_RANGE.1,
    );
    let dpi = dpi.clamp(
        crate::settings::DEVICE_DPI_RANGE.0,
        crate::settings::DEVICE_DPI_RANGE.1,
    );
    let headroom_pct = headroom_pct.clamp(HEADROOM_PCT_RANGE.0, HEADROOM_PCT_RANGE.1);

    let aspect = aspect_ratio_for(inches_tenths);
    let ratio = aspect.0 as f64 / aspect.1 as f64; // long / short

    let diagonal_px = inches_tenths as f64 / 10.0 * dpi as f64;
    let hypot = (1.0 + ratio * ratio).sqrt();
    let short_edge_px = (diagonal_px / hypot).round() as u32;
    let long_edge_px = (diagonal_px * ratio / hypot).round() as u32;

    let raw_clamp_px = (long_edge_px as f64 * headroom_pct as f64 / 100.0).round() as u32;
    let clamp_px = raw_clamp_px.clamp(CLAMP_MIN_PX, WEBP_MAX_DIMENSION);

    ClampBreakdown {
        inches_tenths,
        dpi,
        headroom_pct,
        diagonal_px: diagonal_px.round() as u32,
        short_edge_px,
        long_edge_px,
        aspect,
        raw_clamp_px,
        clamp_px,
        hit_container_limit: raw_clamp_px > WEBP_MAX_DIMENSION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phablet_at_default_headroom() {
        let b = compute(67, 480, 200);
        // 6.7" × 480dpi = 3216px diagonal on a 19.5:9 panel, which lands within
        // a few percent of the 1440×3120 such phones actually ship.
        assert_eq!(b.diagonal_px, 3216);
        assert_eq!(b.short_edge_px, 1348);
        assert_eq!(b.long_edge_px, 2920);
        assert_eq!(b.clamp_px, 5840);
        assert!(!b.hit_container_limit);
    }

    #[test]
    fn fit_headroom_matches_long_edge() {
        let b = compute(61, 460, 100);
        assert_eq!(b.clamp_px, b.long_edge_px);
    }

    #[test]
    fn container_limit_caps_extreme_input() {
        let b = compute(200, 680, 400);
        assert_eq!(b.clamp_px, WEBP_MAX_DIMENSION);
        assert!(b.hit_container_limit);
    }

    #[test]
    fn out_of_range_inputs_are_clamped_not_wrapped() {
        let b = compute(1, 1, 1);
        assert_eq!(b.inches_tenths, crate::settings::DEVICE_INCHES_TENTHS_RANGE.0);
        assert_eq!(b.dpi, crate::settings::DEVICE_DPI_RANGE.0);
        assert_eq!(b.headroom_pct, HEADROOM_PCT_RANGE.0);
        assert!(b.clamp_px >= CLAMP_MIN_PX);
    }

    #[test]
    fn every_profile_yields_a_usable_clamp() {
        for p in DEVICE_PROFILES {
            let b = compute(p.inches_tenths, p.dpi, DEFAULT_HEADROOM_PCT);
            assert!(
                b.clamp_px >= CLAMP_MIN_PX && b.clamp_px <= WEBP_MAX_DIMENSION,
                "{} produced {}px",
                p.label,
                b.clamp_px
            );
        }
    }
}
