use anyhow::Result;

const MAX_THREADS: usize = 16;

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

/// Creates a rayon threadpool.
///
/// Respects `DVS_NUM_THREADS` env var if it's a number higher than 0.
/// Always caps threads to 16 and the amount of available work.
pub fn get_threadpool(work_items: usize) -> Result<rayon::ThreadPool> {
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let env_threads = std::env::var("DVS_NUM_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok());

    let work_limit = work_items.clamp(1, MAX_THREADS);
    let configured = match env_threads {
        Some(n) if n > 0 => n.min(MAX_THREADS),
        _ => available.clamp(1, MAX_THREADS),
    };

    let num_threads = configured.min(work_limit);

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()?;
    Ok(pool)
}
