//! Trade data simulator for testing and development

use crate::PlotTrade;
use std::f64::consts::PI;

/// Simple pseudo-random number generator using xorshift
struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Rng {
            state: seed.max(1), // Avoid zero state
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / ((1u64 << 53) as f64))
    }

    /// Box-Muller transform for Gaussian distribution
    fn next_gaussian(&mut self) -> f64 {
        let u1 = self.next_f64();
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
    }
}

/// Trade data simulator
pub struct TradeSimulator {
    symbol: String,
    base_price: f64,
    current_price: f64,
    volatility: f64,
    trades_per_sec: f64,
    next_emit_ns: i64,
    emit_interval_ns: i64,
    rng: Rng,
    exchange_id: u16,
    /// Volume scaling factor
    base_volume: f64,
    /// Trend factor (-1.0 to 1.0, affects price drift)
    trend: f64,
}

impl TradeSimulator {
    /// Safely convert f64 to f32, clamping to valid range
    #[inline]
    fn to_f32(val: f64) -> f32 {
        if val > f32::MAX as f64 {
            f32::MAX
        } else if val < f32::MIN as f64 {
            f32::MIN
        } else {
            val as f32
        }
    }

    /// Create a new trade simulator
    ///
    /// # Arguments
    /// * `symbol` - Trading symbol (e.g., "SPY", "AAPL")
    /// * `base_price` - Starting price
    /// * `volatility` - Price volatility (standard deviation per second)
    /// * `trades_per_sec` - Average number of trades per second
    /// * `start_time_ns` - Starting timestamp in nanoseconds
    /// * `seed` - Random seed for reproducibility
    pub fn new(
        symbol: String,
        base_price: f64,
        volatility: f64,
        trades_per_sec: f64,
        start_time_ns: i64,
        seed: u64,
    ) -> Self {
        let emit_interval_ns = if trades_per_sec > 0.0 {
            (1_000_000_000.0 / trades_per_sec) as i64
        } else {
            1_000_000_000 // 1 second default
        };

        TradeSimulator {
            symbol,
            base_price,
            current_price: base_price,
            volatility,
            trades_per_sec,
            next_emit_ns: start_time_ns,
            emit_interval_ns,
            rng: Rng::new(seed),
            exchange_id: 0,
            base_volume: 100.0,
            trend: 0.0,
        }
    }

    /// Set the exchange ID for generated trades
    pub fn with_exchange(mut self, exchange_id: u16) -> Self {
        self.exchange_id = exchange_id;
        self
    }

    /// Set base volume for trades
    pub fn with_base_volume(mut self, base_volume: f64) -> Self {
        self.base_volume = base_volume;
        self
    }

    /// Set price trend (-1.0 for downtrend, 0.0 for neutral, 1.0 for uptrend)
    pub fn with_trend(mut self, trend: f64) -> Self {
        self.trend = trend.clamp(-1.0, 1.0);
        self
    }

    /// Generate trades up to the specified time
    ///
    /// Returns a vector of trades that should have occurred between
    /// the last tick and now_ns.
    pub fn tick(&mut self, now_ns: i64) -> Vec<PlotTrade> {
        let mut trades = Vec::new();

        while self.next_emit_ns <= now_ns {
            let trade = self.generate_trade(self.next_emit_ns);
            trades.push(trade);
            self.next_emit_ns += self.emit_interval_ns;
        }

        trades
    }

