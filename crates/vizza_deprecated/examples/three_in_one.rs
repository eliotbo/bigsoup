//! Example demonstrating "today-so-far" historical bars with synthetic live data.
//!
//! This example shows the complete today-so-far workflow:
//! 1. Generating synthetic 5-second bars from market open to current time
//! 2. Injecting these bars as historical backfill via MockBackfillSource
//! 3. Seamlessly transitioning to live synthetic trade generation
//!
//! Run with: cargo run --example today_so_far_synthetic_demo

use vizza::{MockBackfillSource, PlotBuilder};

fn main() -> anyhow::Result<()> {
    println!("=== Today-So-Far Synthetic Demo ===");
    println!();
    println!("This demo showcases the complete today-so-far feature:");
    println!("  1. Synthetic 5-second bars from market open (9:30 AM ET) to now");
    println!("  2. Historical backfill injection into LiveEngine");
    println!("  3. Live synthetic trades continuing from last backfill price");
    println!("  4. Seamless visual transition between historical and live data");
    println!();
    println!("Features demonstrated:");
    println!("  • MockBackfillSource with HistoricalBackfillSource trait");
    println!("  • Automatic bar interval rebuilding (5s → 15s → 30s → 1m)");
    println!("  • Live trade simulation with realistic volatility");
    println!("  • VWAP overlay on live bars");
    println!();
    println!("Controls:");
    println!("  - Scroll: Pan left/right through time");
    println!("  - Ctrl+Scroll: Zoom in/out (change LOD level)");
    println!("  - ESC: Exit the chart");
    println!();
    println!("Note: The chart will show:");
    println!("  - Historical bars (from backfill) with one color");
    println!("  - Live bars (being built in real-time) overlaid in green");
    println!("  - Current open bar (incomplete) with VWAP marker");
    println!();

    // Create MockBackfillSource which supports both backfill AND live trades
    let mock_source = Box::new(MockBackfillSource::new());

    println!("Starting chart...");
    println!();
    let base = "../../data/consolidated/stock-split-dividend-test/";
    let data_path = format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base);

    // Build and run the chart with today-so-far enabled
    PlotBuilder::new()
        .with_grid(1, 1)
        .with_data_paths(vec![data_path]) // Use the generated data path
        .with_allow_missing_history(true) // Allow running without historical data
        .with_custom_live_source(mock_source) // Use our mock source
        .with_today_so_far(true) // Enable today-so-far backfill
        .with_live_data(true, Some(100)) // Update every 100ms
        .with_lod_level(vizza::LodLevel::S5) // Start at 5-second view to match backfill
        .run()?;

    Ok(())
}
