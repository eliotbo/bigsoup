use lod::ntrd_decoder::{NtrdReader, TradeRecord};
use lod::traits::QuoteLike;
use std::collections::BTreeMap;

fn main() {
    let mut reader = NtrdReader::open(
        "/workspace/workspace/bb/beta_breaker/data/consolidated/hedgy-test/CIEN/2018-05-01_to_2025-09-18-trades.ntrd"
    ).unwrap();

    // Read first 1000 trades and aggregate into 1-second candles
    let records = reader.read_records(0, 1000).unwrap();

    // Group trades by second
    let mut candles: BTreeMap<i64, Vec<&TradeRecord>> = BTreeMap::new();
    for record in records.iter() {
        let second = record.timestamp() / 1_000_000_000;
        candles.entry(second).or_insert_with(Vec::new).push(record);
    }

    println!(
        "Aggregated {} trades into {} 1-second candles\n",
        records.len(),
        candles.len()
    );

    // Display first 5 candles
    for (second, trades) in candles.iter().take(5) {
        let open = trades[0].open();
        let high = trades.iter().map(|t| t.high()).fold(0.0_f64, f64::max);
        let low = trades.iter().map(|t| t.low()).fold(f64::INFINITY, f64::min);
        let close = trades[trades.len() - 1].close();
        let volume: f64 = trades.iter().map(|t| t.volume()).sum();
        let count: u32 = trades.len() as u32;

        let buy_volume: f64 = trades
            .iter()
            .filter(|t| t.is_buy())
            .map(|t| t.volume())
            .sum();
        let sell_volume: f64 = trades
            .iter()
            .filter(|t| t.is_sell())
            .map(|t| t.volume())
            .sum();

        println!(
            "Second {}: O={:.4} H={:.4} L={:.4} C={:.4} V={:.0} Count={}",
            second, open, high, low, close, volume, count
        );
        println!(
            "  Buy Volume: {:.0}, Sell Volume: {:.0}, Unknown: {:.0}",
            buy_volume,
            sell_volume,
            volume - buy_volume - sell_volume
        );

        // Show individual trades in first candle
        if second == candles.keys().next().unwrap() {
            println!("  Individual trades:");
            for trade in trades.iter().take(10) {
                println!(
                    "    Price={:.4} Size={} Side={} Aggressor={}",
                    trade.price_as_float(),
                    trade.size,
                    trade.side_char(),
                    trade.aggressor_side()
                );
            }
        }
    }
}
