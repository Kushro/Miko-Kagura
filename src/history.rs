//! Request-history capture for the dashboard.
//!
//! Stores the before/after bytes of each upscale request to a temp directory so
//! the UI can offer a before/after comparison in an external image viewer. Port of
//! the Python `history.py`.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ab_glyph::{FontVec, PxScale};
use image::{Rgb, RgbImage};

const DEFAULT_MAX_ENTRIES: usize = 50;

/// Default byte budget for the session history dir (1 GiB). When exceeded, the
/// oldest records are evicted (files deleted) until the history fits again.
pub const DEFAULT_MAX_BYTES: u64 = 1024 * 1024 * 1024;

/// Which endpoint produced a request — surfaced as a column in the dashboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestSource {
    /// `POST /upscale` — single raw image.
    Single,
    /// `POST /upscale/batch` — one image within a base64 batch.
    Batch,
    /// `POST /upscale/url` — single server-downloaded image.
    SingleUrl,
    /// `POST /upscale/batch/url` — one image within a downloaded URL batch.
    BatchUrl,
}

impl RequestSource {
    /// Short human label for the dashboard "Source" column.
    pub fn label(self) -> &'static str {
        match self {
            RequestSource::Single => "single",
            RequestSource::Batch => "batch",
            RequestSource::SingleUrl => "single uri",
            RequestSource::BatchUrl => "batch uri",
        }
    }
}

/// One captured upscale request, with on-disk before/after images.
#[derive(Clone, Debug, PartialEq)]
pub struct RequestRecord {
    pub id: u64,
    /// Wall-clock time-of-day label (HH:MM:SS, UTC) captured at insertion.
    pub time_label: String,
    /// Endpoint that produced this request.
    pub source: RequestSource,
    /// Batch grouping id — `Some` for batch endpoints, `None` for singles.
    /// Entries sharing a `batch_id` were processed as one client batch.
    pub batch_id: Option<u64>,
    pub model: String,
    pub scale: i32,
    pub noise: i32,
    pub size_in: usize,
    pub size_out: usize,
    pub time_ms: u64,
    pub width_in: u32,
    pub height_in: u32,
    pub width_out: u32,
    pub height_out: u32,
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub compare_path: Option<PathBuf>,
    /// Time spent on oxipng lossless compression, if it was applied.
    pub compress_ms: Option<u64>,
    /// Time spent on the WebP-compat Lanczos3 downscale, if it was applied.
    pub webp_ms: Option<u64>,
    /// Source image URL for the URL endpoints (`Some` only for SingleUrl/BatchUrl).
    /// Lets the dashboard open the origin to verify what the server downloaded.
    pub origin_url: Option<String>,
}

impl RequestRecord {
    /// Effective upscale ratio derived from the actual decoded dimensions.
    pub fn scale_factor(&self) -> f64 {
        if self.width_in > 0 {
            self.width_out as f64 / self.width_in as f64
        } else {
            0.0
        }
    }
}

/// Capped store of recent upscale requests backed by temp files.
pub struct RequestHistory {
    pub max_entries: usize,
    /// Byte budget for the session dir; oldest records are evicted past it.
    max_bytes: u64,
    /// Running total of bytes tracked on disk (before/after/compare files).
    total_bytes: u64,
    records: std::collections::HashMap<u64, RequestRecord>,
    order: VecDeque<u64>,
    counter: u64,
    dir: Option<PathBuf>,
}

impl Default for RequestHistory {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_ENTRIES)
    }
}

