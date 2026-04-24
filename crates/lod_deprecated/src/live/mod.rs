//! Live streaming engine for real-time market data aggregation
//! The simulator produces PlotTrade structs through a simple tick() method.
//! Real APIs would implement the same interface:
//! trait TradeSource {
//!     fn tick(&mut self, now_ns: i64) -> Vec<PlotTrade>;
//! }
//!
//!   Key Design Decisions Supporting Replaceability:
//!
//!   1. Standard data structure: PlotTrade is generic enough to represent data
//!   from any source
//!   2. Time-based polling: The tick(now_ns) pattern works for both simulated
//!   and real data
//!   3. Stateless output: Returns simple vectors of trades without complex
//!   dependencies
//!   4. No simulator-specific logic in LiveEngine: The engine only cares about
//!   PlotTrade objects, not their source
//!
//!   Migration Path:
//!   1. IBKR API: Would replace TradeSimulator::tick() with:
//!     - WebSocket subscription to market data
//!     - Convert IBKR tick data to PlotTrade format
//!     - Buffer trades until tick() is called
//!   2. Databento API: Would implement:
//!     - Connect to Databento's real-time feed
//!     - Transform DBN format to PlotTrade
//!     - Handle reconnection logic internally

//   Minimal Code Changes:

//   // Current: Simulator
//   let mut sim = TradeSimulator::new(...);
//   let trades = sim.tick(now_ns);

//   // Future: Real API
//   let mut source = IbkrTradeSource::new(symbol);
//   let trades = source.tick(now_ns);  // Same interface!

//   The abstraction is clean - LiveEngine doesn't know or care whether trades
//   come from simulation or real markets, making the transition seamless.

pub mod simulator;

use crate::{PlotCandle, PlotTrade};
use std::collections::VecDeque;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

pub use simulator::{MultiStreamSimulator, TradeSimulator};

/// Lock-free SPSC ring buffer for hot trade data
///
/// # Safety Invariants
/// - Single Producer: Only one thread writes (updates head)
/// - Single Consumer: Only one thread reads (updates tail)
/// - Ring uses N-1 slots: head == tail means empty, (head+1) & mask == tail means full
pub struct HotTradesRing {
    buf: Box<[MaybeUninit<PlotTrade>]>,
    head: AtomicUsize,
    tail: AtomicUsize,
    mask: usize, // capacity - 1 for fast modulo
}

impl HotTradesRing {
    /// Create a new ring buffer with power-of-2 capacity
    ///
    /// # Panics
    /// Panics if capacity_log2 < 4 or > 24 (16 to 16M capacity)
    pub fn new(capacity_log2: u32) -> Self {
        assert!(
            capacity_log2 >= 4 && capacity_log2 <= 24,
            "capacity_log2 must be between 4 and 24"
        );

        let capacity = 1 << capacity_log2;

        // Create uninitialized buffer - safe because we track which slots are valid
        let mut buf = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            buf.push(MaybeUninit::uninit());
        }

        HotTradesRing {
            buf: buf.into_boxed_slice(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            mask: capacity - 1,
        }
    }

    /// Push a trade to the ring. Returns false if full.
    pub fn push_trade(&self, trade: PlotTrade) -> bool {
        let head = self.head.load(Ordering::Acquire);
        let next_head = (head + 1) & self.mask;
        let tail = self.tail.load(Ordering::Acquire);

        if next_head == tail {
            return false; // Ring is full, would overwrite unread data
        }

        // SAFETY: We have exclusive write access to slot[head] because:
        // - We're the only producer (SPSC invariant)
        // - head != tail (checked above), so slot is not being read
        unsafe {
            let ptr = self.buf.as_ptr() as *mut MaybeUninit<PlotTrade>;
            (*ptr.add(head)).write(trade);
        }

        self.head.store(next_head, Ordering::Release);
        true
    }

    /// Drain trades into a vector. Returns number of trades drained.
    pub fn drain_to(&self, out: &mut Vec<PlotTrade>) -> usize {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Acquire);

        if tail == head {
            return 0; // Empty
        }

        let mut count = 0;
        let mut current = tail;

