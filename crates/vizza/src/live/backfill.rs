//! Historical backfill support for live data sources.
//!
//! This module provides traits and types for data sources that can backfill
//! historical intraday bars (e.g., "today so far" from market open to current time).

use lod::PlotCandle;

use super::{LiveDataSource, Result};

/// Request parameters for historical backfill.
#[derive(Clone, Debug)]
pub struct BackfillRequest {
    /// Start time in nanoseconds since Unix epoch
    pub start_ns: i64,
    /// End time in nanoseconds since Unix epoch
    pub end_ns: i64,
    /// Bar size in seconds (e.g., 5, 15, 30, 60)
    pub bar_size_secs: u64,
    /// Ticker symbol to fetch
    pub ticker: String,
}

impl BackfillRequest {
    /// Create a new backfill request.
    pub fn new(start_ns: i64, end_ns: i64, bar_size_secs: u64, ticker: String) -> Self {
        Self {
            start_ns,
            end_ns,
            bar_size_secs,
            ticker,
        }
    }

    /// Calculate the expected number of bars for this request.
    pub fn expected_bar_count(&self) -> usize {
        let duration_secs = (self.end_ns - self.start_ns) / 1_000_000_000;
        if duration_secs <= 0 {
            return 0;
        }
        (duration_secs as u64 / self.bar_size_secs) as usize
    }
}

/// Response from a historical backfill request.
#[derive(Debug)]
pub struct BackfillResponse {
    /// The fetched historical bars
    pub bars: Vec<PlotCandle>,
    /// Actual start time of returned data (may differ from request)
    pub actual_start_ns: i64,
    /// Actual end time of returned data (may differ from request)
    pub actual_end_ns: i64,
}

impl BackfillResponse {
    /// Create a new backfill response.
    pub fn new(bars: Vec<PlotCandle>, actual_start_ns: i64, actual_end_ns: i64) -> Self {
        Self {
            bars,
            actual_start_ns,
            actual_end_ns,
        }
    }

    /// Create an empty response.
    pub fn empty() -> Self {
        Self {
            bars: Vec::new(),
            actual_start_ns: 0,
            actual_end_ns: 0,
        }
    }

    /// Check if the response contains any data.
    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }

    /// Get the number of bars returned.
    pub fn bar_count(&self) -> usize {
        self.bars.len()
    }
}

/// Trait for data sources that support historical backfilling.
///
/// This trait extends `LiveDataSource` to provide the ability to fetch
/// historical intraday bars for backfilling the "today so far" period.
pub trait HistoricalBackfillSource: LiveDataSource {
    /// Request historical bars for backfilling.
    ///
    /// # Arguments
    /// * `request` - Parameters describing what historical data to fetch
    ///
    /// # Returns
    /// A `BackfillResponse` containing the fetched bars and actual time range,
    /// or an error if the request fails.
    fn request_historical_bars(&mut self, request: BackfillRequest) -> Result<BackfillResponse>;

    /// Check if this data source supports historical backfill.
    ///
    /// This allows runtime detection of backfill capability.
    fn supports_backfill(&self) -> bool {
        true
    }

    /// Get the recommended bar size for backfilling (in seconds).
    ///
    /// Most implementations will return 5 seconds to match the primary live interval.
    fn recommended_backfill_bar_size(&self) -> u64 {
        5
    }
}

/// Helper to downcast a LiveDataSource to a HistoricalBackfillSource if possible.
pub trait AsBackfillSource {
    /// Attempt to get a mutable reference to this source as a HistoricalBackfillSource.
    fn as_backfill_source_mut(&mut self) -> Option<&mut dyn HistoricalBackfillSource>;
}

impl<T: HistoricalBackfillSource> AsBackfillSource for T {
    fn as_backfill_source_mut(&mut self) -> Option<&mut dyn HistoricalBackfillSource> {
        Some(self)
    }
}

// Default implementation for Box<dyn LiveDataSource>
impl AsBackfillSource for Box<dyn LiveDataSource> {
    fn as_backfill_source_mut(&mut self) -> Option<&mut dyn HistoricalBackfillSource> {
        None
    }
}
