//! Mock implementation of HistoricalBackfillSource for testing and development.
//!
//! This file demonstrates how the HistoricalBackfillSource trait would be
//! implemented for a mock data source that generates synthetic bars.

use chrono::{Local, NaiveTime, Timelike};
use lod::{LevelStore, PlotCandle, PlotTrade};
use std::sync::mpsc::Receiver;

use crate::live::{LiveDataSource, LiveDataError, Result};

/// Request parameters for historical backfill
#[derive(Clone, Debug)]
pub struct BackfillRequest {
    pub start_ns: i64,      // Start time in nanoseconds
    pub end_ns: i64,        // End time in nanoseconds
    pub bar_size_secs: u64, // Bar size (5, 15, 30, etc.)
    pub ticker: String,     // Ticker symbol
}

/// Response from historical backfill request
#[derive(Debug)]
pub struct BackfillResponse {
    pub bars: Vec<PlotCandle>,
    pub actual_start_ns: i64, // Actual start of returned data
    pub actual_end_ns: i64,   // Actual end of returned data
}

/// Trait for data sources that support historical backfill
pub trait HistoricalBackfillSource: LiveDataSource {
    /// Request historical bars for backfilling
    fn request_historical_bars(&mut self, request: BackfillRequest) -> Result<BackfillResponse>;

    /// Check if historical backfill is supported
    fn supports_backfill(&self) -> bool;
}

/// Mock implementation that generates synthetic historical bars
pub struct MockBackfillSource {
    receiver: Option<Receiver<PlotTrade>>,
    ticker: String,
    base_price: f64,
    current_price: f64,
    trades_buffer: Vec<PlotTrade>,
    market_open_hour: u8,
    market_open_minute: u8,
    market_close_hour: u8,
    market_close_minute: u8,
    last_trade_ns: i64,
}

impl MockBackfillSource {
    /// Create a new mock backfill source
    pub fn new(receiver: Option<Receiver<PlotTrade>>) -> Self {
        Self {
            receiver,
            ticker: String::new(),
            base_price: 100.0,
            current_price: 100.0,
            trades_buffer: Vec::new(),
            market_open_hour: 9,
            market_open_minute: 30,
            market_close_hour: 16,
            market_close_minute: 0,
            last_trade_ns: 0,
        }
    }

    /// Set market open time for backfill generation
    pub fn set_market_open(&mut self, hour: u8, minute: u8) {
        self.market_open_hour = hour;
        self.market_open_minute = minute;
    }

    /// Set market close time
    pub fn set_market_close(&mut self, hour: u8, minute: u8) {
        self.market_close_hour = hour;
        self.market_close_minute = minute;
    }

    /// Generate synthetic bars for the requested time range
    fn generate_synthetic_bars(
        &self,
        start_ns: i64,
        end_ns: i64,
        bar_size_secs: u64,
    ) -> Vec<PlotCandle> {
        let mut bars = Vec::new();
        let interval_ns = bar_size_secs as i64 * 1_000_000_000;

        // Use deterministic "randomness" based on timestamp
        let mut price = self.base_price;
        let mut current_ts = start_ns;

        // Align to bar boundaries
        current_ts = (current_ts / interval_ns) * interval_ns;

        while current_ts < end_ns {
            // Generate pseudo-random price movement
            let seed = (current_ts / 1_000_000) as u64; // Use milliseconds as seed
            let rng = (seed.wrapping_mul(1664525).wrapping_add(1013904223) % 1000) as f64 / 1000.0;
            let price_change = (rng - 0.5) * 0.5;

            let open = price;
            let high = price + (rng * 0.3).abs() + 0.05;
            let low = price - (rng * 0.3).abs() - 0.05;
            let close = price + price_change;
            let volume = 1000.0 + (seed % 5000) as f32;

            bars.push(PlotCandle {
                ts: current_ts,
                open: open as f32,
                high: high as f32,
                low: low as f32,
                close: close as f32,
                volume,
            });

            price = close;
            price = price.max(90.0).min(110.0); // Keep within reasonable bounds
            current_ts += interval_ns;
        }

        bars
    }

    /// Check if given time is within market hours
    fn is_within_market_hours(&self, timestamp_ns: i64) -> bool {
        let dt = chrono::DateTime::from_timestamp(timestamp_ns / 1_000_000_000, 0)
            .unwrap_or_else(chrono::Utc::now)
            .with_timezone(&Local);

        let time = dt.time();
        let market_open = NaiveTime::from_hms_opt(
            self.market_open_hour as u32,
            self.market_open_minute as u32,
            0,
        )
        .unwrap();
        let market_close = NaiveTime::from_hms_opt(
            self.market_close_hour as u32,
            self.market_close_minute as u32,
            0,
        )
        .unwrap();

        time >= market_open && time <= market_close
    }

    /// Generate a single trade at the current price
    fn generate_trade(&mut self, timestamp_ns: i64) -> PlotTrade {
        // Simple price movement
        let seed = (timestamp_ns / 1_000_000) as u64;
        let rng = (seed.wrapping_mul(1664525).wrapping_add(1013904223) % 1000) as f64 / 1000.0;
        let price_change = (rng - 0.5) * 0.1;

        self.current_price += price_change;
        self.current_price = self.current_price.max(90.0).min(110.0);

        PlotTrade {
            ts: timestamp_ns,
            price: self.current_price as f32,
            size: (100.0 + (seed % 500) as f32),
            side: if seed % 2 == 0 { b'B' } else { b'S' },
            flags: 0,
            exchange: 0,
        }
    }
}

