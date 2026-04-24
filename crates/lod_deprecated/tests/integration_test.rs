//! Integration tests for the LOD crate

use lod::traits::SimpleCandle;
use lod::{LevelStore, QuoteLike, StreamingAggregator};

#[test]
fn test_streaming_aggregation() {
    // Create test data with 1-second intervals
    let candles: Vec<SimpleCandle> = (0..100)
        .map(|i| SimpleCandle {
            timestamp: i * 1_000_000_000, // 1 second intervals in nanoseconds
            open: 100.0 + i as f64,
            high: 101.0 + i as f64,
            low: 99.0 + i as f64,
            close: 100.5 + i as f64,
            volume: 1000.0,
        })
        .collect();

    // Create aggregator with 5s, 10s, and 30s intervals
    let mut aggregator = StreamingAggregator::new(1, vec![5, 10, 30]);

    // Process candles
    let candle_refs: Vec<&dyn QuoteLike> = candles.iter().map(|c| c as &dyn QuoteLike).collect();
    aggregator.push_batch(&candle_refs);

    // Seal and get results
    let batch = aggregator.seal();

    // Create level store
    let store = LevelStore::from_stream(batch.levels.into_iter().collect());

    // Verify we have data for all intervals
    assert!(store.get(5).is_some());
    assert!(store.get(10).is_some());
    assert!(store.get(30).is_some());

    // Check 5-second interval has correct number of candles
    let level_5s = store.get(5).unwrap();
    assert!(level_5s.len() > 0);
    assert!(level_5s.len() <= 20); // 100 seconds / 5 = 20 max

    // Check 10-second interval
    let level_10s = store.get(10).unwrap();
    assert!(level_10s.len() > 0);
    assert!(level_10s.len() <= 10); // 100 seconds / 10 = 10 max

    // Check 30-second interval
    let level_30s = store.get(30).unwrap();
    assert!(level_30s.len() > 0);
    assert!(level_30s.len() <= 4); // 100 seconds / 30 = 3.33... so max 4

    println!("5s candles: {}", level_5s.len());
    println!("10s candles: {}", level_10s.len());
    println!("30s candles: {}", level_30s.len());
}

#[test]
fn test_nbbo_adapter() {
    use lod::traits::NbboRecord;

    let nbbo = NbboRecord {
        ts_event: 1_000_000_000,       // 1 second
        bid_px: Some(100_000_000_000), // $100
        bid_sz: 100,
        ask_px: Some(100_100_000_000), // $100.10
        ask_sz: 200,
    };

    assert_eq!(nbbo.timestamp(), 1_000_000_000);
    assert_eq!(nbbo.bid(), Some(100.0));
    assert_eq!(nbbo.ask(), Some(100.10));
    assert!((nbbo.open() - 100.05).abs() < 0.0001); // Mid price
}

#[test]
fn test_window_extraction() {
    use lod::levels::PlotCandle;
    use lod::window::{take_window, take_window_duration};
    use std::time::Duration;

    let candles: Vec<PlotCandle> = (0..100)
        .map(|i| {
            PlotCandle::new(
                i * 1_000_000_000, // 1 second intervals
                100.0 + i as f32,
                101.0 + i as f32,
                99.0 + i as f32,
                100.5 + i as f32,
                1000.0,
            )
        })
        .collect();

    // Test centered window
    let range = take_window(&candles, 50.0, 10);
    assert_eq!(range.len(), 10);

    // Test duration window (10 seconds)
    let range = take_window_duration(&candles, 50.0, Duration::from_secs(10));
    assert!(range.len() > 0);
    assert!(range.len() <= 11); // 5 before, 1 center, 5 after
}
