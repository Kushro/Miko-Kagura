//! Disk-cache accounting and cleanup for the temp directories the app creates.

use std::path::Path;

/// Prefix of every temp directory this app creates.
pub const CACHE_PREFIX: &str = "miko-kagura-";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CacheReport {
    /// Bytes used by the CURRENT session's UI history dir (before/after PNGs).
    pub session_bytes: u64,
    /// File count in the current session's UI history dir.
    pub session_files: u64,
    /// Bytes used by stale cache dirs (sesiones anteriores / huérfanos).
    pub stale_bytes: u64,
    /// Number of stale cache directories.
    pub stale_dirs: u64,
    /// File count across stale cache directories.
    pub stale_files: u64,
}

impl CacheReport {
    pub fn total_bytes(&self) -> u64 {
        self.session_bytes + self.stale_bytes
    }
}

/// Scan `std::env::temp_dir()` for this app's cache dirs.
pub fn scan_cache(current_ui_dir: Option<&Path>) -> CacheReport {
    let pid_prefix = current_pid_prefix();
    scan_in(&std::env::temp_dir(), current_ui_dir, &pid_prefix)
}

/// Delete stale cache dirs. Returns (bytes_freed, dirs_removed, errors).
pub fn clear_stale_cache(current_ui_dir: Option<&Path>) -> (u64, u64, Vec<String>) {
    let pid_prefix = current_pid_prefix();
    clear_stale_in(&std::env::temp_dir(), current_ui_dir, &pid_prefix)
}

/// Human-readable size: "0 B", "312 KB", "1.4 MB", "2.1 GB" (1024-based, 1
/// decimal para MB/GB, sin decimales para B/KB).
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{} KB", bytes / KB)
    } else {
        format!("{} B", bytes)
    }
}

// ── Internals (parametrized by base dir so tests can inject their own) ──────

fn current_pid_prefix() -> String {
    format!("{}{}-", CACHE_PREFIX, std::process::id())
}

/// A directory directly under `base` is a candidate cache dir if its name
/// starts with `CACHE_PREFIX`. It is classified as:
/// - excluded entirely: name starts with `current_pid_prefix` (in-flight job
///   dirs of this very process).
/// - session: path equals `current_ui_dir`.
/// - stale: everything else matching the prefix.
fn scan_in(base: &Path, current_ui_dir: Option<&Path>, current_pid_prefix: &str) -> CacheReport {
    let mut report = CacheReport::default();

    let entries = match std::fs::read_dir(base) {
        Ok(e) => e,
        Err(_) => return report,
    };

    for entry in entries.flatten() {
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !file_type.is_dir() {
            continue;
        }

        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };

        if !name.starts_with(CACHE_PREFIX) {
            continue;
        }

        if name.starts_with(current_pid_prefix) {
            // In-flight job dir of this process: excluded entirely.
            continue;
        }

        let path = entry.path();
        let (bytes, files) = dir_size(&path);

        if Some(path.as_path()) == current_ui_dir {
            report.session_bytes += bytes;
            report.session_files += files;
        } else {
            report.stale_bytes += bytes;
            report.stale_files += files;
            report.stale_dirs += 1;
        }
    }

    report
}

fn clear_stale_in(
    base: &Path,
    current_ui_dir: Option<&Path>,
    current_pid_prefix: &str,
) -> (u64, u64, Vec<String>) {
    let mut bytes_freed = 0u64;
    let mut dirs_removed = 0u64;
    let mut errors = Vec::new();

    let entries = match std::fs::read_dir(base) {
        Ok(e) => e,
        Err(e) => {
            errors.push(format!("failed to read {}: {e}", base.display()));
            return (bytes_freed, dirs_removed, errors);
        }
    };

    for entry in entries.flatten() {
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !file_type.is_dir() {
            continue;
        }

        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };

        if !name.starts_with(CACHE_PREFIX) {
            continue;
        }

        if name.starts_with(current_pid_prefix) {
            continue;
        }

        let path = entry.path();

        if Some(path.as_path()) == current_ui_dir {
            continue;
        }

        // Safety: only ever remove a path we just derived from read_dir(base)
        // whose file_name() starts with the literal CACHE_PREFIX, never `base`
        // itself and never a string-concatenated path.
        let (bytes, _files) = dir_size(&path);
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                bytes_freed += bytes;
                dirs_removed += 1;
            }
            Err(e) => {
                errors.push(format!("failed to remove {}: {e}", path.display()));
            }
        }
    }

    (bytes_freed, dirs_removed, errors)
}

