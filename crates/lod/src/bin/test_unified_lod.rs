//! Test program demonstrating unified trade and candle data in LOD system

use lod::levels::{LevelStore, PlotCandle, PlotData};
use lod::ntrd_decoder::{NtrdReader, TradeRecord};
use lod::traits::QuoteLike;
use std::collections::BTreeMap;

fn main() {
    println!("Testing Unified LOD System with Trades and Candles\n");

    // Load trade data
    let mut reader = NtrdReader::open(
        "/workspace/workspace/bb/beta_breaker/data/consolidated/hedgy-test/CIEN/2018-05-01_to_2025-09-18-trades.ntrd"
    ).unwrap();

    // Read first 5000 trades
    let trades = reader.read_records(0, 5000).unwrap();
    println!("Loaded {} trades", trades.len());

    // Create a LevelStore
    let mut store = LevelStore::new();

    // Add raw trades at interval 0
    let plot_trades: Vec<_> = trades
        .iter()
        .take(100) // Just first 100 for demo
        .map(|t| t.to_plot_trade())
        .collect();

    store.add_trades(plot_trades.clone());
    println!("Added {} raw trades to interval 0", plot_trades.len());

    // Aggregate trades into 1-second candles
    let mut one_sec_candles: BTreeMap<i64, Vec<&TradeRecord>> = BTreeMap::new();
    for trade in trades.iter() {
        let second = trade.timestamp() / 1_000_000_000;
        one_sec_candles
            .entry(second)
            .or_insert_with(Vec::new)
            .push(trade);
    }

    // Convert to PlotCandles
    let candles_1s: Vec<PlotCandle> = one_sec_candles
        .iter()
        .map(|(second, trades)| {
            let open = trades[0].open() as f32;
            let high = trades.iter().map(|t| t.high()).fold(0.0_f64, f64::max) as f32;
            let low = trades.iter().map(|t| t.low()).fold(f64::INFINITY, f64::min) as f32;
            let close = trades[trades.len() - 1].close() as f32;
            let volume: f32 = trades.iter().map(|t| t.volume()).sum::<f64>() as f32;

            PlotCandle::new(*second * 1_000_000_000, open, high, low, close, volume)
        })
        .collect();

    // Add 1-second candles to store
    store.append(1, &candles_1s, false);
    println!("Added {} 1-second candles", candles_1s.len());

    // Create 5-second candles from 1-second candles
    let mut five_sec_candles: BTreeMap<i64, Vec<&PlotCandle>> = BTreeMap::new();
    for candle in candles_1s.iter() {
        let five_sec = (candle.ts / 1_000_000_000 / 5) * 5;
        five_sec_candles
            .entry(five_sec)
            .or_insert_with(Vec::new)
            .push(candle);
    }

    let candles_5s: Vec<PlotCandle> = five_sec_candles
        .iter()
        .map(|(five_sec, candles)| {
            let open = candles[0].open;
            let high = candles.iter().map(|c| c.high).fold(0.0_f32, f32::max);
            let low = candles.iter().map(|c| c.low).fold(f32::INFINITY, f32::min);
            let close = candles[candles.len() - 1].close;
            let volume: f32 = candles.iter().map(|c| c.volume).sum();

            PlotCandle::new(*five_sec * 1_000_000_000, open, high, low, close, volume)
        })
        .collect();

    store.append(5, &candles_5s, false);
    println!("Added {} 5-second candles", candles_5s.len());

    // Display summary
    println!("\n=== Level Store Summary ===");
    println!("Total intervals: {:?}", store.all_intervals());
    println!("Total data points: {}", store.total_data_points());
    println!("Is interval 0 trade data? {}", store.is_trade_level(0));
    println!("Is interval 1 trade data? {}", store.is_trade_level(1));

    // Access unified data
    if let Some(raw_trades) = store.get_unified(0) {
        println!("\n=== Raw Trade Data (first 5) ===");
        for (i, data) in raw_trades.iter().take(5).enumerate() {
            if let Some(trade) = data.as_trade() {
                println!(
                    "Trade {}: ts={:.3}s price=${:.2} size={} side={}",
                    i,
                    trade.timestamp_secs(),
                    trade.price,
                    trade.size,
                    trade.side_str()
                );
            }
        }

        // Count buy vs sell trades
        let buy_count = raw_trades
            .iter()
            .filter(|d| d.as_trade().map(|t| t.is_buy()).unwrap_or(false))
            .count();
        let sell_count = raw_trades
            .iter()
            .filter(|d| d.as_trade().map(|t| t.is_sell()).unwrap_or(false))
            .count();
        println!(
            "\nBuy trades: {}, Sell trades: {}, Unknown: {}",
            buy_count,
            sell_count,
            raw_trades.len() - buy_count - sell_count
        );
    }

    // Access candle data through traditional API
    if let Some(candles) = store.get(1) {
        println!("\n=== 1-Second Candles (first 5) ===");
        for (i, candle) in candles.iter().take(5).enumerate() {
            println!(
                "Candle {}: ts={:.3}s O={:.2} H={:.2} L={:.2} C={:.2} V={:.0}",
                i,
                candle.timestamp_secs(),
                candle.open,
                candle.high,
                candle.low,
                candle.close,
                candle.volume
            );
        }
    }

    // Demonstrate converting candles to unified format
    let unified_candles: Vec<PlotData> = candles_5s
        .iter()
        .take(10)
        .map(|c| PlotData::Candle(*c))
        .collect();
    store.add_unified(60, unified_candles); // Add as 60-second data for demo

    if let Some(unified) = store.get_unified(60) {
        println!("\n=== Unified Format Demo (60s pseudo-level) ===");
        for data in unified.iter().take(3) {
            match data {
                PlotData::Candle(c) => {
                    println!(
                        "Candle at {:.3}s: OHLC=[{:.2},{:.2},{:.2},{:.2}]",
                        c.timestamp_secs(),
                        c.open,
                        c.high,
                        c.low,
                        c.close
                    );
                }
                PlotData::Trade(t) => {
                    println!(
                        "Trade at {:.3}s: price=${:.2} size={}",
                        t.timestamp_secs(),
                        t.price,
                        t.size
                    );
                }
            }
        }
    }

    println!("\n✅ Unified LOD system successfully handles both trades and candles!");
}
