//! Live Data API Showcase
//!
//! This example demonstrates the new live data API for vizza, which provides:
//!
//! 1. **Smart Synthetic Data**: Uses actual historical patterns instead of hardcoded values
//!    - Extracts real closing price from last historical bar
//!    - Calculates volatility from recent price movements (20-bar lookback)
//!    - Respects market hours and weekends
//!    - Detects holidays and market closures
//!
//! 2. **Pluggable Architecture**: Easy to swap data sources
//!    - Built-in SyntheticDataSource (default)
//!    - Custom sources can implement LiveDataSource trait
//!    - Future: WebSocket feeds, file replay, etc.
//!
//! 3. **Seamless Continuation**: Live data picks up where historical data ends
//!    - No artificial price jumps
//!    - Smooth transition at chart boundary
//!    - Realistic volatility based on actual market behavior
//!
//! Run with: cargo run --example live_data_showcase
//!
//! ## What to Observe:
//!
//! - Live bars will appear at the right edge of the chart
//! - Price will continue from the last historical close (not jump to 100.0!)
//! - Volatility will match recent price movements
//! - No trades will appear during weekends or after hours
//! - VWAP marker shows live price trend

use anyhow::Result;
use vizza::PlotBuilder;

fn main() -> Result<()> {
    println!("\n=== Vizza Live Data API Showcase ===\n");
    println!("This example demonstrates smart synthetic live data that:");
    println!("  ✓ Uses real closing price from historical data");
    println!("  ✓ Calculates volatility from actual price movements");
    println!("  ✓ Scales volatility for realistic intraday movement");
    println!("  ✓ Respects market hours and weekends");
    println!("  ✓ Seamlessly continues from last historical bar\n");
    println!("Watch the right edge of the chart for live updates!");
    println!("Check console for volatility scaling info\n");

    // Example 1: Basic live data with smart defaults
    example_1_basic_live()?;

    // Example 2: Live data with custom update interval
    // example_2_custom_interval()?;

    // Example 3: Multiple viewports with live data
    // example_3_multi_viewport()?;

    Ok(())
}

/// Example 1: Basic live data with smart synthetic source
///
/// The live data will:
/// - Start from the actual last closing price in the data
/// - Use calculated volatility from recent bars (not hardcoded 2%)
/// - Only generate trades during market hours
/// - Show smooth continuation from historical to live data
fn example_1_basic_live() -> Result<()> {
    println!("Running Example 1: Basic Smart Live Data\n");

    let base = "../../data/consolidated/stock-split-dividend-test/";
    let data_path = format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base);

    PlotBuilder::new()
        .with_data_paths(vec![
            data_path.clone(),
            data_path.clone(),
            data_path.clone(),
            data_path,
        ])
        .with_grid(2, 2)
        .with_window_size(1400, 1400)
        .with_live_data(true, Some(100)) // Enable live data, 100ms updates
        .with_auto_y_scale(true) // Auto-scale Y to fit live data
        .run()
}

/// Example 2: Custom update interval for faster/slower live updates
#[allow(dead_code)]
fn example_2_custom_interval() -> Result<()> {
    println!("Running Example 2: Custom Update Interval\n");
    println!("Using 50ms update interval for faster live data...\n");

    let base = "../../data/consolidated/stock-split-dividend-test/";
    let data_path = format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base);

    PlotBuilder::new()
        .with_data_paths(vec![data_path])
        .with_grid(1, 1)
        .with_window_size(1400, 900)
        .with_live_data(true, Some(50)) // Faster: 50ms updates
        .with_auto_y_scale(true)
        .run()
}

/// Example 3: Multiple viewports with synchronized live data
#[allow(dead_code)]
fn example_3_multi_viewport() -> Result<()> {
    println!("Running Example 3: Multi-Viewport Live Data\n");
    println!("All viewports will show live data from the same source...\n");

    let base = "../../data/consolidated/stock-split-dividend-test/";

    PlotBuilder::new()
        .with_data_paths(vec![
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
            format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base),
        ])
        .with_grid(2, 2)
        .with_window_size(1600, 1600)
        .with_live_data(true, Some(100))
        .with_auto_y_scale(true)
        .run()
}