impl LiveDataSource for MockBackfillSource {
    fn initialize(&mut self, historical_data: &LevelStore, ticker: &str) -> Result<()> {
        self.ticker = ticker.to_string();

        // Get base price from historical data if available
        if let Some(candles) = historical_data.get(60) {
            if let Some(last_candle) = candles.last() {
                self.base_price = last_candle.close as f64;
                self.current_price = self.base_price;
                self.last_trade_ns = last_candle.ts;
            }
        }

        Ok(())
    }

    fn get_trades(&mut self, now_ns: i64) -> Vec<PlotTrade> {
        // If we have a receiver, drain real trades
        if let Some(ref receiver) = self.receiver {
            while let Ok(trade) = receiver.try_recv() {
                self.trades_buffer.push(trade);
                self.current_price = trade.price as f64;
                self.last_trade_ns = trade.ts;
            }
            return std::mem::take(&mut self.trades_buffer);
        }

        // Otherwise, generate synthetic trades
        if now_ns <= self.last_trade_ns {
            return Vec::new(); // No new trades
        }

        let mut trades = Vec::new();

        // Generate trades at ~10Hz
        let trade_interval_ns = 100_000_000i64; // 100ms
        let mut current_ns = self.last_trade_ns + trade_interval_ns;

        while current_ns <= now_ns {
            if self.is_within_market_hours(current_ns) {
                trades.push(self.generate_trade(current_ns));
            }
            current_ns += trade_interval_ns;
        }

        if !trades.is_empty() {
            self.last_trade_ns = trades.last().unwrap().ts;
        }

        trades
    }

    fn is_market_open(&self, now_ns: i64) -> bool {
        self.is_within_market_hours(now_ns)
    }

    fn source_name(&self) -> &str {
        "MockBackfillSource"
    }

    fn current_price(&self) -> f64 {
        self.current_price
    }
}

impl HistoricalBackfillSource for MockBackfillSource {
    fn request_historical_bars(&mut self, request: BackfillRequest) -> Result<BackfillResponse> {
        // Validate request
        if request.bar_size_secs < 5 {
            return Err(LiveDataError::InvalidHistoricalData);
        }

        // Generate synthetic bars
        let bars = self.generate_synthetic_bars(
            request.start_ns,
            request.end_ns,
            request.bar_size_secs,
        );

        // Update current price if we generated bars
        if let Some(last_bar) = bars.last() {
            self.current_price = last_bar.close as f64;
            self.last_trade_ns = last_bar.ts;
        }

        let actual_start_ns = bars.first().map(|b| b.ts).unwrap_or(request.start_ns);
        let actual_end_ns = bars.last().map(|b| b.ts).unwrap_or(request.start_ns);

        Ok(BackfillResponse {
            bars,
            actual_start_ns,
            actual_end_ns,
        })
    }

    fn supports_backfill(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backfill_request() {
        let mut source = MockBackfillSource::new(None);
        source.set_market_open(9, 30);

        let start_ns = 1_000_000_000_000i64; // Some timestamp
        let end_ns = start_ns + 3600 * 1_000_000_000; // 1 hour later

        let request = BackfillRequest {
            start_ns,
            end_ns,
            bar_size_secs: 60, // 1-minute bars
            ticker: "TEST".to_string(),
        };

        let response = source.request_historical_bars(request).unwrap();

        // Should have 60 bars for 1 hour at 1-minute intervals
        assert_eq!(response.bars.len(), 60);

        // First bar should start at or after requested start
        assert!(response.actual_start_ns >= start_ns);

        // Bars should be properly ordered
        for i in 1..response.bars.len() {
            assert!(response.bars[i].ts > response.bars[i - 1].ts);
        }
    }

    #[test]
    fn test_5_second_bars() {
        let mut source = MockBackfillSource::new(None);

        let start_ns = 1_000_000_000_000i64;
        let end_ns = start_ns + 60 * 1_000_000_000; // 1 minute later

        let request = BackfillRequest {
            start_ns,
            end_ns,
            bar_size_secs: 5, // 5-second bars
            ticker: "TEST".to_string(),
        };

        let response = source.request_historical_bars(request).unwrap();

        // Should have 12 bars for 1 minute at 5-second intervals
        assert_eq!(response.bars.len(), 12);

        // Each bar should be 5 seconds apart
        for i in 1..response.bars.len() {
            let delta_ns = response.bars[i].ts - response.bars[i - 1].ts;
            assert_eq!(delta_ns, 5_000_000_000);
        }
    }

    #[test]
    fn test_price_continuity() {
        let mut source = MockBackfillSource::new(None);
        source.base_price = 100.0;

        let start_ns = 1_000_000_000_000i64;
        let end_ns = start_ns + 300 * 1_000_000_000; // 5 minutes

        let request = BackfillRequest {
            start_ns,
            end_ns,
            bar_size_secs: 5,
            ticker: "TEST".to_string(),
        };

        let response = source.request_historical_bars(request).unwrap();

        // Prices should be continuous (close of one bar ~= open of next)
        for i in 1..response.bars.len() {
            let prev_close = response.bars[i - 1].close;
            let curr_open = response.bars[i].open;
            let diff = (prev_close - curr_open).abs();

            // Should be relatively close (allowing for some gap)
            assert!(diff < 2.0, "Price gap too large: {}", diff);
        }
    }
}