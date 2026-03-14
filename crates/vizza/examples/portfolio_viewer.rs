//! Portfolio Viewer Example - Multiple Live Data Sources
//!
//! This example demonstrates vizza's new multi-source capability where each viewport
//! in a grid can have its own independent live data source with its own ticker.
//!
//! Features:
//! - Multiple viewports (2x2 grid = 4 charts)
//! - Each viewport has its own ticker and live data source
//! - Independent synthetic data generation per ticker
//! - Perfect for monitoring a portfolio of stocks simultaneously
//!
//! Run with: cargo run --example portfolio_viewer

use anyhow::Result;
use vizza::{LiveDataSource, MockBackfillSource, PlotBuilder};

fn main() -> Result<()> {
    println!("\n=== Vizza Portfolio Viewer ===\n");
    println!("This example demonstrates multiple live data sources in a single window:");
    println!("  ✓ 2x2 grid showing 4 different tickers");
    println!("  ✓ Each viewport has its own independent live data source");
    println!("  ✓ Each ticker shows its own synthetic price movements");
    println!("  ✓ All updating simultaneously in real-time\n");
    println!("Controls:");
    println!("  - Scroll: Pan through time");
    println!("  - Ctrl+Scroll: Zoom in/out");
    println!("  - ESC: Exit\n");

    // Define the portfolio of tickers to monitor
    let tickers = vec!["AAPL", "MSFT", "GOOGL", "TSLA"];

    // Base data path (we'll use the same historical data for demo purposes)
    let base = "../../data/consolidated/stock-split-dividend-test/";
    let data_path = format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base);

    // Create data paths for each viewport (using same data file for demo)
    let data_paths: Vec<String> = tickers.iter().map(|_| data_path.clone()).collect();

    // Create a separate live data source for each ticker
    println!(
        "Creating live data sources for {} tickers...",
        tickers.len()
    );
    let sources: Vec<(String, Box<dyn LiveDataSource>)> = tickers
        .iter()
        .map(|ticker| {
            println!("  - Setting up live source for {}", ticker);
            let source = Box::new(MockBackfillSource::new()) as Box<dyn LiveDataSource>;
            (ticker.to_string(), source)
        })
        .collect();

    println!(
        "\nStarting portfolio viewer with {} viewports...\n",
        sources.len()
    );

    // Build and run the chart with multiple live sources
    PlotBuilder::new()
        .with_grid(2, 2) // 2x2 grid for 4 tickers
        .with_data_paths(data_paths)
        .with_window_size(1600, 1400)
        .with_custom_live_sources(sources) // NEW: Multiple sources!
        .with_today_so_far(true) // Enable today-so-far backfill for each
        .with_live_data(true, Some(100)) // Update every 100ms
        .with_auto_y_scale(true) // Auto-scale Y axis for each viewport
        .run()
}
