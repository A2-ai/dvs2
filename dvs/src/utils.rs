use anyhow::Result;

const DEFAULT_THREADS_PER_CPU: usize = 4;
/// Maximum thread count threshold when `DVS_NUM_THREADS` is unset
const DEFAULT_MAX_THREADS: usize = 16;
/// Maximum thread count threshold if `DVS_NUM_THREADS` is set
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
    // when `DVS_NUM_THREADS` is unset:
    // num_threads = min(workitems, cpus * 4, 16)
    // else
    // num_threads = min(workitems, DVS_NUM_THREADS, 32)
    let num_threads = {
        let work_limit = work_items.max(1);

        let configured = match env_threads {
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