        while current != head {
            // SAFETY: We have exclusive read access to slot[current] because:
            // - We're the only consumer (SPSC invariant)
            // - current is between tail and head, so slot contains valid data
            // - We'll advance tail past this slot, preventing re-reads
            let trade = unsafe {
                let ptr = self.buf.as_ptr() as *const MaybeUninit<PlotTrade>;
                (*ptr.add(current)).assume_init_read()
            };

            out.push(trade);
            current = (current + 1) & self.mask;
            count += 1;
        }

        self.tail.store(head, Ordering::Release);
        count
    }

    /// Get current buffer usage
    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);

        if head >= tail {
            head - tail
        } else {
            (self.mask + 1) - tail + head
        }
    }

    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Acquire) == self.tail.load(Ordering::Acquire)
    }
}

/// Single bar accumulator for real-time aggregation
#[derive(Debug, Clone)]
pub struct BarAgg {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub vol: f64,
    pub vwap_num: f64, // sum(px * qty)
    pub vwap_den: f64, // sum(qty)
    pub count: u32,
    pub start_ns: i64,
    pub end_ns: i64, // exclusive boundary
}

impl BarAgg {
    /// Create a new empty bar for the given time window
    pub fn new(start_ns: i64, end_ns: i64) -> Self {
        BarAgg {
            open: 0.0,
            high: f64::NEG_INFINITY,
            low: f64::INFINITY,
            close: 0.0,
            vol: 0.0,
            vwap_num: 0.0,
            vwap_den: 0.0,
            count: 0,
            start_ns,
            end_ns,
        }
    }

    /// Add a trade to this bar
    pub fn add_trade(&mut self, trade: &PlotTrade) {
        let price = trade.price as f64;
        let size = trade.size as f64;

        if self.count == 0 {
            self.open = price;
        }

        self.high = self.high.max(price);
        self.low = self.low.min(price);
        self.close = price;
        self.vol += size;
        self.vwap_num += price * size;
        self.vwap_den += size;
        self.count += 1;
    }

    /// Convert to PlotCandle
    pub fn to_candle(&self) -> PlotCandle {
        PlotCandle::new(
            self.start_ns,
            self.open as f32,
            self.high as f32,
            self.low as f32,
            self.close as f32,
            self.vol as f32,
        )
    }

    /// Get VWAP price
    pub fn vwap(&self) -> f64 {
        if self.vwap_den > 0.0 {
            self.vwap_num / self.vwap_den
        } else {
            self.close
        }
    }

    /// Check if this bar contains any trades
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Add a candle to this bar aggregation (for building coarser intervals).
    ///
    /// This combines multiple fine-grained candles into a single coarser candle.
    pub fn add_candle(&mut self, candle: &PlotCandle) {
        let open = candle.open as f64;
        let high = candle.high as f64;
        let low = candle.low as f64;
        let close = candle.close as f64;
        let volume = candle.volume as f64;

        if self.count == 0 {
            self.open = open;
        }

        self.high = self.high.max(high);
        self.low = self.low.min(low);
        self.close = close;
        self.vol += volume;

        // For VWAP, use the candle's close price as representative
        // weighted by its volume
        self.vwap_num += close * volume;
        self.vwap_den += volume;
        self.count += 1;
    }
}

/// Per-interval aggregation state
pub struct LodLane {
    interval_ns: i64,
    current: BarAgg,
    completed: VecDeque<PlotCandle>,
    max_completed: usize,
}

impl LodLane {
    /// Create a new lane for the given interval
    pub fn new(interval_secs: u64, max_completed: usize, base_epoch_ns: i64) -> Self {
        let interval_ns = interval_secs as i64 * 1_000_000_000;
        let current = BarAgg::new(base_epoch_ns, base_epoch_ns + interval_ns);

        LodLane {
            interval_ns,
            current,
            completed: VecDeque::with_capacity(max_completed),
            max_completed,
        }
    }

    /// Process a trade, completing bars as needed
    pub fn process_trade(&mut self, trade: &PlotTrade) {
        // Check if trade belongs to current bar
        if trade.ts >= self.current.end_ns {
            // Complete current bar if it has data
            if !self.current.is_empty() {
                let candle = self.current.to_candle();
                self.completed.push_back(candle);

                // Limit completed buffer size
                if self.completed.len() > self.max_completed {
                    self.completed.pop_front();
                }
            }

            // Create new bar(s) for this trade
            let bar_start = (trade.ts / self.interval_ns) * self.interval_ns;
            let bar_end = bar_start + self.interval_ns;
            self.current = BarAgg::new(bar_start, bar_end);
        }

        self.current.add_trade(trade);
    }

