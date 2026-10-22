//! What a running operation (`add`, `get`) reports about itself beyond its
//! rows.

use serde::Serialize;

/// Byte totals for an `add` or `get` result, dry run included.
///
/// `total` sums the sizes of rows whose size is known; `to_transfer` is what
/// needs uploading (add) or downloading (get); `already_stored` is the rest of
/// `total` (add: the blob is already in storage; get: the local copy already
/// matches), so `to_transfer + already_stored == total`. `transferred` is what
/// actually moved, 0 in a dry run. Rows of unknown size count in none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ByteTotals {
    pub total: u64,
    pub to_transfer: u64,
    pub already_stored: u64,
    pub transferred: u64,
}

impl ByteTotals {
    /// Counts one row of known `size`: in `already_stored` when `stored`,
    /// else in `to_transfer`, and in `transferred` when its bytes moved.
    pub(crate) fn count(&mut self, size: u64, stored: bool, transferred: bool) {
        self.total += size;
        if stored {
            self.already_stored += size;
        } else {
            self.to_transfer += size;
        }
        if transferred {
            self.transferred += size;
        }
    }
}