    /// Generate a single trade at the specified time
    fn generate_trade(&mut self, ts_ns: i64) -> PlotTrade {
        // Geometric Brownian Motion price update
        let dt = self.emit_interval_ns as f64 / 1_000_000_000.0; // Convert to seconds
        let drift = self.trend * self.volatility;
        let diffusion = self.volatility * (dt.sqrt()) * self.rng.next_gaussian();

        // Update price using GBM: dS = μ*S*dt + σ*S*dW
        let price_change = self.current_price * (drift * dt + diffusion);
        self.current_price = (self.current_price + price_change).max(0.01); // Floor at 1 cent

        // Generate volume with log-normal distribution
        let volume_multiplier = (self.rng.next_gaussian() * 1.5).exp();
        let volume = (self.base_volume * volume_multiplier).max(1.0);

        // Randomly assign buy/sell side (60/40 split when trending)
        let buy_probability = if self.trend > 0.0 {
            0.5 + self.trend * 0.1 // Up to 60% buys in uptrend
        } else if self.trend < 0.0 {
            0.5 + self.trend * 0.1 // Down to 40% buys in downtrend
        } else {
            0.5
        };

        let side = if self.rng.next_f64() < buy_probability {
            b'B'
        } else {
            b'S'
        };

        PlotTrade::new(
            ts_ns,
            Self::to_f32(self.current_price),
            Self::to_f32(volume),
            side,
            0, // flags
            self.exchange_id,
        )
    }

    /// Get the current price
    pub fn current_price(&self) -> f64 {
        self.current_price
    }

    /// Get the symbol
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Reset to initial state
    pub fn reset(&mut self, start_time_ns: i64, seed: u64) {
        self.current_price = self.base_price;
        self.next_emit_ns = start_time_ns;
        self.rng = Rng::new(seed);
    }

    /// Skip forward in time without generating trades
    pub fn skip_to(&mut self, time_ns: i64) {
        if time_ns > self.next_emit_ns {
            self.next_emit_ns = time_ns;
        }
    }

    /// Generate a burst of trades for backpressure testing
    ///
    /// Creates many trades with microsecond-level spacing to simulate
    /// extreme market activity bursts.
    pub fn generate_burst(&mut self, now_ns: i64, burst_size: usize) -> Vec<PlotTrade> {
        let mut trades = Vec::with_capacity(burst_size);

        // Generate trades with microsecond spacing (1000ns apart)
        let burst_interval_ns = 1000;
        let mut current_ts = now_ns;

        for _ in 0..burst_size {
            let trade = self.generate_trade(current_ts);
            trades.push(trade);
            current_ts += burst_interval_ns;
        }

        // Update next_emit_ns to resume normal pattern after burst
        if current_ts > self.next_emit_ns {
            self.next_emit_ns = current_ts + self.emit_interval_ns;
        }

        trades
    }

    /// Create a high-frequency simulator (for stress testing)
    pub fn high_frequency(symbol: String, base_price: f64, start_time_ns: i64, seed: u64) -> Self {
        Self::new(symbol, base_price, 0.02, 500.0, start_time_ns, seed).with_base_volume(50.0)
    }

    /// Create a low-frequency simulator (for slower stocks)
    pub fn low_frequency(symbol: String, base_price: f64, start_time_ns: i64, seed: u64) -> Self {
        Self::new(symbol, base_price, 0.01, 10.0, start_time_ns, seed).with_base_volume(200.0)
    }
}

/// Multi-stream simulator for testing concurrent data sources
pub struct MultiStreamSimulator {
    simulators: Vec<TradeSimulator>,
    /// Pre-allocated buffer for collecting trades
    trade_buffer: Vec<Vec<PlotTrade>>,
}

impl MultiStreamSimulator {
    /// Create a new multi-stream simulator
    pub fn new(count: usize, base_time_ns: i64) -> Self {
        let mut simulators = Vec::with_capacity(count);
        let mut trade_buffer = Vec::with_capacity(count);

        for i in 0..count {
            let symbol = format!("SYM{:03}", i);
            let base_price = 100.0 + (i as f64 * 10.0);
            let volatility = 0.01 + (i as f64 * 0.001);
            let trades_per_sec = 50.0 + (i as f64 * 25.0);
            let seed = 12345 + i as u64;

            let sim = TradeSimulator::new(
                symbol,
                base_price,
                volatility,
                trades_per_sec,
                base_time_ns,
                seed,
            )
            .with_exchange(i as u16);

            simulators.push(sim);
            trade_buffer.push(Vec::with_capacity(512)); // Pre-allocate for typical burst
        }

        MultiStreamSimulator {
            simulators,
            trade_buffer,
        }
    }

