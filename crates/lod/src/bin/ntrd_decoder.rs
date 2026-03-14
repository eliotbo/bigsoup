//! NTRD decoder binary - command-line tool for reading .ntrd files

use lod::ntrd_decoder::{NtrdReader, TradeStats};
use std::env;
use std::path::Path;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <ntrd_file> [command]", args[0]);
        eprintln!();
        eprintln!("Commands:");
        eprintln!("  info        - Show file header information (default)");
        eprintln!("  stats       - Calculate and display statistics");
        eprintln!("  head <n>    - Display first n records (default: 10)");
        eprintln!("  tail <n>    - Display last n records (default: 10)");
        eprintln!("  sample <n>  - Display n random samples");
        eprintln!("  export      - Export all records as JSON");
        eprintln!("  buys <n>    - Display first n buy trades");
        eprintln!("  sells <n>   - Display first n sell trades");
        eprintln!("  large <n> <size> - Display first n trades >= size");
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
    let mut reader = NtrdReader::open(file_path)?;

    match command {
        "info" => {
            let header = reader.header();
            println!("NTRD File Information");
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
            let stats = TradeStats::calculate(&records);
            let time_stats = TradeStats::calculate_time_weighted(&records);

            println!("\nTrade Statistics");
            println!("================");
            println!("Total Records: {}", stats.record_count);
            println!(
                "Buy Trades: {} ({:.1}%)",
                stats.buy_count,
                stats.buy_count as f64 / stats.record_count as f64 * 100.0
            );
            println!(
                "Sell Trades: {} ({:.1}%)",
                stats.sell_count,
                stats.sell_count as f64 / stats.record_count as f64 * 100.0
            );
            println!(
                "Other: {} ({:.1}%)",
                stats.other_count,
                stats.other_count as f64 / stats.record_count as f64 * 100.0
            );

            println!("\nSpecial Trade Types:");
            println!(
                "  Odd Lots: {} ({:.1}%)",
                stats.odd_lot_count,
                stats.odd_lot_count as f64 / stats.record_count as f64 * 100.0
            );
            println!("  Opening Trades: {}", stats.opening_count);
            println!("  Closing Trades: {}", stats.closing_count);

            if let Some(min_price) = stats.min_price {
                println!("\nPrice Statistics:");
                println!("  Min: ${:.4}", min_price);
                println!("  Max: ${:.4}", stats.max_price.unwrap());
                println!("  Avg: ${:.4}", stats.avg_price.unwrap());
                if let Some(vwap) = stats.vwap {
                    println!("  VWAP: ${:.4}", vwap);
                }
            }

            if let Some(min_size) = stats.min_size {
                println!("\nSize Statistics:");
                println!("  Min: {}", min_size);
                println!("  Max: {}", stats.max_size.unwrap());
                println!("  Avg: {:.1}", stats.avg_size.unwrap());
                println!("  Total Volume: {}", stats.total_volume);
                println!("  Total Value: ${:.2}", stats.total_value);
            }

            println!("\nTime Statistics:");
            println!(
                "  Duration: {:.2} seconds",
                (time_stats.end_time - time_stats.start_time) as f64 / 1_000_000_000.0
            );
            println!("  Trades/Second: {:.2}", time_stats.trades_per_second);
            println!("  Volume/Second: {:.2}", time_stats.volume_per_second);
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

        "buys" => {
            let n: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(10);

            let records = reader.read_buys(0, n * 10)?; // Read more to ensure we get enough buys
            let limited: Vec<_> = records.into_iter().take(n).collect();
            println!("First {} buy trades:", n);
            print_records(&limited);
        }

        "sells" => {
            let n: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(10);

            let records = reader.read_sells(0, n * 10)?; // Read more to ensure we get enough sells
            let limited: Vec<_> = records.into_iter().take(n).collect();
            println!("First {} sell trades:", n);
            print_records(&limited);
        }

        "large" => {
            let n: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(10);
            let min_size: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1000);

            let records = reader.read_above_size(0, n * 100, min_size)?; // Read more to ensure we get enough
            let limited: Vec<_> = records.into_iter().take(n).collect();
            println!("First {} trades with size >= {}:", n, min_size);
            print_records(&limited);
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
                        "price": r.price_as_float(),
                        "size": r.size,
                        "side": r.side_char().to_string(),
                        "side_str": r.side_str(),
                        "flags": r.flags,
                        "exchange": r.exchange,
                        "trade_id": r.trade_id,
                        "value": r.value(),
                        "is_buy": r.is_buy(),
                        "is_sell": r.is_sell(),
                        "is_odd_lot": r.is_odd_lot(),
                        "is_opening": r.is_opening(),
                        "is_closing": r.is_closing(),
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

fn print_records(records: &[lod::ntrd_decoder::TradeRecord]) {
    println!(
        "{:<5} {:>20} {:>12} {:>8} {:>6} {:>8} {:>8} {:>12}",
        "Index", "Timestamp", "Price", "Size", "Side", "Flags", "Exch", "TradeID"
    );
    println!("{}", "-".repeat(95));

    for (i, record) in records.iter().enumerate() {
        print_record(record, i as u64);
    }
}

fn print_record(record: &lod::ntrd_decoder::TradeRecord, index: u64) {
    let flags_str = format!("{:02x}", record.flags);
    let special = if record.is_odd_lot() {
        "O"
    } else if record.is_opening() {
        "+"
    } else if record.is_closing() {
        "-"
    } else {
        " "
    };

    println!(
        "{:<5} {:>20} {:>12.4} {:>8} {:>6} {:>8} {:>8} {:>12}{}",
        index,
        format_timestamp(record.ts_event),
        record.price_as_float(),
        record.size,
        record.side_str(),
        flags_str,
        record.exchange,
        record.trade_id,
        special
    );
}

fn format_timestamp(ns: u64) -> String {
    let secs = ns / 1_000_000_000;
    let nanos = ns % 1_000_000_000;

    // Simple Unix timestamp formatting
    format!("{}.{:09}", secs, nanos)
}
