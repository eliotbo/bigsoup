//! Example demonstrating per-viewport in-memory OHLCV data with different time ranges.
//!
//! Each viewport has data starting at a different time, and we use `with_start_times()`
//! to set different initial x-axis positions for each viewport.

use std::f32::consts::TAU;

use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::America::New_York;
use lod::{LevelStore, PlotCandle};
use rand::Rng;
use vizza::{MarketData, PlotBuilder, PositionOverlay, PriceLevelQuad, Theme};

// Must be larger than viewport capacity (~107 bars) for with_start_times() to work
const BAR_COUNT: usize = 300;

/// Build a series starting at the given timestamp
fn build_series_at<F>(start: DateTime<Utc>, mut generator: F) -> MarketData
where
    F: FnMut(usize, f32) -> (f32, f32, f32, f32),
{
    let mut level_store = LevelStore::new();

    let mut candles = Vec::with_capacity(BAR_COUNT);
    let denom = (BAR_COUNT - 1).max(1) as f32;

    for i in 0..BAR_COUNT {
        let ts = start + Duration::minutes(i as i64);
        let ts_nanos = ts.timestamp_nanos_opt().expect("timestamp in range");
        let progress = i as f32 / denom;

        let (open, close, range, volume) = generator(i, progress);
        let spread = range.abs();
        let high = open.max(close) + spread;
        let low = open.min(close) - spread;

        candles.push(PlotCandle::new(
            ts_nanos,
            open,
            high,
            low,
            close,
            volume.max(0.0),
        ));
    }

    level_store.append(60, &candles, false);
    MarketData::from_level_store(level_store)
}

fn build_sine_series(start: DateTime<Utc>) -> MarketData {
    build_series_at(start, |_, progress| {
        let t = progress * TAU;
        let open = 100.0 + 2.5 * t.sin();
        let close = 100.0 + 2.5 * (t + 0.35).sin();
        let range = 1.2 + 0.6 * (t * 0.7).cos();
        let volume = 1_050.0 + 140.0 * (t * 0.9).cos();
        (open, close, range, volume)
    })
}

fn build_cosine_trend_series(start: DateTime<Utc>) -> MarketData {
    build_series_at(start, |_, progress| {
        let t = progress * TAU * 1.3;
        let trend = progress * 6.0;
        let open = 120.0 + trend + 3.8 * t.cos();
        let close = 120.0 + trend + 3.2 * (t + 0.5).cos();
        let range = 1.6 + 0.7 * (t * 0.8).sin();
        let volume = 900.0 + 160.0 * (t * 1.1).sin();
        (open, close, range, volume)
    })
}

fn build_trending_wave_series(start: DateTime<Utc>) -> MarketData {
    build_series_at(start, |i, progress| {
        let t = progress * TAU * 2.0;
        let drift = progress * 10.0;
        let noise = (i as f32 * 0.45).sin() + (i as f32 * 0.32).cos();
        let base = 85.0 + drift + noise;
        let open = base;
        let close = base + 1.4 * (t * 0.6).sin();
        let range = 1.9 + 0.5 * (t * 1.7).cos();
        let volume = 780.0 + 110.0 * (i as f32 * 0.22).sin();
        (open, close, range, volume)
    })
}

fn build_square_wave_series(start: DateTime<Utc>) -> MarketData {
    build_series_at(start, |i, _| {
        let cycle = (i / 12) % 2;
        let sign = if cycle == 0 { 1.0 } else { -1.0 };
        let base = 95.0 + 3.0 * sign;
        let open = base + 1.2 * sign;
        let close = base - 1.0 * sign;
        let range = 1.1 + ((i % 12) as f32) * 0.05;
        let volume = 820.0 + 90.0 * sign;
        (open, close, range, volume)
    })
}

fn random_position_overlays(data: &MarketData) -> Vec<PositionOverlay> {
    let mut rng = rand::thread_rng();

    let timestamps: Vec<i64> = {
        let store = data.level_store.lock().expect("level store mutex poisoned");
        let Some(candles) = store.get(60) else {
            return Vec::new();
        };
        candles.iter().map(|c| c.ts).collect()
    };

    if timestamps.len() < 2 {
        return Vec::new();
    }

    let total = timestamps.len();
    let max_start = (total - 2).max(0);
    let overlay_count = rng.gen_range(1..=2);
    let mut overlays = Vec::with_capacity(overlay_count);

    for _ in 0..overlay_count {
        let start_idx = if max_start == 0 {
            0
        } else {
            rng.gen_range(0..=max_start)
        };

        let remaining = total - start_idx;
        let min_len = remaining.min(4).max(2);
        let max_len = remaining.min(18).max(min_len);
        let len = if min_len == max_len {
            min_len
        } else {
            rng.gen_range(min_len..=max_len)
        };

        let end_idx = start_idx + len.saturating_sub(1);
        let start_ts = timestamps[start_idx];
        let end_ts = timestamps[end_idx];

        let overlay = if rng.gen_bool(0.5) {
            PositionOverlay::long(start_ts, end_ts)
        } else {
            PositionOverlay::short(start_ts, end_ts)
        };

        overlays.push(overlay.with_opacity(0.32));
    }

    overlays
}

