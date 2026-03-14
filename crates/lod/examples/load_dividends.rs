//! Example loading dividend events from .divbin file
//!
//! Run with:
//! ```bash
//! cargo run --example load_dividends
//! ```

use lod::discover_and_load_dividends;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_path = Path::new("/../../data/consolidated/stock-split-dividend-test/FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv");

    println!("=== Dividend Discovery Example ===\n");

    // Auto-discover dividends
    let dividends = discover_and_load_dividends(data_path, "FAST", true);

    println!("\nDividend Events:");
    println!("{:-<80}", "");
    println!("{:>5} {:>12} {:>12}",
             "#", "Ex-Date", "Amount");
    println!("{:-<80}", "");

    for (i, div) in dividends.iter().enumerate() {
        println!("{:>5} {:>12} ${:>10.4}",
                 i + 1,
                 div.ex_date.format("%Y-%m-%d"),
                 div.amount);
    }

    println!("\n{:-<80}", "");
    println!("Total: {} dividend events", dividends.len());

    Ok(())
}
