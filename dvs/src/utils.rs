use anyhow::Result;

const MAX_THREADS: usize = 16;

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
