//! Minimal test to verify basic rendering works

use anyhow::Result;
use vizza::PlotBuilder;

fn main() -> Result<()> {
    println!("Testing basic vizza rendering...");

    // Use exact same configuration as plot_builder_demo
    let base = "../../data/consolidated/stock-split-dividend-test/";
    let data_path = format!("{}FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv", base);

    PlotBuilder::new()
        .with_data_paths(vec![
            data_path.clone(),
            data_path.clone(),
            data_path.clone(),
            data_path,
        ])
        .with_live_data(false, None) // Disable live data explicitly
        .run()?;

    Ok(())
}
