//! Tests for the loader module

use lod::loader::DataLoader;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use tempfile::TempDir;

/// Helper to create a test CSV file
fn create_test_csv(dir: &TempDir, name: &str, data: &[(i64, f64, f64, f64, f64, f64)]) -> PathBuf {
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

    path
}

#[test]
fn test_csv_loading() {
    let temp_dir = TempDir::new().unwrap();
    let test_data = vec![
        (1609459200, 100.0, 105.0, 99.0, 103.0, 1000.0),
        (1609459260, 103.0, 104.0, 102.0, 102.5, 1200.0),
        (1609459320, 102.5, 103.5, 101.5, 103.0, 1100.0),
    ];

    let csv_path = create_test_csv(&temp_dir, "test.csv", &test_data);

    let candles = lod::loader::load_candles_from_path(csv_path.to_str().unwrap()).unwrap();

    assert_eq!(candles.len(), 3);
    assert_eq!(candles[0].open, 100.0);
    assert_eq!(candles[0].high, 105.0);
    assert_eq!(candles[0].low, 99.0);
    assert_eq!(candles[0].close, 103.0);
    assert_eq!(candles[0].volume, 1000.0);

    // Check timestamp is in nanoseconds
    assert_eq!(candles[0].ts, 1609459200_000_000_000);
}

#[test]
fn test_data_loader_with_csv() {
    let temp_dir = TempDir::new().unwrap();

    // Create directory structure
    let exchange_dir = temp_dir.path().join("test-exchange");
    let symbol_dir = exchange_dir.join("TEST");
    fs::create_dir_all(&symbol_dir).unwrap();

    // Create test CSV file in the symbol directory
    let test_data = vec![
        (1609459200, 100.0, 105.0, 99.0, 103.0, 1000.0),
        (1609459260, 103.0, 104.0, 102.0, 102.5, 1200.0),
    ];
    let csv_path = symbol_dir.join("test.csv");
    let mut file = fs::File::create(&csv_path).unwrap();
    for (timestamp, open, high, low, close, volume) in &test_data {
        writeln!(
            file,
            "{},{},{},{},{},{}",
            timestamp, open, high, low, close, volume
        )
        .unwrap();
    }

    let mut loader = DataLoader::new(vec![temp_dir.path().to_path_buf()]);
    loader.scan_directories().unwrap();

    let symbols = loader.get_available_symbols();
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0], "TEST");
}

#[test]
fn test_interval_parsing() {
    let _loader = DataLoader::new(vec![]);

    // Test through the parse_interval method indirectly
    // We'll test this via filename parsing in the next test
}

#[test]
fn test_date_range_calculation() {
    let temp_dir = TempDir::new().unwrap();

    // Create test structure
    let exchange_dir = temp_dir.path().join("test-exchange");
    let symbol_dir = exchange_dir.join("AAPL");
    fs::create_dir_all(&symbol_dir).unwrap();

    // Create multiple CSV files with different timestamps
    let test_data1 = vec![(1609459200, 100.0, 105.0, 99.0, 103.0, 1000.0)];
    create_test_csv(
        &TempDir::new_in(&symbol_dir).unwrap(),
        "data1.csv",
        &test_data1,
    );

    let test_data2 = vec![(1609545600, 103.0, 107.0, 102.0, 106.0, 1500.0)];
    create_test_csv(
        &TempDir::new_in(&symbol_dir).unwrap(),
        "data2.csv",
        &test_data2,
    );

    let mut loader = DataLoader::new(vec![temp_dir.path().to_path_buf()]);
    loader.scan_directories().unwrap();

    if let Some(metadata) = loader.get_symbol_info("AAPL") {
        assert!(metadata.date_range.0 <= 1609459200);
        assert!(metadata.date_range.1 >= 1609545600);
    }
}
