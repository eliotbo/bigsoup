//! Mock backfill implementation for testing "today-so-far" functionality.
//!
//! This module provides a synthetic data source that supports both live trading
//! and historical backfill, useful for testing the complete today-so-far workflow.

use chrono::{DateTime, Datelike, TimeZone, Utc, Weekday};

use super::{
    LiveDataError, LiveDataSource, MarketCalendar, Result,
    backfill::{BackfillRequest, BackfillResponse, HistoricalBackfillSource},
};
use lod::{LevelStore, PlotCandle, PlotTrade, TradeSimulator};

/// Mock data source with backfill support for testing.
///
/// This source generates synthetic historical bars for the "today-so-far" period
/// and can also provide live trade simulation.
pub struct MockBackfillSource {
    ticker: String,
    base_price: f64,
    volatility: f64,
    simulator: Option<TradeSimulator>,
    market_calendar: MarketCalendar,
    initialized: bool,
    last_bar_time_ns: i64,

    // Mock configuration
    market_open_hour: u32,
    market_open_minute: u32,
}

impl MockBackfillSource {
    /// Create a new mock backfill source with US market hours (9:30 AM ET).
    pub fn new() -> Self {
        Self {
            ticker: String::new(),
            base_price: 47.7,
            volatility: 0.0001,
            simulator: None,
            market_calendar: MarketCalendar::us_equity(),
            initialized: false,
            last_bar_time_ns: 0,
            market_open_hour: 9,
            market_open_minute: 30,
        }
    }

    /// Set custom market open time (for testing different scenarios).
    pub fn set_market_open(&mut self, hour: u32, minute: u32) {
        self.market_open_hour = hour;
        self.market_open_minute = minute;
    }

    /// Generate synthetic bars for a time range.
    ///
    /// This creates realistic OHLCV bars with trending behavior and volatility.
    fn generate_synthetic_bars(
        &self,
        start_ns: i64,
        end_ns: i64,
        bar_size_secs: u64,
    ) -> Vec<PlotCandle> {
        let mut bars = Vec::new();
        let bar_size_ns = (bar_size_secs as i64) * 1_000_000_000;

        let mut current_time = start_ns;
        let mut current_price = self.base_price;

        // Use a simple random walk with realistic parameters
        let mut rng_state = (start_ns as u64).wrapping_mul(0x123456789abcdef);

        while current_time < end_ns {
            // Simple LCG for deterministic randomness
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let random_factor = (rng_state >> 32) as f64 / u32::MAX as f64;

            // Generate price movement: -1 to +1, scaled by volatility
            let price_change = (random_factor - 0.5) * 2.0 * self.volatility * current_price;
            current_price += price_change;
            current_price = current_price.max(self.base_price * 0.5); // Floor at 50% of base

            // Generate OHLCV for this bar
            let open = current_price;
            let mut high = open;
            let mut low = open;
            let close = open + price_change * 0.3; // Slight drift

            // Add some intrabar movement
            for _ in 0..3 {
                rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let tick_factor = (rng_state >> 32) as f64 / u32::MAX as f64;
                let tick_price = open + (tick_factor - 0.5) * self.volatility * current_price * 2.0;

                high = high.max(tick_price);
                low = low.min(tick_price);
            }

            // Generate volume (roughly proportional to volatility)
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let volume_factor = (rng_state >> 32) as f64 / u32::MAX as f64;
            let base_volume = 1000.0;
            let volume = base_volume * (0.5 + volume_factor);

            bars.push(PlotCandle {
                ts: current_time,
                open: open as f32,
                high: high as f32,
                low: low as f32,
                close: close as f32,
                volume: volume as f32,
            });

            current_time += bar_size_ns;
            current_price = close;
        }

        bars
    }

    /// Calculate today's market open in nanoseconds.
    #[allow(dead_code)]
    fn get_market_open_ns(&self, reference_ns: i64) -> i64 {
        let dt =
            DateTime::from_timestamp(reference_ns / 1_000_000_000, 0).unwrap_or_else(|| Utc::now());

        // Get today at market open
        let market_open = Utc
            .with_ymd_and_hms(
                dt.year(),
                dt.month(),
                dt.day(),
                self.market_open_hour,
                self.market_open_minute,
                0,
            )
            .single()
            .unwrap_or_else(|| Utc::now());

        market_open.timestamp_nanos_opt().unwrap_or(0)
    }
}

