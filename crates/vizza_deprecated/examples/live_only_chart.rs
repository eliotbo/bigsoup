//! Example demonstrating live-only chart without historical data files.
//!
//! This example shows how to use Vizza with only live streaming data,
//! without requiring any historical data files to exist on disk.
//!
//! Run with: cargo run --example live_only_chart

use lod::PlotTrade;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use vizza::{IPCLiveDataSource, PlotBuilder};

fn main() -> anyhow::Result<()> {
    println!("=== Live-Only Chart Example ===");
    println!("This chart should show:");
    println!("  - 1 viewport only (not 4!)");
    println!("  - No historical data");
    println!("  - Live trades appearing in real-time");
    println!();
    println!("Controls:");
    println!("  - Scroll: Pan left/right through time");
    println!("  - Ctrl+Scroll: Change LOD (zoom granularity)");
    println!("  - Available LODs: 5s, 15s, 30s, 1m, 5m, 15m, 30m, 1h, 4h, 1d, 1w, 1month");
    println!("  - Note: Live data stored at 5-second granularity");
    println!("  - Zooming to 1s will show a warning (finer than 5s)");
    println!();

    // Create a channel for sending trades to the chart
    let (tx, rx) = mpsc::channel();

    // Spawn a thread to generate mock live trades
    thread::spawn(move || {
        let mut price = 100.0f64;
        let mut counter = 0u64;

        loop {
            // Simulate market activity with pseudo-random price movements
            // Using counter-based deterministic "randomness" for simplicity
            let price_change = ((counter % 100) as f64 / 100.0 - 0.5) * 0.5;
            price = (price + price_change).max(90.0).min(110.0);

            // Generate current timestamp in nanoseconds
            let now_ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as i64;

            // Create a trade
            let trade = PlotTrade {
                ts: now_ns,
                price: price as f32,
                size: (100.0 + (counter % 500) as f32) as f32,
                side: if counter % 2 == 0 { b'B' } else { b'S' }, // Alternate buy/sell
                flags: 0,
                exchange: 0,
            };

            // Send the trade through the channel
            if tx.send(trade).is_err() {
                println!("Chart closed, stopping trade generation");
                break;
            }

            counter += 1;

            // Generate trades at varying intervals (20-50ms)
            let sleep_ms = 20 + (counter % 30);
            thread::sleep(Duration::from_millis(sleep_ms));
        }
    });

    // Create the IPC data source with the channel receiver
    let data_source = Box::new(IPCLiveDataSource::with_base_price(rx, 100.0));

    // Create and run the chart with:
    // - Single viewport (1x1 grid) - IMPORTANT: Set this first!
    // - No historical data files (nonexistent path + allow_missing_history)
    // - Custom live data source for real-time trades
    // - Live data enabled with 100ms update interval
    PlotBuilder::new()
        .with_grid(1, 1) // Must be set first to override default 2x2 grid
        .with_data_paths(vec!["nonexistent_file.nohlcv".to_string()]) // Override default paths
        .with_allow_missing_history(true)
        .with_custom_live_source(data_source)
        .with_live_data(true, Some(100))
        .run()?;

    Ok(())
}
