use crate::config::Config;
use anyhow::Result;
use lod::{DividendEvent, LevelStore, load_nohlcv_level_store_with_days};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const DAYS_TO_LOAD: i64 = 1000;

/// Dividend event with precomputed bar indices for each LOD level
#[derive(Debug, Clone)]
pub struct DividendWithIndex {
    pub event: DividendEvent,
    /// Map from interval_secs to bar index in that LOD level
    pub indices: HashMap<u64, usize>,
}

/// Market data container with time range information
#[derive(Clone)]
pub struct MarketData {
    pub level_store: Arc<Mutex<LevelStore>>,
    pub start_ts: i64,
    pub end_ts: i64,
    pub min_interval_secs: u64,            // Finest granularity available
    pub dividends: Vec<DividendWithIndex>, // Dividend events with precomputed indices
}

impl MarketData {
    /// Get the time range as a tuple (start, end)
    pub fn time_range(&self) -> (i64, i64) {
        (self.start_ts, self.end_ts)
    }

    /// Construct MarketData from an in-memory LevelStore
    pub fn from_level_store(level_store: LevelStore) -> Self {
        let intervals = level_store.intervals();
        let min_interval_secs = intervals.iter().copied().min().unwrap_or(60);

        let mut first_ts_nanos: Option<i64> = None;
        let mut last_ts_nanos: Option<i64> = None;

        for interval in &intervals {
            if let Some(candles) = level_store.get(*interval) {
                if let (Some(first), Some(last)) = (candles.first(), candles.last()) {
                    first_ts_nanos = Some(match first_ts_nanos {
                        Some(existing) => existing.min(first.ts),
                        None => first.ts,
                    });

                    last_ts_nanos = Some(match last_ts_nanos {
                        Some(existing) => existing.max(last.ts),
                        None => last.ts,
                    });
                }
            }
        }

        let now_secs = chrono::Utc::now().timestamp();
        let (start_ts, end_ts) = match (first_ts_nanos, last_ts_nanos) {
            (Some(start_nanos), Some(end_nanos)) => {
                (start_nanos / 1_000_000_000, end_nanos / 1_000_000_000)
            }
            _ => (now_secs, now_secs),
        };

        MarketData {
            level_store: Arc::new(Mutex::new(level_store)),
            start_ts,
            end_ts,
            min_interval_secs,
            dividends: Vec::new(),
        }
    }
}

/// Load market data from disk with multiple time intervals
pub fn load_market_data() -> Result<MarketData> {
    load_market_data_with_config(&Config::default())
}