fn create_price_level_quads(data: &MarketData, base_price: f32) -> Vec<PriceLevelQuad> {
    let timestamps: Vec<i64> = {
        let store = data.level_store.lock().expect("level store mutex poisoned");
        let Some(candles) = store.get(60) else {
            return Vec::new();
        };
        candles.iter().map(|c| c.ts).collect()
    };

    if timestamps.len() < 2 {
        return Vec::new();
    }

    let mut quads = Vec::new();

    // Create a stop-loss quad in the first third of the data
    let stop_loss_start_idx = timestamps.len() / 4;
    let stop_loss_end_idx = timestamps.len() / 2;
    let stop_loss_start_ts = timestamps[stop_loss_start_idx];
    let stop_loss_end_ts = timestamps[stop_loss_end_idx];

    quads.push(PriceLevelQuad::stop_loss(
        stop_loss_start_ts,
        stop_loss_end_ts,
        base_price,
        base_price - 2.0, // Stop-loss 2 units below entry
    ));

    // Create a take-profit quad in the second half of the data
    let take_profit_start_idx = timestamps.len() / 2;
    let take_profit_end_idx = (timestamps.len() * 3) / 4;
    let take_profit_start_ts = timestamps[take_profit_start_idx];
    let take_profit_end_ts = timestamps[take_profit_end_idx];

    quads.push(PriceLevelQuad::take_profit(
        take_profit_start_ts,
        take_profit_end_ts,
        base_price,
        base_price + 3.0, // Take-profit 3 units above entry
    ));

    quads
}

fn main() -> anyhow::Result<()> {
    // Each viewport has data starting at a different time (NYC timezone)
    // This mirrors the real use case of viewing different time periods
    let start1 = New_York
        .with_ymd_and_hms(2024, 1, 2, 9, 30, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let start2 = New_York
        .with_ymd_and_hms(2024, 1, 3, 10, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc); // Next day, 10:00 NYC
    let start3 = New_York
        .with_ymd_and_hms(2024, 1, 4, 14, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc); // Day after, 14:00 NYC

    let datasets = vec![
        build_sine_series(start1),
        build_cosine_trend_series(start2),
        build_trending_wave_series(start3),
    ];

    let position_overlays: Vec<Vec<PositionOverlay>> =
        datasets.iter().map(random_position_overlays).collect();

    // Create price level quads for each dataset with appropriate base prices
    let price_level_quads: Vec<Vec<PriceLevelQuad>> = vec![
        create_price_level_quads(&datasets[0], 100.0), // Sine wave around 100
        create_price_level_quads(&datasets[1], 123.0), // Cosine trend around 123
        create_price_level_quads(&datasets[2], 90.0),  // Trending wave around 90
    ];

    // Calculate initial left-edge timestamps for each viewport
    // Some(ts) positions the viewport's left edge at that timestamp
    // None shows the most recent data (right edge at data end)
    let start_times = vec![
        Some(start1.timestamp() + 30 * 60),  // Viewport 0: left edge 30 min into sine data
        Some(start2.timestamp() + 120 * 60), // Viewport 1: left edge 120 min into cosine data
        None,                                // Viewport 2: shows most recent data
    ];

    PlotBuilder::new()
        .with_grid(2, 2)
        .with_tickers(vec![
            Some("SINE".to_string()),
            Some("COSINE".to_string()),
            Some("TREND".to_string()),
        ])
        .with_titles(vec![
            Some("Sine (start +30min)".to_string()),
            Some("Cosine (start +50min)".to_string()),
            Some("Trend (default start)".to_string()),
        ])
        .with_window_size(1400, 900)
        .with_bar_width_px(3)
        .with_market_data_views(datasets)
        .with_position_overlays(position_overlays)
        .with_price_level_quads(price_level_quads)
        .with_start_times(start_times)
        .with_allow_missing_history(true)
        .with_theme(Theme::Dark)
        .run()
}
