//! NOHLCV decoder binary - command-line tool for reading .nohlcv files

use lod::nohlcv_decoder::{NohlcvReader, OhlcvStats};
use std::env;
use std::path::Path;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_usage(&args[0]);
        process::exit(1);
    }

    let file_path = &args[1];
    let command = args.get(2).map(|s| s.as_str()).unwrap_or("info");

    if !Path::new(file_path).exists() {
        eprintln!("Error: File '{}' does not exist", file_path);
        process::exit(1);
    }

    match run_command(file_path, command, &args[3..]) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}

fn print_usage(program: &str) {
    eprintln!("Usage: {} <nohlcv_file> [command] [args]", program);
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  info        - Show file header information (default)");
    eprintln!("  stats       - Calculate and display statistics");
    eprintln!("  head <n>    - Display first n records (default: 10)");
    eprintln!("  tail <n>    - Display last n records (default: 10)");
    eprintln!("  sample <n>  - Display n evenly-spaced samples");
    eprintln!("  validate    - Validate all records for OHLC consistency");
    eprintln!("  export      - Export all records as JSON");
    eprintln!("  stream      - Stream records with progress indicator");
}

fn run_command(
    file_path: &str,
    command: &str,
    args: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = NohlcvReader::open(file_path)?;

    match command {
        "info" => {
            let header = reader.header();
            println!("NOHLCV File Information");
            println!("=======================");
            println!("Symbol: {}", header.symbol);
            println!("Version: {}", header.version);
            println!("Record Count: {}", header.record_count);
            println!("Instrument ID: {}", header.instrument_id);
            println!("Created: {}", format_timestamp(header.created_at_ns));
            println!("Header Size: {} bytes", header.header_length);
            println!("Footprint Flags: 0x{:02x}", header.footprint_flags);
            println!("Expected File Size: {} bytes", header.expected_file_size());

            if !header.metadata.is_empty() {
                println!("\nMetadata:");
                for (key, value) in &header.metadata {
                    println!("  {}: {}", key, value);
                }
            }
        }

        "stats" => {
            println!("Loading records...");
            let records = reader.read_all()?;
            let stats = OhlcvStats::calculate(&records);

            println!("\nNOHLCV Statistics");
            println!("=================");
            println!("Total Records: {}", stats.record_count);
            println!(
                "Valid Records: {} ({:.1}%)",
                stats.valid_count,
                stats.valid_count as f64 / stats.record_count as f64 * 100.0
            );
            println!(
                "Invalid Records: {} ({:.1}%)",
                stats.invalid_count,
                stats.invalid_count as f64 / stats.record_count as f64 * 100.0
            );

            if let Some(min_price) = stats.min_price {
                println!("\nPrice Range:");
                println!("  Min: ${:.4}", min_price);
                println!("  Max: ${:.4}", stats.max_price.unwrap());
                println!("  Avg Close: ${:.4}", stats.avg_close.unwrap_or(0.0));
            }

            println!("\nVolume Metrics:");
            println!("  Total Volume: {}", format_number(stats.total_volume));
            println!("  Total Turnover: ${}", format_number(stats.total_turnover));
            println!("  Total Trades: {}", format_number(stats.total_trades));

            if let Some(time_range) = stats.time_range_secs() {
                let days = time_range / 86400.0;
                println!("\nTime Range: {:.1} days", days);
            }
        }

        "head" => {
            let n: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(10);

            let records = reader.read_records(0, n)?;
            print_records(&records);
        }

        "tail" => {
            let n: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(10);

            let total = reader.record_count();
            let start = if total > n as u64 {
                total - n as u64
            } else {
                0
            };
            let records = reader.read_records(start, n)?;
            print_records(&records);
        }

        "sample" => {
            let n: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(10);

            let total = reader.record_count();
            let step = (total / n as u64).max(1);

            println!("Sampling {} records (every {} records)", n, step);
            println!();

            for i in 0..n {
                let idx = (i as u64 * step).min(total - 1);
                if idx >= total {
                    break;
                }
                let record = reader.read_record(idx)?;
                print_record(&record, idx);
            }
        }

        "validate" => {
            println!("Validating all records...");
            let mut valid_count = 0;
            let mut invalid_count = 0;
            let mut issues = Vec::new();

            for (idx, result) in reader.iter().enumerate() {
                let record = result?;

                if record.validate_ohlc() {
                    valid_count += 1;
                } else {
                    invalid_count += 1;
                    if issues.len() < 10 {
                        issues.push((idx, describe_validation_issue(&record)));
                    }
                }

                // Progress indicator
                if idx % 100000 == 0 && idx > 0 {
                    print!(".");
                    use std::io::Write;
                    std::io::stdout().flush().ok();
                }
            }
            println!();

            println!("\nValidation Results:");
            println!("  Valid: {} records", valid_count);
            println!("  Invalid: {} records", invalid_count);

            if !issues.is_empty() {
                println!("\nFirst {} validation issues:", issues.len());
                for (idx, issue) in issues {
                    println!("  Record {}: {}", idx, issue);
                }
            }
        }

        "export" => {
            use serde_json;

            let records = reader.read_all()?;
            let json_records: Vec<_> = records
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "ts_event": r.ts_event,
                        "timestamp_secs": r.timestamp_secs(),
                        "open": r.open(),
                        "high": r.high(),
                        "low": r.low(),
                        "close": r.close(),
                        "volume": r.volume,
                        "turnover": r.turnover,
                        "trade_count": r.trade_count,
                        "vwap": r.vwap(),
                        "typical_price": r.typical_price(),
                        "valid": r.validate_ohlc(),
                    })
                })
                .collect();

            println!("{}", serde_json::to_string_pretty(&json_records)?);
        }

        "stream" => {
            println!("Streaming records...");
            let mut count = 0;
            let total = reader.record_count();

            for result in reader.iter() {
                let _record = result?;
                count += 1;

                // Progress indicator
                if count % 10000 == 0 {
                    let pct = (count as f64 / total as f64) * 100.0;
                    print!("\rProcessed: {} / {} ({:.1}%)", count, total, pct);
                    use std::io::Write;
                    std::io::stdout().flush().ok();
                }
            }
            println!("\nCompleted: {} records", count);
        }

        _ => {
            eprintln!("Unknown command: {}", command);
            return Err("Invalid command".into());
        }
    }

    Ok(())
}

