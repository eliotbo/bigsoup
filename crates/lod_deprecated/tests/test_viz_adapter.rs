//! Tests for the viz_adapter module

use lod::viz_adapter::*;
use std::fs;
use std::io::Write;
use tempfile::TempDir;

/// Helper to create a test CSV file
fn create_test_csv(dir: &TempDir, name: &str, data: &[(i64, f64, f64, f64, f64, f64)]) -> String {
    let path = dir.path().join(name);
    let mut file = fs::File::create(&path).unwrap();

    for (timestamp, open, high, low, close, volume) in data {
        writeln!(
            file,
            "{},{},{},{},{},{}",
            timestamp, open, high, low, close, volume
        )
        .unwrap();
    }

    path.to_string_lossy().to_string()
}

#[test]
fn test_import_candles() {
    let temp_dir = TempDir::new().unwrap();
    let test_data = vec![
        (1609459200, 100.0, 105.0, 99.0, 103.0, 1000.0),
        (1609459260, 103.0, 104.0, 102.0, 102.5, 1200.0),
        (1609459320, 102.5, 103.5, 101.5, 103.0, 1100.0),
    ];

    let csv_path = create_test_csv(&temp_dir, "test.csv", &test_data);

    let candles = import_candles(&csv_path, None).unwrap();

    assert_eq!(candles.len(), 3);
    assert_eq!(candles[0].timestamp, 1609459200);
    assert_eq!(candles[0].open, 100.0);
    assert_eq!(candles[0].high, 105.0);
    assert_eq!(candles[0].low, 99.0);
    assert_eq!(candles[0].close, 103.0);
    assert_eq!(candles[0].volume, 1000.0);
    assert_eq!(candles[0].source, DataSource::Csv);
}

#[test]
fn test_import_candles_with_range() {
    let temp_dir = TempDir::new().unwrap();
    let test_data = vec![
        (1609459200, 100.0, 105.0, 99.0, 103.0, 1000.0),
        (1609459260, 103.0, 104.0, 102.0, 102.5, 1200.0),
        (1609459320, 102.5, 103.5, 101.5, 103.0, 1100.0),
    ];

    let csv_path = create_test_csv(&temp_dir, "test.csv", &test_data);

    // Test with range that excludes the last candle
    let range = Some((1609459200, 1609459260));
    let candles = import_candles(&csv_path, range).unwrap();

    assert_eq!(candles.len(), 2);
    assert_eq!(candles[0].timestamp, 1609459200);
    assert_eq!(candles[1].timestamp, 1609459260);
}

#[test]
fn test_aggregate_by_interval() {
    let candles = vec![
        Candle {
            timestamp: 1609459200, // Minute 0
            open: 100.0,
            high: 105.0,
            low: 99.0,
            close: 103.0,
            volume: 1000.0,
            source: DataSource::Csv,
            metadata: None,
        },
        Candle {
            timestamp: 1609459230, // Still within the same minute
            open: 103.0,
            high: 104.0,
            low: 102.0,
            close: 102.5,
            volume: 1200.0,
            source: DataSource::Csv,
            metadata: None,
        },
        Candle {
            timestamp: 1609459320, // Next minute
            open: 102.5,
            high: 103.5,
            low: 101.5,
            close: 103.0,
            volume: 1100.0,
            source: DataSource::Csv,
            metadata: None,
        },
    ];

    let aggregated = aggregate_by_interval(&candles, 1); // 1-minute aggregation

    assert_eq!(aggregated.len(), 2);

    // First minute should combine first two candles
    assert_eq!(aggregated[0].timestamp, 1609459200);
    assert_eq!(aggregated[0].open, 100.0);
    assert_eq!(aggregated[0].high, 105.0); // Max of both highs
    assert_eq!(aggregated[0].low, 99.0); // Min of both lows
    assert_eq!(aggregated[0].close, 102.5); // Close of last candle in bucket
    assert_eq!(aggregated[0].volume, 2200.0); // Sum of volumes

    // Second minute should have just the third candle
    assert_eq!(aggregated[1].timestamp, 1609459320);
    assert_eq!(aggregated[1].open, 102.5);
}