impl Default for MockBackfillSource {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveDataSource for MockBackfillSource {
    fn initialize(&mut self, _historical_data: &LevelStore, ticker: &str) -> Result<()> {
        self.ticker = ticker.to_string();

        // // Try to get last bar from historical data, but allow initialization without it
        // if let Some(candles) = historical_data.get(60) {
        //     if let Some(last_bar) = candles.last() {
        //         self.last_bar_time_ns = last_bar.ts;
        //         self.base_price = last_bar.close as f64;
        //     }
        // }

        // If no historical data, use current time as base
        if self.last_bar_time_ns == 0 {
            self.last_bar_time_ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        }

        // self.volatility = 0.001; // 0.5% volatility

        // Create simulator for live trades
        let interval_ns = 60i64 * 1_000_000_000;
        let base_epoch_ns = self.last_bar_time_ns.saturating_add(interval_ns);

        self.simulator = Some(TradeSimulator::new(
            format!("MOCK_{}", ticker),
            self.base_price,
            self.volatility * 0.1, // Scale down for live trades
            120.0,                 // ~2 trades per second
            base_epoch_ns,
            0xDEADBEEF,
        ));

        self.initialized = true;

        eprintln!(
            "✓ MockBackfillSource initialized: ticker={}, base_price={:.4}",
            self.ticker, self.base_price
        );

        Ok(())
    }

    fn get_trades(&mut self, now_ns: i64) -> Vec<PlotTrade> {
        if !self.initialized || !self.is_market_open(now_ns) {
            return Vec::new();
        }

        if let Some(ref mut simulator) = self.simulator {
            simulator.tick(now_ns)
        } else {
            Vec::new()
        }
    }

    fn is_market_open(&self, now_ns: i64) -> bool {
        if !self.initialized {
            return false;
        }

        // Check for holiday
        if self
            .market_calendar
            .is_likely_holiday(self.last_bar_time_ns, now_ns)
        {
            return false;
        }

        // Check for weekend
        let dt = DateTime::from_timestamp(now_ns / 1_000_000_000, 0).unwrap_or_else(|| Utc::now());

        !matches!(dt.weekday(), Weekday::Sat | Weekday::Sun)
    }

    fn source_name(&self) -> &str {
        "MockBackfillSource"
    }

    fn current_price(&self) -> f64 {
        self.base_price
    }

    fn as_backfill_source(&mut self) -> Option<&mut dyn HistoricalBackfillSource> {
        Some(self)
    }
}

impl HistoricalBackfillSource for MockBackfillSource {
    fn request_historical_bars(&mut self, request: BackfillRequest) -> Result<BackfillResponse> {
        if !self.initialized {
            return Err(LiveDataError::NotInitialized);
        }

        eprintln!(
            "✓ MockBackfillSource: Generating {} bars from {} to {}",
            request.expected_bar_count(),
            request.start_ns,
            request.end_ns
        );

        // Generate synthetic bars for the requested range
        let bars =
            self.generate_synthetic_bars(request.start_ns, request.end_ns, request.bar_size_secs);

        let actual_start_ns = bars.first().map(|b| b.ts).unwrap_or(request.start_ns);
        let actual_end_ns = bars.last().map(|b| b.ts).unwrap_or(request.end_ns);

        eprintln!("✓ MockBackfillSource: Generated {} bars", bars.len());

        // Update base price from last backfill bar and recreate simulator
        if let Some(last_bar) = bars.last() {
            self.base_price = last_bar.close as f64;

            // Recreate the TradeSimulator to start from the end of backfill data
            // Add one bar interval (5s) so trades start from the next bar
            let bar_size_ns = (request.bar_size_secs as i64) * 1_000_000_000;
            let base_epoch_ns = last_bar.ts + bar_size_ns;

            self.simulator = Some(TradeSimulator::new(
                format!("MOCK_{}", self.ticker),
                self.base_price,
                self.volatility * 0.1,
                120.0, // ~2 trades per second
                base_epoch_ns,
                0xDEADBEEF,
            ));

            eprintln!(
                "✓ Updated simulator: base_price={:.2}, starting from last bar + {}s",
                self.base_price, request.bar_size_secs
            );
        }

        Ok(BackfillResponse::new(bars, actual_start_ns, actual_end_ns))
    }

    fn supports_backfill(&self) -> bool {
        true
    }

    fn recommended_backfill_bar_size(&self) -> u64 {
        5
    }
}
