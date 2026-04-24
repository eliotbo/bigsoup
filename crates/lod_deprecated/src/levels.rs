//! Cache infrastructure for level-of-detail data

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Compact plot-optimized candle representation
#[repr(C)]
#[cfg_attr(feature = "mmap", derive(zerocopy::FromBytes, zerocopy::FromZeroes))]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlotCandle {
    /// Timestamp in nanoseconds
    pub ts: i64,
    /// Opening price
    pub open: f32,
    /// High price
    pub high: f32,
    /// Low price
    pub low: f32,
    /// Close price
    pub close: f32,
    /// Volume
    pub volume: f32,
}

/// Compact plot-optimized trade representation
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlotTrade {
    /// Timestamp in nanoseconds
    pub ts: i64,
    /// Trade price
    pub price: f32,
    /// Trade size/volume
    pub size: f32,
    /// Aggressor side: 'B' for buy (lifted ask), 'S' for sell (hit bid), 'N' for unknown
    pub side: u8,
    /// Trade flags (exchange-specific conditions)
    pub flags: u8,
    /// Exchange ID
    pub exchange: u16,
}

impl PlotTrade {
    /// Create a new PlotTrade
    pub fn new(ts: i64, price: f32, size: f32, side: u8, flags: u8, exchange: u16) -> Self {
        PlotTrade {
            ts,
            price,
            size,
            side,
            flags,
            exchange,
        }
    }

    /// Convert timestamp to seconds since Unix epoch
    pub fn timestamp_secs(&self) -> f64 {
        self.ts as f64 / 1_000_000_000.0
    }

    /// Check if this is a buy trade (lifted ask)
    pub fn is_buy(&self) -> bool {
        self.side == b'B'
    }

    /// Check if this is a sell trade (hit bid)
    pub fn is_sell(&self) -> bool {
        self.side == b'S' || self.side == b'A'
    }

    /// Get human-readable side string
    pub fn side_str(&self) -> &str {
        match self.side {
            b'B' => "Buy",
            b'S' | b'A' => "Sell",
            _ => "Unknown",
        }
    }
}

/// Unified plot data that can represent either candles or trades
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlotData {
    /// Aggregated candle data
    Candle(PlotCandle),
    /// Individual trade data
    Trade(PlotTrade),
}

impl PlotData {
    /// Get the timestamp regardless of data type
    pub fn timestamp(&self) -> i64 {
        match self {
            PlotData::Candle(c) => c.ts,
            PlotData::Trade(t) => t.ts,
        }
    }

    /// Get the timestamp in seconds
    pub fn timestamp_secs(&self) -> f64 {
        self.timestamp() as f64 / 1_000_000_000.0
    }

    /// Check if this is candle data
    pub fn is_candle(&self) -> bool {
        matches!(self, PlotData::Candle(_))
    }

    /// Check if this is trade data
    pub fn is_trade(&self) -> bool {
        matches!(self, PlotData::Trade(_))
    }

    /// Get as candle if it is one
    pub fn as_candle(&self) -> Option<&PlotCandle> {
        match self {
            PlotData::Candle(c) => Some(c),
            _ => None,
        }
    }

    /// Get as trade if it is one
    pub fn as_trade(&self) -> Option<&PlotTrade> {
        match self {
            PlotData::Trade(t) => Some(t),
            _ => None,
        }
    }
}

impl PlotCandle {
    /// Create a new PlotCandle
    pub fn new(ts: i64, open: f32, high: f32, low: f32, close: f32, volume: f32) -> Self {
        PlotCandle {
            ts,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    /// Convert timestamp to seconds since Unix epoch
    pub fn timestamp_secs(&self) -> f64 {
        self.ts as f64 / 1_000_000_000.0
    }
}

impl crate::traits::QuoteLike for PlotCandle {
    fn timestamp(&self) -> i64 {
        self.ts
    }

    fn open(&self) -> f64 {
        self.open as f64
    }

    fn high(&self) -> f64 {
        self.high as f64
    }

    fn low(&self) -> f64 {
        self.low as f64
    }

    fn close(&self) -> f64 {
        self.close as f64
    }

    fn volume(&self) -> f64 {
        self.volume as f64
    }
}

/// Metadata for a level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelInfo {
    /// First timestamp in the level
    pub first_ts: i64,
    /// Last timestamp in the level
    pub last_ts: i64,
    /// Bucket span in seconds
    pub bucket_span_secs: u64,
}

/// Storage for multi-resolution level data
pub struct LevelStore {
    /// Map from interval (seconds) to candle data
    levels: HashMap<u64, Arc<[PlotCandle]>>,
    /// Map from interval (seconds) to unified plot data (candles or trades)
    /// Interval 0 is reserved for raw trade data
    unified_levels: HashMap<u64, Arc<[PlotData]>>,
    /// Metadata for each level
    info: HashMap<u64, LevelInfo>,
}

impl LevelStore {
    /// Create a new empty store
    pub fn new() -> Self {
        LevelStore {
            levels: HashMap::new(),
            unified_levels: HashMap::new(),
            info: HashMap::new(),
        }
    }

