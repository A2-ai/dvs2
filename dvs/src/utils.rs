use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Result, bail};

const DEFAULT_THREADS_PER_CPU: usize = 4;
/// Maximum thread count threshold when `DVS_NUM_THREADS` is unset
const DEFAULT_MAX_THREADS: usize = 16;
/// Maximum thread count threshold if `DVS_NUM_THREADS` is set
const ENV_MAX_THREADS: usize = 32;

/// Global thread count override. 0 = unset (use env var or default).
static NUM_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Set the number of threads for DVS parallel operations.
/// Pass 0 to clear (revert to env var / automatic detection).
pub fn set_num_threads(n: usize) {
    NUM_THREADS.store(n, Ordering::Relaxed);
}

/// Returns the configured thread count, or `None` if unset.
pub(crate) fn get_num_threads() -> Option<usize> {
    match NUM_THREADS.load(Ordering::Relaxed) {
        0 => None,
        n => Some(n),
    }
}

const KB: u64 = 1_024;
const MB: u64 = 1_024 * 1_024;
const GB: u64 = 1_024 * 1_024 * 1_024;
const TB: u64 = 1_024 * 1_024 * 1_024 * 1_024;

/// Formats a byte count into a human-readable string (e.g. "10.5 MB", "3 MB").
/// Uses base-1024 divisors to match `ls -h` / `du -h` output.
///
/// - Whole numbers are rendered without a trailing `.0` (`1 MB`, not `1.0 MB`).
/// - Values that would round to `1024.0 <unit>` promote to the next unit
///   (`1048575 B` → `1 MB`, never `1024.0 KB`).
pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];

    if bytes < KB {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut idx = 0;
    while value >= 1024.0 && idx + 1 < UNITS.len() {
        value /= 1024.0;
        idx += 1;
    }

    // If rounding to one decimal would hit 1024.0, promote to the next unit.
    let rounded = (value * 10.0).round() / 10.0;
    if rounded >= 1024.0 && idx + 1 < UNITS.len() {
        value = rounded / 1024.0;
        idx += 1;
    }

    if ((value * 10.0).round() as i64) % 10 == 0 {
        format!("{:.0} {}", value, UNITS[idx])
    } else {
        format!("{:.1} {}", value, UNITS[idx])
    }
}

/// Takes a human formatted byte size and convert it to the number of bytes
pub fn parse_size(size: &str) -> Result<u64> {
    let s = size.trim();

    if s == "0" {
        return Ok(0);
    }

    let num_end = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    let (num_str, unit_str) = s.split_at(num_end);
    let num_str = num_str.trim();
    let unit_str = unit_str.trim().to_ascii_uppercase();

    if num_str.is_empty() {
        bail!("Invalid size string: {s:?} (no number found)");
    }

    let num: f64 = num_str
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid number in size string: {num_str:?}"))?;

    let multiplier = match unit_str.as_str() {
        "" | "B" => 1u64,
        "K" | "KB" | "KIB" => KB,
        "M" | "MB" | "MIB" => MB,
        "G" | "GB" | "GIB" => GB,
        "T" | "TB" | "TIB" => TB,
        _ => bail!("Unknown size unit: {unit_str:?}"),
    };

    Ok((num * multiplier as f64) as u64)
}

/// Creates a rayon thread pool.
///
/// Thread count priority (highest to lowest):
/// 1. Global override via [`set_num_threads`] (capped at 32)
/// 2. `DVS_NUM_THREADS` environment variable (capped at 32)
/// 3. Default: `available_cpus * 4` (capped at 16)
///
/// The result is always clamped to the number of work items.
pub fn get_threadpool(work_items: usize) -> Result<rayon::ThreadPool> {
    debug_assert_ne!(
        work_items, 0,
        "the thread pool should not be instantiated when there are no work items to process"
    );
    // a proxy for available logical cpu cores
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1);
    let env_threads = std::env::var("DVS_NUM_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0);
    // Priority: set_num_threads() > DVS_NUM_THREADS env var > default
    let num_threads = {
        let work_limit = work_items.max(1);

        let configured = match get_num_threads().or(env_threads) {
            Some(n) => n.min(ENV_MAX_THREADS),
            None => available
                .saturating_mul(DEFAULT_THREADS_PER_CPU)
                .min(DEFAULT_MAX_THREADS),
        };
        configured.min(work_limit)
    };

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()?;
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_size_basic() {
        assert_eq!(parse_size("0").unwrap(), 0);
        assert_eq!(parse_size("500MB").unwrap(), 500 * MB);
        assert_eq!(parse_size("500 MB").unwrap(), 500 * MB);
        assert_eq!(parse_size("1GB").unwrap(), GB);
        assert_eq!(parse_size("1 gb").unwrap(), GB);
        assert_eq!(parse_size("2TB").unwrap(), 2 * TB);
        assert_eq!(parse_size("100KB").unwrap(), 100 * KB);
        assert_eq!(parse_size("1024B").unwrap(), 1024);
        assert_eq!(parse_size("1.5GB").unwrap(), (1.5 * GB as f64) as u64);
        assert!(parse_size("").is_err());
        assert!(parse_size("MB").is_err());
        assert!(parse_size("500XB").is_err());
    }

    #[test]
    fn format_size_drops_trailing_zero() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(KB), "1 KB");
        assert_eq!(format_size(MB), "1 MB");
        assert_eq!(format_size(GB), "1 GB");
        assert_eq!(format_size(TB), "1 TB");
        assert_eq!(format_size(3 * MB), "3 MB");
    }

    #[test]
    fn format_size_keeps_fractions() {
        assert_eq!(format_size(KB + KB / 2), "1.5 KB");
        assert_eq!(format_size(MB + MB / 2), "1.5 MB");
    }

    #[test]
    fn format_size_rolls_over_at_1024() {
        // bytes just below the next unit must not render as "1024.0 <unit>"
        assert_eq!(format_size(MB - 1), "1 MB");
        assert_eq!(format_size(GB - 1), "1 GB");
        assert_eq!(format_size(TB - 1), "1 TB");
    }
}
