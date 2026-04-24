//! Integration tests for NOHLCV functionality

use lod::nohlcv_decoder::{NohlcvReader, OhlcvRecord as DecoderOhlcvRecord, OhlcvStats};
use lod::traits::OhlcvRecord as TraitOhlcvRecord;
use lod::{LevelStore, QuoteLike, StreamingAggregator};

/// Convert decoder record to trait record
fn convert_record(rec: &DecoderOhlcvRecord) -> TraitOhlcvRecord {
    TraitOhlcvRecord {
        ts_event: rec.ts_event,
        open_px: rec.open_px,
        high_px: rec.high_px,
        low_px: rec.low_px,
        close_px: rec.close_px,
        volume: rec.volume,
        trade_count: rec.trade_count,
    }
}

#[test]
fn test_ohlcv_quote_like_implementation() {
    let record = TraitOhlcvRecord {
        ts_event: 1234567890000000000,
        open_px: 100_500_000_000,  // $100.50
        high_px: 101_000_000_000,  // $101.00
        low_px: 100_000_000_000,   // $100.00
        close_px: 100_750_000_000, // $100.75
        volume: 1000000,
        trade_count: 250,
    };

    // Test QuoteLike implementation
    assert_eq!(record.timestamp(), 1234567890000000000);
    assert!((record.open() - 100.50).abs() < 0.0001);
    assert!((record.high() - 101.00).abs() < 0.0001);
    assert!((record.low() - 100.00).abs() < 0.0001);
    assert!((record.close() - 100.75).abs() < 0.0001);
    assert_eq!(record.volume(), 1000000.0);
    assert_eq!(record.count(), Some(250));
}

#[test]
fn test_ohlcv_aggregation() {
    // Create test OHLCV data with 1-second intervals
    let records: Vec<TraitOhlcvRecord> = (0..100)
        .map(|i| TraitOhlcvRecord {
            ts_event: i * 1_000_000_000,                     // 1 second intervals
            open_px: (100_000 + i as i64 * 100) * 1_000_000, // Prices increasing
            high_px: (100_100 + i as i64 * 100) * 1_000_000,
            low_px: (99_900 + i as i64 * 100) * 1_000_000,
            close_px: (100_050 + i as i64 * 100) * 1_000_000,
            volume: 1000 + i * 10,
            trade_count: 10 + i as u32,
        })
        .collect();

    // Create aggregator with multiple intervals
    let mut aggregator = StreamingAggregator::new(1, vec![5, 10, 30]);

    // Process records
    let quote_refs: Vec<&dyn QuoteLike> = records.iter().map(|r| r as &dyn QuoteLike).collect();
    aggregator.push_batch(&quote_refs);

    // Seal and create store
    let batch = aggregator.seal();
    let store = LevelStore::from_stream(batch.levels.into_iter().collect());

    // Verify aggregation
    assert!(store.get(5).is_some());
    assert!(store.get(10).is_some());
    assert!(store.get(30).is_some());

    // Check 5-second aggregation
    let level_5s = store.get(5).unwrap();
    assert!(level_5s.len() > 0);
    assert!(level_5s.len() <= 20); // 100 seconds / 5 = 20 max

    // Verify first aggregated candle
    let first = &level_5s[0];
    assert_eq!(first.ts, 0); // Should start at timestamp 0

    // Verify aggregated OHLC makes sense
    // The high should be >= open, close, low
    // The low should be <= open, close, high
    assert!(first.high >= first.open);
    assert!(first.high >= first.close);
    assert!(first.low <= first.open);
    assert!(first.low <= first.close);
}

#[test]
#[ignore] // Run with: cargo test --package lod test_real_nohlcv_file -- --ignored
fn test_real_nohlcv_file() {
    // Try to find a real NOHLCV file
    let possible_paths = [
        "/workspace/workspace/bb/beta_breaker/data/consolidated/hedgy-test/CIEN/ohlcv-1m.nohlcv",
        "/workspace/workspace/bb/beta_breaker/data/nohlcv-1m.nohlcv",
    ];

    let file_path = possible_paths
        .iter()
        .find(|p| std::path::Path::new(p).exists());

    let Some(file_path) = file_path else {
        eprintln!("No NOHLCV file found, skipping test");
        return;
    };

    println!("Testing with file: {}", file_path);

    // Open and read file
    let mut reader = NohlcvReader::open(file_path).expect("Failed to open NOHLCV file");

    println!("File info:");
    println!("  Symbol: {}", reader.symbol());
    println!("  Records: {}", reader.record_count());

    // Read sample records
    let sample_size = 1000.min(reader.record_count() as usize);
    let records = reader
        .read_records_validated(0, sample_size)
        .expect("Failed to read records");

    println!("  Valid records: {}/{}", records.len(), sample_size);

    if records.is_empty() {
        println!("No valid records found");
        return;
    }

    // Calculate statistics
    let stats = OhlcvStats::calculate(&records);

    println!("\nStatistics:");
    println!(
        "  Price range: ${:.4} - ${:.4}",
        stats.min_price.unwrap_or(0.0),
        stats.max_price.unwrap_or(0.0)
    );
    println!("  Avg close: ${:.4}", stats.avg_close.unwrap_or(0.0));
    println!("  Total volume: {}", stats.total_volume);
    println!("  Total trades: {}", stats.total_trades);

    // Test aggregation with real data
    let trait_records: Vec<TraitOhlcvRecord> = records.iter().map(convert_record).collect();

    let mut aggregator = StreamingAggregator::new(60, vec![60, 300, 900]);

    let quote_refs: Vec<&dyn QuoteLike> =
        trait_records.iter().map(|r| r as &dyn QuoteLike).collect();
    aggregator.push_batch(&quote_refs);

    let batch = aggregator.seal();
    let store = LevelStore::from_stream(batch.levels.into_iter().collect());

    for interval in store.intervals() {
        let candles = store.get(interval).unwrap();
        println!("  {} sec interval: {} candles", interval, candles.len());
    }
}

