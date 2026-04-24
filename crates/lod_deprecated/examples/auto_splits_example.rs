//! Example demonstrating automatic split discovery and adjustment
//!
//! This example loads OHLCV data and automatically discovers and applies
//! split adjustments from a splits.json file in the same directory.
//!
//! Run with:
//! ```bash
//! cargo run --example auto_splits_example
//! ```

use lod::nohlcv_decoder::NohlcvReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Path to FAST test data
    let data_path = "../../../../data/consolidated/stock-split-dividend-test/FAST/2025-01-15_to_2025-10-03-ohlcv-1m.nohlcv";

    println!("=== Auto-Discovery Split Adjustment Example ===\n");

    // Open with automatic split discovery
    println!("Opening OHLCV file with auto-discovery...");
    let mut reader = NohlcvReader::open_with_auto_splits(data_path)?;

    println!("\nFile info:");
    println!("  Symbol: {}", reader.symbol());
    println!("  Records: {}", reader.record_count());

    // Read some records to see the adjustment
    println!("\nReading first 5 records (split-adjusted):");
    println!("{:>5} {:>20} {:>12} {:>12} {:>12} {:>12}",
             "Index", "Timestamp", "Open", "High", "Low", "Close");
    println!("{:-<80}", "");

    for i in 0..5 {
        let record = reader.read_record(i)?;
        if let (Some(open), Some(high), Some(low), Some(close)) =
            (record.open(), record.high(), record.low(), record.close()) {
            println!("{:>5} {:>20} {:>12.4} {:>12.4} {:>12.4} {:>12.4}",
                     i, record.ts_event, open, high, low, close);
        }
    }

    // Find records around the 2025-05-22 split date
    println!("\n\nSearching for records around 2025-05-22 split...");

    // 2025-05-22 00:00:00 UTC in nanoseconds
    let split_date_ns = 1747872000_000_000_000u64;

    // Find records around this date
    for i in 0..reader.record_count() {
        let record = reader.read_record(i)?;

        // Check if within 1 day of split
        let diff = if record.ts_event > split_date_ns {
            record.ts_event - split_date_ns
        } else {
            split_date_ns - record.ts_event
        };

        let one_day_ns = 86400_000_000_000u64;

        if diff < one_day_ns && record.has_valid_prices() {
            if let (Some(open), Some(close)) = (record.open(), record.close()) {
                let date = chrono::DateTime::from_timestamp(
                    (record.ts_event / 1_000_000_000) as i64,
                    0
                ).unwrap();
                println!("  {}: open={:.4}, close={:.4}",
                         date.format("%Y-%m-%d %H:%M"), open, close);
            }

            // Show a few records
            if i > 100 {
                break;
            }
        }
    }

    println!("\n=== Example Complete ===");
    println!("\nNote: All prices shown are split-adjusted.");
    println!("The 2025-05-22 2:1 split was automatically applied to pre-split data.");

    Ok(())
}