#[test]
fn test_aggregate_with_metadata() {
    let metadata1 = Metadata {
        symbol: "TEST".to_string(),
        instrument_id: 12345,
        trade_count: Some(10),
        turnover: Some(100000),
    };

    let metadata2 = Metadata {
        symbol: "TEST".to_string(),
        instrument_id: 12345,
        trade_count: Some(15),
        turnover: Some(150000),
    };

    let candles = vec![
        Candle {
            timestamp: 1609459200,
            open: 100.0,
            high: 105.0,
            low: 99.0,
            close: 103.0,
            volume: 1000.0,
            source: DataSource::Nohlcv,
            metadata: Some(metadata1),
        },
        Candle {
            timestamp: 1609459230, // Same minute
            open: 103.0,
            high: 104.0,
            low: 102.0,
            close: 102.5,
            volume: 1200.0,
            source: DataSource::Nohlcv,
            metadata: Some(metadata2),
        },
    ];

    let aggregated = aggregate_by_interval(&candles, 1);

    assert_eq!(aggregated.len(), 1);

    let agg_metadata = aggregated[0].metadata.as_ref().unwrap();
    assert_eq!(agg_metadata.symbol, "TEST");
    assert_eq!(agg_metadata.instrument_id, 12345);
    assert_eq!(agg_metadata.trade_count, Some(25)); // 10 + 15
    assert_eq!(agg_metadata.turnover, Some(250000)); // 100000 + 150000
}

#[test]
fn test_candle_to_plot_candle_conversion() {
    let candle = Candle {
        timestamp: 1609459200,
        open: 100.5,
        high: 105.75,
        low: 99.25,
        close: 103.0,
        volume: 1000.5,
        source: DataSource::Csv,
        metadata: None,
    };

    let plot_candle = candle_to_plot_candle(&candle);

    assert_eq!(plot_candle.ts, 1609459200_000_000_000); // Converted to nanoseconds
    assert_eq!(plot_candle.open, 100.5);
    assert_eq!(plot_candle.high, 105.75);
    assert_eq!(plot_candle.low, 99.25);
    assert_eq!(plot_candle.close, 103.0);
    assert_eq!(plot_candle.volume, 1000.5);
}

#[test]
fn test_plot_candle_to_candle_conversion() {
    let plot_candle =
        lod::PlotCandle::new(1609459200_000_000_000, 100.5, 105.75, 99.25, 103.0, 1000.5);

    let candle = plot_candle_to_candle(&plot_candle);

    assert_eq!(candle.timestamp, 1609459200); // Converted from nanoseconds
    assert_eq!(candle.open as f32, 100.5);
    assert_eq!(candle.high as f32, 105.75);
    assert_eq!(candle.low as f32, 99.25);
    assert_eq!(candle.close as f32, 103.0);
    assert_eq!(candle.volume as f32, 1000.5);
    assert_eq!(candle.source, DataSource::Nohlcv);
}

#[test]
fn test_ohlc_trait() {
    let candle = Candle {
        timestamp: 1609459200,
        open: 100.0,
        high: 105.0,
        low: 99.0,
        close: 103.0,
        volume: 1000.0,
        source: DataSource::Csv,
        metadata: None,
    };

    assert_eq!(candle.time(), Time(1609459200));
    assert_eq!(candle.unix_time(), 1609459200.0);
    assert_eq!(candle.open(), 100.0);
    assert_eq!(candle.high(), 105.0);
    assert_eq!(candle.low(), 99.0);
    assert_eq!(candle.close(), 103.0);
    assert_eq!(candle.volume(), 1000.0);
}

#[test]
fn test_format_timestamp() {
    // Test with day-level step
    let formatted = format_timestamp(1609459200.0, 86400.0);
    assert!(formatted.contains("2021-01-01") || formatted.contains("2020-12-31"));

    // Test with minute-level step
    let formatted = format_timestamp(1609459200.0, 60.0);
    assert!(formatted.contains(":")); // Should contain time
}

#[test]
fn test_view_utils() {
    use view_utils::*;

    // Test nice_step
    let step = nice_step(100.0, 10);
    assert_eq!(step, 10.0);

    // Test slice_centered
    let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9];
    let sliced = slice_centered(&data, 4, 2);
    assert_eq!(sliced, vec![3, 4, 5, 6, 7]);

    // Test visible_rows
    let rows = visible_rows(100, 500.0, 20.0);
    assert_eq!(rows, 25);
}

#[test]
fn test_format_detection() {
    use format_detection::*;

    let temp_dir = TempDir::new().unwrap();

    // Test CSV detection
    let csv_path = temp_dir.path().join("test.csv");
    fs::write(&csv_path, "1,2,3,4,5,6").unwrap();

    let format = detect_format(csv_path.to_str().unwrap()).unwrap();
    assert_eq!(format, DataFormat::Csv);

    // Test NOHLCV detection by content
    let nohlcv_path = temp_dir.path().join("test.dat");
    fs::write(&nohlcv_path, b"NOHL").unwrap();

    let format = detect_format(nohlcv_path.to_str().unwrap()).unwrap();
    assert_eq!(format, DataFormat::Nohlcv);
}