    /// Tick all simulators and return all generated trades
    pub fn tick(&mut self, now_ns: i64) -> Vec<PlotTrade> {
        // Clear buffers from last tick
        for buffer in &mut self.trade_buffer {
            buffer.clear();
        }

        // Collect trades from each simulator into separate buffers
        let mut total_trades = 0;
        for (i, sim) in self.simulators.iter_mut().enumerate() {
            let trades = sim.tick(now_ns);
            total_trades += trades.len();
            self.trade_buffer[i] = trades;
        }

        // If only one stream has trades, return directly (common case)
        if total_trades == 0 {
            return Vec::new();
        }

        // K-way merge of sorted streams (each simulator produces sorted trades)
        let mut result = Vec::with_capacity(total_trades);
        let mut indices = vec![0usize; self.simulators.len()];

        for _ in 0..total_trades {
            let mut min_ts = i64::MAX;
            let mut min_stream = None;

            // Find the stream with the earliest trade
            for (stream_idx, buffer) in self.trade_buffer.iter().enumerate() {
                if indices[stream_idx] < buffer.len() {
                    let trade_ts = buffer[indices[stream_idx]].ts;
                    if trade_ts < min_ts {
                        min_ts = trade_ts;
                        min_stream = Some(stream_idx);
                    }
                }
            }

            // Add the earliest trade and advance that stream's index
            if let Some(stream_idx) = min_stream {
                let trade_idx = indices[stream_idx];
                result.push(self.trade_buffer[stream_idx][trade_idx]);
                indices[stream_idx] += 1;
            }
        }

        result
    }

    /// Get number of streams
    pub fn stream_count(&self) -> usize {
        self.simulators.len()
    }

    /// Get a specific simulator
    pub fn get_simulator(&self, index: usize) -> Option<&TradeSimulator> {
        self.simulators.get(index)
    }