    /// Get snapshot of completed candles
    pub fn completed(&self) -> &VecDeque<PlotCandle> {
        &self.completed
    }

    /// Get current open bar
    pub fn open_bar(&self) -> Option<PlotCandle> {
        if self.current.is_empty() {
            None
        } else {
            Some(self.current.to_candle())
        }
    }

    /// Drain old completed bars, keeping only the last N
    pub fn drain_old(&mut self, keep_last: usize) -> Vec<PlotCandle> {
        if self.completed.len() <= keep_last {
            return Vec::new();
        }

        let drain_count = self.completed.len() - keep_last;
        self.completed.drain(..drain_count).collect()
    }
}

/// Main live streaming engine
pub struct LiveEngine {
    trades: HotTradesRing,
    lods: Vec<LodLane>,
    intervals_secs: Vec<u64>,
    #[allow(dead_code)]
    base_epoch_ns: i64,
    #[allow(dead_code)]
    reorder_ns: i64,
    last_update_ns: AtomicI64,
    trades_dropped: AtomicU64,
    trades_processed: AtomicU64,
    last_drop_ns: AtomicI64,
}

impl LiveEngine {
    /// Create a new live engine with specified intervals
    pub fn new(
        intervals_secs: Vec<u64>,
        base_epoch_ns: i64,
        reorder_ns: i64,
        ring_capacity_log2: u32,
        max_completed_per_lane: usize,
    ) -> Self {
        let lods = intervals_secs
            .iter()
            .map(|&interval| LodLane::new(interval, max_completed_per_lane, base_epoch_ns))
            .collect();

        LiveEngine {
            trades: HotTradesRing::new(ring_capacity_log2),
            lods,
            intervals_secs,
            base_epoch_ns,
            reorder_ns,
            last_update_ns: AtomicI64::new(0),
            trades_dropped: AtomicU64::new(0),
            trades_processed: AtomicU64::new(0),
            last_drop_ns: AtomicI64::new(0),
        }
    }

    /// Ingest a trade into the hot ring
    pub fn ingest_trade(&self, trade: PlotTrade) -> bool {
        let result = self.trades.push_trade(trade);

        if !result {
            self.trades_dropped.fetch_add(1, Ordering::Relaxed);
            self.last_drop_ns.store(trade.ts, Ordering::Relaxed);
        }

        result
    }

    /// Process trades from ring into aggregators
    pub fn process(&mut self) {
        let mut temp_trades = Vec::with_capacity(1024);
        let count = self.trades.drain_to(&mut temp_trades);

        if count == 0 {
            return;
        }

        self.trades_processed
            .fetch_add(count as u64, Ordering::Relaxed);

        // Update all LOD lanes
        for trade in &temp_trades {
            for lane in &mut self.lods {
                lane.process_trade(trade);
            }

            self.last_update_ns.store(trade.ts, Ordering::Relaxed);
        }
    }

    /// Get snapshot for a specific interval
    pub fn snapshot(&self, interval_secs: u64) -> Option<LiveSnapshot> {
        let lane_idx = self
            .intervals_secs
            .iter()
            .position(|&i| i == interval_secs)?;

        let lane = &self.lods[lane_idx];
        let completed: Arc<[PlotCandle]> = lane.completed().iter().copied().collect();
        let open_bar = lane.open_bar();
        let last_trade_ns = self.last_update_ns.load(Ordering::Relaxed);

        Some(LiveSnapshot {
            completed,
            open_bar,
            last_trade_ns,
        })
    }

    /// Drain old bars from a specific interval, keeping last N
    pub fn drain_old(&mut self, interval_secs: u64, keep_last: usize) -> Vec<PlotCandle> {
        let lane_idx = self.intervals_secs.iter().position(|&i| i == interval_secs);

        match lane_idx {
            Some(idx) => self.lods[idx].drain_old(keep_last),
            None => Vec::new(),
        }
    }