// ============================================================================
// ADVANCED: Custom Data Source Example
// ============================================================================
//
// The following shows how to create a custom LiveDataSource.
// This is commented out but demonstrates the extensibility of the API.
//
// ```rust
// use vizza::live::{LiveDataSource, LiveDataError, Result as LiveResult};
// use lod::{LevelStore, PlotTrade};
//
// /// Example custom data source that generates predictable test patterns
// struct TestPatternSource {
//     base_price: f64,
//     current_time_ns: i64,
//     ticker: String,
// }
//
// impl LiveDataSource for TestPatternSource {
//     fn initialize(&mut self, historical_data: &LevelStore, ticker: &str) -> LiveResult<()> {
//         self.ticker = ticker.to_string();
//
//         if let Some(candles) = historical_data.get(60) {
//             if let Some(last) = candles.last() {
//                 self.base_price = last.close as f64;
//                 self.current_time_ns = last.ts;
//                 return Ok(());
//             }
//         }
//
//         Err(LiveDataError::InvalidHistoricalData)
//     }
//
//     fn get_trades(&mut self, now_ns: i64) -> Vec<PlotTrade> {
//         // Generate a predictable sine wave pattern for testing
//         let elapsed_s = (now_ns - self.current_time_ns) as f64 / 1e9;
//         let price = self.base_price + (elapsed_s.sin() * 5.0);
//
//         vec![PlotTrade {
//             ts: now_ns,
//             price: price as f32,
//             size: 100.0,
//             flags: 0,
//         }]
//     }
//
//     fn is_market_open(&self, _now_ns: i64) -> bool {
//         true  // Always open for testing
//     }
//
//     fn source_name(&self) -> &str {
//         "TestPatternSource"
//     }
//
//     fn current_price(&self) -> f64 {
//         self.base_price
//     }
// }
//
// // Usage:
// fn example_custom_source() -> Result<()> {
//     use vizza::live_view::LiveDataManager;
//     use std::sync::{Arc, Mutex};
//
//     let level_store = Arc::new(Mutex::new(LevelStore::new()));
//     let custom_source = Box::new(TestPatternSource {
//         base_price: 100.0,
//         current_time_ns: 0,
//         ticker: "TEST".to_string(),
//     });
//
//     let _live_manager = LiveDataManager::with_data_source(
//         level_store,
//         custom_source,
//         "TEST",
//     );
//
//     // Use live_manager in your application...
//     Ok(())
// }
// ```
//
// ============================================================================

/// Helper function to explain the improvements
#[allow(dead_code)]
fn print_improvements() {
    println!("\n=== Live Data API Improvements ===\n");

    println!("BEFORE (Old Implementation):");
    println!("  ❌ Hardcoded base price of 100.0");
    println!("  ❌ Hardcoded 2% volatility");
    println!("  ❌ No market hours awareness");
    println!("  ❌ Trades during weekends/holidays");
    println!("  ❌ Price jumps at historical/live boundary\n");

    println!("AFTER (New Implementation):");
    println!("  ✅ Real base price from last historical bar");
    println!("  ✅ Calculated volatility from actual price movements");
    println!("  ✅ Respects market hours (9:30-16:00 ET)");
    println!("  ✅ No trades on weekends or holidays");
    println!("  ✅ Smooth continuation from historical data");
    println!("  ✅ Pluggable architecture for custom sources\n");

    println!("Key Features:");
    println!("  • Volatility: Calculated from 20-bar log returns");
    println!("  • Market Hours: US equity hours with weekend detection");
    println!("  • Holiday Detection: Identifies gaps > 3 days");
    println!("  • Price Continuity: Starts from actual last close");
    println!("  • Extensibility: Implement LiveDataSource trait for custom feeds\n");
}