fn print_records(records: &[lod::nohlcv_decoder::OhlcvRecord]) {
    println!(
        "{:<5} {:>20} {:>12} {:>12} {:>12} {:>12} {:>12} {:>8}",
        "Index", "Timestamp", "Open", "High", "Low", "Close", "Volume", "Trades"
    );
    println!("{}", "-".repeat(110));

    for (i, record) in records.iter().enumerate() {
        print_record(record, i as u64);
    }
}

fn print_record(record: &lod::nohlcv_decoder::OhlcvRecord, index: u64) {
    let open_str = record
        .open()
        .map(|p| format!("${:.4}", p))
        .unwrap_or_else(|| "NULL".to_string());

    let high_str = record
        .high()
        .map(|p| format!("${:.4}", p))
        .unwrap_or_else(|| "NULL".to_string());

    let low_str = record
        .low()
        .map(|p| format!("${:.4}", p))
        .unwrap_or_else(|| "NULL".to_string());

    let close_str = record
        .close()
        .map(|p| format!("${:.4}", p))
        .unwrap_or_else(|| "NULL".to_string());

    println!(
        "{:<5} {:>20} {:>12} {:>12} {:>12} {:>12} {:>12} {:>8}",
        index,
        format_timestamp(record.ts_event),
        open_str,
        high_str,
        low_str,
        close_str,
        format_number(record.volume),
        record.trade_count
    );
}

fn format_timestamp(ns: u64) -> String {
    let secs = ns / 1_000_000_000;
    let nanos = ns % 1_000_000_000;
    format!("{}.{:09}", secs, nanos)
}

fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn describe_validation_issue(record: &lod::nohlcv_decoder::OhlcvRecord) -> String {
    if !record.has_valid_prices() {
        return "Contains null or invalid price values".to_string();
    }

    let (o, h, l, c) = match (record.open(), record.high(), record.low(), record.close()) {
        (Some(o), Some(h), Some(l), Some(c)) => (o, h, l, c),
        _ => return "Missing price data".to_string(),
    };

    let mut issues = Vec::new();

    if h < o || h < l || h < c {
        issues.push(format!("High ({:.4}) not highest", h));
    }
    if l > o || l > h || l > c {
        issues.push(format!("Low ({:.4}) not lowest", l));
    }

    if issues.is_empty() {
        "Unknown validation issue".to_string()
    } else {
        issues.join(", ")
    }
}
