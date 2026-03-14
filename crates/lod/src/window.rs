//! Zero-copy window utilities for efficient data access

use crate::levels::PlotCandle;
use std::ops::Range;
use std::time::Duration;

/// Extract a window of data centered around a timestamp
///
/// Returns the range of indices that should be displayed
pub fn take_window(data: &[PlotCandle], center_ts: f64, count: usize) -> Range<usize> {
    if data.is_empty() || count == 0 {
        return 0..0;
    }

    let center_ns = (center_ts * 1_000_000_000.0) as i64;

    // Binary search for the closest timestamp
    let center_idx = match data.binary_search_by_key(&center_ns, |c| c.ts) {
        Ok(idx) => idx,
        Err(idx) => idx.saturating_sub(1),
    };

    // Calculate window bounds
    let half_count = count / 2;
    let start = center_idx.saturating_sub(half_count);
    let end = (start + count).min(data.len());

    // Adjust start if we hit the end
    let adjusted_start = if end == data.len() && end >= count {
        end - count
    } else {
        start
    };

    adjusted_start..end
}

/// Extract a window based on time duration
pub fn take_window_duration(data: &[PlotCandle], center_ts: f64, window: Duration) -> Range<usize> {
    if data.is_empty() {
        return 0..0;
    }

    let center_ns = (center_ts * 1_000_000_000.0) as i64;
    let half_window_ns = (window.as_secs_f64() * 0.5 * 1_000_000_000.0) as i64;

    let start_ts = center_ns - half_window_ns;
    let end_ts = center_ns + half_window_ns;

    // Find start index
    let start_idx = match data.binary_search_by_key(&start_ts, |c| c.ts) {
        Ok(idx) => idx,
        Err(idx) => idx,
    };

    // Find end index
    let end_idx = match data.binary_search_by_key(&end_ts, |c| c.ts) {
        Ok(idx) => idx + 1,
        Err(idx) => idx,
    };

    start_idx..end_idx.min(data.len())
}

/// Zero-copy reference to a chunk of data
pub struct ChunkRef<'a> {
    base: &'a [PlotCandle],
    range: Range<usize>,
}

impl<'a> ChunkRef<'a> {
    /// Create a new chunk reference
    pub fn new(base: &'a [PlotCandle], range: Range<usize>) -> Self {
        ChunkRef { base, range }
    }

    /// Get the slice this chunk refers to
    pub fn slice(&self) -> &'a [PlotCandle] {
        &self.base[self.range.clone()]
    }

    /// Get the range
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// Check if unchanged from another chunk
    pub fn unchanged(&self, other: &ChunkRef) -> bool {
        self.range == other.range && std::ptr::eq(self.base, other.base)
    }

    /// Get the number of candles in this chunk
    pub fn len(&self) -> usize {
        self.range.end - self.range.start
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.range.start >= self.range.end
    }

    /// Get first candle if available
    pub fn first(&self) -> Option<&PlotCandle> {
        if !self.is_empty() {
            Some(&self.base[self.range.start])
        } else {
            None
        }
    }

    /// Get last candle if available
    pub fn last(&self) -> Option<&PlotCandle> {
        if !self.is_empty() {
            Some(&self.base[self.range.end - 1])
        } else {
            None
        }
    }
}

/// Convert a range to an owned vector (compatibility helper)
pub fn to_vec(range: Range<usize>, base: &[PlotCandle]) -> Vec<PlotCandle> {
    base[range].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_data() -> Vec<PlotCandle> {
        (0..100)
            .map(|i| {
                PlotCandle::new(
                    i * 1_000_000_000, // 1 second intervals
                    100.0 + i as f32,
                    101.0 + i as f32,
                    99.0 + i as f32,
                    100.5 + i as f32,
                    1000.0,
                )
            })
            .collect()
    }

    #[test]
    fn test_take_window() {
        let data = make_test_data();

        // Center at 50 seconds
        let range = take_window(&data, 50.0, 10);
        assert_eq!(range, 45..55);

        // Near start
        let range = take_window(&data, 2.0, 10);
        assert_eq!(range, 0..10);

        // Near end
        let range = take_window(&data, 98.0, 10);
        assert_eq!(range, 90..100);
    }

    #[test]
    fn test_take_window_duration() {
        let data = make_test_data();

        // 10 second window centered at 50s
        let range = take_window_duration(&data, 50.0, Duration::from_secs(10));
        assert_eq!(range, 45..56);
    }

    #[test]
    fn test_chunk_ref() {
        let data = make_test_data();
        let chunk = ChunkRef::new(&data, 10..20);

        assert_eq!(chunk.len(), 10);
        assert!(!chunk.is_empty());
        assert_eq!(chunk.first().unwrap().ts, 10_000_000_000);
        assert_eq!(chunk.last().unwrap().ts, 19_000_000_000);
    }
}