    /// Get a mutable reference to a specific simulator
    pub fn get_simulator_mut(&mut self, index: usize) -> Option<&mut TradeSimulator> {
        self.simulators.get_mut(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rng_reproducibility() {
        let mut rng1 = Rng::new(42);
        let mut rng2 = Rng::new(42);

        for _ in 0..100 {
            assert_eq!(rng1.next_u64(), rng2.next_u64());
        }
    }

    #[test]
    fn test_simulator_basic() {
        let mut sim = TradeSimulator::new(
            "TEST".to_string(),
            100.0,
            0.02,
            10.0, // 10 trades per second
            0,
            42,
        );

        // Tick forward 1 second
        let trades = sim.tick(1_000_000_000);

        // Should generate approximately 10 trades
        assert!(
            trades.len() >= 8 && trades.len() <= 12,
            "Got {} trades",
            trades.len()
        );

        // All trades should have correct symbol context (exchange, etc.)
        for trade in &trades {
            assert!(trade.price > 0.0);
            assert!(trade.size > 0.0);
            assert!(trade.side == b'B' || trade.side == b'S');
        }
    }

    #[test]
    fn test_simulator_reproducibility() {
        let mut sim1 = TradeSimulator::new("TEST".to_string(), 100.0, 0.02, 10.0, 0, 42);
        let mut sim2 = TradeSimulator::new("TEST".to_string(), 100.0, 0.02, 10.0, 0, 42);

        let trades1 = sim1.tick(1_000_000_000);
        let trades2 = sim2.tick(1_000_000_000);

        assert_eq!(trades1.len(), trades2.len());

        for (t1, t2) in trades1.iter().zip(trades2.iter()) {
            assert_eq!(t1.ts, t2.ts);
            assert_eq!(t1.price, t2.price);
            assert_eq!(t1.size, t2.size);
            assert_eq!(t1.side, t2.side);
        }
    }

    #[test]
    fn test_simulator_price_movement() {
        let mut sim = TradeSimulator::new("TEST".to_string(), 100.0, 0.05, 100.0, 0, 42);

        let start_price = sim.current_price();

        // Generate 10 seconds of trades
        for i in 1..=10 {
            sim.tick(i * 1_000_000_000);
        }

        let end_price = sim.current_price();

        // Price should have moved (very unlikely to be exactly the same)
        assert_ne!(start_price, end_price);

        // Price should still be somewhat reasonable (within 50% of starting price)
        assert!(end_price > start_price * 0.5);
        assert!(end_price < start_price * 1.5);
    }

    #[test]
    fn test_simulator_with_trend() {
        let mut sim_up =
            TradeSimulator::new("TEST".to_string(), 100.0, 0.02, 50.0, 0, 42).with_trend(1.0);

        let mut sim_down =
            TradeSimulator::new("TEST".to_string(), 100.0, 0.02, 50.0, 0, 43).with_trend(-1.0);

        // Run for 5 seconds
        for i in 1..=5 {
            sim_up.tick(i * 1_000_000_000);
            sim_down.tick(i * 1_000_000_000);
        }

        // Uptrend should generally increase price, downtrend should decrease
        // (Though randomness means this isn't guaranteed for short runs)
        let up_price = sim_up.current_price();
        let down_price = sim_down.current_price();

        // At least verify they're different
        assert_ne!(up_price, down_price);
    }

    #[test]
    fn test_high_frequency_simulator() {
        let mut sim = TradeSimulator::high_frequency("SPY".to_string(), 450.0, 0, 42);

        // Should generate ~500 trades per second
        let trades = sim.tick(1_000_000_000);
        assert!(trades.len() >= 450 && trades.len() <= 550);
    }

    #[test]
    fn test_multi_stream_simulator() {
        let mut multi = MultiStreamSimulator::new(5, 0);

        assert_eq!(multi.stream_count(), 5);

        // Tick forward 1 second
        let trades = multi.tick(1_000_000_000);

        // Should have trades from all 5 streams
        assert!(trades.len() > 0);

        // Verify trades are sorted by timestamp
        for i in 1..trades.len() {
            assert!(trades[i].ts >= trades[i - 1].ts);
        }
    }

    #[test]
    fn test_simulator_reset() {
        let mut sim = TradeSimulator::new("TEST".to_string(), 100.0, 0.02, 10.0, 0, 42);

        // Generate some trades
        let trades1 = sim.tick(1_000_000_000);

        // Reset
        sim.reset(0, 42);

        // Should generate identical trades
        let trades2 = sim.tick(1_000_000_000);

        assert_eq!(trades1.len(), trades2.len());
        for (t1, t2) in trades1.iter().zip(trades2.iter()) {
            assert_eq!(t1.price, t2.price);
            assert_eq!(t1.size, t2.size);
        }
    }

    #[test]
    fn test_simulator_skip_to() {
        let mut sim = TradeSimulator::new("TEST".to_string(), 100.0, 0.02, 10.0, 0, 42);

        // Skip to 10 seconds
        sim.skip_to(10_000_000_000);

        // Next tick should not generate any trades for times before skip
        let trades = sim.tick(10_500_000_000);

        // Should only generate trades for the 0.5 second window
        assert!(trades.len() <= 10);

        // All trades should be after the skip time
        for trade in &trades {
            assert!(trade.ts >= 10_000_000_000);
        }
    }

    #[test]
    fn test_burst_generation() {
        let mut sim = TradeSimulator::new("TEST".to_string(), 100.0, 0.02, 100.0, 0, 42);

        // Generate a burst of 1000 trades
        let burst_trades = sim.generate_burst(1_000_000_000, 1000);

        assert_eq!(burst_trades.len(), 1000);

        // Check that trades are microsecond-spaced
        for i in 1..burst_trades.len() {
            let time_diff = burst_trades[i].ts - burst_trades[i - 1].ts;
            assert_eq!(time_diff, 1000); // 1 microsecond spacing
        }

        // All trades should have valid prices
        for trade in &burst_trades {
            assert!(trade.price > 0.0);
            assert!(trade.size > 0.0);
        }
    }
}