impl RequestHistory {
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries,
            max_bytes: DEFAULT_MAX_BYTES,
            total_bytes: 0,
            records: std::collections::HashMap::new(),
            order: VecDeque::new(),
            counter: 0,
            dir: None,
        }
    }

    /// Current byte budget for the session history.
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Bytes currently tracked on disk by this history.
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// Change the byte budget, immediately evicting the oldest records until
    /// the history fits. Returns (bytes_freed, records_evicted).
    pub fn set_max_bytes(&mut self, max_bytes: u64) -> (u64, u64) {
        self.max_bytes = max_bytes;
        let mut bytes_freed = 0u64;
        let mut evicted = 0u64;
        // Always keep the newest record, even if it alone exceeds the budget.
        while self.order.len() > 1 && self.total_bytes > self.max_bytes {
            bytes_freed += self.evict_oldest();
            evicted += 1;
        }
        (bytes_freed, evicted)
    }

    /// Remove the oldest record, deleting its files. Returns the bytes freed.
    fn evict_oldest(&mut self) -> u64 {
        let Some(old_id) = self.order.pop_front() else {
            return 0;
        };
        let Some(old) = self.records.remove(&old_id) else {
            return 0;
        };
        let mut freed = 0u64;
        for path in [
            Some(&old.input_path),
            Some(&old.output_path),
            old.compare_path.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if let Ok(meta) = std::fs::metadata(path) {
                freed += meta.len();
            }
            safe_unlink(Some(path));
        }
        self.total_bytes = self.total_bytes.saturating_sub(freed);
        freed
    }

    /// Lazily create the temp directory the first time it is needed.
    fn dir(&mut self) -> PathBuf {
        if self.dir.is_none() {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let d = std::env::temp_dir().join(format!("{}ui-{nanos}", crate::cache::CACHE_PREFIX));
            let _ = std::fs::create_dir_all(&d);
            self.dir = Some(d);
        }
        self.dir.clone().unwrap()
    }

    /// Persist a request's before/after images and return its record.
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        input_bytes: &[u8],
        output_bytes: &[u8],
        model: &str,
        scale: i32,
        noise: i32,
        time_ms: u64,
        compress_ms: Option<u64>,
        webp_ms: Option<u64>,
        source: RequestSource,
        batch_id: Option<u64>,
        origin_url: Option<String>,
    ) -> RequestRecord {
        self.counter += 1;
        let rid = self.counter;

        let (w_in, h_in) = image_size(input_bytes);
        let (w_out, h_out) = image_size(output_bytes);

        let dir = self.dir();
        let in_path = dir.join(format!("{rid:06}_before.png"));
        let out_path = dir.join(format!("{rid:06}_after.png"));
        let _ = std::fs::write(&in_path, input_bytes);
        let _ = std::fs::write(&out_path, output_bytes);

        let record = RequestRecord {
            id: rid,
            time_label: time_of_day_label(),
            source,
            batch_id,
            model: model.to_string(),
            scale,
            noise,
            size_in: input_bytes.len(),
            size_out: output_bytes.len(),
            time_ms,
            width_in: w_in,
            height_in: h_in,
            width_out: w_out,
            height_out: h_out,
            input_path: in_path,
            output_path: out_path,
            compare_path: None,
            compress_ms,
            webp_ms,
            origin_url,
        };

        self.records.insert(rid, record.clone());
        self.order.push_back(rid);
        self.total_bytes += (input_bytes.len() + output_bytes.len()) as u64;

        // Evict oldest records past either cap (entry count or byte budget),
        // always keeping at least the record just added.
        while self.order.len() > 1
            && (self.order.len() > self.max_entries || self.total_bytes > self.max_bytes)
        {
            self.evict_oldest();
        }

        record
    }

    pub fn get(&self, rid: u64) -> Option<RequestRecord> {
        self.records.get(&rid).cloned()
    }

    /// Path of the current session's history dir, if it has been created.
    pub fn current_dir(&self) -> Option<PathBuf> {
        self.dir.clone()
    }

    /// Delete every captured file and the session dir, returning (bytes_freed, files_removed).
    /// Resets records/order and clears `dir` so it is lazily recreated on next use. Keeps `counter`.
    pub fn clear_session(&mut self) -> (u64, u64) {
        let mut bytes_freed = 0u64;
        let mut files_removed = 0u64;

        for rec in self.records.values() {
            for path in [
                Some(&rec.input_path),
                Some(&rec.output_path),
                rec.compare_path.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                if let Ok(meta) = std::fs::metadata(path) {
                    bytes_freed += meta.len();
                    files_removed += 1;
                }
                let _ = std::fs::remove_file(path);
            }
        }

        self.records.clear();
        self.order.clear();
        self.total_bytes = 0;

        if let Some(dir) = self.dir.take() {
            // Account for any residual files left in the session dir that
            // weren't tracked by a record (e.g. from a prior partial cleanup).
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        if meta.is_file() {
                            bytes_freed += meta.len();
                            files_removed += 1;
                        }
                    }
                }
            }
            // Safety: remove_dir_all is called only on `dir`, the exact path
            // this struct created and stored — never on a derived or
            // string-concatenated path.
            let _ = std::fs::remove_dir_all(&dir);
        }

        (bytes_freed, files_removed)
    }

    /// Record a built comparison path so repeated opens reuse the file.
    /// Newly tracked comparison files count toward the byte budget.
    pub fn set_compare_path(&mut self, rid: u64, path: PathBuf) {
        let Some(rec) = self.records.get_mut(&rid) else {
            return;
        };
        if rec.compare_path.as_deref() == Some(path.as_path()) {
            return; // already tracked — don't double-count
        }
        let old = rec.compare_path.replace(path.clone());
        if let Some(old) = old {
            if let Ok(meta) = std::fs::metadata(&old) {
                self.total_bytes = self.total_bytes.saturating_sub(meta.len());
            }
        }
        if let Ok(meta) = std::fs::metadata(&path) {
            self.total_bytes += meta.len();
        }
    }

    /// Point the history at an injected directory so tests never touch the
    /// real per-session temp dir.
    #[cfg(test)]
    fn set_dir_for_tests(&mut self, d: PathBuf) {
        let _ = std::fs::create_dir_all(&d);
        self.dir = Some(d);
    }

    /// Delete every captured file and the temp directory. Call on app exit.
    pub fn cleanup(&mut self) {
        for rec in self.records.values() {
            safe_unlink(Some(&rec.input_path));
            safe_unlink(Some(&rec.output_path));
            safe_unlink(rec.compare_path.as_deref());
        }
        self.records.clear();
        self.order.clear();
        self.total_bytes = 0;
        if let Some(dir) = self.dir.take() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

// ── Free helpers ──────────────────────────────────────────────────────

/// Read width/height from image bytes. (0, 0) on failure.
fn image_size(data: &[u8]) -> (u32, u32) {
    match image::load_from_memory(data) {
        Ok(img) => (img.width(), img.height()),
        Err(_) => (0, 0),
    }
}

fn safe_unlink(path: Option<&Path>) {
    if let Some(p) = path {
        let _ = std::fs::remove_file(p);
    }
}

/// Format the current time-of-day as HH:MM:SS (UTC, no external deps).
fn time_of_day_label() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = secs % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// Try to load a readable TrueType font for comparison labels.
fn load_font() -> Option<FontVec> {
    let candidates: &[&str] = &[
        r"C:\Windows\Fonts\arial.ttf",
        r"C:\Windows\Fonts\segoeui.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/Library/Fonts/Arial.ttf",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(font) = FontVec::try_from_vec(bytes) {
                return Some(font);
            }
        }
    }
    None
}

