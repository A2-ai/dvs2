use std::io::{self, Read};
use std::path::Path;

/// Wraps a `Read` and reports bytes read to a callback.
pub struct ProgressReader<'a, R> {
    inner: R,
    on_bytes: &'a (dyn Fn(u64) + Send + Sync),
}

impl<'a, R: Read> ProgressReader<'a, R> {
    pub fn new(inner: R, on_bytes: &'a (dyn Fn(u64) + Send + Sync)) -> Self {
        Self { inner, on_bytes }
    }
}

impl<R: Read> Read for ProgressReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            (self.on_bytes)(n as u64);
        }
        Ok(n)
    }
}

/// Callback type for `on_file_start` parameters in `add_files`/`get_files`.
pub type OnFileStart = dyn Fn(&Path, u64) -> FileProgress + Send + Sync;

pub struct FileProgress {
    /// Called as bytes are transferred for this specific file.
    pub on_bytes: Box<dyn Fn(u64) + Send + Sync>,
}
