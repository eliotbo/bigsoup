/// Example demonstrating line overlays on a candlestick chart.
///
/// Generates synthetic price data (random walk), computes a 20-period EMA,
/// and renders the EMA as a light-blue line behind the candles.
///
/// Run with: cargo run --example ema_line_overlay --release
use anyhow::Result;
use lod::{LevelStore, PlotCandle};
use vizza::{LineOverlay, LodLevel, MarketData, PlotBuilder};

fn aggregate_candles(
    candles_1s: &[PlotCandle],
    ticks_per_candle: usize,
) -> Vec<PlotCandle> {
    candles_1s
        .chunks(ticks_per_candle)
        .filter_map(|chunk| {
            let open = chunk[0].open;
            let close = chunk[chunk.len() - 1].close;
            let high = chunk.iter().map(|c| c.high).reduce(f32::max)?;
            let low = chunk.iter().map(|c| c.low).reduce(f32::min)?;
            let volume: f32 = chunk.iter().map(|c| c.volume).sum();
            let ts = chunk[0].ts;
            Some(PlotCandle::new(ts, open, high, low, close, volume))
        })
        .collect()
}

fn build_level_store(candles_1s: &[PlotCandle]) -> LevelStore {
    let lod_intervals: &[u64] = &[1, 5, 15, 30, 60, 300, 900, 1800, 3600, 14400, 86400];

    let mut levels: Vec<(u64, Vec<PlotCandle>)> = Vec::new();
    for &interval_secs in lod_intervals {
        let ticks_per_candle = interval_secs as usize; // 1s base => ticks_per_candle == interval_secs
        if ticks_per_candle > candles_1s.len() {
            break;
        }
        let candles = if ticks_per_candle == 1 {
            candles_1s.to_vec()
        } else {
            aggregate_candles(candles_1s, ticks_per_candle)
        };
        if candles.len() >= 2 {
            levels.push((interval_secs, candles));
        }
    }

    LevelStore::from_stream(levels)
}

fn main() -> Result<()> {
    let n_candles = 2000;
    let base_ts_ns = 1_700_000_000_000_000_000i64; // arbitrary anchor
    let interval_ns = 1_000_000_000i64; // 1 second per candle

    // --- Generate synthetic OHLCV via random walk ---
    let mut rng_state: u64 = 42;
    let next_f32 = |state: &mut u64| -> f32 {
        // xorshift64
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        ((*state as f32) / (u64::MAX as f32)) * 2.0 - 1.0
    };

    let mut price = 100.0_f32;
    let mut candles = Vec::with_capacity(n_candles);

    for i in 0..n_candles {
        let ts = base_ts_ns + i as i64 * interval_ns;
        let open = price;
        let moves: Vec<f32> = (0..4).map(|_| next_f32(&mut rng_state) * 0.5).collect();
        let close = open + moves.iter().sum::<f32>();
        let high = open.max(close) + next_f32(&mut rng_state).abs() * 0.3;
        let low = open.min(close) - next_f32(&mut rng_state).abs() * 0.3;
        let volume = 1000.0 + next_f32(&mut rng_state).abs() * 5000.0;
        candles.push(PlotCandle::new(ts, open, high, low, close, volume));
        price = close;
    }

    // --- Compute 20-period EMA over closing prices ---
    let ema_period = 20;
    let k = 2.0 / (ema_period as f32 + 1.0);
    let mut ema_points: Vec<(i64, f32)> = Vec::with_capacity(n_candles);
    let mut ema = candles[0].close;
    for candle in &candles {
        ema = candle.close * k + ema * (1.0 - k);
        ema_points.push((candle.ts, ema));
    }

    let ema_overlay = LineOverlay::new(ema_points, [0.4, 0.7, 1.0, 0.9]);

    // --- Build multi-LOD LevelStore ---
    let level_store = build_level_store(&candles);
    let market_data = MarketData::from_level_store(level_store);

    PlotBuilder::new()
        .with_window_size(1400, 700)
        .with_grid(1, 1)
        .with_market_data(market_data)
        .with_lod_level(LodLevel::S1)
        .with_line_overlays(vec![vec![ema_overlay]])
        .with_title("Synthetic data + EMA(20)")
        .run()
}
