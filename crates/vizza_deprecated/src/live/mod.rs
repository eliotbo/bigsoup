//! Live data source abstraction for pluggable market data feeds.

pub mod backfill;
mod ipc_source;
mod market_calendar;
mod mock_backfill;
mod synthetic_source;

pub use backfill::{AsBackfillSource, BackfillRequest, BackfillResponse, HistoricalBackfillSource};
pub use ipc_source::IPCLiveDataSource;
pub use market_calendar::MarketCalendar;
pub use mock_backfill::MockBackfillSource;
pub use synthetic_source::SyntheticDataSource;

use lod::{LevelStore, PlotTrade};

/// Error type for live data operations.
#[derive(Debug)]
pub enum LiveDataError {
    NotInitialized,
    InvalidHistoricalData,
    SourceError(String),
}

impl std::fmt::Display for LiveDataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInitialized => write!(f, "Live data source not initialized"),
            Self::InvalidHistoricalData => write!(f, "Invalid or missing historical data"),
            Self::SourceError(msg) => write!(f, "Live data source error: {}", msg),
        }
    }
}

impl std::error::Error for LiveDataError {}

pub type Result<T> = std::result::Result<T, LiveDataError>;

/// Trait for pluggable live data sources.
///
/// Implementers can provide real-time market data from various sources:
/// - Synthetic data based on historical patterns
/// - WebSocket feeds from exchanges
/// - File-based replay systems
pub trait LiveDataSource: Send + Sync {
    /// Initialize the data source with historical context.
    ///
    /// # Arguments
    /// * `historical_data` - Historical price data for context
    /// * `ticker` - The ticker symbol being tracked
    fn initialize(&mut self, historical_data: &LevelStore, ticker: &str) -> Result<()>;

    /// Generate or fetch trades up to the current time.
    ///
    /// # Arguments
    /// * `now_ns` - Current time in nanoseconds since epoch
    ///
    /// # Returns
    /// Vector of trades that occurred since the last call
    fn get_trades(&mut self, now_ns: i64) -> Vec<PlotTrade>;

    /// Check if the market is currently open.
    ///
    /// For synthetic sources, this respects market hours and holidays.
    /// For live sources, this may always return true.
    ///
    /// # Arguments
    /// * `now_ns` - Current time in nanoseconds since epoch
    fn is_market_open(&self, now_ns: i64) -> bool;

    /// Get the name of this data source for logging/debugging.
    fn source_name(&self) -> &str;

    /// Get the current base price (last known price).
    fn current_price(&self) -> f64;

    /// Try to get a mutable reference to this source as a HistoricalBackfillSource.
    ///
    /// Default implementation returns None. Sources that support backfill should override this.
    fn as_backfill_source(&mut self) -> Option<&mut dyn backfill::HistoricalBackfillSource> {
        None
    }
}