/// Compose a labeled, full-resolution side-by-side before|after PNG.
///
/// Returns the path, or None if the source images could not be read. Labels are
/// drawn only when a system font is available; otherwise the layout is unlabeled.
pub fn build_comparison(record: &RequestRecord) -> Option<PathBuf> {
    if let Some(existing) = &record.compare_path {
        if existing.exists() {
            return Some(existing.clone());
        }
    }

    let before = image::open(&record.input_path).ok()?.to_rgb8();
    let after = image::open(&record.output_path).ok()?.to_rgb8();

    let gap: u32 = 24;
    let label_h: u32 = 56;
    let bg = Rgb([24u8, 24, 28]);
    let content_h = before.height().max(after.height());
    let width = before.width() + after.width() + gap * 3;
    let height = content_h + label_h + gap * 2;

    let mut canvas = RgbImage::from_pixel(width, height, bg);

    let before_x = gap;
    let after_x = before.width() + gap * 2;
    let content_y = label_h + gap;

    // Vertically center each image within the shared content band.
    overlay(
        &mut canvas,
        &before,
        before_x,
        content_y + (content_h - before.height()) / 2,
    );
    overlay(
        &mut canvas,
        &after,
        after_x,
        content_y + (content_h - after.height()) / 2,
    );

    if let Some(font) = load_font() {
        let scale = PxScale::from(32.0);
        let before_label = format!("BEFORE  {}x{}", before.width(), before.height());
        let after_label = format!(
            "AFTER  {}x{}  ({:.2}x)  {} n{}",
            after.width(),
            after.height(),
            record.scale_factor(),
            record.model,
            record.noise
        );
        imageproc::drawing::draw_text_mut(
            &mut canvas,
            Rgb([220u8, 220, 220]),
            before_x as i32,
            gap as i32,
            scale,
            &font,
            &before_label,
        );
        imageproc::drawing::draw_text_mut(
            &mut canvas,
            Rgb([150u8, 230, 150]),
            after_x as i32,
            gap as i32,
            scale,
            &font,
            &after_label,
        );
    }

    let out_path = record
        .input_path
        .with_file_name(format!("{:06}_compare.png", record.id));
    canvas.save(&out_path).ok()?;
    Some(out_path)
}