    /// Get performance metrics
    pub fn metrics(&self) -> LiveMetrics {
        let processed = self.trades_processed.load(Ordering::Relaxed);
        let dropped = self.trades_dropped.load(Ordering::Relaxed);
        let total = processed + dropped;

        let drop_rate = if total > 0 {
            dropped as f64 / total as f64
        } else {
            0.0
        };

        LiveMetrics {
            trades_processed: processed,
            trades_dropped: dropped,
            drop_rate,
            last_drop_ns: self.last_drop_ns.load(Ordering::Relaxed),
            avg_latency_us: 0.0, // TODO: implement latency tracking
        }
    }

    /// Get available intervals
    pub fn intervals(&self) -> &[u64] {
        &self.intervals_secs
    }

    /// Inject pre-completed bars into a specific interval lane.
    ///
    /// This is used for backfilling historical data (e.g., "today-so-far" bars).
    /// The bars are added directly to the completed buffer, not processed as trades.
    ///
    /// # Arguments
    /// * `interval_secs` - The interval to inject bars into
    /// * `bars` - Pre-computed historical bars to inject
    ///
    /// # Returns
    /// The number of bars successfully injected
    pub fn inject_completed_bars(&mut self, interval_secs: u64, bars: Vec<PlotCandle>) -> usize {
        let lane_idx = match self.intervals_secs.iter().position(|&i| i == interval_secs) {
            Some(idx) => idx,
            None => return 0,
        };

        let lane = &mut self.lods[lane_idx];
        let mut injected = 0;

        for bar in bars {
            // Add to completed queue, respecting max size
            lane.completed.push_back(bar);
            if lane.completed.len() > lane.max_completed {
                lane.completed.pop_front();
            }
            injected += 1;
        }

        injected
    }

    /// Rebuild coarser intervals from the finest interval's completed bars.
    ///
    /// This is used after injecting backfill bars into the finest interval
    /// to build the corresponding coarser interval bars (e.g., 15s, 30s from 5s).
    ///
    /// This method aggregates completed bars from the first interval into
    /// all coarser intervals.
    pub fn rebuild_coarser_intervals(&mut self) {
        if self.intervals_secs.len() < 2 {
            return; // Nothing to rebuild
        }

        // Get the finest interval's completed bars
        let finest_bars: Vec<PlotCandle> = self.lods[0].completed.iter().copied().collect();

        if finest_bars.is_empty() {
            return;
        }

        // Rebuild each coarser interval
        for lane_idx in 1..self.lods.len() {
            let interval_secs = self.intervals_secs[lane_idx];
            let interval_ns = (interval_secs as i64) * 1_000_000_000;

            // Aggregate finest bars into this interval
            let mut aggregated_bars: Vec<PlotCandle> = Vec::new();
            let mut current_agg: Option<BarAgg> = None;

            for bar in &finest_bars {
                let bar_start = (bar.ts / interval_ns) * interval_ns;
                let bar_end = bar_start + interval_ns;

                // Check if we need to start a new aggregation
                match &mut current_agg {
                    None => {
                        // Start new aggregation
                        let mut agg = BarAgg::new(bar_start, bar_end);
                        agg.add_candle(bar);
                        current_agg = Some(agg);
                    }
                    Some(agg) if bar.ts >= agg.end_ns => {
                        // Complete current and start new
                        aggregated_bars.push(agg.to_candle());
                        let mut new_agg = BarAgg::new(bar_start, bar_end);
                        new_agg.add_candle(bar);
                        current_agg = Some(new_agg);
                    }
                    Some(agg) => {
                        // Add to current aggregation
                        agg.add_candle(bar);
                    }
                }
            }

            // Don't include the final incomplete bar
            // (it might still be receiving updates from live trades)

            // Inject the aggregated bars into this lane
            let lane = &mut self.lods[lane_idx];
            lane.completed.clear();
            for bar in aggregated_bars {
                lane.completed.push_back(bar);
                if lane.completed.len() > lane.max_completed {
                    lane.completed.pop_front();
                }
            }
        }
    }
}

/// Render-ready snapshot of live data
#[derive(Debug, Clone)]
pub struct LiveSnapshot {
    pub completed: Arc<[PlotCandle]>,
    pub open_bar: Option<PlotCandle>,
    pub last_trade_ns: i64,
}

