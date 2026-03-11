use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use dvs::TimingRecord;

const BUF_SIZE: usize = 256 * 1024; // 256 KB write buffer

/// Handle to the background CSV timing writer thread.
///
/// Records sent via `sender()` are buffered and written to a CSV file.
/// Call `finish()` to flush and close the file.
pub struct TimingHandle {
    tx: Option<Sender<TimingRecord>>,
    join: Option<JoinHandle<()>>,
    path: PathBuf,
}

impl TimingHandle {
    /// Start a new CSV timing writer. The file is named `dvs-timings-<timestamp>.csv`
    /// in the current directory.
    pub fn new() -> anyhow::Result<Self> {
        let timestamp = jiff::Timestamp::now().strftime("%Y%m%d-%H%M%S").to_string();
        let path = PathBuf::from(format!("dvs-timings-{timestamp}.csv"));
        let file = std::fs::File::create(&path)?;
        let buf = BufWriter::with_capacity(BUF_SIZE, file);

        let (tx, rx) = mpsc::channel::<TimingRecord>();

        let join = thread::spawn(move || {
            let mut wtr = csv::Writer::from_writer(buf);
            for record in rx {
                if let Err(e) = wtr.serialize(&record) {
                    eprintln!("Warning: failed to write timing record: {e}");
                }
            }
            if let Err(e) = wtr.flush() {
                eprintln!("Warning: failed to flush timing CSV: {e}");
            }
        });

        Ok(Self {
            tx: Some(tx),
            join: Some(join),
            path,
        })
    }

    /// Get a clone of the sender for passing into OutputOptions.
    pub fn sender(&self) -> Sender<TimingRecord> {
        self.tx.as_ref().expect("sender already consumed").clone()
    }

    /// Path to the CSV file being written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Drop the sender and wait for the writer thread to flush and exit.
    pub fn finish(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        // Drop our copy of tx so the writer thread sees the channel close
        self.tx.take();
        if let Some(join) = self.join.take() {
            let _ = join.join();
            eprintln!("Timing log saved: {}", self.path.display());
        }
    }
}

impl Drop for TimingHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}
