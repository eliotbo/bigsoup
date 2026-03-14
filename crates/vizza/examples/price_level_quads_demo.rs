//! Price Level Quads Example - Stop-Loss and Take-Profit Visualization
//!
//! This example demonstrates the new PriceLevelQuad feature which allows you to
//! visualize horizontal price zones such as stop-loss and take-profit levels on
//! your charts.
//!
//! Features:
//! - Stop-loss zones (red, semi-transparent)
//! - Take-profit zones (green, semi-transparent)
//! - Multiple trade scenarios in a 2x2 grid
//! - Synthetic data with realistic price movements
//! - Initial x-axis positioning using with_start_time()
//!
//! Run with: cargo run --example price_level_quads_demo

use std::f32::consts::TAU;

use chrono::{Duration, TimeZone, Utc};
use lod::{LevelStore, PlotCandle};
use vizza::{MarketData, PlotBuilder, PositionOverlay, PriceLevelQuad};

const BAR_COUNT: usize = 120;

/// Build a synthetic market data series with controlled price action
fn build_synthetic_data(
    base_price: f32,
    volatility: f32,
    trend: f32,
) -> MarketData {
    let mut level_store = LevelStore::new();
    let start = Utc
        .with_ymd_and_hms(2024, 1, 2, 9, 30, 0)
        .single()
        .expect("valid start timestamp");

    let mut candles = Vec::with_capacity(BAR_COUNT);
    let denom = (BAR_COUNT - 1).max(1) as f32;

    for i in 0..BAR_COUNT {
        let ts = start + Duration::minutes(i as i64);
        let ts_nanos = ts.timestamp_nanos_opt().expect("timestamp in range");
        let progress = i as f32 / denom;

        // Create price movement with sine wave and trend
        let t = progress * TAU * 2.0;
        let wave = volatility * t.sin();
        let drift = trend * progress;

        let mid_price = base_price + wave + drift;
        let spread = volatility * 0.3;

        let open = mid_price - spread * 0.5;
        let close = mid_price + spread * 0.5;
        let high = mid_price + spread;
        let low = mid_price - spread;
        let volume = 1000.0 + 200.0 * (t * 0.7).cos();

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

/// Create stop-loss and take-profit quads for a winning long trade
fn winning_long_trade(data: &MarketData) -> (Vec<PriceLevelQuad>, Vec<PositionOverlay>) {
    let timestamps = extract_timestamps(data);
    if timestamps.len() < 40 {
        return (Vec::new(), Vec::new());
    }

    let entry_idx = 10;
    let exit_idx = 35;
    let entry_ts = timestamps[entry_idx];
    let exit_ts = timestamps[exit_idx];

    let buy_price = 100.0;
    let stop_loss = 95.0;
    let take_profit = 110.0;

    let quads = vec![
        // Red stop-loss zone below entry
        PriceLevelQuad::stop_loss(entry_ts, exit_ts, buy_price, stop_loss),
        // Green take-profit zone above entry
        PriceLevelQuad::take_profit(entry_ts, exit_ts, buy_price, take_profit),
    ];

    // Add a green overlay to show the position duration
    let overlays = vec![
        // PositionOverlay::long(entry_ts, exit_ts).with_opacity(0.25),
    ];

    (quads, overlays)
}

/// Create stop-loss and take-profit quads for a stopped-out long trade
fn stopped_long_trade(data: &MarketData) -> (Vec<PriceLevelQuad>, Vec<PositionOverlay>) {
    let timestamps = extract_timestamps(data);
    if timestamps.len() < 40 {
        return (Vec::new(), Vec::new());
    }

    let entry_idx = 15;
    let exit_idx = 28; // Exit earlier when stop is hit
    let entry_ts = timestamps[entry_idx];
    let exit_ts = timestamps[exit_idx];

    let buy_price = 100.0;
    let stop_loss = 95.0;
    let take_profit = 108.0;

    let quads = vec![
        PriceLevelQuad::stop_loss(entry_ts, exit_ts, buy_price, stop_loss),
        PriceLevelQuad::take_profit(entry_ts, exit_ts, buy_price, take_profit),
    ];

    // Red overlay to indicate a losing trade
    let overlays = vec![
        // PositionOverlay::short(entry_ts, exit_ts).with_opacity(0.25),
    ];

    (quads, overlays)
}

/// Create multiple trades with overlapping zones
fn multiple_trades(data: &MarketData) -> (Vec<PriceLevelQuad>, Vec<PositionOverlay>) {
    let timestamps = extract_timestamps(data);
    if timestamps.len() < 80 {
        return (Vec::new(), Vec::new());
    }

    let mut quads = Vec::new();
    let mut overlays = Vec::new();

    // First trade
    let entry1_ts = timestamps[10];
    let exit1_ts = timestamps[30];
    quads.push(PriceLevelQuad::stop_loss(entry1_ts, exit1_ts, 100.0, 93.0));
    quads.push(PriceLevelQuad::take_profit(entry1_ts, exit1_ts, 100.0, 112.0));
    // overlays.push(PositionOverlay::long(entry1_ts, exit1_ts).with_opacity(0.2));

    // Second trade (overlapping)
    let entry2_ts = timestamps[25];
    let exit2_ts = timestamps[50];
    quads.push(PriceLevelQuad::stop_loss(entry2_ts, exit2_ts, 105.0, 98.0));
    quads.push(PriceLevelQuad::take_profit(entry2_ts, exit2_ts, 105.0, 115.0));
    // overlays.push(PositionOverlay::long(entry2_ts, exit2_ts).with_opacity(0.2));

    // Third trade
    let entry3_ts = timestamps[55];
    let exit3_ts = timestamps[75];
    quads.push(PriceLevelQuad::stop_loss(entry3_ts, exit3_ts, 98.0, 92.0));
    quads.push(PriceLevelQuad::take_profit(entry3_ts, exit3_ts, 98.0, 106.0));
    // overlays.push(PositionOverlay::short(entry3_ts, exit3_ts).with_opacity(0.2));

    (quads, overlays)
}

/// Create custom colored price zones
fn custom_zones(data: &MarketData) -> (Vec<PriceLevelQuad>, Vec<PositionOverlay>) {
    let timestamps = extract_timestamps(data);
    if timestamps.len() < 60 {
        return (Vec::new(), Vec::new());
    }

    let start_ts = timestamps[20];
    let end_ts = timestamps[55];

    let quads = vec![
        // Blue zone: Support level
        PriceLevelQuad::new(
            start_ts,
            end_ts,
            95.0,
            97.0,
            [0.2, 0.4, 0.8, 0.25], // Blue with 25% opacity
        ),
        // Orange zone: Resistance level
        PriceLevelQuad::new(
            start_ts,
            end_ts,
            108.0,
            110.0,
            [0.9, 0.5, 0.2, 0.25], // Orange with 25% opacity
        ),
        // Yellow zone: High volatility warning zone
        PriceLevelQuad::new(
            start_ts,
            end_ts,
            100.0,
            103.0,
            [0.9, 0.9, 0.3, 0.15], // Yellow with 15% opacity
        ),
    ];

    (quads, Vec::new())
}

/// Extract timestamps from market data
fn extract_timestamps(data: &MarketData) -> Vec<i64> {
    let store = data.level_store.lock().expect("level store mutex poisoned");
    let Some(candles) = store.get(60) else {
        return Vec::new();
    };
    candles.iter().map(|c| c.ts).collect()
}

fn main() -> anyhow::Result<()> {
    println!("\n=== Price Level Quads Demo ===\n");
    println!("This example showcases the new PriceLevelQuad feature:");
    println!("  ✓ Red semi-transparent zones for stop-loss levels");
    println!("  ✓ Green semi-transparent zones for take-profit levels");
    println!("  ✓ Custom colored zones for support/resistance");
    println!("  ✓ Multiple overlapping trades visualization");
    println!("  ✓ Initial x-axis position (starting at 09:45 instead of 09:30)\n");
    println!("Grid layout:");
    println!("  Top-left:     Winning long trade (hits take-profit)");
    println!("  Top-right:    Stopped long trade (hits stop-loss)");
    println!("  Bottom-left:  Multiple overlapping trades");
    println!("  Bottom-right: Custom support/resistance zones\n");
    println!("Controls:");
    println!("  - Scroll: Pan through time");
    println!("  - Ctrl+Scroll: Zoom in/out");
    println!("  - ESC: Exit\n");

    // Generate synthetic data for each viewport
    let datasets = vec![
        build_synthetic_data(100.0, 3.0, 8.0),  // Upward trending
        build_synthetic_data(100.0, 4.0, -5.0), // Downward trending
        build_synthetic_data(100.0, 5.0, 2.0),  // Slight upward with volatility
        build_synthetic_data(100.0, 3.5, 0.0),  // Sideways
    ];

    // Create price level quads for each scenario
    let (quads1, overlays1) = winning_long_trade(&datasets[0]);
    let (quads2, overlays2) = stopped_long_trade(&datasets[1]);
    let (quads3, overlays3) = multiple_trades(&datasets[2]);
    let (quads4, overlays4) = custom_zones(&datasets[3]);

    let price_level_quads = vec![quads1, quads2, quads3, quads4];
    let position_overlays = vec![overlays1, overlays2, overlays3, overlays4];

    // Example: Start the chart at 2024-01-02 09:45:00 UTC (15 minutes into the data)
    // The synthetic data begins at 09:30:00, so this will show the chart starting
    // at the 15-minute mark, demonstrating the new with_start_time() feature.
    let start_time_utc = Utc
        .with_ymd_and_hms(2024, 1, 2, 9, 45, 0)
        .single()
        .expect("valid start time")
        .timestamp();

    PlotBuilder::new()
        .with_grid(2, 2)
        .with_tickers(vec![
            Some("WIN".to_string()),
            Some("STOP".to_string()),
            Some("MULTI".to_string()),
            Some("ZONES".to_string()),
        ])
        .with_titles(vec![
            Some("Winning Trade (Take-Profit)".to_string()),
            Some("Stopped Trade (Stop-Loss)".to_string()),
            Some("Multiple Overlapping Trades".to_string()),
            Some("Custom Support/Resistance".to_string()),
        ])
        .with_window_size(1600, 1200)
        .with_bar_width_px(5)
        .with_market_data_views(datasets)
        .with_price_level_quads(price_level_quads)
        .with_position_overlays(position_overlays)
        .with_allow_missing_history(true)
        .with_auto_y_scale(true)
        // NEW: Start the chart 15 minutes into the data
        // Instead of showing from 09:30:00, we start at 09:45:00
        .with_start_time(start_time_utc)
        .run()
}
