//! Synthetic data source using historical patterns.

use super::{LiveDataError, LiveDataSource, MarketCalendar, Result};
use chrono::{DateTime, Datelike, Weekday};
use lod::{LevelStore, PlotCandle, PlotTrade, TradeSimulator};

/// Smart synthetic data source that uses historical patterns.
///
/// Unlike naive random walks, this source:
/// - Uses actual closing price from last historical bar
/// - Calculates real volatility from recent price movements
/// - Respects market hours and weekends
/// - Generates realistic volume patterns
pub struct SyntheticDataSource {
    ticker: String,
    last_bar_time_ns: i64,
    base_price: f64,
    volatility: f64,
    raw_volatility: f64,
    simulator: Option<TradeSimulator>,
    market_calendar: MarketCalendar,
    initialized: bool,
}

impl SyntheticDataSource {
    /// Create a new synthetic data source.
    pub fn new() -> Self {
        Self {
            ticker: String::new(),
            last_bar_time_ns: 0,
            base_price: 100.0,
            volatility: 0.005,
            raw_volatility: 0.0,
            simulator: None,
            market_calendar: MarketCalendar::us_equity(),
            initialized: false,
        }
    }

    /// Calculate volatility from historical candles using standard deviation of returns.
    ///
    /// # Arguments
    /// * `candles` - Historical candle data
    /// * `lookback` - Number of bars to look back (default 20)
    ///
    /// # Returns
    /// Annualized volatility as a decimal (e.g., 0.25 = 25%)
    fn calculate_volatility(candles: &[PlotCandle], lookback: usize) -> f64 {
        if candles.len() < 2 {
            return 0.02; // Default 2% if insufficient data
        }

        let start_idx = candles.len().saturating_sub(lookback);
        let recent_candles = &candles[start_idx..];

        if recent_candles.len() < 2 {
            return 0.02;
        }

        // Calculate log returns
        let mut returns = Vec::with_capacity(recent_candles.len() - 1);
        for i in 1..recent_candles.len() {
            let prev_close = recent_candles[i - 1].close as f64;
            let curr_close = recent_candles[i].close as f64;

            if prev_close > 0.0 && curr_close > 0.0 {
                let log_return = (curr_close / prev_close).ln();
                returns.push(log_return);
            }
        }

        if returns.is_empty() {
            return 0.02;
        }

        // Calculate standard deviation
        let mean = returns.iter().sum::<f64>() / returns.len() as f64;
        let variance = returns
            .iter()
            .map(|r| {
                let diff = r - mean;
                diff * diff
            })
            .sum::<f64>()
            / returns.len() as f64;

        let std_dev = variance.sqrt();

        // Annualize volatility (assuming daily bars, 252 trading days)
        // For 1-minute bars, we'd use sqrt(252 * 390) where 390 = minutes per day
        let annualized_vol = std_dev * (252.0_f64).sqrt();

        // Clamp to reasonable range
        annualized_vol.max(0.01).min(2.0)
    }

    /// Extract volume profile from historical data (placeholder for future enhancement).
    fn _extract_volume_profile(_candles: &[PlotCandle]) -> f64 {
        // For now, return a simple average volume
        // Future: build time-of-day distribution
        120.0 // trades per minute
    }
}

impl Default for SyntheticDataSource {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveDataSource for SyntheticDataSource {
    fn initialize(&mut self, historical_data: &LevelStore, ticker: &str) -> Result<()> {
        self.ticker = ticker.to_string();

        // Get 60-second candles (primary interval)
        let candles = historical_data
            .get(60)
            .ok_or(LiveDataError::InvalidHistoricalData)?;

        if candles.is_empty() {
            return Err(LiveDataError::InvalidHistoricalData);
        }

        // CRITICAL: Get the last bar to extract real base price
        let last_bar = candles.last().ok_or(LiveDataError::InvalidHistoricalData)?;

        self.last_bar_time_ns = last_bar.ts;
        self.base_price = last_bar.close as f64;

        // Calculate actual volatility from recent price movements
        self.raw_volatility = Self::calculate_volatility(&candles, 20);

        // Scale down volatility for intraday simulation
        // TradeSimulator applies volatility per trade, which can compound quickly
        // Use a much smaller fraction for realistic intraday movement
        self.volatility = (self.raw_volatility * 0.0005).clamp(0.0001, 0.02);

        // Calculate base epoch for live data (start of next bar)
        let interval_ns = 60i64 * 1_000_000_000; // 60 seconds
        let base_epoch_ns = self.last_bar_time_ns.saturating_add(interval_ns);

        // Create simulator with realistic parameters
        self.simulator = Some(TradeSimulator::new(
            format!("SYN_{}", ticker),
            self.base_price,
            self.volatility,
            120.0, // ~2 trades per second on average
            base_epoch_ns,
            0xC0FFEE,
        ));

        self.initialized = true;

        // Debug output
        let last_bar_date = DateTime::from_timestamp(self.last_bar_time_ns / 1_000_000_000, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "unknown".to_string());

        eprintln!(
            "✓ SyntheticDataSource initialized: ticker={}, base_price={:.4}",
            self.ticker, self.base_price
        );
        eprintln!(
            "  Volatility: raw={:.4}, scaled={:.4} ({:.2}%)",
            self.raw_volatility,
            self.volatility,
            (self.volatility * 100.0)
        );
        eprintln!(
            "  Last bar: {} ({} ns)",
            last_bar_date, self.last_bar_time_ns
        );

        Ok(())
    }

    fn get_trades(&mut self, now_ns: i64) -> Vec<PlotTrade> {
        if !self.initialized {
            return Vec::new();
        }

        // Check if market is open
        if !self.is_market_open(now_ns) {
            return Vec::new();
        }

        // Generate trades using simulator
        if let Some(ref mut simulator) = self.simulator {
            let trades = simulator.tick(now_ns);

            // Debug: Log first batch of trades
            static mut FIRST_TRADES_LOGGED: bool = false;
            unsafe {
                if !FIRST_TRADES_LOGGED && !trades.is_empty() {
                    eprintln!(
                        "✓ Live trades generated: {} trades at t={} ns",
                        trades.len(),
                        now_ns
                    );
                    FIRST_TRADES_LOGGED = true;
                }
            }

            trades
        } else {
            Vec::new()
        }
    }

    fn is_market_open(&self, now_ns: i64) -> bool {
        if !self.initialized {
            return false;
        }

        // Check for likely holiday
        if self
            .market_calendar
            .is_likely_holiday(self.last_bar_time_ns, now_ns)
        {
            return false;
        }

        // For synthetic data, be lenient about market hours
        // Only check for weekends, not exact time-of-day
        // This allows visualization during off-hours for testing
        let dt = chrono::DateTime::from_timestamp(now_ns / 1_000_000_000, 0)
            .unwrap_or_else(|| chrono::Utc::now());

        // Only block on weekends, allow all weekday hours
        !matches!(dt.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun)
    }

    fn source_name(&self) -> &str {
        "SyntheticDataSource"
    }

    fn current_price(&self) -> f64 {
        self.base_price
    }
}