#[test]
fn test_ohlcv_decoder_iterator() {
    // Create test data in memory (would normally be a file)
    let test_records = vec![
        DecoderOhlcvRecord {
            ts_event: 1000000000,
            open_px: 100_000_000_000,
            high_px: 101_000_000_000,
            low_px: 99_000_000_000,
            close_px: 100_500_000_000,
            volume: 1000,
            turnover: 100000,
            trade_count: 10,
            _reserved: 0,
        },
        DecoderOhlcvRecord {
            ts_event: 2000000000,
            open_px: 100_500_000_000,
            high_px: 102_000_000_000,
            low_px: 100_000_000_000,
            close_px: 101_500_000_000,
            volume: 2000,
            turnover: 200000,
            trade_count: 20,
            _reserved: 0,
        },
    ];

    // Test record serialization/deserialization
    for record in &test_records {
        let bytes = record.to_bytes();
        let decoded = DecoderOhlcvRecord::from_bytes(&bytes);
        assert_eq!(*record, decoded);
    }

    // Test validation
    assert!(test_records[0].validate_ohlc());
    assert!(test_records[1].validate_ohlc());

    // Test VWAP calculation
    let vwap = test_records[0].vwap();
    assert!(vwap.is_some());
    assert!((vwap.unwrap() - 100.0).abs() < 0.1);

    // Test typical price
    let typical = test_records[0].typical_price();
    assert!(typical.is_some());
    // Typical = (High + Low + Close) / 3 = (101 + 99 + 100.5) / 3 = 100.17
    assert!((typical.unwrap() - 100.17).abs() < 0.01);
}

#[test]
fn test_ohlcv_stats_calculation() {
    let records = vec![
        DecoderOhlcvRecord {
            ts_event: 1000000000,
            open_px: 100_000_000_000,
            high_px: 101_000_000_000,
            low_px: 99_000_000_000,
            close_px: 100_500_000_000,
            volume: 1000,
            turnover: 100000,
            trade_count: 10,
            _reserved: 0,
        },
        DecoderOhlcvRecord {
            ts_event: 2000000000,
            open_px: 100_500_000_000,
            high_px: 102_000_000_000,
            low_px: 100_000_000_000,
            close_px: 101_500_000_000,
            volume: 2000,
            turnover: 200000,
            trade_count: 20,
            _reserved: 0,
        },
        DecoderOhlcvRecord {
            ts_event: 3000000000,
            open_px: i64::MAX, // Invalid price
            high_px: i64::MAX,
            low_px: i64::MAX,
            close_px: i64::MAX,
            volume: 0,
            turnover: 0,
            trade_count: 0,
            _reserved: 0,
        },
    ];

    let stats = OhlcvStats::calculate(&records);

    assert_eq!(stats.record_count, 3);
    assert_eq!(stats.valid_count, 2);
    assert_eq!(stats.invalid_count, 1);

    // Check price range (from valid records)
    assert!(stats.min_price.is_some());
    assert!((stats.min_price.unwrap() - 99.0).abs() < 0.01);
    assert!((stats.max_price.unwrap() - 102.0).abs() < 0.01);

    // Check volume metrics
    assert_eq!(stats.total_volume, 3000); // 1000 + 2000 + 0
    assert_eq!(stats.total_turnover, 300000); // 100000 + 200000 + 0
    assert_eq!(stats.total_trades, 30); // 10 + 20 + 0

    // Check time range
    assert_eq!(stats.min_timestamp, Some(1000000000));
    assert_eq!(stats.max_timestamp, Some(3000000000));
    assert!((stats.time_range_secs().unwrap() - 2.0).abs() < 0.01);
}
