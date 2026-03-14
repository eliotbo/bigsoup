//! NBBO decoder binary - command-line tool for reading .nbbo files

use lod::nbbo_decoder::{NbboReader, NbboStats};
use std::env;
use std::path::Path;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <nbbo_file> [command]", args[0]);
        eprintln!();
        eprintln!("Commands:");
        eprintln!("  info        - Show file header information (default)");
        eprintln!("  stats       - Calculate and display statistics");
        eprintln!("  head <n>    - Display first n records (default: 10)");
        eprintln!("  tail <n>    - Display last n records (default: 10)");
        eprintln!("  sample <n>  - Display n random samples");
        eprintln!("  export      - Export all records as JSON");
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

fn run_command(
    file_path: &str,
    command: &str,
    args: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = NbboReader::open(file_path)?;

    match command {
        "info" => {
            let header = reader.header();
            println!("NBBO File Information");
            println!("====================");
            println!("Symbol: {}", header.symbol);
            println!("Version: {}", header.version);
            println!("Record Count: {}", header.record_count);
            println!("Instrument ID: {}", header.instrument_id);
            println!("Created: {}", format_timestamp(header.created_at_ns));
            println!("Header Size: {} bytes", header.header_length);
            println!("Footprint Flags: 0x{:02x}", header.footprint_flags);

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
            let stats = NbboStats::calculate(&records);

            println!("\nNBBO Statistics");
            println!("===============");
            println!("Total Records: {}", stats.record_count);
            println!(
                "Two-sided Quotes: {} ({:.1}%)",
                stats.two_sided_count,
                stats.two_sided_count as f64 / stats.record_count as f64 * 100.0
            );
            println!(
                "Bid-only: {} ({:.1}%)",
                stats.bid_only_count,
                stats.bid_only_count as f64 / stats.record_count as f64 * 100.0
            );
            println!(
                "Ask-only: {} ({:.1}%)",
                stats.ask_only_count,
                stats.ask_only_count as f64 / stats.record_count as f64 * 100.0
            );
            println!(
                "Empty: {} ({:.1}%)",
                stats.empty_count,
                stats.empty_count as f64 / stats.record_count as f64 * 100.0
            );

            if let Some(min_bid) = stats.min_bid {
                println!("\nBid Statistics:");
                println!("  Min: ${:.4}", min_bid);
                println!("  Max: ${:.4}", stats.max_bid.unwrap());
                println!("  Total Size: {}", stats.total_bid_size);
            }

            if let Some(min_ask) = stats.min_ask {
                println!("\nAsk Statistics:");
                println!("  Min: ${:.4}", min_ask);
                println!("  Max: ${:.4}", stats.max_ask.unwrap());
                println!("  Total Size: {}", stats.total_ask_size);
            }

            if let Some(min_spread) = stats.min_spread {
                println!("\nSpread Statistics:");
                println!("  Min: ${:.6}", min_spread);
                println!("  Max: ${:.6}", stats.max_spread.unwrap());
                println!("  Avg: ${:.6}", stats.avg_spread.unwrap());
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
            let step = total / n as u64;

            println!("Sampling {} records (every {} records)", n, step);
            for i in 0..n {
                let idx = i as u64 * step;
                if idx >= total {
                    break;
                }
                let record = reader.read_record(idx)?;
                print_record(&record, idx);
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
                        "ts_recv": r.ts_recv,
                        "bid": r.bid(),
                        "bid_sz": r.bid_sz,
                        "bid_ct": r.bid_ct,
                        "ask": r.ask(),
                        "ask_sz": r.ask_sz,
                        "ask_ct": r.ask_ct,
                        "spread": r.spread(),
                        "mid": r.mid_price(),
                    })
                })
                .collect();

            println!("{}", serde_json::to_string_pretty(&json_records)?);
        }

        _ => {
            eprintln!("Unknown command: {}", command);
            return Err("Invalid command".into());
        }
    }

    Ok(())
}

fn print_records(records: &[lod::nbbo_decoder::NbboRecord]) {
    println!(
        "{:<5} {:>20} {:>12} {:>8} {:>12} {:>8} {:>12}",
        "Index", "Timestamp", "Bid", "BidSz", "Ask", "AskSz", "Spread"
    );
    println!("{}", "-".repeat(90));

    for (i, record) in records.iter().enumerate() {
        print_record(record, i as u64);
    }
}

fn print_record(record: &lod::nbbo_decoder::NbboRecord, index: u64) {
    let bid_str = record
        .bid()
        .map(|p| format!("${:.4}", p))
        .unwrap_or_else(|| "---".to_string());

    let ask_str = record
        .ask()
        .map(|p| format!("${:.4}", p))
        .unwrap_or_else(|| "---".to_string());

    let spread_str = record
        .spread()
        .map(|s| format!("${:.6}", s))
        .unwrap_or_else(|| "---".to_string());

    println!(
        "{:<5} {:>20} {:>12} {:>8} {:>12} {:>8} {:>12}",
        index,
        format_timestamp(record.ts_event),
        bid_str,
        record.bid_sz,
        ask_str,
        record.ask_sz,
        spread_str
    );
}

fn format_timestamp(ns: u64) -> String {
    let secs = ns / 1_000_000_000;
    let nanos = ns % 1_000_000_000;

    // Simple Unix timestamp formatting
    format!("{}.{:09}", secs, nanos)
}
