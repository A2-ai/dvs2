use anyhow::Result;

const DEFAULT_THREADS_PER_CPU: usize = 4;
const DEFAULT_MAX_THREADS: usize = 16;
const ENV_MAX_THREADS: usize = 32;

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

/// Creates a rayon thread pool.
///
/// If `DVS_NUM_THREADS` is set to a positive integer, uses that value
/// capped at 32 and clamped to the amount of available work.
/// Otherwise, defaults to up to 4 threads per available unit of
/// parallelism, capped at 16 and clamped to the amount of available work.
pub fn get_threadpool(work_items: usize) -> Result<rayon::ThreadPool> {
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let env_threads = std::env::var("DVS_NUM_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0);
    let num_threads = {
        let available_parallelism = available.max(1);
        let work_limit = work_items.max(1);

        let configured = match env_threads {
            Some(n) => n.min(ENV_MAX_THREADS),
            None => available_parallelism
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
