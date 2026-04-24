//! Integration test with real NBBO data

use lod::nbbo_decoder::{NbboReader, NbboRecord as DecoderNbboRecord};
use lod::traits::NbboRecord;
use lod::{LevelStore, QuoteLike, StreamingAggregator};

/// Convert decoder record to trait record
fn convert_record(rec: &DecoderNbboRecord) -> NbboRecord {
    NbboRecord {
        ts_event: rec.ts_event,
        bid_px: rec.bid_px,
        bid_sz: rec.bid_sz,
        ask_px: rec.ask_px,
        ask_sz: rec.ask_sz,
    }
}

#[test]
#[ignore] // Run with: cargo test --package lod real_data_test -- --ignored
fn test_real_nbbo_aggregation() {
    let file_path = "/workspace/workspace/bb/beta_breaker/data/consolidated/hedgy-test/CIEN/2018-05-01_to_2025-09-18-nbbo-1m.nbbo";

    // Check if file exists
    if !std::path::Path::new(file_path).exists() {
        eprintln!("Skipping test: NBBO file not found at {}", file_path);
        return;
    }

    // Open the file
    let mut reader = NbboReader::open(file_path).expect("Failed to open NBBO file");

    println!("File info:");
    println!("  Symbol: {}", reader.symbol());
    println!("  Records: {}", reader.record_count());

    // Read a sample of records (first 10000)
    let sample_size = 10000.min(reader.record_count() as usize);
    let records = reader
        .read_records(0, sample_size)
        .expect("Failed to read records");

    // Filter for valid records (reasonable timestamps and prices)
    let valid_records: Vec<_> = records
        .iter()
        .filter(|r| {
            // Check for reasonable timestamp (year 2018-2025)
            let ts_secs = r.ts_event / 1_000_000_000;
            let valid_ts = ts_secs >= 1514764800 && ts_secs <= 1767225600; // 2018-01-01 to 2025-12-31

            // Check for reasonable prices (between $0.01 and $10,000)
            let valid_prices = match (r.bid_px, r.ask_px) {
                (Some(bid), Some(ask)) => {
                    let bid_price = bid.abs() as f64 / 1_000_000_000.0;
                    let ask_price = ask.abs() as f64 / 1_000_000_000.0;
                    bid_price > 0.01
                        && bid_price < 10000.0
                        && ask_price > 0.01
                        && ask_price < 10000.0
                }
                _ => false,
            };

            valid_ts && valid_prices && r.is_two_sided()
        })
        .map(|r| convert_record(r))
        .collect();

    println!("Valid records: {}/{}", valid_records.len(), sample_size);

    if valid_records.is_empty() {
        println!("No valid records found for aggregation");
        return;
    }

    // Create aggregator with multiple intervals (1min, 5min, 15min, 1hour)
    let mut aggregator = StreamingAggregator::new(60, vec![60, 300, 900, 3600]);

    // Process valid records
    let quote_refs: Vec<&dyn QuoteLike> =
        valid_records.iter().map(|r| r as &dyn QuoteLike).collect();
    aggregator.push_batch(&quote_refs);

    // Seal and create store
    let batch = aggregator.seal();
    let store = LevelStore::from_stream(batch.levels.into_iter().collect());

    // Verify results
    println!("\nAggregation results:");
    for interval in store.intervals() {
        let candles = store.get(interval).unwrap();
        let _info = store.info(interval).unwrap();

        let duration_str = match interval {
            60 => "1min",
            300 => "5min",
            900 => "15min",
            3600 => "1hour",
            _ => "unknown",
        };

        println!("  {} interval: {} candles", duration_str, candles.len());

        if candles.len() > 0 {
            let first = &candles[0];
            let last = &candles[candles.len() - 1];

            println!(
                "    First: ts={:.0}, OHLC={:.2}/{:.2}/{:.2}/{:.2}, vol={:.0}",
                first.timestamp_secs(),
                first.open,
                first.high,
                first.low,
                first.close,
                first.volume
            );

            println!(
                "    Last:  ts={:.0}, OHLC={:.2}/{:.2}/{:.2}/{:.2}, vol={:.0}",
                last.timestamp_secs(),
                last.open,
                last.high,
                last.low,
                last.close,
                last.volume
            );
        }
    }

    // Test window extraction
    if let Some(candles_1m) = store.get(60) {
        if candles_1m.len() > 10 {
            use lod::window::take_window;

            let mid_idx = candles_1m.len() / 2;
            let mid_ts = candles_1m[mid_idx].timestamp_secs();

            let window_range = take_window(&candles_1m, mid_ts, 10);
            println!("\nWindow extraction test:");
            println!("  Centered at ts={:.0}, window size=10", mid_ts);
            println!(
                "  Range: {:?} (len={})",
                window_range,
                window_range.end - window_range.start
            );

            assert_eq!(
                window_range.end - window_range.start,
                10.min(candles_1m.len())
            );
        }
    }
}

#[test]
fn test_nbbo_decoder_stats() {
    let file_path = "/workspace/workspace/bb/beta_breaker/data/consolidated/hedgy-test/CIEN/2018-05-01_to_2025-09-18-nbbo-1m.nbbo";

    if !std::path::Path::new(file_path).exists() {
        eprintln!("Skipping test: NBBO file not found");
        return;
    }

    let mut reader = NbboReader::open(file_path).expect("Failed to open NBBO file");

    // Read a smaller sample for stats
    let records = reader
        .read_records(0, 1000)
        .expect("Failed to read records");

    // Calculate stats on valid records only
    let valid_records: Vec<_> = records
        .into_iter()
        .filter(|r| {
            let ts_secs = r.ts_event / 1_000_000_000;
            ts_secs >= 1514764800 && ts_secs <= 1767225600 && r.is_two_sided()
        })
        .collect();

    if !valid_records.is_empty() {
        let stats = lod::nbbo_decoder::NbboStats::calculate(&valid_records);

        println!("Statistics for {} valid records:", stats.record_count);
        println!("  Two-sided: {}", stats.two_sided_count);
        if let Some(min_bid) = stats.min_bid {
            println!(
                "  Bid range: ${:.4} - ${:.4}",
                min_bid,
                stats.max_bid.unwrap()
            );
        }
        if let Some(min_ask) = stats.min_ask {
            println!(
                "  Ask range: ${:.4} - ${:.4}",
                min_ask,
                stats.max_ask.unwrap()
            );
        }
        if let Some(avg_spread) = stats.avg_spread {
            println!("  Avg spread: ${:.6}", avg_spread);
        }
    }
}
