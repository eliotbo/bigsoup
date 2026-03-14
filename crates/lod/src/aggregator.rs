//! Streaming aggregator for multi-resolution data

use crate::levels::PlotCandle;
use crate::traits::{LevelBatch, LevelGenerator, QuoteLike};
use std::collections::BTreeMap;

/// State for a single aggregation interval
#[derive(Debug, Clone)]
pub struct IntervalState {
    /// Current bucket start timestamp (ns)
    pub bucket_start: i64,
    /// Interval duration in seconds
    pub interval_secs: u64,
    /// OHLCV accumulators
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    /// Number of items in current bucket
    pub count: u32,
    /// Whether this bucket has data
    pub has_data: bool,
}

impl IntervalState {
    /// Create a new interval state
    pub fn new(interval_secs: u64) -> Self {
        IntervalState {
            bucket_start: 0,
            interval_secs,
            open: 0.0,
            high: f64::MIN,
            low: f64::MAX,
            close: 0.0,
            volume: 0.0,
            count: 0,
            has_data: false,
        }
    }

    /// Calculate the bucket for a given timestamp
    fn get_bucket_start(&self, ts: i64) -> i64 {
        let interval_ns = self.interval_secs as i64 * 1_000_000_000;
        (ts / interval_ns) * interval_ns
    }

    /// Update state with a new quote
    pub fn update(&mut self, item: &dyn QuoteLike) -> Option<PlotCandle> {
        let ts = item.timestamp();
        let bucket = self.get_bucket_start(ts);

        // Check if we need to emit the current bucket
        let mut emit = None;
        if self.has_data && bucket != self.bucket_start {
            emit = Some(PlotCandle::new(
                self.bucket_start,
                self.open as f32,
                self.high as f32,
                self.low as f32,
                self.close as f32,
                self.volume as f32,
            ));

            // Reset for new bucket
            self.open = item.open();
            self.high = item.high();
            self.low = item.low();
            self.close = item.close();
            self.volume = item.volume();
            self.count = 1;
        } else if !self.has_data {
            // First data point
            self.open = item.open();
            self.high = item.high();
            self.low = item.low();
            self.close = item.close();
            self.volume = item.volume();
            self.count = 1;
        } else {
            // Update current bucket
            self.high = self.high.max(item.high());
            self.low = self.low.min(item.low());
            self.close = item.close();
            self.volume += item.volume();
            self.count += 1;
        }

        self.bucket_start = bucket;
        self.has_data = true;

        emit
    }

    /// Seal the current bucket
    pub fn seal(&self) -> Option<PlotCandle> {
        if self.has_data {
            Some(PlotCandle::new(
                self.bucket_start,
                self.open as f32,
                self.high as f32,
                self.low as f32,
                self.close as f32,
                self.volume as f32,
            ))
        } else {
            None
        }
    }
}

/// Basic level generator implementation
#[derive(Clone)]
pub struct BasicLevelGenerator {
    intervals: Vec<u64>,
    states: Vec<IntervalState>,
    results: BTreeMap<u64, Vec<PlotCandle>>,
}

impl BasicLevelGenerator {
    pub fn new(intervals: Vec<u64>) -> Self {
        let states = intervals.iter().map(|&i| IntervalState::new(i)).collect();
        BasicLevelGenerator {
            intervals,
            states,
            results: BTreeMap::new(),
        }
    }
}

impl LevelGenerator for BasicLevelGenerator {
    fn ingest(&mut self, item: &dyn QuoteLike) {
        for (i, state) in self.states.iter_mut().enumerate() {
            if let Some(candle) = state.update(item) {
                self.results
                    .entry(self.intervals[i])
                    .or_insert_with(Vec::new)
                    .push(candle);
            }
        }
    }

    fn finalize(mut self: Box<Self>) -> LevelBatch {
        // Seal any open buckets
        for (i, state) in self.states.iter().enumerate() {
            if let Some(candle) = state.seal() {
                self.results
                    .entry(self.intervals[i])
                    .or_insert_with(Vec::new)
                    .push(candle);
            }
        }

        let mut levels = BTreeMap::new();
        for (interval, candles) in self.results {
            levels.insert(interval, candles);
        }

        LevelBatch {
            levels,
            trailing_state: None,
        }
    }

    fn reset(&mut self) {
        self.states = self
            .intervals
            .iter()
            .map(|&i| IntervalState::new(i))
            .collect();
        self.results.clear();
    }

    fn clone_box(&self) -> Box<dyn LevelGenerator> {
        Box::new(self.clone())
    }
}

/// Streaming aggregator for multi-resolution data
pub struct StreamingAggregator {
    #[allow(dead_code)]
    base_interval_secs: u64,
    levels: Vec<u64>,
    generator: Box<dyn LevelGenerator>,
}

impl StreamingAggregator {
    /// Create a new streaming aggregator
    pub fn new(base_interval_secs: u64, levels: Vec<u64>) -> Self {
        let generator = Box::new(BasicLevelGenerator::new(levels.clone()));
        StreamingAggregator {
            base_interval_secs,
            levels,
            generator,
        }
    }

    /// Create with a custom generator
    pub fn with_generator(
        base_interval_secs: u64,
        levels: Vec<u64>,
        generator: Box<dyn LevelGenerator>,
    ) -> Self {
        StreamingAggregator {
            base_interval_secs,
            levels,
            generator,
        }
    }

    /// Push a new item for aggregation
    pub fn push(&mut self, item: &dyn QuoteLike) {
        self.generator.ingest(item);
    }

    /// Push multiple items
    pub fn push_batch(&mut self, items: &[&dyn QuoteLike]) {
        for item in items {
            self.push(*item);
        }
    }

    /// Seal and return the aggregated data
    pub fn seal(self) -> LevelBatch {
        self.generator.finalize()
    }

    /// Get the configured intervals
    pub fn intervals(&self) -> &[u64] {
        &self.levels
    }
}