/// Load market data from disk with a specific config
pub fn load_market_data_with_config(config: &Config) -> Result<MarketData> {
    // Check if we have any data paths
    if config.data_paths.is_empty() {
        println!("No historical data paths provided, creating empty market data");

        // Create an empty LevelStore
        let level_store = Arc::new(Mutex::new(LevelStore::new()));

        // Use current time as time range
        let now = chrono::Utc::now().timestamp();

        return Ok(MarketData {
            level_store,
            start_ts: now,
            end_ts: now,
            min_interval_secs: 60,
            dividends: Vec::new(),
        });
    }

    // Use the first data path for now (all paths currently point to same data)
    let data_path = &config.data_paths[0];

    println!(
        "Loading last {} days of market data from {}...",
        DAYS_TO_LOAD, data_path
    );

    let level_store = if config.allow_missing_history {
        // Try to load, but if it fails, create an empty LevelStore
        match load_nohlcv_level_store_with_days(
            data_path,
            60, // base_interval_secs (1 minute)
            &[60, 300, 900, 1800, 3600, 14400, 86400, 604800, 2629746], // M1, M5, M15, M30, H1, H4, D1, W1, Month1
            Some(DAYS_TO_LOAD),
        ) {
            Ok(store) => {
                println!("Historical data loaded successfully");
                store
            }
            Err(e) => {
                println!(
                    "No historical data found ({}), starting with empty store",
                    e
                );
                LevelStore::new()
            }
        }
    } else {
        // Original behavior: fail if data cannot be loaded
        load_nohlcv_level_store_with_days(
            data_path,
            60, // base_interval_secs (1 minute)
            &[60, 300, 900, 1800, 3600, 14400, 86400, 604800, 2629746], // M1, M5, M15, M30, H1, H4, D1, W1, Month1
            Some(DAYS_TO_LOAD),
        )
        .map_err(|e| anyhow::anyhow!("Failed to load data: {}", e))?
    };

    // Auto-discover and load dividends from the same directory
    // We need to extract the ticker from the data path
    // For now, we'll use "FAST" since that's the default config
    // In the future, we could parse it from the file header
    let raw_dividends = lod::discover_and_load_dividends(
        std::path::Path::new(data_path),
        "FAST", // TODO: Get ticker from file header or config
        true,
    );

    // Precompute dividend indices for each LOD level
    let intervals = [60u64, 300, 900, 1800, 3600, 14400, 86400, 604800, 2629746];
    let mut dividends = Vec::new();

    for dividend in raw_dividends {
        let mut indices = HashMap::new();

        // Convert ex_date to timestamp in nanoseconds
        let ex_timestamp_nanos = dividend
            .ex_date
            .and_hms_opt(9, 30, 0) // Market open time
            .and_then(|dt| dt.and_utc().timestamp_nanos_opt())
            .unwrap_or(0);

        // For each LOD level, find the bar index using binary search
        for &interval_secs in &intervals {
            if let Some(candles) = level_store.get(interval_secs) {
                if !candles.is_empty() {
                    // Check if dividend is within data range
                    let first_candle_ts = candles[0].ts;
                    let last_candle_ts = candles[candles.len() - 1].ts;

                    // Only process dividends that fall within or after the data range
                    if ex_timestamp_nanos >= first_candle_ts && ex_timestamp_nanos <= last_candle_ts
                    {
                        let idx = match candles.binary_search_by_key(&ex_timestamp_nanos, |c| c.ts)
                        {
                            Ok(i) => i,
                            Err(i) => i.saturating_sub(1),
                        };

                        // Only store if index is valid
                        if idx < candles.len() {
                            indices.insert(interval_secs, idx);
                        }
                    }
                }
            }
        }

        dividends.push(DividendWithIndex {
            event: dividend,
            indices,
        });
    }

    println!(
        "Loaded {} dividends with precomputed indices",
        dividends.len()
    );

    // Collect metadata before moving level_store into Arc<Mutex<_>>
    let total_candles = level_store.total_candles();
    let interval_count = level_store.intervals().len();

    // Get data time range from the base 60s interval
    let (start_ts, end_ts) = if let Some(base_data) = level_store.get(60) {
        if let (Some(first), Some(last)) = (base_data.first(), base_data.last()) {
            let start_ts_nanos = first.ts;
            let end_ts_nanos = last.ts;

            // Convert nanoseconds to seconds for calendar map
            let start_ts = start_ts_nanos / 1_000_000_000;
            let end_ts = end_ts_nanos / 1_000_000_000;

            println!(
                "Loaded {} candles across {} intervals",
                total_candles, interval_count
            );
            println!("Data range: {} to {} (UTC seconds)", start_ts, end_ts);

            (start_ts, end_ts)
        } else {
            // Empty data - use current time
            println!("No historical data available, using current time as base");
            let now = chrono::Utc::now().timestamp();
            (now, now)
        }
    } else if config.allow_missing_history {
        // No 60s interval data and we allow missing history
        println!("No historical data available, using current time as base");
        let now = chrono::Utc::now().timestamp();
        (now, now)
    } else {
        // No 60s interval data and we don't allow missing history
        return Err(anyhow::anyhow!("No 60s interval data found"));
    };

    let level_store = Arc::new(Mutex::new(level_store));

    Ok(MarketData {
        level_store,
        start_ts,
        end_ts,
        min_interval_secs: 60, // 1 minute is the finest granularity
        dividends,
    })
}
