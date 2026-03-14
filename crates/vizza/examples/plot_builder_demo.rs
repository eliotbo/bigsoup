/// Example demonstrating the PlotBuilder API for creating vizza plots
///
/// This example shows different ways to use the PlotBuilder API:
/// 1. Default configuration
/// 2. Custom grid layout
/// 3. Custom data paths for multiple viewports
/// 4. Full configuration with live data enabled
///
/// Run with: cargo run --example plot_builder_demo
use anyhow::Result;
use vizza::{Config, LodLevel, PlotBuilder, ViewSettings};

fn main() -> Result<()> {
    // Choose which example to run by uncommenting one of the functions below:

    // Example 1: Simple plot with default settings
    example_1_simple_default()?;

    // Example 2: Custom grid layout (3x3)
    // example_2_custom_grid()?;

    // Example 3: Custom data paths for each viewport
    // example_3_custom_data_paths()?;

    // Example 4: Full configuration with all options
    // example_4_full_config()?;

    // Example 5: Using the simple plot() function
    // example_5_simple_function()?;

    Ok(())
}

/// Example 1: Simple plot with default settings
/// Creates a 2x2 grid with default data paths
fn example_1_simple_default() -> Result<()> {
    println!("Running Example 1: Simple plot with defaults");

    PlotBuilder::new().run()
}

/// Example 2: Custom grid layout
/// Creates a 3x3 grid of viewports
#[allow(dead_code)]
fn example_2_custom_grid() -> Result<()> {
    println!("Running Example 2: Custom 3x3 grid");

    PlotBuilder::new()
        .with_grid(3, 3)
        .with_window_size(1600, 1600)
        .run()
}

/// Example 3: Custom data paths for each viewport
/// Shows how to specify different data files for each viewport
#[allow(dead_code)]
fn example_3_custom_data_paths() -> Result<()> {
    println!("Running Example 3: Custom data paths");

    let base = "../../data/consolidated/stock-split-dividend-test/";

    PlotBuilder::new()
        .with_data_paths(vec![
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
        ])
        .with_grid(2, 2)
        .run()
}

/// Example 4: Full configuration with all options
/// Demonstrates all available configuration options
#[allow(dead_code)]
fn example_4_full_config() -> Result<()> {
    println!("Running Example 4: Full configuration");

    let base = "../../data/consolidated/stock-split-dividend-test/";

    PlotBuilder::new()
        .with_data_paths(vec![
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
        ])
        .with_window_size(1400, 1400)
        .with_grid(2, 2)
        .with_lod_level(LodLevel::H1) // Start at 1-hour timeframe
        .with_auto_y_scale(true)
        .with_live_data(false, None) // Set to true for live data updates
        .run()
}

/// Example 5: Using the simple plot() function
/// Shows the simplified API for basic use cases
#[allow(dead_code)]
fn example_5_simple_function() -> Result<()> {
    println!("Running Example 5: Simple plot() function");

    let base = "../../data/consolidated/stock-split-dividend-test/";

    vizza::plot(
        vec![
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
        ],
        2,     // rows
        2,     // cols
        false, // live data
    )
}

/// Example 6: Using plot_with_config() for complete control
/// Shows how to build a Config manually and use it
#[allow(dead_code)]
fn _example_6_plot_with_config() -> Result<()> {
    println!("Running Example 6: plot_with_config()");

    let base = "../../data/consolidated/stock-split-dividend-test/";

    let config = Config {
        default_lod_level: LodLevel::D1,
        data_paths: vec![
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
        ],
        view_settings: ViewSettings { auto_y_scale: true },
        window_width: 1200,
        window_height: 1200,
        grid_rows: 2,
        grid_cols: 2,
        allow_missing_history: false,
        position_overlays: Vec::new(),
        tickers: Vec::new(),
        titles: Vec::new(),
        bar_width_px: 3,
    };

    vizza::plot_with_config(
        config, false, // use_live_data
        None,  // live_update_interval_ms
    )
}
