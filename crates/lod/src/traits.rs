//! Trait definitions for generic aggregation over quote-like data

use std::collections::BTreeMap;

/// Core trait for any data type that can be aggregated into LOD levels
///
/// This trait provides the minimal interface needed for the aggregator
/// to process time-series data. Implementors must provide a timestamp
/// and relevant price/volume accessors.
pub trait QuoteLike {
    /// Return the timestamp in nanoseconds since Unix epoch
    fn timestamp(&self) -> i64;

    /// Opening price (first price in the period)
    fn open(&self) -> f64;

    /// Highest price in the period
    fn high(&self) -> f64;

    /// Lowest price in the period
    fn low(&self) -> f64;

    /// Closing price (last price in the period)
    fn close(&self) -> f64;

    /// Volume traded in the period
    fn volume(&self) -> f64;

    /// Optional bid price (for NBBO data)
    fn bid(&self) -> Option<f64> {
        None
    }

    /// Optional ask price (for NBBO data)
    fn ask(&self) -> Option<f64> {
        None
    }

    /// Optional trade count
    fn count(&self) -> Option<u32> {
        None
    }
}

/// Batch of aggregated level data
pub struct LevelBatch {
    /// Map of interval (in seconds) to aggregated rows
    pub levels: BTreeMap<u64, Vec<crate::levels::PlotCandle>>,

    /// Trailing state for incremental updates
    pub trailing_state: Option<Box<dyn std::any::Any + Send + Sync>>,
}

/// Trait for generating level-of-detail data from quote-like inputs
///
/// Implementors define how to aggregate raw data into multi-resolution
/// representations. The trait supports both batch and streaming modes.
pub trait LevelGenerator: Send + Sync {
    /// Ingest a single item for aggregation
    fn ingest(&mut self, item: &dyn QuoteLike);

    /// Finalize aggregation and return the batch of levels
    fn finalize(self: Box<Self>) -> LevelBatch;

    /// Reset the generator for a new aggregation cycle
    fn reset(&mut self);

    /// Clone the generator (for parallel processing)
    fn clone_box(&self) -> Box<dyn LevelGenerator>;
}

/// Simple candle struct for testing and adapters
#[derive(Debug, Clone, Copy)]
pub struct SimpleCandle {
    pub timestamp: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl QuoteLike for SimpleCandle {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }

    fn open(&self) -> f64 {
        self.open
    }

    fn high(&self) -> f64 {
        self.high
    }

    fn low(&self) -> f64 {
        self.low
    }

    fn close(&self) -> f64 {
        self.close
    }

    fn volume(&self) -> f64 {
        self.volume
    }
}

/// NBBO record adapter
#[derive(Debug, Clone, Copy)]
pub struct NbboRecord {
    pub ts_event: u64,
    pub bid_px: Option<i64>,
    pub bid_sz: u32,
    pub ask_px: Option<i64>,
    pub ask_sz: u32,
}

impl NbboRecord {
    fn price_to_f64(price: i64) -> f64 {
        price as f64 / 1_000_000_000.0
    }
}

impl QuoteLike for NbboRecord {
    fn timestamp(&self) -> i64 {
        self.ts_event as i64
    }

    fn open(&self) -> f64 {
        // For NBBO, use mid-price as OHLC values
        match (self.bid_px, self.ask_px) {
            (Some(bid), Some(ask)) => Self::price_to_f64((bid + ask) / 2),
            (Some(bid), None) => Self::price_to_f64(bid),
            (None, Some(ask)) => Self::price_to_f64(ask),
            _ => 0.0,
        }
    }

    fn high(&self) -> f64 {
        self.open()
    }

    fn low(&self) -> f64 {
        self.open()
    }

    fn close(&self) -> f64 {
        self.open()
    }

    fn volume(&self) -> f64 {
        (self.bid_sz + self.ask_sz) as f64
    }

    fn bid(&self) -> Option<f64> {
        self.bid_px.map(Self::price_to_f64)
    }

    fn ask(&self) -> Option<f64> {
        self.ask_px.map(Self::price_to_f64)
    }
}

/// OHLCV record adapter for NOHLCV data
#[derive(Debug, Clone, Copy)]
pub struct OhlcvRecord {
    pub ts_event: u64,
    pub open_px: i64,
    pub high_px: i64,
    pub low_px: i64,
    pub close_px: i64,
    pub volume: u64,
    pub trade_count: u32,
}

impl OhlcvRecord {
    const PRICE_SCALE: f64 = 1_000_000_000.0;

    fn price_to_f64(price: i64) -> f64 {
        price as f64 / Self::PRICE_SCALE
    }
}

impl QuoteLike for OhlcvRecord {
    fn timestamp(&self) -> i64 {
        self.ts_event as i64
    }

    fn open(&self) -> f64 {
        Self::price_to_f64(self.open_px)
    }

    fn high(&self) -> f64 {
        Self::price_to_f64(self.high_px)
    }

    fn low(&self) -> f64 {
        Self::price_to_f64(self.low_px)
    }

    fn close(&self) -> f64 {
        Self::price_to_f64(self.close_px)
    }

    fn volume(&self) -> f64 {
        self.volume as f64
    }

    fn count(&self) -> Option<u32> {
        Some(self.trade_count)
    }
}
