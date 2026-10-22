//! What a running operation reports about itself beyond its rows: byte
//! totals, and the clock helpers its timing is reported with.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

/// Whole milliseconds since `since`.
pub fn duration_ms(since: Instant) -> u64 {
    millis(since.elapsed())
}

/// Whole milliseconds in `duration`, for work summed over several steps.
pub fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Unix milliseconds, for event `ts`.
pub fn unix_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// RFC 3339 UTC with milliseconds, for `started_at`/`finished_at`,
/// e.g. `2025-10-21T14:30:00.123Z`.
pub fn rfc3339_ms(time: SystemTime) -> String {
    match jiff::Timestamp::try_from(time) {
        Ok(ts) => format!("{ts:.3}"),
        Err(_) => format!("{:.3}", jiff::Timestamp::UNIX_EPOCH),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_unix_ms_and_rfc3339_with_milliseconds() {
        let time = UNIX_EPOCH + Duration::from_millis(1_761_057_000_123);
        assert_eq!(unix_ms(time), 1_761_057_000_123);
        assert_eq!(rfc3339_ms(time), "2025-10-21T14:30:00.123Z");
        assert_eq!(rfc3339_ms(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
    }
}
