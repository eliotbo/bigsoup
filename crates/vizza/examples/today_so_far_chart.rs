//! Example demonstrating the "today-so-far" backfill feature with MockBackfillSource.
//!
//! This example shows how to use the new backfill infrastructure:
//! 1. MockBackfillSource implements both LiveDataSource and HistoricalBackfillSource
//! 2. The PlotBuilder can be configured with .with_today_so_far(true)
//! 3. Historical bars are generated from "market open" to current time
//! 4. Live trades seamlessly continue from the last backfill bar
//!
//! Run with: cargo run --example today_so_far_chart

use vizza::{MockBackfillSource, PlotBuilder};

fn main() -> anyhow::Result<()> {
    println!("=== Today-So-Far Chart Example ===");
    println!();
    println!("This example demonstrates the new backfill infrastructure:");
    println!("  • MockBackfillSource provides both historical backfill and live trades");
    println!("  • PlotBuilder.with_today_so_far(true) enables the feature");
    println!("  • Synthetic 5-second bars generated from market open to now");
    println!("  • Live trades continue seamlessly from the last bar");
    println!();

    // Create a mock backfill source
    let mock_source = Box::new(MockBackfillSource::new());

    println!("✓ Created MockBackfillSource");
    println!("  → Supports historical backfill: Yes");
    println!("  → Supports live trades: Yes");
    println!("  → Base interval: 5 seconds");
    println!();

    println!("Controls:");
    println!("  - Scroll: Pan left/right");
    println!("  - Ctrl+Scroll: Change zoom level");
    println!("  - ESC: Exit");
    println!();

    println!("Starting chart with today-so-far backfill enabled...");
    println!();

    // Build and run the chart
    PlotBuilder::new()
        .with_grid(1, 1)
        .with_data_paths(vec!["/media/data10t/databento/nasdaq_50biggest_2018/QQQ/consolidated/2018-05-01_to_2025-11-23-ohlcv-1m.nohlcv".to_string()])
        .with_allow_missing_history(true) // Allow running even if historical file is missing
        .with_custom_live_source(mock_source)
        .with_today_so_far(true) // Enable today-so-far backfill
        .with_live_data(true, Some(100)) // Update every 100ms
        .run()?;

    Ok(())
}