/// Paste `src` onto `dst` at (x, y).
fn overlay(dst: &mut RgbImage, src: &RgbImage, x: u32, y: u32) {
    image::imageops::overlay(dst, src, x as i64, y as i64);
}

/// Open a file in the OS default image viewer (non-blocking).
pub fn open_in_viewer(path: &Path) {
    open_with_default_app(&path.to_string_lossy());
}

/// Open a URL in the OS default browser (non-blocking).
pub fn open_url(url: &str) {
    // Only hand real http(s) URLs to the shell.
    if url.starts_with("http://") || url.starts_with("https://") {
        open_with_default_app(url);
    }
}

/// Shell-open any target (file path or URL) with the OS default handler.
fn open_with_default_app(target: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // raw_arg with explicit quotes: URLs contain `&`, which cmd would treat as a
        // command separator when the argument is passed unquoted.
        let _ = std::process::Command::new("cmd")
            .raw_arg(format!("/C start \"\" \"{}\"", target.replace('"', "")))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(target).spawn();
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique per-test base dir under the real temp dir; the history's session
    /// dir is injected inside it so no real per-session dir is ever touched.
    struct Fixture {
        base: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let base = std::env::temp_dir().join(format!(
                "miko-kagura-history-rs-test-{}-{}",
                std::process::id(),
                nanos
            ));
            std::fs::create_dir_all(&base).expect("create fixture base");
            Fixture { base }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Safety check: only ever remove the exact fixture base created above.
            debug_assert!(self
                .base
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("miko-kagura-history-rs-test-")));
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    fn add_payload(h: &mut RequestHistory, in_len: usize, out_len: usize) -> RequestRecord {
        h.add(
            &vec![0u8; in_len],
            &vec![0u8; out_len],
            "test-model",
            2,
            0,
            1,
            None,
            None,
            RequestSource::Single,
            None,
            None,
        )
    }

    #[test]
    fn byte_budget_evicts_oldest_records() {
        let fx = Fixture::new();
        let mut h = RequestHistory::new(50);
        h.set_dir_for_tests(fx.base.join("session"));
        h.set_max_bytes(250);

        let a = add_payload(&mut h, 60, 40); // 100 bytes
        let b = add_payload(&mut h, 60, 40); // 200 bytes
        assert_eq!(h.total_bytes(), 200);
        assert!(a.input_path.exists());

        let c = add_payload(&mut h, 60, 40); // 300 > 250 → evict `a`
        assert!(!a.input_path.exists());
        assert!(!a.output_path.exists());
        assert!(b.input_path.exists());
        assert!(c.input_path.exists());
        assert_eq!(h.total_bytes(), 200);
        assert!(h.get(a.id).is_none());
        assert!(h.get(b.id).is_some());
    }

    #[test]
    fn shrinking_budget_evicts_immediately_but_keeps_newest() {
        let fx = Fixture::new();
        let mut h = RequestHistory::new(50);
        h.set_dir_for_tests(fx.base.join("session"));
        h.set_max_bytes(1000);

        let a = add_payload(&mut h, 50, 50);
        let b = add_payload(&mut h, 50, 50);

        let (freed, evicted) = h.set_max_bytes(150);
        assert_eq!(evicted, 1);
        assert_eq!(freed, 100);
        assert!(!a.input_path.exists());
        assert!(b.input_path.exists());

        // A budget smaller than the single remaining record still keeps it.
        let (_, evicted) = h.set_max_bytes(10);
        assert_eq!(evicted, 0);
        assert!(b.input_path.exists());
        assert_eq!(h.total_bytes(), 100);
    }

    #[test]
    fn entry_cap_still_applies() {
        let fx = Fixture::new();
        let mut h = RequestHistory::new(2);
        h.set_dir_for_tests(fx.base.join("session"));

        let a = add_payload(&mut h, 10, 10);
        let _b = add_payload(&mut h, 10, 10);
        let _c = add_payload(&mut h, 10, 10);
        assert!(!a.input_path.exists());
        assert_eq!(h.total_bytes(), 40);
    }

    #[test]
    fn clear_session_resets_byte_tracking() {
        let fx = Fixture::new();
        let mut h = RequestHistory::new(50);
        h.set_dir_for_tests(fx.base.join("session"));

        add_payload(&mut h, 30, 70);
        assert_eq!(h.total_bytes(), 100);

        let (freed, files) = h.clear_session();
        assert_eq!(freed, 100);
        assert_eq!(files, 2);
        assert_eq!(h.total_bytes(), 0);
    }
}