/// Recursively measure a directory's total file size and file count.
/// Ignores symlinks. IO errors on unreadable entries are silently skipped.
fn dir_size(path: &Path) -> (u64, u64) {
    let mut bytes = 0u64;
    let mut files = 0u64;

    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return (bytes, files),
    };

    for entry in entries.flatten() {
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if file_type.is_symlink() {
            continue;
        } else if file_type.is_dir() {
            let (sub_bytes, sub_files) = dir_size(&entry.path());
            bytes += sub_bytes;
            files += sub_files;
        } else if file_type.is_file() {
            if let Ok(meta) = entry.metadata() {
                bytes += meta.len();
                files += 1;
            }
        }
    }

    (bytes, files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A self-contained fixture rooted at a unique subdir of the real temp
    /// dir, so tests never touch %TEMP% (or its siblings) directly and can be
    /// torn down with a single, exactly-targeted remove_dir_all.
    struct Fixture {
        base: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            // A timestamp is not unique enough here: Windows' system clock only
            // ticks about every 15ms, so two tests starting in the same tick
            // would share a fixture directory and clobber each other's counts.
            // A counter guarantees one directory per fixture.
            static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let base = std::env::temp_dir().join(format!(
                "miko-kagura-cache-rs-test-{}-{}",
                std::process::id(),
                seq
            ));
            std::fs::create_dir_all(&base).expect("create fixture base");
            Fixture { base }
        }

        fn mkdir(&self, name: &str) -> PathBuf {
            let p = self.base.join(name);
            std::fs::create_dir_all(&p).expect("create fixture subdir");
            p
        }

        fn write_file(&self, dir: &Path, name: &str, contents: &[u8]) {
            std::fs::write(dir.join(name), contents).expect("write fixture file");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Safety check: only ever remove the exact fixture base we created
            // above, under the real temp dir, never a derived/concatenated path.
            let base = &self.base;
            debug_assert!(base
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("miko-kagura-cache-rs-test-")));
            let _ = std::fs::remove_dir_all(base);
        }
    }

    #[test]
    fn scan_classifies_session_stale_pid_and_foreign_dirs() {
        let fx = Fixture::new();
        let pid_prefix = format!("{}{}-", CACHE_PREFIX, std::process::id());

        // Stale dir from a previous session.
        let stale_dir = fx.mkdir("miko-kagura-ui-1111");
        fx.write_file(&stale_dir, "a_before.png", &[0u8; 100]);
        fx.write_file(&stale_dir, "a_after.png", &[0u8; 50]);

        // Another stale dir (orphaned upscaler working dir from a dead pid).
        let stale_dir2 = fx.mkdir("miko-kagura-9999-2222");
        fx.write_file(&stale_dir2, "in.png", &[0u8; 30]);

        // Current session's UI history dir.
        let session_dir = fx.mkdir("miko-kagura-ui-3333");
        fx.write_file(&session_dir, "b_before.png", &[0u8; 10]);
        fx.write_file(&session_dir, "b_after.png", &[0u8; 20]);
        fx.write_file(&session_dir, "b_compare.png", &[0u8; 5]);

        // In-flight job dir of the current process: excluded entirely.
        let current_pid = std::process::id();
        let pid_dir = fx.mkdir(&format!("miko-kagura-{current_pid}-4444"));
        fx.write_file(&pid_dir, "job.png", &[0u8; 999]);

        // Unrelated app's dir: must be ignored.
        let foreign_dir = fx.mkdir("otherapp-x");
        fx.write_file(&foreign_dir, "whatever.bin", &[0u8; 500]);

        let report = scan_in(&fx.base, Some(&session_dir), &pid_prefix);

        assert_eq!(report.session_bytes, 35);
        assert_eq!(report.session_files, 3);
        assert_eq!(report.stale_bytes, 180);
        assert_eq!(report.stale_files, 3);
        assert_eq!(report.stale_dirs, 2);
        assert_eq!(report.total_bytes(), 215);
    }

    #[test]
    fn scan_with_no_current_ui_dir_treats_ui_dirs_as_stale() {
        let fx = Fixture::new();
        let pid_prefix = format!("{}{}-", CACHE_PREFIX, std::process::id());

        let dir = fx.mkdir("miko-kagura-ui-5555");
        fx.write_file(&dir, "x.png", &[0u8; 42]);

        let report = scan_in(&fx.base, None, &pid_prefix);

        assert_eq!(report.session_bytes, 0);
        assert_eq!(report.session_files, 0);
        assert_eq!(report.stale_bytes, 42);
        assert_eq!(report.stale_dirs, 1);
        assert_eq!(report.stale_files, 1);
    }

    #[test]
    fn clear_stale_removes_only_stale_dirs_and_reports_totals() {
        let fx = Fixture::new();
        let pid_prefix = format!("{}{}-", CACHE_PREFIX, std::process::id());

        let stale_dir = fx.mkdir("miko-kagura-ui-6666");
        fx.write_file(&stale_dir, "a.png", &[0u8; 64]);

        let session_dir = fx.mkdir("miko-kagura-ui-7777");
        fx.write_file(&session_dir, "keep.png", &[0u8; 16]);

        let current_pid = std::process::id();
        let pid_dir = fx.mkdir(&format!("miko-kagura-{current_pid}-8888"));
        fx.write_file(&pid_dir, "job.png", &[0u8; 8]);

        let foreign_dir = fx.mkdir("otherapp-y");
        fx.write_file(&foreign_dir, "keep2.bin", &[0u8; 4]);

        let (bytes_freed, dirs_removed, errors) =
            clear_stale_in(&fx.base, Some(&session_dir), &pid_prefix);

        assert_eq!(bytes_freed, 64);
        assert_eq!(dirs_removed, 1);
        assert!(errors.is_empty());

        assert!(!stale_dir.exists());
        assert!(session_dir.exists(), "current session dir must survive");
        assert!(pid_dir.exists(), "in-flight pid dir must survive");
        assert!(foreign_dir.exists(), "foreign dir must survive");
    }

    #[test]
    fn clear_stale_with_no_current_ui_dir_removes_all_matching() {
        let fx = Fixture::new();
        let pid_prefix = format!("{}{}-", CACHE_PREFIX, std::process::id());

        let dir_a = fx.mkdir("miko-kagura-ui-1010");
        fx.write_file(&dir_a, "a.png", &[0u8; 12]);
        let dir_b = fx.mkdir("miko-kagura-2020-3030");
        fx.write_file(&dir_b, "b.png", &[0u8; 8]);

        let (bytes_freed, dirs_removed, errors) = clear_stale_in(&fx.base, None, &pid_prefix);

        assert_eq!(bytes_freed, 20);
        assert_eq!(dirs_removed, 2);
        assert!(errors.is_empty());
        assert!(!dir_a.exists());
        assert!(!dir_b.exists());
    }

    #[test]
    fn format_bytes_thresholds() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1 KB");
        assert_eq!(format_bytes(1024 * 5), "5 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes((1024.0 * 1024.0 * 1.4) as u64), "1.4 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(
            format_bytes((1024.0 * 1024.0 * 1024.0 * 2.1) as u64),
            "2.1 GB"
        );
    }

    #[test]
    fn dir_size_is_recursive() {
        let fx = Fixture::new();
        let root = fx.mkdir("miko-kagura-ui-nested");
        fx.write_file(&root, "top.png", &[0u8; 10]);
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("deep.png"), [0u8; 7]).unwrap();

        let (bytes, files) = dir_size(&root);
        assert_eq!(bytes, 17);
        assert_eq!(files, 2);
    }
}
