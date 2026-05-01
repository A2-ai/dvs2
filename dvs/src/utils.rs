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

/// Formats a byte count into a human-readable string (e.g. "10.5 MB").
/// Uses base-1024 divisors to match `ls -h` / `du -h` output.
pub fn format_size(bytes: u64) -> String {
    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
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

/// Creates a rayon thread pool with a thread count resolved from a 3-tier
/// priority chain, each clamped to `work_items`:
///
/// 1. **Override** — `set_num_threads(n)` (capped at `ENV_MAX_THREADS`)
/// 2. **Env** — `DVS_NUM_THREADS` env var, must parse to `usize > 0`
///    (capped at `ENV_MAX_THREADS`)
/// 3. **Default** — `available_cpus * DEFAULT_THREADS_PER_CPU`
///    (capped at `DEFAULT_MAX_THREADS`)
pub fn get_threadpool(work_items: usize) -> Result<rayon::ThreadPool> {
    debug_assert_ne!(
        work_items, 0,
        "the thread pool should not be instantiated when there are no work items to process"
    );
    let work_limit = work_items.max(1);
    let available_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1);

    let (num_threads, source) = if let Some(n) = get_num_threads() {
        (n.min(ENV_MAX_THREADS).min(work_limit), "override")
    } else if let Some(n) = std::env::var("DVS_NUM_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
    {
        (n.min(ENV_MAX_THREADS).min(work_limit), "env")
    } else {
        let n = available_cpus
            .saturating_mul(DEFAULT_THREADS_PER_CPU)
            .min(DEFAULT_MAX_THREADS)
            .min(work_limit);
        (n, "default")
    };

    log::debug!(
        "thread pool: {num_threads} threads (source: {source}, work_items={work_items})",
    );

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
}