    /// Create from streaming aggregator output
    pub fn from_stream(levels: Vec<(u64, Vec<PlotCandle>)>) -> Self {
        let mut store = LevelStore::new();

        for (interval, candles) in levels {
            if !candles.is_empty() {
                let first_ts = candles.first().unwrap().ts;
                let last_ts = candles.last().unwrap().ts;

                let info = LevelInfo {
                    first_ts,
                    last_ts,
                    bucket_span_secs: interval,
                };

                store.info.insert(interval, info);
                store.levels.insert(interval, candles.into());
            }
        }

        store
    }

    /// Get candle data for a specific interval
    pub fn get(&self, interval: u64) -> Option<Arc<[PlotCandle]>> {
        self.levels.get(&interval).cloned()
    }

    /// Get metadata for a specific interval
    pub fn info(&self, interval: u64) -> Option<&LevelInfo> {
        self.info.get(&interval)
    }

    /// Append new data to an existing level
    pub fn append(&mut self, interval: u64, new_data: &[PlotCandle], continuity_check: bool) {
        if new_data.is_empty() {
            return;
        }

        let new_first = new_data.first().unwrap().ts;
        let new_last = new_data.last().unwrap().ts;

        // Check continuity if requested
        if continuity_check {
            if let Some(info) = self.info.get(&interval) {
                let expected_gap = interval as i64 * 1_000_000_000;
                let actual_gap = new_first - info.last_ts;

                if actual_gap > expected_gap * 2 {
                    eprintln!(
                        "Warning: Large gap detected for interval {}s: expected ~{}s, got {}s",
                        interval,
                        interval,
                        actual_gap / 1_000_000_000
                    );
                }
            }
        }

        // Merge with existing data or create new
        match self.levels.get(&interval) {
            Some(existing) => {
                let mut combined = Vec::with_capacity(existing.len() + new_data.len());
                combined.extend_from_slice(&existing);
                combined.extend_from_slice(new_data);
                self.levels.insert(interval, combined.into());
            }
            None => {
                self.levels.insert(interval, new_data.to_vec().into());
            }
        }

        // Update metadata
        let info = self.info.entry(interval).or_insert_with(|| LevelInfo {
            first_ts: new_first,
            last_ts: new_last,
            bucket_span_secs: interval,
        });

        if new_first < info.first_ts {
            info.first_ts = new_first;
        }
        if new_last > info.last_ts {
            info.last_ts = new_last;
        }
    }

    /// Convert to shareable Arc reference
    pub fn into_shared(self) -> Arc<Self> {
        Arc::new(self)
    }

    /// Get all available intervals
    pub fn intervals(&self) -> Vec<u64> {
        let mut intervals: Vec<_> = self.levels.keys().copied().collect();
        intervals.sort_unstable();
        intervals
    }

    /// Get the total number of candles across all levels
    pub fn total_candles(&self) -> usize {
        self.levels.values().map(|v| v.len()).sum()
    }

    /// Add raw trade data (stored at interval 0)
    pub fn add_trades(&mut self, trades: Vec<PlotTrade>) {
        if trades.is_empty() {
            return;
        }

        let first_ts = trades[0].ts;
        let last_ts = trades[trades.len() - 1].ts;

        // Convert to PlotData
        let plot_data: Vec<PlotData> = trades.into_iter().map(PlotData::Trade).collect();

        // Store at interval 0 (reserved for raw trades)
        self.unified_levels.insert(0, plot_data.into());

        // Update metadata
        self.info.insert(
            0,
            LevelInfo {
                first_ts,
                last_ts,
                bucket_span_secs: 0, // 0 indicates raw trade data
            },
        );
    }

    /// Get unified data for a specific interval
    /// Returns None if no data exists, or Some with either trade or candle data
    pub fn get_unified(&self, interval: u64) -> Option<Arc<[PlotData]>> {
        self.unified_levels.get(&interval).cloned()
    }

    /// Add unified data (candles or trades) for a specific interval
    pub fn add_unified(&mut self, interval: u64, data: Vec<PlotData>) {
        if data.is_empty() {
            return;
        }

        let first_ts = data[0].timestamp();
        let last_ts = data[data.len() - 1].timestamp();

        self.unified_levels.insert(interval, data.into());

        // Update metadata
        self.info.insert(
            interval,
            LevelInfo {
                first_ts,
                last_ts,
                bucket_span_secs: interval,
            },
        );
    }

    /// Check if data at interval is trade data
    pub fn is_trade_level(&self, interval: u64) -> bool {
        interval == 0
    }

    /// Get all intervals including unified data
    pub fn all_intervals(&self) -> Vec<u64> {
        let mut intervals: HashSet<u64> = self.levels.keys().copied().collect();
        intervals.extend(self.unified_levels.keys().copied());
        let mut sorted: Vec<_> = intervals.into_iter().collect();
        sorted.sort_unstable();
        sorted
    }

    /// Total data points (candles + trades)
    pub fn total_data_points(&self) -> usize {
        let candles = self.levels.values().map(|v| v.len()).sum::<usize>();
        let unified = self.unified_levels.values().map(|v| v.len()).sum::<usize>();
        candles + unified
    }
}

impl Default for LevelStore {
    fn default() -> Self {
        Self::new()
    }
}