/// Performance monitoring metrics
#[derive(Debug, Clone)]
pub struct LiveMetrics {
    pub trades_processed: u64,
    pub trades_dropped: u64,
    pub drop_rate: f64,
    pub last_drop_ns: i64,
    pub avg_latency_us: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "capacity_log2 must be between 4 and 24")]
    fn test_hot_trades_ring_invalid_capacity() {
        let _ = HotTradesRing::new(3); // Too small
    }

    #[test]
    fn test_hot_trades_ring_basic() {
        let ring = HotTradesRing::new(4); // 16 capacity

        // Push some trades
        for i in 0..10 {
            let trade = PlotTrade::new(i, 100.0 + i as f32, 1.0, b'B', 0, 0);
            assert!(ring.push_trade(trade));
        }

        assert_eq!(ring.len(), 10);

        // Drain trades
        let mut out = Vec::new();
        let count = ring.drain_to(&mut out);
        assert_eq!(count, 10);
        assert_eq!(out.len(), 10);
        assert!(ring.is_empty());
    }

    #[test]
    fn test_hot_trades_ring_overflow() {
        let ring = HotTradesRing::new(4); // 16 capacity, but only 15 usable slots

        // Fill to usable capacity (capacity - 1)
        for i in 0..15 {
            let trade = PlotTrade::new(i, 100.0, 1.0, b'B', 0, 0);
            assert!(ring.push_trade(trade));
        }

        // Next push should fail (ring is full)
        let trade = PlotTrade::new(100, 100.0, 1.0, b'B', 0, 0);
        assert!(!ring.push_trade(trade));
    }

    #[test]
    fn test_bar_agg_single_trade() {
        let mut bar = BarAgg::new(0, 1_000_000_000);
        let trade = PlotTrade::new(500_000_000, 100.5, 10.0, b'B', 0, 0);

        bar.add_trade(&trade);

        assert_eq!(bar.open, 100.5);
        assert_eq!(bar.high, 100.5);
        assert_eq!(bar.low, 100.5);
        assert_eq!(bar.close, 100.5);
        assert_eq!(bar.vol, 10.0);
        assert_eq!(bar.count, 1);
    }

    #[test]
    fn test_bar_agg_multiple_trades() {
        let mut bar = BarAgg::new(0, 1_000_000_000);

        let trades = vec![
            PlotTrade::new(100_000_000, 100.0, 5.0, b'B', 0, 0),
            PlotTrade::new(200_000_000, 102.0, 3.0, b'B', 0, 0),
            PlotTrade::new(300_000_000, 99.0, 2.0, b'S', 0, 0),
            PlotTrade::new(400_000_000, 101.0, 4.0, b'B', 0, 0),
        ];

        for trade in &trades {
            bar.add_trade(trade);
        }

        assert_eq!(bar.open, 100.0);
        assert_eq!(bar.high, 102.0);
        assert_eq!(bar.low, 99.0);
        assert_eq!(bar.close, 101.0);
        assert_eq!(bar.vol, 14.0);
        assert_eq!(bar.count, 4);

        // VWAP = (100*5 + 102*3 + 99*2 + 101*4) / 14 = 100.57...
        let vwap = bar.vwap();
        assert!((vwap - 100.571428).abs() < 0.001);
    }

    #[test]
    fn test_lod_lane_bar_completion() {
        let interval_secs = 1;
        let base_epoch = 0;
        let mut lane = LodLane::new(interval_secs, 100, base_epoch);

        // Add trades in first bar
        let trade1 = PlotTrade::new(500_000_000, 100.0, 1.0, b'B', 0, 0);
        lane.process_trade(&trade1);

        assert_eq!(lane.completed().len(), 0);
        assert!(lane.open_bar().is_some());

        // Add trade in second bar (should complete first bar)
        let trade2 = PlotTrade::new(1_500_000_000, 101.0, 1.0, b'B', 0, 0);
        lane.process_trade(&trade2);

        assert_eq!(lane.completed().len(), 1);
        let completed = &lane.completed()[0];
        assert_eq!(completed.ts, 0);
        assert_eq!(completed.close, 100.0);
    }

    #[test]
    fn test_live_engine_basic() {
        let intervals = vec![1, 5, 15];
        let base_epoch = 0;
        let reorder_ns = 100_000_000;
        let mut engine = LiveEngine::new(intervals, base_epoch, reorder_ns, 4, 100);

        // Ingest trades
        for i in 0..10 {
            let trade = PlotTrade::new(i * 100_000_000, 100.0 + i as f32, 1.0, b'B', 0, 0);
            assert!(engine.ingest_trade(trade));
        }

        // Process trades
        engine.process();

        // Check metrics
        let metrics = engine.metrics();
        assert_eq!(metrics.trades_processed, 10);
        assert_eq!(metrics.trades_dropped, 0);
        assert_eq!(metrics.drop_rate, 0.0);
    }

    #[test]
    fn test_live_engine_snapshot() {
        let intervals = vec![1];
        let base_epoch = 0;
        let reorder_ns = 100_000_000;
        let mut engine = LiveEngine::new(intervals, base_epoch, reorder_ns, 4, 100);

        // Ingest trades in first bar
        let trade1 = PlotTrade::new(500_000_000, 100.0, 1.0, b'B', 0, 0);
        engine.ingest_trade(trade1);

        // Ingest trade in second bar
        let trade2 = PlotTrade::new(1_500_000_000, 101.0, 1.0, b'B', 0, 0);
        engine.ingest_trade(trade2);

        engine.process();

        // Get snapshot
        let snapshot = engine.snapshot(1).unwrap();
        assert_eq!(snapshot.completed.len(), 1);
        assert!(snapshot.open_bar.is_some());
    }

    #[test]
    fn test_simulator_integration_with_engine() {
        use crate::live::simulator::TradeSimulator;

        // Create simulator
        let mut sim = TradeSimulator::new(
            "SPY".to_string(),
            450.0,
            0.02,
            100.0, // 100 trades/sec
            0,
            42,
        );

        // Create engine with multiple intervals
        let intervals = vec![1, 5, 15];
        let base_epoch = 0;
        let reorder_ns = 100_000_000;
        let mut engine = LiveEngine::new(intervals, base_epoch, reorder_ns, 14, 200);

        // Simulate 3 seconds of trading
        for i in 1..=3 {
            let now_ns = i * 1_000_000_000;
            let trades = sim.tick(now_ns);

            // Ingest all trades
            for trade in trades {
                assert!(engine.ingest_trade(trade));
            }

            // Process aggregations
            engine.process();
        }

        // Check metrics
        let metrics = engine.metrics();
        assert!(metrics.trades_processed > 250); // ~300 trades total
        assert_eq!(metrics.trades_dropped, 0);

        // Check 1-second interval snapshot
        let snapshot_1s = engine.snapshot(1).unwrap();
        assert_eq!(snapshot_1s.completed.len(), 3); // 3 complete bars
        assert!(snapshot_1s.open_bar.is_some());

        // Verify bars have data
        for candle in snapshot_1s.completed.iter() {
            assert!(candle.open > 0.0);
            assert!(candle.high >= candle.low);
            assert!(candle.volume > 0.0);
        }

        // Check 5-second interval snapshot
        let snapshot_5s = engine.snapshot(5).unwrap();
        assert_eq!(snapshot_5s.completed.len(), 0); // No complete 5s bar yet
        assert!(snapshot_5s.open_bar.is_some()); // But there's an open one
    }

    #[test]
    fn test_multi_stream_integration() {
        use crate::live::simulator::MultiStreamSimulator;

        // Create 3 streams
        let mut multi_sim = MultiStreamSimulator::new(3, 0);

        // Create engine
        let intervals = vec![1];
        let base_epoch = 0;
        let reorder_ns = 100_000_000;
        let mut engine = LiveEngine::new(intervals, base_epoch, reorder_ns, 14, 200);

        // Simulate 2 seconds
        for i in 1..=2 {
            let now_ns = i * 1_000_000_000;
            let trades = multi_sim.tick(now_ns);

            // Ingest all trades from all streams
            for trade in trades {
                engine.ingest_trade(trade);
            }

            engine.process();
        }

        // Should have processed trades from all 3 streams
        let metrics = engine.metrics();
        assert!(metrics.trades_processed > 100); // Multiple streams, many trades

        // Should have aggregated bars
        let snapshot = engine.snapshot(1).unwrap();
        assert_eq!(snapshot.completed.len(), 2); // 2 complete bars after 2 seconds
    }
}
